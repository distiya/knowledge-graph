use std::sync::Arc;

use async_trait::async_trait;
use ckg_domain::{CanonicalId, DEPENDENCY_KINDS, GraphEdge, GraphNode, NodeKind, RelationKind};
use ckg_graph_delta::{DeltaOp, GraphDelta};
use neo4j::address::Address;
use neo4j::driver::auth::AuthToken;
use neo4j::driver::{ConnectionConfig, Driver, DriverConfig, EagerResult};
use neo4j::session::SessionConfig;
use neo4j::transaction::Transaction;
use neo4j::{ValueReceive, value_map};

use crate::analytics::{
    AnalyticsEngine, AnalyticsMode, AnalyticsReport, AnalyticsRow, AnalyticsSpec,
};
use crate::error::StoreError;
use crate::mapping::{
    self, Params, edge_from_fields, edge_params, endpoints_params, id_param, node_from_value,
    node_kind_token, node_params, parse_node_kind, relation_type_token,
};

const SCHEMA_CYPHER: &str =
    "CREATE CONSTRAINT node_id_unique IF NOT EXISTS FOR (n:Node) REQUIRE n.id IS UNIQUE";
const UPSERT_NODE_CYPHER: &str = "MERGE (n:Node {id: $id}) SET n = $props";
const DELETE_NODE_CYPHER: &str = "MATCH (n:Node {id: $id}) \
     OPTIONAL MATCH (c:Node)-[:CONTAINS_STATE_OF]->(n) \
     WITH n, count(c) AS state_count \
     FOREACH (_ IN CASE WHEN state_count = 0 THEN [1] ELSE [] END | DETACH DELETE n) \
     RETURN state_count";
const GET_NODE_CYPHER: &str = "MATCH (n:Node {id: $id}) RETURN n";
const FIND_BY_NAME_CYPHER: &str =
    "MATCH (n:Node) WHERE toLower(n.name) CONTAINS toLower($name) RETURN n LIMIT $limit";
const GET_EDGES_FROM_CYPHER: &str = "MATCH (a:Node {id: $id})-[r]->(b:Node) \
     WHERE $kind IS NULL OR type(r) = $kind \
     RETURN type(r) AS kind, a.id AS from_id, b.id AS to_id, \
             r.content_hash AS content_hash, r.properties_json AS properties_json, \
             r.provenance_json AS provenance_json \
     LIMIT $limit";
const GET_EDGES_TO_CYPHER: &str = "MATCH (a:Node)-[r]->(b:Node {id: $id}) \
     WHERE $kind IS NULL OR type(r) = $kind \
     RETURN type(r) AS kind, a.id AS from_id, b.id AS to_id, \
             r.content_hash AS content_hash, r.properties_json AS properties_json, \
             r.provenance_json AS provenance_json \
     LIMIT $limit";
const RESET_CYPHER: &str = "MATCH (n) DETACH DELETE n";

const MAX_ANALYTICS_DEPTH: u8 = 10;
const MAX_ANALYTICS_LIMIT: u32 = 500;

/// Outcome counters returned by applying a delta.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub created_nodes: u64,
    pub updated_nodes: u64,
    pub deleted_nodes: u64,
    pub created_rels: u64,
    pub deleted_rels: u64,
    /// DeleteNode ops skipped because other commits still reference the node.
    pub nodes_retained: u64,
    pub errors: Vec<String>,
}

