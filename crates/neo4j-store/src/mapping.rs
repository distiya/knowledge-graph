use std::collections::HashMap;

use ckg_domain::{
    CanonicalId, GraphEdge, GraphNode, NodeKind, Provenance, RelationKind, SourceLocation,
};
use ckg_graph_delta::{node_kind_str, relation_kind_str};
use neo4j::ValueReceive;

use crate::error::StoreError;

pub type Params = HashMap<String, neo4j::ValueSend>;

pub fn parse_node_kind(raw: &str) -> Result<NodeKind, StoreError> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned()))
        .map_err(|e| StoreError::Data(format!("unknown node kind {raw:?}: {e}")))
}

pub fn parse_relation_kind(raw: &str) -> Result<RelationKind, StoreError> {
    serde_json::from_value(serde_json::Value::String(raw.to_owned()))
        .map_err(|e| StoreError::Data(format!("unknown relation kind {raw:?}: {e}")))
}

/// Validate a relation kind for safe interpolation into Cypher relationship patterns.
pub fn relation_type_token(kind: RelationKind) -> Result<String, StoreError> {
    let name = relation_kind_str(kind);
    let mut chars = name.chars();
    let valid = !name.is_empty()
        && chars
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(StoreError::Data(format!(
            "relation kind is not a valid cypher type token: {name:?}"
        )));
    }
    Ok(name)
}

pub fn node_kind_token(kind: NodeKind) -> String {
    node_kind_str(kind)
}

pub fn node_params(node: &GraphNode) -> Result<Params, StoreError> {
    let mut props: Params = Params::new();
    props.insert("id".into(), node.id.as_str().to_owned().into());
    props.insert("kind".into(), node_kind_token(node.kind).into());
    props.insert("name".into(), node.name.clone().into());
    props.insert("qualified_name".into(), node.qualified_name.clone().into());
    if let Some(language) = &node.language {
        props.insert("language".into(), language.clone().into());
    }
    if let Some(hash) = &node.content_hash {
        props.insert("content_hash".into(), hash.clone().into());
    }
    if let Some(location) = &node.location {
        props.insert(
            "location_json".into(),
            serde_json::to_string(location)?.into(),
        );
    }
    props.insert(
        "properties_json".into(),
        serde_json::to_string(&node.properties)?.into(),
    );
    props.insert(
        "provenance_json".into(),
        serde_json::to_string(&node.provenance)?.into(),
    );

    let mut params = Params::new();
    params.insert("id".into(), node.id.as_str().to_owned().into());
    params.insert("props".into(), neo4j::ValueSend::Map(props));
    Ok(params)
}

pub fn id_param(id: &CanonicalId) -> Params {
    let mut params = Params::new();
    params.insert("id".into(), id.as_str().to_owned().into());
    params
}

pub fn edge_params(edge: &GraphEdge) -> Result<Params, StoreError> {
    let mut params = Params::new();
    params.insert("from".into(), edge.from.as_str().to_owned().into());
    params.insert("to".into(), edge.to.as_str().to_owned().into());
    params.insert(
        "content_hash".into(),
        edge.content_hash
            .clone()
            .map(neo4j::ValueSend::String)
            .unwrap_or(neo4j::ValueSend::Null),
    );
    params.insert(
        "properties_json".into(),
        serde_json::to_string(&edge.properties)?.into(),
    );
    params.insert(
        "provenance_json".into(),
        serde_json::to_string(&edge.provenance)?.into(),
    );
    Ok(params)
}

pub fn endpoints_params(from: &CanonicalId, to: &CanonicalId) -> Params {
    let mut params = Params::new();
    params.insert("from".into(), from.as_str().to_owned().into());
    params.insert("to".into(), to.as_str().to_owned().into());
    params
}

fn prop_string<'a>(props: &'a HashMap<String, ValueReceive>, key: &str) -> Option<&'a String> {
    props.get(key).and_then(|v| v.as_string())
}

fn required_string(props: &HashMap<String, ValueReceive>, key: &str) -> Result<String, StoreError> {
    prop_string(props, key)
        .cloned()
        .ok_or_else(|| StoreError::Data(format!("node property {key:?} missing")))
}

fn parse_json<T: serde::de::DeserializeOwned>(
    raw: Option<&String>,
    what: &str,
) -> Result<Option<T>, StoreError> {
    raw.map(|s| serde_json::from_str(s.as_str()))
        .transpose()
        .map_err(|e| StoreError::Data(format!("invalid {what} json: {e}")))
}

pub fn node_from_props(props: &HashMap<String, ValueReceive>) -> Result<GraphNode, StoreError> {
    let id = CanonicalId::new(required_string(props, "id")?);
    let kind_raw = required_string(props, "kind")?;
    let kind = parse_node_kind(&kind_raw)?;
    let name = required_string(props, "name")?;
    let qualified_name = required_string(props, "qualified_name")?;
    let language = prop_string(props, "language").cloned();
    let content_hash = prop_string(props, "content_hash").cloned();
    let location: Option<SourceLocation> =
        parse_json(prop_string(props, "location_json"), "location")?;
    let properties: serde_json::Map<String, serde_json::Value> =
        parse_json(prop_string(props, "properties_json"), "properties")?.unwrap_or_default();
    let provenance: Vec<Provenance> =
        parse_json(prop_string(props, "provenance_json"), "provenance")?.unwrap_or_default();

    Ok(GraphNode {
        id,
        kind,
        name,
        qualified_name,
        language,
        location,
        content_hash,
        properties,
        provenance,
    })
}

pub fn node_from_value(value: &ValueReceive) -> Result<GraphNode, StoreError> {
    match value {
        ValueReceive::Node(node) => node_from_props(&node.properties),
        other => Err(StoreError::Data(format!(
            "expected node value, got {other:?}"
        ))),
    }
}

pub fn edge_from_fields(
    kind: &str,
    from: &str,
    to: &str,
    content_hash: Option<&String>,
    properties_json: Option<&String>,
    provenance_json: Option<&String>,
) -> Result<GraphEdge, StoreError> {
    let relation = parse_relation_kind(kind)?;
    let properties: serde_json::Map<String, serde_json::Value> = properties_json
        .map(|s| serde_json::from_str(s.as_str()))
        .transpose()
        .map_err(|e| StoreError::Data(format!("invalid edge properties json: {e}")))?
        .unwrap_or_default();
    let provenance: Vec<Provenance> = provenance_json
        .map(|s| serde_json::from_str(s.as_str()))
        .transpose()
        .map_err(|e| StoreError::Data(format!("invalid edge provenance json: {e}")))?
        .unwrap_or_default();

    Ok(GraphEdge {
        kind: relation,
        from: CanonicalId::new(from),
        to: CanonicalId::new(to),
        content_hash: content_hash.cloned(),
        properties,
        provenance,
    })
}
