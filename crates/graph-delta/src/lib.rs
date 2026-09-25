use std::collections::BTreeMap;

use ckg_domain::{CanonicalId, GraphEdge, GraphNode, RelationKind};

/// Nodes keyed by canonical id string.
pub type NodeMap = BTreeMap<String, GraphNode>;
/// Edges keyed by [`EdgeKey`].
pub type EdgeMap = BTreeMap<EdgeKey, GraphEdge>;

/// Identity of a relationship: (kind, from, to), all as canonical strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeKey {
    pub kind: String,
    pub from: String,
    pub to: String,
}

impl EdgeKey {
    pub fn new(kind: RelationKind, from: &CanonicalId, to: &CanonicalId) -> Self {
        Self {
            kind: relation_kind_str(kind),
            from: from.to_string(),
            to: to.to_string(),
        }
    }

    pub fn from_edge(edge: &GraphEdge) -> Self {
        Self::new(edge.kind, &edge.from, &edge.to)
    }
}

/// SCREAMING_SNAKE_CASE wire name of a relation kind (matches serde encoding).
pub fn relation_kind_str(kind: RelationKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// SCREAMING_SNAKE_CASE wire name of a node kind (matches serde encoding).
pub fn node_kind_str(kind: ckg_domain::NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A single mutation against the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeltaOp {
    CreateNode(GraphNode),
    UpdateNode(GraphNode),
    DeleteNode(CanonicalId),
    CreateRelationship(GraphEdge),
    UpdateRelationship(GraphEdge),
    DeleteRelationship {
        kind: RelationKind,
        from: CanonicalId,
        to: CanonicalId,
    },
}

/// An ordered batch of graph mutations between two revisions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphDelta {
    pub ops: Vec<DeltaOp>,
    pub base_revision: Option<String>,
    pub target_revision: Option<String>,
}

impl GraphDelta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }
}

/// Fluent builder for assembling a [`GraphDelta`] by hand.
#[derive(Debug, Default)]
pub struct DeltaBuilder {
    delta: GraphDelta,
}

impl DeltaBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn base_revision(mut self, revision: impl Into<String>) -> Self {
        self.delta.base_revision = Some(revision.into());
        self
    }

    pub fn target_revision(mut self, revision: impl Into<String>) -> Self {
        self.delta.target_revision = Some(revision.into());
        self
    }

    pub fn create_node(mut self, node: GraphNode) -> Self {
        self.delta.ops.push(DeltaOp::CreateNode(node));
        self
    }

    pub fn update_node(mut self, node: GraphNode) -> Self {
        self.delta.ops.push(DeltaOp::UpdateNode(node));
        self
    }

    pub fn delete_node(mut self, id: impl Into<CanonicalId>) -> Self {
        self.delta.ops.push(DeltaOp::DeleteNode(id.into()));
        self
    }

    pub fn create_relationship(mut self, edge: GraphEdge) -> Self {
        self.delta.ops.push(DeltaOp::CreateRelationship(edge));
        self
    }

    pub fn update_relationship(mut self, edge: GraphEdge) -> Self {
        self.delta.ops.push(DeltaOp::UpdateRelationship(edge));
        self
    }

    pub fn delete_relationship(
        mut self,
        kind: RelationKind,
        from: impl Into<CanonicalId>,
        to: impl Into<CanonicalId>,
    ) -> Self {
        self.delta.ops.push(DeltaOp::DeleteRelationship {
            kind,
            from: from.into(),
            to: to.into(),
        });
        self
    }

    pub fn build(self) -> GraphDelta {
        self.delta
    }
}

fn node_content_differs(old: &GraphNode, new: &GraphNode) -> bool {
    old.content_hash != new.content_hash
        || old.properties != new.properties
        || old.location != new.location
}

fn edge_content_differs(old: &GraphEdge, new: &GraphEdge) -> bool {
    old.content_hash != new.content_hash || old.properties != new.properties
}