impl ApplyReport {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Storage backend for the enterprise code knowledge graph.
#[async_trait]
pub trait GraphStore: Send + Sync {
    async fn apply_delta(&self, delta: &GraphDelta) -> Result<ApplyReport, StoreError>;
    async fn get_node(&self, id: &CanonicalId) -> Result<Option<GraphNode>, StoreError>;
    async fn find_nodes_by_name(
        &self,
        name: &str,
        limit: u32,
    ) -> Result<Vec<GraphNode>, StoreError>;
    async fn get_edges_from(
        &self,
        id: &CanonicalId,
        kind: Option<RelationKind>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>, StoreError>;
    async fn get_edges_to(
        &self,
        id: &CanonicalId,
        kind: Option<RelationKind>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>, StoreError>;
    /// Drop all nodes and relationships (test helper).
    async fn reset(&self) -> Result<(), StoreError>;
    async fn health(&self) -> Result<(), StoreError>;

    /// Run a bounded analytics query. Stores without an analytics backend
    /// return [`StoreError::Unsupported`].
    async fn run_analytics(&self, spec: &AnalyticsSpec) -> Result<AnalyticsReport, StoreError> {
        let _ = spec;
        Err(StoreError::Unsupported("run_analytics".into()))
    }
}

enum OpKind {
    CreateNode,
    UpdateNode,
    DeleteNode,
    CreateRelationship,
    UpdateRelationship,
    DeleteRelationship,
}

struct PreparedOp {
    kind: OpKind,
    cypher: String,
    params: Params,
}

struct QueryOutcome {
    row_present: bool,
    nodes_created: i64,
    nodes_deleted: i64,
    relationships_created: i64,
    relationships_deleted: i64,
}

pub struct Neo4jStore {
    driver: Driver,
    database: Arc<String>,
}

impl Neo4jStore {
    pub fn connect(
        uri: &str,
        user: &str,
        password: &str,
        database: &str,
    ) -> Result<Self, StoreError> {
        let (address, routing) = parse_uri(uri)?;
        let auth = AuthToken::new_basic_auth(user, password);
        let connection = ConnectionConfig::new(address).with_routing(routing);
        let config = DriverConfig::new().with_auth(Arc::new(auth));
        let driver = Driver::new(connection, config);
        let store = Self {
            driver,
            database: Arc::new(database.to_owned()),
        };
        store.driver.execute_query(SCHEMA_CYPHER).run()?;
        Ok(store)
    }

    fn database(&self) -> Arc<String> {
        Arc::clone(&self.database)
    }

    fn execute(&self, cypher: &str, params: Params) -> Result<EagerResult, StoreError> {
        Ok(self
            .driver
            .execute_query(cypher)
            .with_database(self.database())
            .with_parameters(params)
            .run()?)
    }

    fn prepare(delta: &GraphDelta) -> Result<Vec<PreparedOp>, StoreError> {
        let mut prepared = Vec::with_capacity(delta.ops.len());
        for op in &delta.ops {
            prepared.push(match op {
                DeltaOp::CreateNode(node) => PreparedOp {
                    kind: OpKind::CreateNode,
                    cypher: UPSERT_NODE_CYPHER.to_owned(),
                    params: node_params(node)?,
                },
                DeltaOp::UpdateNode(node) => PreparedOp {
                    kind: OpKind::UpdateNode,
                    cypher: UPSERT_NODE_CYPHER.to_owned(),
                    params: node_params(node)?,
                },
                DeltaOp::DeleteNode(id) => PreparedOp {
                    kind: OpKind::DeleteNode,
                    cypher: DELETE_NODE_CYPHER.to_owned(),
                    params: id_param(id),
                },
                DeltaOp::CreateRelationship(edge) => PreparedOp {
                    kind: OpKind::CreateRelationship,
                    cypher: upsert_rel_cypher(edge.kind)?,
                    params: edge_params(edge)?,
                },
                DeltaOp::UpdateRelationship(edge) => PreparedOp {
                    kind: OpKind::UpdateRelationship,
                    cypher: upsert_rel_cypher(edge.kind)?,
                    params: edge_params(edge)?,
                },
                DeltaOp::DeleteRelationship { kind, from, to } => PreparedOp {
                    kind: OpKind::DeleteRelationship,
                    cypher: delete_rel_cypher(*kind)?,
                    params: endpoints_params(from, to),
                },
            });
        }
        Ok(prepared)
    }
}

fn upsert_rel_cypher(kind: RelationKind) -> Result<String, StoreError> {
    let token = relation_type_token(kind)?;
    Ok(format!(
        "MATCH (a:Node {{id: $from}}), (b:Node {{id: $to}}) \
         MERGE (a)-[r:{token}]->(b) \
         SET r.content_hash = $content_hash, r.properties_json = $properties_json, \
             r.provenance_json = $provenance_json \
         RETURN true AS matched"
    ))
}

fn delete_rel_cypher(kind: RelationKind) -> Result<String, StoreError> {
    let token = relation_type_token(kind)?;
    Ok(format!(
        "MATCH (a:Node {{id: $from}})-[r:{token}]->(b:Node {{id: $to}}) DELETE r"
    ))
}

/// `:A|B|C` alternation over every dependency relation kind. Tokens are
/// validated by [`relation_type_token`], so the result is safe to
/// interpolate into a relationship pattern.
fn dep_type_alternation() -> Result<String, StoreError> {
    let mut out = String::new();
    for kind in DEPENDENCY_KINDS {
        if !out.is_empty() {
            out.push('|');
        }
        out.push_str(&relation_type_token(kind)?);
    }
    Ok(out)
}

/// APOC relationship filter traversing every dependency kind inbound
/// (`<A|<B|...`): nodes pointing at the start node.
fn dep_inbound_filter() -> Result<String, StoreError> {
    let mut out = String::new();
    for kind in DEPENDENCY_KINDS {
        if !out.is_empty() {
            out.push('|');
        }
        out.push('<');
        out.push_str(&relation_type_token(kind)?);
    }
    Ok(out)
}

/// `['A','B',...]` literal over every dependency kind (validated tokens).
fn dep_type_list_literal() -> Result<String, StoreError> {
    let mut out = String::new();
    for kind in DEPENDENCY_KINDS {
        if !out.is_empty() {
            out.push(',');
        }
        out.push('\'');
        out.push_str(&relation_type_token(kind)?);
        out.push('\'');
    }
    Ok(out)
}

/// Node and relationship Cypher projections for anonymous GDS graphs:
/// all nodes, only dependency-typed relationships. The relationship query
/// is embedded inside a single-quoted Cypher string, so its type list uses
/// double-quoted literals.
fn gds_projection_queries() -> Result<(String, String), StoreError> {
    let mut types = String::new();
    for kind in DEPENDENCY_KINDS {
        if !types.is_empty() {
            types.push(',');
        }
        types.push('"');
        types.push_str(&relation_type_token(kind)?);
        types.push('"');
    }
    Ok((
        "MATCH (n:Node) RETURN id(n) AS id".to_owned(),
        format!(
            "MATCH (a:Node)-[r]->(b:Node) WHERE type(r) IN [{types}] \
             RETURN id(a) AS source, id(b) AS target"
        ),
    ))
}

/// Unique per-call GDS projection name so concurrent analytics calls never
/// collide on the graph catalog.
fn analytics_graph_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("ckg_analytics_{nanos}")
}

fn record_number(record: &neo4j::driver::Record, key: &str) -> f64 {
    match record.value(key) {
        Some(ValueReceive::Float(value)) => *value,
        Some(ValueReceive::Integer(value)) => *value as f64,
        _ => 0.0,
    }
}

fn analytics_row(record: &neo4j::driver::Record) -> Result<AnalyticsRow, StoreError> {
    let id = record
        .value("id")
        .and_then(ValueReceive::as_string)
        .ok_or_else(|| StoreError::Data("analytics row missing id".into()))?;
    let name = record
        .value("name")
        .and_then(ValueReceive::as_string)
        .cloned()
        .unwrap_or_default();
    let kind_raw = record
        .value("kind")
        .and_then(ValueReceive::as_string)
        .ok_or_else(|| StoreError::Data("analytics row missing kind".into()))?;
    Ok(AnalyticsRow {
        id: CanonicalId::new(id.clone()),
        name,
        kind: parse_node_kind(&kind_raw)?,
        score: record_number(record, "score"),
        distance: record
            .value("distance")
            .and_then(ValueReceive::as_int)
            .and_then(|d| u8::try_from(d).ok()),
        component: record.value("component").and_then(ValueReceive::as_int),
    })
}

/// Apply the caller-visible limit; queries fetch `limit + 1` rows so the
/// extra row only signals truncation.
fn truncate_rows(mut rows: Vec<AnalyticsRow>, limit: u32) -> (Vec<AnalyticsRow>, bool) {
    let truncated = rows.len() as u32 > limit;
    if truncated {
        rows.truncate(limit as usize);
    }
    (rows, truncated)
}

fn run_write(tx: &Transaction, cypher: &str, params: &Params) -> neo4j::Result<QueryOutcome> {
    let mut stream = tx.query(cypher).with_parameters(params).run()?;
    let row_present = match stream.next() {
        Some(record) => {
            record?;
            true
        }
        None => false,
    };
    let counters = stream
        .consume()?
        .map(|summary| summary.counters)
        .unwrap_or_default();
    Ok(QueryOutcome {
        row_present,
        nodes_created: counters.nodes_created,
        nodes_deleted: counters.nodes_deleted,
        relationships_created: counters.relationships_created,
        relationships_deleted: counters.relationships_deleted,
    })
}

fn record_opt_string(record: &neo4j::driver::Record, key: &str) -> Option<String> {
    record.value(key).and_then(ValueReceive::as_string).cloned()
}

fn edge_from_record(record: &neo4j::driver::Record) -> Result<GraphEdge, StoreError> {
    let kind = record
        .value("kind")
        .and_then(ValueReceive::as_string)
        .ok_or_else(|| StoreError::Data("edge record missing kind".into()))?;
    let from = record
        .value("from_id")
        .and_then(ValueReceive::as_string)
        .ok_or_else(|| StoreError::Data("edge record missing from_id".into()))?;
    let to = record
        .value("to_id")
        .and_then(ValueReceive::as_string)
        .ok_or_else(|| StoreError::Data("edge record missing to_id".into()))?;
    edge_from_fields(
        kind,
        from,
        to,
        record_opt_string(record, "content_hash").as_ref(),
        record_opt_string(record, "properties_json").as_ref(),
        record_opt_string(record, "provenance_json").as_ref(),
    )
}

fn parse_uri(uri: &str) -> Result<(Address, bool), StoreError> {
    let trimmed = uri.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(StoreError::InvalidUri("empty uri".into()));
    }
    let (scheme, rest) = match trimmed.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, trimmed),
    };
    let host_port = rest.split('/').next().unwrap_or(rest);
    if host_port.is_empty() {
        return Err(StoreError::InvalidUri(format!("no host in {uri:?}")));
    }
    let routing = match scheme {
        None | Some("bolt") => false,
        Some("neo4j") => true,
        Some(other) => {
            return Err(StoreError::InvalidUri(format!(
                "unsupported scheme {other:?} (use bolt:// or neo4j://)"
            )));
        }
    };
    Ok((Address::from(host_port), routing))
}

