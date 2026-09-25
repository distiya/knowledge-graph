use std::cmp::Ordering;
use std::collections::BTreeMap;

use ckg_domain::{
    AnalyzerOutput, CanonicalId, GraphEdge, GraphNode, ProcessingStatus, Provenance,
    ProvenanceSource, RawRelation, RawSymbol, RelationKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeKey(pub CanonicalId);

impl NodeKey {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl From<CanonicalId> for NodeKey {
    fn from(id: CanonicalId) -> Self {
        Self(id)
    }
}

impl Ord for NodeKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.as_str().cmp(other.0.as_str())
    }
}

impl PartialOrd for NodeKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeKey {
    pub kind: RelationKind,
    pub from: CanonicalId,
    pub to: CanonicalId,
}

impl EdgeKey {
    pub fn new(kind: RelationKind, from: CanonicalId, to: CanonicalId) -> Self {
        Self { kind, from, to }
    }
}

impl From<(RelationKind, CanonicalId, CanonicalId)> for EdgeKey {
    fn from((kind, from, to): (RelationKind, CanonicalId, CanonicalId)) -> Self {
        Self { kind, from, to }
    }
}

impl Ord for EdgeKey {
    fn cmp(&self, other: &Self) -> Ordering {
        (kind_rank(self.kind), self.from.as_str(), self.to.as_str()).cmp(&(
            kind_rank(other.kind),
            other.from.as_str(),
            other.to.as_str(),
        ))
    }
}