/// Compute the minimal delta that turns the old graph into the new graph.
///
/// Emission order: node upserts, edge upserts, edge deletes, node deletes —
/// so that edge deletes still find their endpoints and node creates precede
/// edge creates.
///
/// `diff_graphs(g, g, g, g)` yields zero ops (idempotency).
pub fn diff_graphs(
    old_nodes: &NodeMap,
    new_nodes: &NodeMap,
    old_edges: &EdgeMap,
    new_edges: &EdgeMap,
) -> GraphDelta {
    let mut ops = Vec::new();

    for (id, new_node) in new_nodes {
        match old_nodes.get(id) {
            None => ops.push(DeltaOp::CreateNode(new_node.clone())),
            Some(old_node) if node_content_differs(old_node, new_node) => {
                ops.push(DeltaOp::UpdateNode(new_node.clone()))
            }
            Some(_) => {}
        }
    }

    for (key, new_edge) in new_edges {
        match old_edges.get(key) {
            None => ops.push(DeltaOp::CreateRelationship(new_edge.clone())),
            Some(old_edge) if edge_content_differs(old_edge, new_edge) => {
                ops.push(DeltaOp::UpdateRelationship(new_edge.clone()))
            }
            Some(_) => {}
        }
    }

    for key in old_edges.keys() {
        if !new_edges.contains_key(key) {
            let old_edge = &old_edges[key];
            ops.push(DeltaOp::DeleteRelationship {
                kind: old_edge.kind,
                from: old_edge.from.clone(),
                to: old_edge.to.clone(),
            });
        }
    }

    for id in old_nodes.keys() {
        if !new_nodes.contains_key(id) {
            ops.push(DeltaOp::DeleteNode(CanonicalId::new(id.clone())));
        }
    }

    GraphDelta {
        ops,
        base_revision: None,
        target_revision: None,
    }
}