#[async_trait]
impl GraphStore for Neo4jStore {
    async fn apply_delta(&self, delta: &GraphDelta) -> Result<ApplyReport, StoreError> {
        if delta.is_empty() {
            return Ok(ApplyReport::default());
        }
        let prepared = Self::prepare(delta)?;
        let mut session = self
            .driver
            .session(SessionConfig::new().with_database(self.database()));

        let result = session.transaction().run(|tx| {
            let mut report = ApplyReport::default();
            for op in &prepared {
                let outcome = run_write(&tx, &op.cypher, &op.params)?;
                match op.kind {
                    OpKind::CreateNode => {
                        if outcome.nodes_created > 0 {
                            report.created_nodes += outcome.nodes_created as u64;
                        } else {
                            report.updated_nodes += 1;
                        }
                    }
                    OpKind::UpdateNode => {
                        report.updated_nodes += 1;
                    }
                    OpKind::DeleteNode => {
                        if outcome.nodes_deleted > 0 {
                            report.deleted_nodes += outcome.nodes_deleted as u64;
                        } else if outcome.row_present {
                            report.nodes_retained += 1;
                        }
                    }
                    OpKind::CreateRelationship => {
                        if !outcome.row_present {
                            report
                                .errors
                                .push("create relationship: endpoint node not found".into());
                        } else if outcome.relationships_created > 0 {
                            report.created_rels += outcome.relationships_created as u64;
                        }
                    }
                    OpKind::UpdateRelationship => {
                        if !outcome.row_present {
                            report
                                .errors
                                .push("update relationship: endpoint node not found".into());
                        }
                    }
                    OpKind::DeleteRelationship => {
                        report.deleted_rels += outcome.relationships_deleted as u64;
                    }
                }
            }
            tx.commit()?;
            Ok(report)
        });

        match result {
            Ok(report) => Ok(report),
            Err(err) => Ok(ApplyReport {
                errors: vec![err.to_string()],
                ..ApplyReport::default()
            }),
        }
    }