impl PartialOrd for EdgeKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn kind_rank(kind: RelationKind) -> u8 {
    match kind {
        RelationKind::HasBranch => 0,
        RelationKind::PointsTo => 1,
        RelationKind::ContainsStateOf => 2,
        RelationKind::DefinedIn => 3,
        RelationKind::Calls => 4,
        RelationKind::References => 5,
        RelationKind::Implements => 6,
        RelationKind::Extends => 7,
        RelationKind::Imports => 8,
        RelationKind::DependsOn => 9,
        RelationKind::Exposes => 10,
        RelationKind::ConsumedBy => 11,
        RelationKind::Uses => 12,
        RelationKind::DeployedTo => 13,
        RelationKind::HasCommit => 14,
        RelationKind::HasFile => 15,
        RelationKind::PartOf => 16,
        RelationKind::CallsApi => 17,
        RelationKind::PublishTo => 18,
        RelationKind::ConsumeFrom => 19,
        RelationKind::ReadsFrom => 20,
        RelationKind::WritesTo => 21,
        RelationKind::Invokes => 22,
        RelationKind::ConnectsTo => 23,
        RelationKind::Queries => 24,
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NormalizedGraph {
    pub nodes: BTreeMap<NodeKey, GraphNode>,
    pub edges: BTreeMap<EdgeKey, GraphEdge>,
    pub errors: Vec<String>,
}

impl NormalizedGraph {
    pub fn node(&self, id: &CanonicalId) -> Option<&GraphNode> {
        self.nodes.get(&NodeKey(id.clone()))
    }

    pub fn edge(
        &self,
        kind: RelationKind,
        from: &CanonicalId,
        to: &CanonicalId,
    ) -> Option<&GraphEdge> {
        self.edges
            .get(&EdgeKey::new(kind, from.clone(), to.clone()))
    }
}

pub struct Normalizer;

impl Normalizer {
    pub fn normalize(outputs: Vec<AnalyzerOutput>) -> NormalizedGraph {
        let mut graph = NormalizedGraph::default();
        for output in outputs {
            let tag = source_tag(&output);
            for error in &output.errors {
                graph.errors.push(format!("[{tag}] {error}"));
            }
            for symbol in &output.symbols {
                merge_node(&mut graph, symbol);
            }
            for relation in &output.relations {
                merge_edge(&mut graph, relation);
            }
        }
        graph
    }
}

fn source_tag(output: &AnalyzerOutput) -> String {
    match output.provenance.first().map(|p| &p.source) {
        Some(ProvenanceSource::Git) => "git".to_string(),
        Some(ProvenanceSource::TreeSitter) => "tree-sitter".to_string(),
        Some(ProvenanceSource::Scip) => "scip".to_string(),
        Some(ProvenanceSource::Joern) => "joern".to_string(),
        Some(ProvenanceSource::CiCd) => "ci-cd".to_string(),
        Some(ProvenanceSource::DeploymentSystem) => "deployment".to_string(),
        Some(ProvenanceSource::Manual) => "manual".to_string(),
        Some(ProvenanceSource::Other(other)) => other.clone(),
        None => "unknown".to_string(),
    }
}

fn priority(source: &ProvenanceSource) -> u8 {
    match source {
        ProvenanceSource::Joern => 2,
        ProvenanceSource::Scip => 1,
        _ => 0,
    }
}

fn provenance_key(
    p: &Provenance,
) -> (
    ProvenanceSource,
    Option<String>,
    ProcessingStatus,
    Option<String>,
) {
    (
        p.source.clone(),
        p.analyzer_version.clone(),
        p.status,
        p.detail.clone(),
    )
}

fn union_provenance(existing: &mut Vec<Provenance>, incoming: &Provenance) {
    let key = provenance_key(incoming);
    if !existing.iter().any(|e| provenance_key(e) == key) {
        existing.push(incoming.clone());
    }
}

fn provenance_priority(node_provenance: &[Provenance]) -> u8 {
    node_provenance
        .iter()
        .map(|p| priority(&p.source))
        .max()
        .unwrap_or(0)
}

fn merge_properties(
    existing: &mut serde_json::Map<String, serde_json::Value>,
    incoming: &serde_json::Map<String, serde_json::Value>,
    incoming_priority: u8,
    existing_priority: u8,
) {
    for (key, value) in incoming {
        if !existing.contains_key(key) || incoming_priority > existing_priority {
            existing.insert(key.clone(), value.clone());
        }
    }
}

fn merge_node(graph: &mut NormalizedGraph, symbol: &RawSymbol) {
    let key = NodeKey(symbol.canonical_id.clone());
    let incoming_priority = priority(&symbol.provenance.source);
    match graph.nodes.get_mut(&key) {
        None => {
            let mut node = GraphNode::new(symbol.kind, symbol.canonical_id.clone(), &symbol.name);
            node.qualified_name = symbol.qualified_name.clone();
            node.language = symbol.language.clone();
            node.location = symbol.location.clone();
            node.content_hash = symbol.content_hash.clone();
            node.properties = symbol.properties.clone();
            node.provenance = vec![symbol.provenance.clone()];
            graph.nodes.insert(key, node);
        }
        Some(node) => {
            let existing_priority = provenance_priority(&node.provenance);
            if incoming_priority > existing_priority {
                node.kind = symbol.kind;
                node.name = symbol.name.clone();
                node.qualified_name = symbol.qualified_name.clone();
            }
            if node.language.is_none() {
                node.language = symbol.language.clone();
            }
            if node.location.is_none() {
                node.location = symbol.location.clone();
            }
            match (&node.content_hash, &symbol.content_hash) {
                (None, Some(hash)) => node.content_hash = Some(hash.clone()),
                (Some(_), Some(hash)) if incoming_priority > existing_priority => {
                    node.content_hash = Some(hash.clone())
                }
                _ => {}
            }
            merge_properties(
                &mut node.properties,
                &symbol.properties,
                incoming_priority,
                existing_priority,
            );
            union_provenance(&mut node.provenance, &symbol.provenance);
        }
    }
}

fn merge_edge(graph: &mut NormalizedGraph, relation: &RawRelation) {
    let key = EdgeKey::new(relation.kind, relation.from.clone(), relation.to.clone());
    let incoming_priority = priority(&relation.provenance.source);
    match graph.edges.get_mut(&key) {
        None => {
            let mut edge =
                GraphEdge::new(relation.kind, relation.from.clone(), relation.to.clone());
            edge.properties = relation.properties.clone();
            edge.provenance = vec![relation.provenance.clone()];
            graph.edges.insert(key, edge);
        }
        Some(edge) => {
            let existing_priority = provenance_priority(&edge.provenance);
            merge_properties(
                &mut edge.properties,
                &relation.properties,
                incoming_priority,
                existing_priority,
            );
            union_provenance(&mut edge.provenance, &relation.provenance);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::{NodeKind, SourceLocation};

    fn ts_provenance() -> Provenance {
        Provenance::tree_sitter("0.1.0")
    }

    fn scip_provenance() -> Provenance {
        Provenance::scip("0.2.0")
    }

    fn joern_provenance(status: ProcessingStatus) -> Provenance {
        Provenance::joern("0.3.0").with_status(status)
    }

    fn location(start_line: u32) -> SourceLocation {
        SourceLocation::new(
            "org/repo",
            "sha",
            "src/lib.rs",
            start_line,
            1,
            start_line + 1,
            1,
        )
    }

    fn symbol(
        id: &str,
        name: &str,
        hash: &str,
        loc: Option<SourceLocation>,
        provenance: Provenance,
        properties: Vec<(&str, serde_json::Value)>,
    ) -> RawSymbol {
        let mut props = serde_json::Map::new();
        for (key, value) in properties {
            props.insert(key.to_string(), value);
        }
        RawSymbol {
            canonical_id: CanonicalId::from(id),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("rust::org/repo::{name}"),
            language: Some("rust".to_string()),
            location: loc,
            content_hash: Some(hash.to_string()),
            properties: props,
            provenance,
        }
    }

    fn output(
        symbols: Vec<RawSymbol>,
        relations: Vec<RawRelation>,
        provenance: Provenance,
        errors: Vec<String>,
    ) -> AnalyzerOutput {
        AnalyzerOutput {
            symbols,
            relations,
            provenance: vec![provenance],
            errors,
            ..AnalyzerOutput::default()
        }
    }

    #[test]
    fn same_symbol_from_multiple_analyzers_merges() {
        let ts = output(
            vec![symbol(
                "sym-1",
                "compute",
                "hash-ts",
                Some(location(10)),
                ts_provenance(),
                vec![("from_ts", serde_json::Value::Bool(true))],
            )],
            vec![],
            ts_provenance(),
            vec![],
        );
        let scip = output(
            vec![symbol(
                "sym-1",
                "compute",
                "hash-scip",
                Some(location(99)),
                scip_provenance(),
                vec![
                    ("from_scip", serde_json::Value::Bool(true)),
                    ("from_ts", serde_json::Value::Bool(false)),
                ],
            )],
            vec![],
            scip_provenance(),
            vec![],
        );

        let graph = Normalizer::normalize(vec![ts, scip]);

        assert_eq!(graph.nodes.len(), 1);
        let node = graph.node(&CanonicalId::from("sym-1")).expect("node");
        assert_eq!(node.content_hash.as_deref(), Some("hash-scip"));
        assert_eq!(node.location.as_ref().unwrap().start_line, 10);
        assert_eq!(node.provenance.len(), 2);
        assert!(
            node.provenance
                .iter()
                .any(|p| p.source == ProvenanceSource::TreeSitter)
        );
        assert!(
            node.provenance
                .iter()
                .any(|p| p.source == ProvenanceSource::Scip)
        );
        assert_eq!(
            node.properties.get("from_ts").unwrap(),
            &serde_json::Value::Bool(false)
        );
        assert_eq!(
            node.properties.get("from_scip").unwrap(),
            &serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn first_non_none_location_wins() {
        let ts = output(
            vec![symbol("sym-2", "f", "h1", None, ts_provenance(), vec![])],
            vec![],
            ts_provenance(),
            vec![],
        );
        let scip = output(
            vec![symbol(
                "sym-2",
                "f",
                "h2",
                Some(location(42)),
                scip_provenance(),
                vec![],
            )],
            vec![],
            scip_provenance(),
            vec![],
        );

        let graph = Normalizer::normalize(vec![ts, scip]);
        let node = graph.node(&CanonicalId::from("sym-2")).expect("node");
        assert_eq!(node.location.as_ref().unwrap().start_line, 42);
    }

    #[test]
    fn conflicting_content_hash_keeps_both_provenance_statuses() {
        let ts = output(
            vec![symbol(
                "sym-3",
                "f",
                "hash-a",
                Some(location(1)),
                ts_provenance(),
                vec![],
            )],
            vec![],
            ts_provenance(),
            vec![],
        );
        let joern = output(
            vec![symbol(
                "sym-3",
                "f",
                "hash-b",
                Some(location(2)),
                joern_provenance(ProcessingStatus::Partial),
                vec![],
            )],
            vec![],
            joern_provenance(ProcessingStatus::Partial),
            vec![],
        );

        let graph = Normalizer::normalize(vec![ts, joern]);
        let node = graph.node(&CanonicalId::from("sym-3")).expect("node");
        assert_eq!(node.content_hash.as_deref(), Some("hash-b"));
        assert_eq!(node.location.as_ref().unwrap().start_line, 1);
        assert_eq!(node.provenance.len(), 2);
        assert!(node.provenance.iter().any(|p| {
            p.source == ProvenanceSource::TreeSitter && p.status == ProcessingStatus::Complete
        }));
        assert!(node.provenance.iter().any(|p| {
            p.source == ProvenanceSource::Joern && p.status == ProcessingStatus::Partial
        }));
    }

    #[test]
    fn edges_are_deduplicated_and_merge_provenance() {
        let rel = |prov: Provenance| RawRelation {
            kind: RelationKind::Calls,
            from: CanonicalId::from("caller"),
            to: CanonicalId::from("callee"),
            properties: serde_json::Map::new(),
            provenance: prov,
        };
        let ts = output(vec![], vec![rel(ts_provenance())], ts_provenance(), vec![]);
        let scip = output(
            vec![],
            vec![rel(scip_provenance())],
            scip_provenance(),
            vec![],
        );

        let graph = Normalizer::normalize(vec![ts, scip]);
        assert_eq!(graph.edges.len(), 1);
        let edge = graph
            .edge(
                RelationKind::Calls,
                &CanonicalId::from("caller"),
                &CanonicalId::from("callee"),
            )
            .expect("edge");
        assert_eq!(edge.provenance.len(), 2);
    }

    #[test]
    fn analyzer_errors_are_preserved_with_source_tag() {
        let ts = AnalyzerOutput {
            symbols: vec![],
            relations: vec![],
            provenance: vec![ts_provenance()],
            errors: vec!["src/lib.rs:3:1: syntax error".to_string()],
            ..AnalyzerOutput::default()
        };
        let graph = Normalizer::normalize(vec![ts]);
        assert_eq!(
            graph.errors,
            vec!["[tree-sitter] src/lib.rs:3:1: syntax error".to_string()]
        );
    }

    #[test]
    fn output_ordering_is_deterministic() {
        let make = || {
            let a = output(
                vec![
                    symbol("id-b", "b", "hb", None, ts_provenance(), vec![]),
                    symbol("id-a", "a", "ha", None, ts_provenance(), vec![]),
                ],
                vec![],
                ts_provenance(),
                vec![],
            );
            let b = output(
                vec![symbol("id-c", "c", "hc", None, scip_provenance(), vec![])],
                vec![],
                scip_provenance(),
                vec![],
            );
            (a, b)
        };

        let (a1, b1) = make();
        let graph1 = Normalizer::normalize(vec![a1, b1]);
        let (a2, b2) = make();
        let graph2 = Normalizer::normalize(vec![a2, b2]);
        let summarize = |g: &NormalizedGraph| {
            g.nodes
                .iter()
                .map(|(k, n)| {
                    (
                        k.as_str().to_string(),
                        n.name.clone(),
                        n.content_hash.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(summarize(&graph1), summarize(&graph2));
        assert_eq!(graph1.errors, graph2.errors);

        let (a3, b3) = make();
        let graph3 = Normalizer::normalize(vec![b3, a3]);
        let keys1: Vec<&NodeKey> = graph1.nodes.keys().collect();
        let keys3: Vec<&NodeKey> = graph3.nodes.keys().collect();
        assert_eq!(keys1, keys3);
        assert_eq!(
            keys1.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            vec!["id-a", "id-b", "id-c"]
        );
    }

    #[test]
    fn duplicate_symbols_within_one_output_collapse() {
        let out = output(
            vec![
                symbol("dup", "f", "h1", Some(location(1)), ts_provenance(), vec![]),
                symbol("dup", "f", "h2", Some(location(2)), ts_provenance(), vec![]),
            ],
            vec![],
            ts_provenance(),
            vec![],
        );
        let graph = Normalizer::normalize(vec![out]);
        assert_eq!(graph.nodes.len(), 1);
        let node = graph.node(&CanonicalId::from("dup")).expect("node");
        assert_eq!(node.location.as_ref().unwrap().start_line, 1);
        assert_eq!(node.provenance.len(), 1);
    }
}
