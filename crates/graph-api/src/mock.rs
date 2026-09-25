use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use ckg_domain::{CanonicalId, DEPENDENCY_KINDS, GraphEdge, GraphNode, NodeKind, RelationKind};
use ckg_graph_delta::{DeltaOp, GraphDelta};
use ckg_neo4j_store::{
    AnalyticsEngine, AnalyticsMode, AnalyticsReport, AnalyticsRow, AnalyticsSpec, ApplyReport,
    GraphStore, StoreError,
};

#[derive(Default)]
pub struct MockStore {
    nodes: Mutex<Vec<GraphNode>>,
    edges: Mutex<Vec<GraphEdge>>,
    fail: AtomicBool,
}

impl MockStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    pub fn node_count(&self) -> usize {
        self.nodes.lock().expect("mock store lock").len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.lock().expect("mock store lock").len()
    }

    fn guard(&self) -> Result<(), StoreError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(StoreError::Data("mock store failure".into()))
        } else {
            Ok(())
        }
    }

    fn upsert_node(nodes: &mut Vec<GraphNode>, node: &GraphNode) -> bool {
        if let Some(pos) = nodes.iter().position(|n| n.id == node.id) {
            nodes[pos] = node.clone();
            true
        } else {
            nodes.push(node.clone());
            false
        }
    }

    fn upsert_edge(edges: &mut Vec<GraphEdge>, edge: &GraphEdge) -> bool {
        if let Some(pos) = edges
            .iter()
            .position(|e| e.kind == edge.kind && e.from == edge.from && e.to == edge.to)
        {
            edges[pos] = edge.clone();
            true
        } else {
            edges.push(edge.clone());
            false
        }
    }
}

#[async_trait]
impl GraphStore for MockStore {
    async fn apply_delta(&self, delta: &GraphDelta) -> Result<ApplyReport, StoreError> {
        self.guard()?;
        let mut report = ApplyReport::default();
        let mut nodes = self.nodes.lock().expect("mock store lock");
        let mut edges = self.edges.lock().expect("mock store lock");

        for op in &delta.ops {
            match op {
                DeltaOp::CreateNode(node) => {
                    if Self::upsert_node(&mut nodes, node) {
                        report.updated_nodes += 1;
                    } else {
                        report.created_nodes += 1;
                    }
                }
                DeltaOp::UpdateNode(node) => {
                    Self::upsert_node(&mut nodes, node);
                    report.updated_nodes += 1;
                }
                DeltaOp::DeleteNode(id) => {
                    if let Some(pos) = nodes.iter().position(|n| n.id == *id) {
                        nodes.remove(pos);
                        report.deleted_nodes += 1;
                    }
                }
                DeltaOp::CreateRelationship(edge) => {
                    if !Self::upsert_edge(&mut edges, edge) {
                        report.created_rels += 1;
                    }
                }
                DeltaOp::UpdateRelationship(edge) => {
                    Self::upsert_edge(&mut edges, edge);
                }
                DeltaOp::DeleteRelationship { kind, from, to } => {
                    if let Some(pos) = edges
                        .iter()
                        .position(|e| e.kind == *kind && e.from == *from && e.to == *to)
                    {
                        edges.remove(pos);
                        report.deleted_rels += 1;
                    }
                }
            }
        }

        Ok(report)
    }

    async fn get_node(&self, id: &CanonicalId) -> Result<Option<GraphNode>, StoreError> {
        self.guard()?;
        let nodes = self.nodes.lock().expect("mock store lock");
        Ok(nodes.iter().find(|n| n.id == *id).cloned())
    }