    async fn get_node(&self, id: &CanonicalId) -> Result<Option<GraphNode>, StoreError> {
        let result = self.execute(GET_NODE_CYPHER, id_param(id))?;
        for record in &result.records {
            if let Some(value) = record.value("n") {
                return node_from_value(value).map(Some);
            }
        }
        Ok(None)
    }

    async fn find_nodes_by_name(
        &self,
        name: &str,
        limit: u32,
    ) -> Result<Vec<GraphNode>, StoreError> {
        let params = value_map!({ "name": name, "limit": limit });
        let result = self.execute(FIND_BY_NAME_CYPHER, params)?;
        result
            .records
            .iter()
            .filter_map(|record| record.value("n"))
            .map(node_from_value)
            .collect()
    }

    async fn get_edges_from(
        &self,
        id: &CanonicalId,
        kind: Option<RelationKind>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>, StoreError> {
        self.get_edges(GET_EDGES_FROM_CYPHER, id, kind, limit).await
    }

    async fn get_edges_to(
        &self,
        id: &CanonicalId,
        kind: Option<RelationKind>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>, StoreError> {
        self.get_edges(GET_EDGES_TO_CYPHER, id, kind, limit).await
    }

    async fn reset(&self) -> Result<(), StoreError> {
        self.execute(RESET_CYPHER, Params::new())?;
        Ok(())
    }

    async fn health(&self) -> Result<(), StoreError> {
        self.driver.verify_connectivity()?;
        Ok(())
    }

    async fn run_analytics(&self, spec: &AnalyticsSpec) -> Result<AnalyticsReport, StoreError> {
        let limit = spec.limit.clamp(1, MAX_ANALYTICS_LIMIT);
        match spec.mode {
            AnalyticsMode::BlastRadius => self.blast_radius(spec, limit).await,
            AnalyticsMode::CriticalResources => self.critical_resources(limit).await,
            AnalyticsMode::Bridges => self.bridges(limit).await,
            AnalyticsMode::Clusters => self.clusters(limit).await,
        }
    }
}

impl Neo4jStore {
    async fn get_edges(
        &self,
        cypher: &str,
        id: &CanonicalId,
        kind: Option<RelationKind>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>, StoreError> {
        let mut params = id_param(id);
        match kind {
            Some(kind) => {
                params.insert("kind".into(), mapping::relation_type_token(kind)?.into());
            }
            None => {
                params.insert("kind".into(), neo4j::ValueSend::Null);
            }
        }
        params.insert("limit".into(), limit.into());
        let result = self.execute(cypher, params)?;
        result.records.iter().map(edge_from_record).collect()
    }

    async fn procedure_exists(&self, name: &str) -> bool {
        let safe = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !safe {
            return false;
        }
        let cypher = format!(
            "SHOW PROCEDURES YIELD name WHERE name = '{name}' RETURN count(*) AS available"
        );
        match self.execute(&cypher, Params::new()) {
            Ok(result) => {
                result
                    .records
                    .first()
                    .and_then(|record| record.value("available"))
                    .and_then(ValueReceive::as_int)
                    .unwrap_or(0)
                    > 0
            }
            Err(_) => false,
        }
    }

    async fn query_rows(
        &self,
        cypher: &str,
        params: Params,
    ) -> Result<Vec<AnalyticsRow>, StoreError> {
        let result = self.execute(cypher, params)?;
        result.records.iter().map(analytics_row).collect()
    }

    async fn blast_radius(
        &self,
        spec: &AnalyticsSpec,
        limit: u32,
    ) -> Result<AnalyticsReport, StoreError> {
        let mode = AnalyticsMode::BlastRadius;
        let id = spec
            .id
            .clone()
            .ok_or_else(|| StoreError::Data("blast_radius requires a node id".into()))?;
        if spec.depth == 0 {
            return Ok(AnalyticsReport {
                mode,
                engine: AnalyticsEngine::Cypher,
                rows: Vec::new(),
                truncated: false,
                degraded: false,
                note: None,
            });
        }
        let depth = spec.depth.min(MAX_ANALYTICS_DEPTH);
        let tokens = dep_type_alternation()?;
        let note = if self.procedure_exists("apoc.path.subgraphAll").await {
            match self.blast_radius_apoc(&id, depth, limit, &tokens).await {
                Ok((rows, truncated)) => {
                    return Ok(AnalyticsReport {
                        mode,
                        engine: AnalyticsEngine::Apoc,
                        rows,
                        truncated,
                        degraded: false,
                        note: None,
                    });
                }
                Err(err) => Some(format!(
                    "apoc.path.subgraphAll failed: {err}; fell back to Cypher traversal"
                )),
            }
        } else {
            Some("apoc.path.subgraphAll unavailable; used Cypher traversal".into())
        };
        let (rows, truncated) = self.blast_radius_cypher(&id, depth, limit, &tokens).await?;
        Ok(AnalyticsReport {
            mode,
            engine: AnalyticsEngine::Cypher,
            rows,
            truncated,
            degraded: true,
            note,
        })
    }

    async fn blast_radius_cypher(
        &self,
        id: &CanonicalId,
        depth: u8,
        limit: u32,
        tokens: &str,
    ) -> Result<(Vec<AnalyticsRow>, bool), StoreError> {
        let cypher = format!(
            "MATCH (n:Node {{id: $id}}) \
             MATCH p = (n)<-[:{tokens}*1..{depth}]-(m:Node) \
             WITH m, min(size(nodes(p)) - 1) AS distance \
             RETURN m.id AS id, m.name AS name, m.kind AS kind, distance \
             ORDER BY distance, m.id \
             LIMIT $limit"
        );
        let fetch = limit + 1;
        let params = value_map!({ "id": id.as_str(), "limit": fetch });
        let rows = self.query_rows(&cypher, params).await?;
        Ok(truncate_rows(rows, limit))
    }

    async fn blast_radius_apoc(
        &self,
        id: &CanonicalId,
        depth: u8,
        limit: u32,
        tokens: &str,
    ) -> Result<(Vec<AnalyticsRow>, bool), StoreError> {
        let filter = dep_inbound_filter()?;
        let cypher = format!(
            "MATCH (n:Node {{id: $id}}) \
             CALL apoc.path.subgraphAll(n, {{relationshipFilter: $filter, minLevel: 1, maxLevel: $depth}}) \
             YIELD nodes \
             WITH n, [m IN nodes WHERE m <> n] AS affected \
             UNWIND affected AS m \
             OPTIONAL MATCH p = (n)<-[:{tokens}*1..{depth}]-(m) \
             WITH m, min(size(nodes(p)) - 1) AS distance \
             RETURN m.id AS id, m.name AS name, m.kind AS kind, distance \
             ORDER BY distance, m.id \
             LIMIT $limit"
        );
        let fetch = limit + 1;
        let params = value_map!({
            "id": id.as_str(),
            "filter": filter,
            "depth": depth,
            "limit": fetch
        });
        let rows = self.query_rows(&cypher, params).await?;
        Ok(truncate_rows(rows, limit))
    }

    async fn critical_resources(&self, limit: u32) -> Result<AnalyticsReport, StoreError> {
        let mode = AnalyticsMode::CriticalResources;
        let note = if self.procedure_exists("gds.pageRank.stream").await {
            match self.critical_resources_gds(limit).await {
                Ok((rows, truncated)) => {
                    return Ok(AnalyticsReport {
                        mode,
                        engine: AnalyticsEngine::Gds,
                        rows,
                        truncated,
                        degraded: false,
                        note: None,
                    });
                }
                Err(err) => Some(format!(
                    "gds.pageRank failed: {err}; fell back to dependency in-degree ranking"
                )),
            }
        } else {
            Some("gds.pageRank unavailable; used dependency in-degree ranking".into())
        };
        let (rows, truncated) = self.critical_resources_degree(limit).await?;
        Ok(AnalyticsReport {
            mode,
            engine: AnalyticsEngine::Cypher,
            rows,
            truncated,
            degraded: true,
            note,
        })
    }

    async fn critical_resources_gds(
        &self,
        limit: u32,
    ) -> Result<(Vec<AnalyticsRow>, bool), StoreError> {
        let name = analytics_graph_name();
        self.project_analytics_graph(&name)?;
        let cypher = format!(
            "CALL gds.pageRank.stream('{name}') \
             YIELD nodeId, score \
             MATCH (n:Node) WHERE id(n) = nodeId AND n.kind = $kind \
             RETURN n.id AS id, n.name AS name, n.kind AS kind, score \
             ORDER BY score DESC, n.id \
             LIMIT $limit"
        );
        let fetch = limit + 1;
        let params = value_map!({ "kind": node_kind_token(NodeKind::Resource), "limit": fetch });
        let rows = self.query_rows(&cypher, params).await;
        self.drop_analytics_graph(&name);
        Ok(truncate_rows(rows?, limit))
    }

    async fn critical_resources_degree(
        &self,
        limit: u32,
    ) -> Result<(Vec<AnalyticsRow>, bool), StoreError> {
        let types = dep_type_list_literal()?;
        let cypher = format!(
            "MATCH (r:Node) WHERE r.kind = $kind \
             OPTIONAL MATCH (c)-[d]->(r) WHERE type(d) IN [{types}] \
             WITH r, count(d) AS degree \
             RETURN r.id AS id, r.name AS name, r.kind AS kind, toFloat(degree) AS score \
             ORDER BY score DESC, r.id \
             LIMIT $limit"
        );
        let fetch = limit + 1;
        let params = value_map!({ "kind": node_kind_token(NodeKind::Resource), "limit": fetch });
        let rows = self.query_rows(&cypher, params).await?;
        Ok(truncate_rows(rows, limit))
    }

    async fn bridges(&self, limit: u32) -> Result<AnalyticsReport, StoreError> {
        let mode = AnalyticsMode::Bridges;
        if !self.procedure_exists("gds.betweenness.stream").await {
            return Ok(AnalyticsReport::skipped(
                mode,
                AnalyticsEngine::Gds,
                "gds.betweenness.stream unavailable; bridges analysis skipped",
            ));
        }
        match self.bridges_gds(limit).await {
            Ok((rows, truncated)) => Ok(AnalyticsReport {
                mode,
                engine: AnalyticsEngine::Gds,
                rows,
                truncated,
                degraded: false,
                note: None,
            }),
            Err(err) => Ok(AnalyticsReport::skipped(
                mode,
                AnalyticsEngine::Gds,
                format!("gds.betweenness failed: {err}; bridges analysis skipped"),
            )),
        }
    }

    async fn bridges_gds(&self, limit: u32) -> Result<(Vec<AnalyticsRow>, bool), StoreError> {
        let name = analytics_graph_name();
        self.project_analytics_graph(&name)?;
        let cypher = format!(
            "CALL gds.betweenness.stream('{name}') \
             YIELD nodeId, score \
             MATCH (n:Node) WHERE id(n) = nodeId \
             RETURN n.id AS id, n.name AS name, n.kind AS kind, score \
             ORDER BY score DESC, n.id \
             LIMIT $limit"
        );
        let fetch = limit + 1;
        let params = value_map!({ "limit": fetch });
        let rows = self.query_rows(&cypher, params).await;
        self.drop_analytics_graph(&name);
        Ok(truncate_rows(rows?, limit))
    }

    async fn clusters(&self, limit: u32) -> Result<AnalyticsReport, StoreError> {
        let mode = AnalyticsMode::Clusters;
        if !self.procedure_exists("gds.wcc.stream").await {
            return Ok(AnalyticsReport::skipped(
                mode,
                AnalyticsEngine::Gds,
                "gds.wcc.stream unavailable; clusters analysis skipped",
            ));
        }
        match self.clusters_gds(limit).await {
            Ok((rows, truncated)) => Ok(AnalyticsReport {
                mode,
                engine: AnalyticsEngine::Gds,
                rows,
                truncated,
                degraded: false,
                note: None,
            }),
            Err(err) => Ok(AnalyticsReport::skipped(
                mode,
                AnalyticsEngine::Gds,
                format!("gds.wcc failed: {err}; clusters analysis skipped"),
            )),
        }
    }

    async fn clusters_gds(&self, limit: u32) -> Result<(Vec<AnalyticsRow>, bool), StoreError> {
        let name = analytics_graph_name();
        self.project_analytics_graph(&name)?;
        let cypher = format!(
            "CALL gds.wcc.stream('{name}') \
             YIELD nodeId, componentId \
             MATCH (n:Node) WHERE id(n) = nodeId \
             WITH componentId AS component, collect({{id: n.id, name: n.name, kind: n.kind}}) AS members, count(*) AS size \
             UNWIND members AS m \
             WITH component, size, m \
             ORDER BY size DESC, component, m.id \
             LIMIT $limit \
             RETURN component, size, m.id AS id, m.name AS name, m.kind AS kind"
        );
        let fetch = limit + 1;
        let params = value_map!({ "limit": fetch });
        let result = self.execute(&cypher, params);
        self.drop_analytics_graph(&name);
        let result = result?;
        let mut rows = Vec::with_capacity(result.records.len());
        for record in &result.records {
            let mut row = analytics_row(record)?;
            row.component = record.value("component").and_then(ValueReceive::as_int);
            row.score = record_number(record, "size");
            rows.push(row);
        }
        Ok(truncate_rows(rows, limit))
    }

    /// Project all nodes plus dependency-typed relationships into a named
    /// GDS graph under a unique per-call name.
    fn project_analytics_graph(&self, name: &str) -> Result<(), StoreError> {
        let (node_query, relationship_query) = gds_projection_queries()?;
        let cypher = format!(
            "CALL gds.graph.project.cypher('{name}', '{node_query}', '{relationship_query}') \
             YIELD graphName RETURN graphName"
        );
        self.execute(&cypher, Params::new())?;
        Ok(())
    }

    /// Best-effort removal of a per-call GDS projection.
    fn drop_analytics_graph(&self, name: &str) {
        let cypher =
            format!("CALL gds.graph.drop('{name}', false) YIELD graphName RETURN graphName");
        let _ = self.execute(&cypher, Params::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bolt_uri() {
        let (address, routing) = parse_uri("bolt://localhost:7687").unwrap();
        assert_eq!(address.host(), "localhost");
        assert_eq!(address.port(), 7687);
        assert!(!routing);
    }

    #[test]
    fn parses_neo4j_uri_with_routing() {
        let (address, routing) = parse_uri("neo4j://db.internal:7687/").unwrap();
        assert_eq!(address.host(), "db.internal");
        assert!(routing);
    }

    #[test]
    fn parses_bare_host_with_default_port() {
        let (address, routing) = parse_uri("localhost").unwrap();
        assert_eq!(address.host(), "localhost");
        assert_eq!(address.port(), 7687);
        assert!(!routing);
    }

    #[test]
    fn rejects_unsupported_scheme() {
        assert!(parse_uri("http://localhost:7474").is_err());
        assert!(parse_uri("").is_err());
    }
}