/// Apply a delta to in-memory node/edge maps (test helper).
///
/// Applying a delta produced from identical graphs leaves the maps unchanged.
pub fn apply_ops_to_maps(nodes: &mut NodeMap, edges: &mut EdgeMap, delta: &GraphDelta) {
    for op in &delta.ops {
        match op {
            DeltaOp::CreateNode(node) | DeltaOp::UpdateNode(node) => {
                nodes.insert(node.id.to_string(), node.clone());
            }
            DeltaOp::DeleteNode(id) => {
                nodes.remove(id.as_str());
            }
            DeltaOp::CreateRelationship(edge) | DeltaOp::UpdateRelationship(edge) => {
                edges.insert(EdgeKey::from_edge(edge), edge.clone());
            }
            DeltaOp::DeleteRelationship { kind, from, to } => {
                edges.remove(&EdgeKey::new(*kind, from, to));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::{NodeKind, Provenance, ProvenanceSource};

    fn node(id: &str, name: &str, hash: &str) -> GraphNode {
        GraphNode::new(NodeKind::Function, CanonicalId::from(id), name)
            .with_content_hash(hash)
            .with_provenance(Provenance::from(ProvenanceSource::TreeSitter))
    }

    fn edge(kind: RelationKind, from: &str, to: &str, hash: &str) -> GraphEdge {
        GraphEdge::new(kind, CanonicalId::from(from), CanonicalId::from(to)).with_content_hash(hash)
    }

    fn node_map(nodes: Vec<GraphNode>) -> NodeMap {
        nodes.into_iter().map(|n| (n.id.to_string(), n)).collect()
    }

    fn edge_map(edges: Vec<GraphEdge>) -> EdgeMap {
        edges
            .into_iter()
            .map(|e| (EdgeKey::from_edge(&e), e))
            .collect()
    }

    #[test]
    fn diff_identical_graphs_is_empty() {
        let nodes = node_map(vec![node("a", "foo", "h1"), node("b", "bar", "h2")]);
        let edges = edge_map(vec![edge(RelationKind::Calls, "a", "b", "e1")]);

        let delta = diff_graphs(&nodes, &nodes, &edges, &edges);
        assert!(delta.is_empty());
        assert_eq!(delta.len(), 0);
    }

    #[test]
    fn diff_empty_to_populated_creates_everything() {
        let empty_nodes = NodeMap::new();
        let empty_edges = EdgeMap::new();
        let nodes = node_map(vec![node("a", "foo", "h1"), node("b", "bar", "h2")]);
        let edges = edge_map(vec![edge(RelationKind::Calls, "a", "b", "e1")]);

        let delta = diff_graphs(&empty_nodes, &nodes, &empty_edges, &edges);
        assert_eq!(delta.len(), 3);
        assert!(
            delta
                .ops
                .iter()
                .any(|op| matches!(op, DeltaOp::CreateNode(n) if n.id.as_str() == "a"))
        );
        assert!(
            delta
                .ops
                .iter()
                .any(|op| matches!(op, DeltaOp::CreateNode(n) if n.id.as_str() == "b"))
        );
        assert!(
            delta
                .ops
                .iter()
                .any(|op| matches!(op, DeltaOp::CreateRelationship(_)))
        );

        let mut got_nodes = NodeMap::new();
        let mut got_edges = EdgeMap::new();
        apply_ops_to_maps(&mut got_nodes, &mut got_edges, &delta);
        assert_eq!(got_nodes, nodes);
        assert_eq!(got_edges, edges);
    }

    #[test]
    fn diff_update_node_and_edge_content() {
        let old_nodes = node_map(vec![node("a", "foo", "h1"), node("b", "bar", "h2")]);
        let old_edges = edge_map(vec![edge(RelationKind::Calls, "a", "b", "e1")]);

        let mut changed = node("a", "foo", "h1-changed");
        changed
            .properties
            .insert("lines".into(), serde_json::json!(42));
        let new_nodes = node_map(vec![changed, node("b", "bar", "h2")]);
        let new_edges = edge_map(vec![edge(RelationKind::Calls, "a", "b", "e1-updated")]);

        let delta = diff_graphs(&old_nodes, &new_nodes, &old_edges, &new_edges);
        assert_eq!(delta.len(), 2);
        assert!(delta
            .ops
            .iter()
            .any(|op| matches!(op, DeltaOp::UpdateNode(n) if n.id.as_str() == "a" && n.content_hash.as_deref() == Some("h1-changed"))));
        assert!(delta
            .ops
            .iter()
            .any(|op| matches!(op, DeltaOp::UpdateRelationship(e) if e.content_hash.as_deref() == Some("e1-updated"))));

        let mut got_nodes = old_nodes.clone();
        let mut got_edges = old_edges.clone();
        apply_ops_to_maps(&mut got_nodes, &mut got_edges, &delta);
        assert_eq!(got_nodes, new_nodes);
        assert_eq!(got_edges, new_edges);
    }

    #[test]
    fn diff_delete_node_and_edge() {
        let old_nodes = node_map(vec![node("a", "foo", "h1"), node("b", "bar", "h2")]);
        let old_edges = edge_map(vec![edge(RelationKind::Calls, "a", "b", "e1")]);
        let new_nodes = node_map(vec![node("a", "foo", "h1")]);
        let new_edges = EdgeMap::new();

        let delta = diff_graphs(&old_nodes, &new_nodes, &old_edges, &new_edges);
        assert_eq!(delta.len(), 2);
        assert!(
            delta
                .ops
                .iter()
                .any(|op| matches!(op, DeltaOp::DeleteNode(id) if id.as_str() == "b"))
        );
        assert!(delta.ops.iter().any(
            |op| matches!(op, DeltaOp::DeleteRelationship { kind, from, to }
                if *kind == RelationKind::Calls
                    && from.as_str() == "a"
                    && to.as_str() == "b")
        ));

        let mut got_nodes = old_nodes.clone();
        let mut got_edges = old_edges.clone();
        apply_ops_to_maps(&mut got_nodes, &mut got_edges, &delta);
        assert_eq!(got_nodes, new_nodes);
        assert_eq!(got_edges, new_edges);
    }

    #[test]
    fn incremental_single_property_change_yields_single_op() {
        let old_nodes = node_map(vec![node("a", "foo", "h1")]);
        let empty_edges = EdgeMap::new();

        let mut changed = node("a", "foo", "h1");
        changed
            .properties
            .insert("loc".into(), serde_json::json!("x"));
        let new_nodes = node_map(vec![changed]);

        let delta = diff_graphs(&old_nodes, &new_nodes, &empty_edges, &empty_edges);
        assert_eq!(delta.len(), 1);
        assert!(matches!(delta.ops[0], DeltaOp::UpdateNode(_)));
    }

    #[test]
    fn identical_content_hash_and_properties_not_updated() {
        let old_nodes = node_map(vec![node("a", "old-name", "h1")]);
        let new_nodes = node_map(vec![node("a", "new-name", "h1")]);
        let empty_edges = EdgeMap::new();

        let delta = diff_graphs(&old_nodes, &new_nodes, &empty_edges, &empty_edges);
        assert!(delta.is_empty());
    }

    #[test]
    fn location_change_triggers_update() {
        use ckg_domain::SourceLocation;
        let mut old = node("a", "foo", "h1");
        old.location = Some(SourceLocation::new("org/r", "sha1", "src/a.rs", 1, 0, 2, 0));
        let mut new = node("a", "foo", "h1");
        new.location = Some(SourceLocation::new("org/r", "sha1", "src/a.rs", 5, 0, 6, 0));
        let old_nodes = node_map(vec![old]);
        let new_nodes = node_map(vec![new]);
        let empty_edges = EdgeMap::new();

        let delta = diff_graphs(&old_nodes, &new_nodes, &empty_edges, &empty_edges);
        assert_eq!(delta.len(), 1);
        assert!(matches!(delta.ops[0], DeltaOp::UpdateNode(_)));
    }

    #[test]
    fn diff_still_emits_delete_node_for_removed_symbols() {
        let old_nodes = node_map(vec![
            node("shared", "shared", "h1"),
            node("gone", "gone", "h2"),
        ]);
        let old_edges = edge_map(vec![edge(
            RelationKind::ContainsStateOf,
            "c1",
            "shared",
            "e1",
        )]);
        let new_nodes = node_map(vec![node("shared", "shared", "h1")]);
        let new_edges = edge_map(vec![edge(
            RelationKind::ContainsStateOf,
            "c1",
            "shared",
            "e1",
        )]);

        let delta = diff_graphs(&old_nodes, &new_nodes, &old_edges, &new_edges);
        assert!(
            delta
                .ops
                .iter()
                .any(|op| matches!(op, DeltaOp::DeleteNode(id) if id.as_str() == "gone")),
            "graph-delta must still emit DeleteNode; store guards the actual delete"
        );
    }

    #[test]
    fn apply_delta_from_identical_graphs_is_idempotent() {
        let nodes = node_map(vec![node("a", "foo", "h1"), node("b", "bar", "h2")]);
        let edges = edge_map(vec![edge(RelationKind::Calls, "a", "b", "e1")]);

        let delta = diff_graphs(&nodes, &nodes, &edges, &edges);
        let mut got_nodes = nodes.clone();
        let mut got_edges = edges.clone();
        apply_ops_to_maps(&mut got_nodes, &mut got_edges, &delta);

        assert_eq!(got_nodes, nodes);
        assert_eq!(got_edges, edges);
        assert!(delta.is_empty());
    }

    #[test]
    fn apply_then_rediff_is_empty() {
        let empty_nodes = NodeMap::new();
        let empty_edges = EdgeMap::new();
        let target_nodes = node_map(vec![node("a", "foo", "h1")]);
        let target_edges = edge_map(vec![edge(RelationKind::DefinedIn, "a", "file", "e1")]);

        let delta = diff_graphs(&empty_nodes, &target_nodes, &empty_edges, &target_edges);
        let mut nodes = empty_nodes.clone();
        let mut edges = empty_edges.clone();
        apply_ops_to_maps(&mut nodes, &mut edges, &delta);

        let second = diff_graphs(&nodes, &target_nodes, &edges, &target_edges);
        assert!(second.is_empty());
    }

    #[test]
    fn builder_assembles_delta_with_revisions() {
        let delta = DeltaBuilder::new()
            .base_revision("rev-1")
            .target_revision("rev-2")
            .create_node(node("a", "foo", "h1"))
            .delete_node("b")
            .create_relationship(edge(RelationKind::Calls, "a", "b", "e1"))
            .delete_relationship(RelationKind::Calls, "a", "b")
            .build();

        assert_eq!(delta.base_revision.as_deref(), Some("rev-1"));
        assert_eq!(delta.target_revision.as_deref(), Some("rev-2"));
        assert_eq!(delta.len(), 4);
    }

    #[test]
    fn edge_keys_are_stable_and_orderable() {
        let e1 = edge(RelationKind::Calls, "a", "b", "h");
        let e2 = edge(RelationKind::Calls, "a", "b", "h-other");
        assert_eq!(EdgeKey::from_edge(&e1), EdgeKey::from_edge(&e2));

        let mut map = EdgeMap::new();
        map.insert(EdgeKey::from_edge(&e1), e1.clone());
        map.insert(EdgeKey::from_edge(&e2), e2);
        assert_eq!(map.len(), 1);
        assert_eq!(
            map[&EdgeKey::from_edge(&e1)].content_hash.as_deref(),
            Some("h-other")
        );
    }
}