    async fn find_nodes_by_name(
        &self,
        name: &str,
        limit: u32,
    ) -> Result<Vec<GraphNode>, StoreError> {
        self.guard()?;
        let nodes = self.nodes.lock().expect("mock store lock");
        Ok(nodes
            .iter()
            .filter(|n| n.name == name || n.qualified_name == name)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn get_edges_from(
        &self,
        id: &CanonicalId,
        kind: Option<RelationKind>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>, StoreError> {
        self.guard()?;
        let edges = self.edges.lock().expect("mock store lock");
        Ok(edges
            .iter()
            .filter(|e| e.from == *id && kind.map(|k| e.kind == k).unwrap_or(true))
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn get_edges_to(
        &self,
        id: &CanonicalId,
        kind: Option<RelationKind>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>, StoreError> {
        self.guard()?;
        let edges = self.edges.lock().expect("mock store lock");
        Ok(edges
            .iter()
            .filter(|e| e.to == *id && kind.map(|k| e.kind == k).unwrap_or(true))
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn reset(&self) -> Result<(), StoreError> {
        self.guard()?;
        self.nodes.lock().expect("mock store lock").clear();
        self.edges.lock().expect("mock store lock").clear();
        Ok(())
    }

    async fn health(&self) -> Result<(), StoreError> {
        self.guard()
    }

    async fn run_analytics(&self, spec: &AnalyticsSpec) -> Result<AnalyticsReport, StoreError> {
        self.guard()?;
        let limit = spec.limit.clamp(1, 500);
        let nodes = self.nodes.lock().expect("mock store lock");
        let edges = self.edges.lock().expect("mock store lock");
        match spec.mode {
            AnalyticsMode::BlastRadius => {
                let id = spec
                    .id
                    .clone()
                    .ok_or_else(|| StoreError::Data("blast_radius requires a node id".into()))?;
                let mut rows = Vec::new();
                if spec.depth > 0 {
                    let mut visited = HashSet::from([id.clone()]);
                    let mut frontier = vec![id];
                    for distance in 1..=spec.depth.min(10) {
                        let mut next = Vec::new();
                        for node_id in &frontier {
                            for edge in edges.iter() {
                                if edge.to == *node_id
                                    && DEPENDENCY_KINDS.contains(&edge.kind)
                                    && visited.insert(edge.from.clone())
                                {
                                    next.push(edge.from.clone());
                                }
                            }
                        }
                        for node_id in &next {
                            if let Some(node) = nodes.iter().find(|n| n.id == *node_id) {
                                rows.push(AnalyticsRow {
                                    id: node.id.clone(),
                                    name: node.name.clone(),
                                    kind: node.kind,
                                    score: 0.0,
                                    distance: Some(distance),
                                    component: None,
                                });
                            }
                        }
                        if next.is_empty() {
                            break;
                        }
                        frontier = next;
                    }
                }
                rows.sort_by(|a, b| {
                    a.distance
                        .cmp(&b.distance)
                        .then_with(|| a.id.as_str().cmp(b.id.as_str()))
                });
                let truncated = rows.len() as u32 > limit;
                if truncated {
                    rows.truncate(limit as usize);
                }
                Ok(AnalyticsReport {
                    mode: spec.mode,
                    engine: AnalyticsEngine::Cypher,
                    rows,
                    truncated,
                    degraded: true,
                    note: Some(
                        "apoc.path.subgraphAll unavailable; used in-memory traversal".into(),
                    ),
                })
            }
            AnalyticsMode::CriticalResources => {
                let mut ranked: Vec<(CanonicalId, f64)> = nodes
                    .iter()
                    .filter(|n| n.kind == NodeKind::Resource)
                    .map(|n| {
                        let degree = edges
                            .iter()
                            .filter(|e| e.to == n.id && DEPENDENCY_KINDS.contains(&e.kind))
                            .count();
                        (n.id.clone(), degree as f64)
                    })
                    .collect();
                ranked.sort_by(|a, b| {
                    b.1.total_cmp(&a.1)
                        .then_with(|| a.0.as_str().cmp(b.0.as_str()))
                });
                let truncated = ranked.len() as u32 > limit;
                ranked.truncate(limit as usize);
                let rows = ranked
                    .into_iter()
                    .filter_map(|(id, score)| {
                        nodes.iter().find(|n| n.id == id).map(|node| AnalyticsRow {
                            id,
                            name: node.name.clone(),
                            kind: node.kind,
                            score,
                            distance: None,
                            component: None,
                        })
                    })
                    .collect();
                Ok(AnalyticsReport {
                    mode: spec.mode,
                    engine: AnalyticsEngine::Cypher,
                    rows,
                    truncated,
                    degraded: true,
                    note: Some(
                        "gds.pageRank unavailable; used dependency in-degree ranking".into(),
                    ),
                })
            }
            AnalyticsMode::Bridges | AnalyticsMode::Clusters => Ok(AnalyticsReport::skipped(
                spec.mode,
                AnalyticsEngine::Gds,
                "graph-data-science plugin unavailable; analysis skipped",
            )),
        }
    }
}
