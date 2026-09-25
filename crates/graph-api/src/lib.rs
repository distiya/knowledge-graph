mod mock;

pub use ckg_graph_delta::{DeltaOp, GraphDelta};
pub use ckg_neo4j_store::{
    AnalyticsEngine, AnalyticsMode, AnalyticsReport, AnalyticsRow, AnalyticsSpec, ApplyReport,
    GraphStore, StoreError,
};
pub use mock::MockStore;

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use ckg_domain::{
    CanonicalId, DEPENDENCY_KINDS, GraphEdge, GraphNode, NodeKind, Provenance, RelationKind,
};
use serde::{Deserialize, Serialize};

fn default_branch_set_limit() -> u32 {
    10_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryLimits {
    pub max_depth: u8,
    pub max_results: u32,
    #[serde(default = "default_branch_set_limit")]
    pub branch_set_limit: u32,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_results: 50,
            branch_set_limit: default_branch_set_limit(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("graph store error: {0}")]
    Store(#[from] StoreError),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiResult<T> {
    pub data: T,
    #[serde(default)]
    pub provenance: Vec<Provenance>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallEdge {
    pub edge: GraphEdge,
    pub depth: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchContext {
    pub repository: Option<GraphNode>,
    pub branch: Option<GraphNode>,
    pub commit: Option<GraphNode>,
    pub state_sample: Vec<GraphNode>,
    #[serde(default)]
    pub indexed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangeImpact {
    pub root: Option<GraphNode>,
    pub callers: Vec<CallEdge>,
    pub dependents: Vec<GraphEdge>,
    pub impacted: Vec<GraphNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeploymentInfo {
    pub commit: GraphNode,
    pub edge: GraphEdge,
    pub deployed_at: Option<String>,
    pub status: Option<String>,
    pub release: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeploymentState {
    pub repository: String,
    pub environment: String,
    pub environment_node: Option<GraphNode>,
    pub deployments: Vec<DeploymentInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchStatusEvidence {
    pub branch: String,
    pub found: bool,
    /// False when the branch (or its head) is not present in the graph; `found` is then not meaningful.
    #[serde(default)]
    pub indexed: bool,
    pub matches: Vec<GraphNode>,
    pub provenance: Vec<Provenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImplementationStatus {
    pub repository: String,
    pub capability: String,
    pub branches: Vec<BranchStatusEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchComparison {
    pub repository: String,
    pub branch_a: String,
    pub branch_b: String,
    #[serde(default)]
    pub branch_a_indexed: bool,
    #[serde(default)]
    pub branch_b_indexed: bool,
    pub common: Vec<String>,
    pub only_in_a: Vec<String>,
    pub only_in_b: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureContext {
    pub root: Option<GraphNode>,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeContext {
    pub symbols: Vec<GraphNode>,
    pub path_matches: Vec<GraphNode>,
    pub contexts: Vec<ArchitectureContext>,
}

#[derive(Clone)]
pub struct GraphApi {
    pub store: Arc<dyn GraphStore>,
    pub limits: QueryLimits,
}

struct TraversalEdge {
    edge: GraphEdge,
    depth: u8,
    other: CanonicalId,
}

fn provenance_from_nodes<'a, I>(nodes: I) -> Vec<Provenance>
where
    I: IntoIterator<Item = &'a GraphNode>,
{
    let mut out: Vec<Provenance> = Vec::new();
    for node in nodes {
        for p in &node.provenance {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
    }
    out
}

fn provenance_from_edges<'a, I>(edges: I) -> Vec<Provenance>
where
    I: IntoIterator<Item = &'a GraphEdge>,
{
    let mut out: Vec<Provenance> = Vec::new();
    for edge in edges {
        for p in &edge.provenance {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
    }
    out
}

fn node_in_repo(node: &GraphNode, repo: &str) -> bool {
    if let Some(location) = &node.location {
        if location.repository == repo {
            return true;
        }
    }
    matches!(
        node.properties.get("repository"),
        Some(v) if v.as_str() == Some(repo)
    )
}

fn edge_prop_string(edge: &GraphEdge, key: &str) -> Option<String> {
    edge.properties
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn edge_allows_branch(edge: &GraphEdge, branch: &str) -> bool {
    if !matches!(
        edge.kind,
        RelationKind::CallsApi
            | RelationKind::PublishTo
            | RelationKind::ConsumeFrom
            | RelationKind::ReadsFrom
            | RelationKind::WritesTo
            | RelationKind::Invokes
            | RelationKind::ConnectsTo
            | RelationKind::Queries
    ) {
        return true;
    }
    match edge.properties.get("branch_pairs") {
        Some(serde_json::Value::Array(pairs)) => pairs.iter().any(|v| v.as_str() == Some(branch)),
        _ => true,
    }
}

fn is_symbol_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Function
            | NodeKind::Method
            | NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Symbol
            | NodeKind::Variable
            | NodeKind::Api
            | NodeKind::Module
            | NodeKind::Package
    )
}

impl GraphApi {
    pub fn new(store: Arc<dyn GraphStore>, limits: QueryLimits) -> Self {
        Self { store, limits }
    }

    pub fn with_defaults(store: Arc<dyn GraphStore>) -> Self {
        Self::new(store, QueryLimits::default())
    }

    fn clamp_depth(&self, depth: u8) -> u8 {
        depth.min(self.limits.max_depth)
    }

    fn clamp_limit(&self, requested: Option<u32>) -> u32 {
        let max = self.limits.max_results.max(1);
        requested.unwrap_or(max).clamp(1, max)
    }

    fn check_id(&self, id: &CanonicalId) -> Result<(), ApiError> {
        if id.as_str().is_empty() {
            return Err(ApiError::InvalidArgument("id must not be empty".into()));
        }
        Ok(())
    }

    fn check_non_empty(&self, value: &str, what: &str) -> Result<(), ApiError> {
        if value.is_empty() {
            return Err(ApiError::InvalidArgument(format!(
                "{what} must not be empty"
            )));
        }
        Ok(())
    }

    fn check_branch(&self, branch: Option<&str>) -> Result<(), ApiError> {
        if let Some(branch) = branch {
            self.check_non_empty(branch, "branch")?;
        }
        Ok(())
    }

    async fn node_allowed(&self, node: &GraphNode, branch: Option<&str>) -> Result<bool, ApiError> {
        match branch {
            None => Ok(true),
            Some(branch) => self.node_in_branch(node, branch).await,
        }
    }

    pub async fn find_symbol(
        &self,
        name: &str,
        repo_filter: Option<&str>,
        limit: u32,
        branch: Option<&str>,
    ) -> Result<ApiResult<Vec<GraphNode>>, ApiError> {
        self.check_non_empty(name, "name")?;
        self.check_branch(branch)?;
        let lim = self.clamp_limit(Some(limit));
        let raw = self.store.find_nodes_by_name(name, lim).await?;
        let mut data: Vec<GraphNode> = Vec::new();
        for node in raw {
            if data.len() as u32 >= lim {
                break;
            }
            if let Some(repo) = repo_filter {
                if !node_in_repo(&node, repo) {
                    continue;
                }
            }
            if !self.node_allowed(&node, branch).await? {
                continue;
            }
            data.push(node);
        }
        let truncated = data.len() as u32 >= lim;
        let provenance = provenance_from_nodes(&data);
        Ok(ApiResult {
            data,
            provenance,
            truncated,
        })
    }

    pub async fn get_symbol(
        &self,
        id: &CanonicalId,
        branch: Option<&str>,
    ) -> Result<ApiResult<Option<GraphNode>>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let mut node = self.store.get_node(id).await?;
        if let (Some(branch), Some(n)) = (branch, node.as_ref()) {
            if !self.node_in_branch(n, branch).await? {
                node = None;
            }
        }
        let provenance = provenance_from_nodes(node.iter());
        Ok(ApiResult {
            data: node,
            provenance,
            truncated: false,
        })
    }

    async fn traverse(
        &self,
        start: &CanonicalId,
        depth: u8,
        kinds: &[RelationKind],
        inbound: bool,
        budget: u32,
        branch: Option<&str>,
    ) -> Result<(Vec<TraversalEdge>, bool), ApiError> {
        let eff_depth = self.clamp_depth(depth);
        let mut visited: HashSet<CanonicalId> = HashSet::new();
        visited.insert(start.clone());
        let mut frontier: Vec<CanonicalId> = vec![start.clone()];
        let mut out: Vec<TraversalEdge> = Vec::new();
        let mut truncated = false;
        let mut level: u8 = 0;

        while level < eff_depth {
            if frontier.is_empty() {
                break;
            }
            if out.len() as u32 >= budget {
                truncated = true;
                break;
            }
            level += 1;
            let mut next: Vec<CanonicalId> = Vec::new();
            let mut hit_budget = false;

            'level: for node_id in &frontier {
                for kind in kinds {
                    if out.len() as u32 >= budget {
                        truncated = true;
                        hit_budget = true;
                        break 'level;
                    }
                    let remaining = budget - out.len() as u32;
                    let edges = if inbound {
                        self.store
                            .get_edges_to(node_id, Some(*kind), remaining)
                            .await?
                    } else {
                        self.store
                            .get_edges_from(node_id, Some(*kind), remaining)
                            .await?
                    };
                    for edge in edges {
                        let other = if inbound {
                            edge.from.clone()
                        } else {
                            edge.to.clone()
                        };
                        if let Some(branch) = branch {
                            let Some(other_node) = self.store.get_node(&other).await? else {
                                continue;
                            };
                            if !self.node_in_branch(&other_node, branch).await? {
                                continue;
                            }
                        }
                        if visited.insert(other.clone()) {
                            next.push(other.clone());
                        }
                        out.push(TraversalEdge {
                            edge,
                            depth: level,
                            other,
                        });
                        if out.len() as u32 >= budget {
                            truncated = true;
                            hit_budget = true;
                            break 'level;
                        }
                    }
                }
            }

            frontier = next;
            if hit_budget {
                break;
            }
        }

        if level > 0 && level >= eff_depth && !frontier.is_empty() {
            truncated = true;
        }

        Ok((out, truncated))
    }

    pub async fn get_callers(
        &self,
        id: &CanonicalId,
        depth: u8,
        branch: Option<&str>,
    ) -> Result<ApiResult<Vec<CallEdge>>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let budget = self.clamp_limit(None);
        let (edges, truncated) = self
            .traverse(id, depth, &[RelationKind::Calls], true, budget, branch)
            .await?;
        let data: Vec<CallEdge> = edges
            .into_iter()
            .map(|t| CallEdge {
                edge: t.edge,
                depth: t.depth,
            })
            .collect();
        let provenance = provenance_from_edges(data.iter().map(|c| &c.edge));
        Ok(ApiResult {
            data,
            provenance,
            truncated,
        })
    }

    pub async fn get_callees(
        &self,
        id: &CanonicalId,
        depth: u8,
        branch: Option<&str>,
    ) -> Result<ApiResult<Vec<CallEdge>>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let budget = self.clamp_limit(None);
        let (edges, truncated) = self
            .traverse(id, depth, &[RelationKind::Calls], false, budget, branch)
            .await?;
        let data: Vec<CallEdge> = edges
            .into_iter()
            .map(|t| CallEdge {
                edge: t.edge,
                depth: t.depth,
            })
            .collect();
        let provenance = provenance_from_edges(data.iter().map(|c| &c.edge));
        Ok(ApiResult {
            data,
            provenance,
            truncated,
        })
    }

    async fn filter_inbound_edges(
        &self,
        id: &CanonicalId,
        kind: RelationKind,
        limit: u32,
        branch: Option<&str>,
    ) -> Result<Vec<GraphEdge>, ApiError> {
        let lim = self.clamp_limit(Some(limit));
        let fetch = lim.saturating_mul(4).saturating_add(lim);
        let raw = self.store.get_edges_to(id, Some(kind), fetch).await?;
        let mut data: Vec<GraphEdge> = Vec::new();
        for edge in raw {
            if data.len() as u32 >= lim {
                break;
            }
            if let Some(branch) = branch {
                if !edge_allows_branch(&edge, branch) {
                    continue;
                }
                let Some(from) = self.store.get_node(&edge.from).await? else {
                    continue;
                };
                if !self.node_in_branch(&from, branch).await? {
                    continue;
                }
            }
            data.push(edge);
        }
        Ok(data)
    }

    async fn filter_outbound_edges(
        &self,
        id: &CanonicalId,
        kind: RelationKind,
        limit: u32,
        branch: Option<&str>,
    ) -> Result<Vec<GraphEdge>, ApiError> {
        let lim = self.clamp_limit(Some(limit));
        let fetch = lim.saturating_mul(4).saturating_add(lim);
        let raw = self.store.get_edges_from(id, Some(kind), fetch).await?;
        let mut data: Vec<GraphEdge> = Vec::new();
        for edge in raw {
            if data.len() as u32 >= lim {
                break;
            }
            if let Some(branch) = branch {
                if !edge_allows_branch(&edge, branch) {
                    continue;
                }
                let Some(to) = self.store.get_node(&edge.to).await? else {
                    continue;
                };
                if !self.node_in_branch(&to, branch).await? {
                    continue;
                }
            }
            data.push(edge);
        }
        Ok(data)
    }

    pub async fn get_references(
        &self,
        id: &CanonicalId,
        limit: u32,
        branch: Option<&str>,
    ) -> Result<ApiResult<Vec<GraphEdge>>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let lim = self.clamp_limit(Some(limit));
        let mut data = self
            .filter_inbound_edges(id, RelationKind::References, lim, branch)
            .await?;
        data.truncate(lim as usize);
        let truncated = data.len() as u32 >= lim;
        let provenance = provenance_from_edges(&data);
        Ok(ApiResult {
            data,
            provenance,
            truncated,
        })
    }

    pub async fn get_implementations(
        &self,
        id: &CanonicalId,
        limit: u32,
        branch: Option<&str>,
    ) -> Result<ApiResult<Vec<GraphEdge>>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let lim = self.clamp_limit(Some(limit));
        let mut data = self
            .filter_inbound_edges(id, RelationKind::Implements, lim, branch)
            .await?;
        data.truncate(lim as usize);
        let truncated = data.len() as u32 >= lim;
        let provenance = provenance_from_edges(&data);
        Ok(ApiResult {
            data,
            provenance,
            truncated,
        })
    }

    pub async fn get_dependencies(
        &self,
        id: &CanonicalId,
        limit: u32,
        branch: Option<&str>,
    ) -> Result<ApiResult<Vec<GraphEdge>>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let lim = self.clamp_limit(Some(limit));
        let mut data: Vec<GraphEdge> = Vec::new();
        for kind in DEPENDENCY_KINDS {
            if data.len() as u32 >= lim {
                break;
            }
            let remaining = lim.saturating_sub(data.len() as u32);
            let edges = self
                .filter_outbound_edges(id, kind, remaining, branch)
                .await?;
            data.extend(edges);
        }
        data.truncate(lim as usize);
        let truncated = data.len() as u32 >= lim;
        let provenance = provenance_from_edges(&data);
        Ok(ApiResult {
            data,
            provenance,
            truncated,
        })
    }

    pub async fn get_dependents(
        &self,
        id: &CanonicalId,
        limit: u32,
        branch: Option<&str>,
    ) -> Result<ApiResult<Vec<GraphEdge>>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let lim = self.clamp_limit(Some(limit));
        let mut data: Vec<GraphEdge> = Vec::new();
        for kind in DEPENDENCY_KINDS {
            if data.len() as u32 >= lim {
                break;
            }
            let remaining = lim.saturating_sub(data.len() as u32);
            let edges = self
                .filter_inbound_edges(id, kind, remaining, branch)
                .await?;
            data.extend(edges);
        }
        data.truncate(lim as usize);
        let truncated = data.len() as u32 >= lim;
        let provenance = provenance_from_edges(&data);
        Ok(ApiResult {
            data,
            provenance,
            truncated,
        })
    }

    async fn locate_repository(&self, repo: &str) -> Result<Option<GraphNode>, ApiError> {
        let candidates = self
            .store
            .find_nodes_by_name(repo, self.clamp_limit(Some(16)))
            .await?;
        Ok(candidates.into_iter().find(|n| {
            n.kind == NodeKind::Repository && (n.name == repo || n.qualified_name == repo)
        }))
    }

    async fn locate_branch(
        &self,
        repo_node: Option<&GraphNode>,
        branch: &str,
    ) -> Result<Option<GraphNode>, ApiError> {
        if let Some(repo) = repo_node {
            let edges = self
                .store
                .get_edges_from(
                    &repo.id,
                    Some(RelationKind::HasBranch),
                    self.clamp_limit(Some(128)),
                )
                .await?;
            for edge in edges {
                if let Some(node) = self.store.get_node(&edge.to).await? {
                    if node.kind == NodeKind::Branch && node.name == branch {
                        return Ok(Some(node));
                    }
                }
            }
            return Ok(None);
        }
        let candidates = self
            .store
            .find_nodes_by_name(branch, self.clamp_limit(Some(16)))
            .await?;
        Ok(candidates
            .into_iter()
            .find(|n| n.kind == NodeKind::Branch && n.name == branch))
    }

    async fn locate_branch_head(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<(Option<GraphNode>, Option<GraphNode>, Option<GraphNode>), ApiError> {
        let repo_node = self.locate_repository(repo).await?;
        let branch_node = self.locate_branch(repo_node.as_ref(), branch).await?;
        let mut commit = None;
        if let Some(branch_node) = &branch_node {
            let edges = self
                .store
                .get_edges_from(&branch_node.id, Some(RelationKind::PointsTo), 1)
                .await?;
            if let Some(edge) = edges.first() {
                commit = self.store.get_node(&edge.to).await?;
            }
        }
        Ok((repo_node, branch_node, commit))
    }

    pub async fn get_branch_context(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<ApiResult<BranchContext>, ApiError> {
        self.check_non_empty(repo, "repo")?;
        self.check_non_empty(branch, "branch")?;
        let (repo_node, branch_node, commit) = self.locate_branch_head(repo, branch).await?;

        let mut state_sample: Vec<GraphNode> = Vec::new();
        let mut truncated = false;
        if let Some(commit) = &commit {
            let lim = self.clamp_limit(None);
            let edges = self
                .store
                .get_edges_from(&commit.id, Some(RelationKind::ContainsStateOf), lim)
                .await?;
            truncated = edges.len() as u32 >= lim;
            for edge in edges {
                if let Some(node) = self.store.get_node(&edge.to).await? {
                    state_sample.push(node);
                }
            }
        }

        let indexed = branch_node.is_some() && commit.is_some();
        let provenance = provenance_from_nodes(
            repo_node
                .iter()
                .chain(branch_node.iter())
                .chain(commit.iter())
                .chain(state_sample.iter()),
        );

        Ok(ApiResult {
            data: BranchContext {
                repository: repo_node,
                branch: branch_node,
                commit,
                state_sample,
                indexed,
            },
            provenance,
            truncated,
        })
    }

    pub async fn get_change_impact(
        &self,
        id: &CanonicalId,
        depth: u8,
        branch: Option<&str>,
    ) -> Result<ApiResult<ChangeImpact>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let budget = self.clamp_limit(None);

        let (call_edges, mut truncated) = self
            .traverse(id, depth, &[RelationKind::Calls], true, budget, branch)
            .await?;

        let remaining = budget.saturating_sub(call_edges.len() as u32);
        let mut dep_edges: Vec<TraversalEdge> = Vec::new();
        if remaining > 0 {
            let (edges, dep_truncated) = self
                .traverse(id, depth, &DEPENDENCY_KINDS, true, remaining, branch)
                .await?;
            dep_edges = edges;
            truncated = truncated || dep_truncated;
        }

        let root = self.store.get_node(id).await?;

        let mut impacted_ids: Vec<CanonicalId> = Vec::new();
        for t in call_edges.iter().chain(dep_edges.iter()) {
            if t.other != *id && !impacted_ids.contains(&t.other) {
                impacted_ids.push(t.other.clone());
            }
        }

        let mut impacted: Vec<GraphNode> = Vec::new();
        for node_id in &impacted_ids {
            if impacted.len() as u32 >= budget {
                truncated = true;
                break;
            }
            if let Some(node) = self.store.get_node(node_id).await? {
                if self.node_allowed(&node, branch).await? {
                    impacted.push(node);
                }
            }
        }

        let callers: Vec<CallEdge> = call_edges
            .into_iter()
            .map(|t| CallEdge {
                edge: t.edge,
                depth: t.depth,
            })
            .collect();
        let dependents: Vec<GraphEdge> = dep_edges.into_iter().map(|t| t.edge).collect();

        let provenance = provenance_from_nodes(root.iter().chain(impacted.iter()))
            .into_iter()
            .chain(provenance_from_edges(
                callers.iter().map(|c| &c.edge).chain(dependents.iter()),
            ))
            .fold(Vec::new(), |mut acc, p| {
                if !acc.contains(&p) {
                    acc.push(p);
                }
                acc
            });

        Ok(ApiResult {
            data: ChangeImpact {
                root,
                callers,
                dependents,
                impacted,
            },
            provenance,
            truncated,
        })
    }

    pub async fn get_deployment_state(
        &self,
        repo: &str,
        env: &str,
    ) -> Result<ApiResult<DeploymentState>, ApiError> {
        self.check_non_empty(repo, "repo")?;
        self.check_non_empty(env, "env")?;

        let environment_node = self
            .store
            .find_nodes_by_name(env, self.clamp_limit(Some(16)))
            .await?
            .into_iter()
            .find(|n| n.kind == NodeKind::DeploymentEnvironment && n.name == env);

        let lim = self.clamp_limit(None);
        let mut deployments: Vec<DeploymentInfo> = Vec::new();
        let mut truncated = false;

        if let Some(environment) = &environment_node {
            let edges = self
                .store
                .get_edges_to(&environment.id, Some(RelationKind::DeployedTo), lim)
                .await?;
            truncated = edges.len() as u32 >= lim;
            for edge in edges {
                let Some(commit) = self.store.get_node(&edge.from).await? else {
                    continue;
                };
                if !node_in_repo(&commit, repo) {
                    continue;
                }
                deployments.push(DeploymentInfo {
                    commit,
                    deployed_at: edge_prop_string(&edge, "deployed_at"),
                    status: edge_prop_string(&edge, "status"),
                    release: edge_prop_string(&edge, "release"),
                    edge,
                });
            }
        }

        let provenance = provenance_from_nodes(environment_node.iter())
            .into_iter()
            .chain(deployments.iter().flat_map(|d| {
                provenance_from_nodes(std::iter::once(&d.commit))
                    .into_iter()
                    .chain(provenance_from_edges(std::iter::once(&d.edge)))
            }))
            .fold(Vec::new(), |mut acc, p| {
                if !acc.contains(&p) {
                    acc.push(p);
                }
                acc
            });

        Ok(ApiResult {
            data: DeploymentState {
                repository: repo.to_string(),
                environment: env.to_string(),
                environment_node,
                deployments,
            },
            provenance,
            truncated,
        })
    }

    async fn node_in_branch(&self, node: &GraphNode, branch: &str) -> Result<bool, ApiError> {
        if let Some(b) = node.location.as_ref().and_then(|loc| loc.branch.as_ref()) {
            if b == branch {
                return Ok(true);
            }
        }
        if let Some(b) = node.properties.get("branch").and_then(|v| v.as_str()) {
            if b == branch {
                return Ok(true);
            }
        }
        let lim = self.clamp_limit(Some(16));
        let containing = self
            .store
            .get_edges_to(&node.id, Some(RelationKind::ContainsStateOf), lim)
            .await?;
        for edge in containing {
            let branch_edges = self
                .store
                .get_edges_to(&edge.from, Some(RelationKind::PointsTo), lim)
                .await?;
            for branch_edge in branch_edges {
                if let Some(branch_node) = self.store.get_node(&branch_edge.from).await? {
                    if branch_node.kind == NodeKind::Branch && branch_node.name == branch {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    pub async fn get_implementation_status(
        &self,
        repo: &str,
        capability_query: &str,
        branches: &[String],
    ) -> Result<ApiResult<ImplementationStatus>, ApiError> {
        self.check_non_empty(repo, "repo")?;
        self.check_non_empty(capability_query, "capability_query")?;

        let lim = self.clamp_limit(None);
        let candidates = self
            .find_symbol(capability_query, Some(repo), lim, None)
            .await?;

        let mut statuses: Vec<BranchStatusEvidence> = Vec::new();
        for branch in branches {
            let (_, branch_node, commit) = self.locate_branch_head(repo, branch).await?;
            let indexed = branch_node.is_some() && commit.is_some();
            let mut matches: Vec<GraphNode> = Vec::new();
            if indexed {
                for node in &candidates.data {
                    if self.node_in_branch(node, branch).await? {
                        matches.push(node.clone());
                    }
                }
            }
            let provenance = provenance_from_nodes(&matches);
            statuses.push(BranchStatusEvidence {
                branch: branch.clone(),
                found: indexed && !matches.is_empty(),
                indexed,
                matches,
                provenance,
            });
        }

        let provenance = provenance_from_nodes(&candidates.data);
        Ok(ApiResult {
            data: ImplementationStatus {
                repository: repo.to_string(),
                capability: capability_query.to_string(),
                branches: statuses,
            },
            provenance,
            truncated: candidates.truncated,
        })
    }

    async fn branch_symbol_names(
        &self,
        repo: &str,
        branch: &str,
    ) -> Result<(Vec<String>, bool, bool), ApiError> {
        let (_repo_node, branch_node, commit) = self.locate_branch_head(repo, branch).await?;
        let indexed = branch_node.is_some() && commit.is_some();
        let Some(commit) = commit else {
            return Ok((Vec::new(), false, indexed));
        };
        let lim = self.limits.branch_set_limit.max(self.limits.max_results);
        let edges = self
            .store
            .get_edges_from(&commit.id, Some(RelationKind::ContainsStateOf), lim)
            .await?;
        let truncated = edges.len() as u32 >= lim;
        let mut names: Vec<String> = Vec::new();
        for edge in edges {
            if let Some(node) = self.store.get_node(&edge.to).await? {
                if is_symbol_kind(node.kind) && !names.contains(&node.name) {
                    names.push(node.name.clone());
                }
            }
        }
        Ok((names, truncated, indexed))
    }

    pub async fn compare_branch_state(
        &self,
        repo: &str,
        branch_a: &str,
        branch_b: &str,
    ) -> Result<ApiResult<BranchComparison>, ApiError> {
        self.check_non_empty(repo, "repo")?;
        self.check_non_empty(branch_a, "branch_a")?;
        self.check_non_empty(branch_b, "branch_b")?;

        let (names_a, trunc_a, indexed_a) = self.branch_symbol_names(repo, branch_a).await?;
        let (names_b, trunc_b, indexed_b) = self.branch_symbol_names(repo, branch_b).await?;

        let set_a: HashSet<String> = names_a.into_iter().collect();
        let set_b: HashSet<String> = names_b.into_iter().collect();

        let mut common: Vec<String> = set_a.intersection(&set_b).cloned().collect();
        let mut only_in_a: Vec<String> = set_a.difference(&set_b).cloned().collect();
        let mut only_in_b: Vec<String> = set_b.difference(&set_a).cloned().collect();
        common.sort();
        only_in_a.sort();
        only_in_b.sort();

        Ok(ApiResult {
            data: BranchComparison {
                repository: repo.to_string(),
                branch_a: branch_a.to_string(),
                branch_b: branch_b.to_string(),
                branch_a_indexed: indexed_a,
                branch_b_indexed: indexed_b,
                common,
                only_in_a,
                only_in_b,
            },
            provenance: Vec::new(),
            truncated: trunc_a || trunc_b,
        })
    }

    async fn traverse_undirected(
        &self,
        start: &CanonicalId,
        depth: u8,
        budget: u32,
        branch: Option<&str>,
    ) -> Result<(Vec<GraphEdge>, bool), ApiError> {
        let eff_depth = self.clamp_depth(depth);
        let mut visited: HashSet<CanonicalId> = HashSet::new();
        visited.insert(start.clone());
        let mut seen_edges: HashSet<(RelationKind, CanonicalId, CanonicalId)> = HashSet::new();
        let mut out: Vec<GraphEdge> = Vec::new();
        let mut frontier: Vec<CanonicalId> = vec![start.clone()];
        let mut truncated = false;
        let mut level: u8 = 0;

        while level < eff_depth {
            if frontier.is_empty() {
                break;
            }
            if out.len() as u32 >= budget {
                truncated = true;
                break;
            }
            level += 1;
            let mut next: Vec<CanonicalId> = Vec::new();
            let mut hit_budget = false;

            'level: for node_id in &frontier {
                let outbound = self.store.get_edges_from(node_id, None, budget).await?;
                for edge in outbound {
                    let key = (edge.kind, edge.from.clone(), edge.to.clone());
                    let other = edge.to.clone();
                    if let Some(branch) = branch {
                        let Some(other_node) = self.store.get_node(&other).await? else {
                            continue;
                        };
                        if !self.node_in_branch(&other_node, branch).await? {
                            continue;
                        }
                    }
                    if !seen_edges.contains(&key) {
                        if out.len() as u32 >= budget {
                            truncated = true;
                            hit_budget = true;
                            break 'level;
                        }
                        seen_edges.insert(key);
                        out.push(edge);
                    }
                    if visited.insert(other.clone()) {
                        next.push(other);
                    }
                }

                let inbound = self.store.get_edges_to(node_id, None, budget).await?;
                for edge in inbound {
                    let key = (edge.kind, edge.from.clone(), edge.to.clone());
                    let other = edge.from.clone();
                    if let Some(branch) = branch {
                        let Some(other_node) = self.store.get_node(&other).await? else {
                            continue;
                        };
                        if !self.node_in_branch(&other_node, branch).await? {
                            continue;
                        }
                    }
                    if !seen_edges.contains(&key) {
                        if out.len() as u32 >= budget {
                            truncated = true;
                            hit_budget = true;
                            break 'level;
                        }
                        seen_edges.insert(key);
                        out.push(edge);
                    }
                    if visited.insert(other.clone()) {
                        next.push(other);
                    }
                }
            }

            frontier = next;
            if hit_budget {
                break;
            }
        }

        if level > 0 && level >= eff_depth && !frontier.is_empty() {
            truncated = true;
        }

        Ok((out, truncated))
    }

    pub async fn get_architecture_context(
        &self,
        id: &CanonicalId,
        depth: u8,
        branch: Option<&str>,
    ) -> Result<ApiResult<ArchitectureContext>, ApiError> {
        self.check_id(id)?;
        self.check_branch(branch)?;
        let budget = self.clamp_limit(None);

        let (edges, mut truncated) = self.traverse_undirected(id, depth, budget, branch).await?;

        let root = self.store.get_node(id).await?;

        let mut neighbor_ids: Vec<CanonicalId> = Vec::new();
        for edge in &edges {
            for endpoint in [&edge.from, &edge.to] {
                if endpoint != id && !neighbor_ids.contains(endpoint) {
                    neighbor_ids.push(endpoint.clone());
                }
            }
        }

        let mut nodes: Vec<GraphNode> = Vec::new();
        for node_id in &neighbor_ids {
            if nodes.len() as u32 >= budget {
                truncated = true;
                break;
            }
            if let Some(node) = self.store.get_node(node_id).await? {
                if self.node_allowed(&node, branch).await? {
                    nodes.push(node);
                }
            }
        }

        let provenance = provenance_from_nodes(root.iter().chain(nodes.iter()))
            .into_iter()
            .chain(provenance_from_edges(&edges))
            .fold(Vec::new(), |mut acc, p| {
                if !acc.contains(&p) {
                    acc.push(p);
                }
                acc
            });

        Ok(ApiResult {
            data: ArchitectureContext { root, nodes, edges },
            provenance,
            truncated,
        })
    }

    pub async fn get_code_context(
        &self,
        ids: &[CanonicalId],
        paths: &[String],
        depth: u8,
        branch: Option<&str>,
    ) -> Result<ApiResult<CodeContext>, ApiError> {
        if ids.is_empty() && paths.is_empty() {
            return Err(ApiError::InvalidArgument(
                "at least one of ids or paths is required".into(),
            ));
        }
        self.check_branch(branch)?;

        let lim = self.clamp_limit(None);
        let mut symbols: Vec<GraphNode> = Vec::new();
        let mut contexts: Vec<ArchitectureContext> = Vec::new();
        for id in ids {
            self.check_id(id)?;
            if let Some(node) = self.store.get_node(id).await? {
                if !self.node_allowed(&node, branch).await? {
                    continue;
                }
                symbols.push(node);
                let context = self.get_architecture_context(id, depth, branch).await?;
                contexts.push(context.data);
            }
        }

        let mut path_matches: Vec<GraphNode> = Vec::new();
        for path in paths {
            self.check_non_empty(path, "path")?;
            let p = Path::new(path.as_str());
            let mut queries: Vec<String> = vec![path.clone()];
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                queries.push(name.to_string());
            }
            if let Some(stem) = p.file_stem().and_then(|n| n.to_str()) {
                queries.push(stem.to_string());
            }
            for query in queries {
                let candidates = self.store.find_nodes_by_name(&query, lim).await?;
                for node in candidates {
                    if path_matches.len() as u32 >= lim {
                        break;
                    }
                    if path_matches.iter().any(|n| n.id == node.id) {
                        continue;
                    }
                    let location_match = node
                        .location
                        .as_ref()
                        .map(|l| l.path == *path || l.path.ends_with(path.as_str()))
                        .unwrap_or(false);
                    let name_match = node.name == *path;
                    if (location_match || name_match) && self.node_allowed(&node, branch).await? {
                        path_matches.push(node);
                    }
                }
            }
        }

        let provenance = provenance_from_nodes(symbols.iter().chain(path_matches.iter()));
        Ok(ApiResult {
            data: CodeContext {
                symbols,
                path_matches,
                contexts,
            },
            provenance,
            truncated: false,
        })
    }

    /// Run a bounded graph analytics query. `id` is required for
    /// `blast_radius` and rejected for the other modes. When `branch` is
    /// given, rows whose nodes are absent from that branch are dropped.
    pub async fn get_dependency_analytics(
        &self,
        id: Option<&CanonicalId>,
        mode: AnalyticsMode,
        depth: Option<u8>,
        limit: Option<u32>,
        branch: Option<&str>,
    ) -> Result<ApiResult<AnalyticsReport>, ApiError> {
        self.check_branch(branch)?;
        match mode {
            AnalyticsMode::BlastRadius => {
                let Some(id) = id else {
                    return Err(ApiError::InvalidArgument(
                        "id is required for blast_radius".into(),
                    ));
                };
                self.check_id(id)?;
            }
            other => {
                if id.is_some() {
                    return Err(ApiError::InvalidArgument(format!(
                        "id is not valid for {} mode",
                        other.as_str()
                    )));
                }
            }
        }
        let spec = AnalyticsSpec {
            mode,
            id: id.cloned(),
            depth: self.clamp_depth(depth.unwrap_or(self.limits.max_depth)),
            limit: self.clamp_limit(limit),
        };
        let mut report = self.store.run_analytics(&spec).await?;
        let mut provenance: Vec<Provenance> = Vec::new();
        if let Some(branch) = branch {
            let mut kept = Vec::new();
            for row in std::mem::take(&mut report.rows) {
                let Some(node) = self.store.get_node(&row.id).await? else {
                    continue;
                };
                if !self.node_in_branch(&node, branch).await? {
                    continue;
                }
                for p in &node.provenance {
                    if !provenance.contains(p) {
                        provenance.push(p.clone());
                    }
                }
                kept.push(row);
            }
            report.rows = kept;
        }
        let truncated = report.truncated;
        Ok(ApiResult {
            data: report,
            provenance,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::{CanonicalIdBuilder, ProvenanceSource, SourceLocation};

    fn commit_id(repo: &str, sha: &str) -> CanonicalId {
        CanonicalIdBuilder::new()
            .repository(repo)
            .container("Commit")
            .symbol(sha)
            .build()
    }

    fn fn_node(id: &str, name: &str) -> GraphNode {
        GraphNode::new(NodeKind::Function, CanonicalId::from(id), name)
    }

    fn calls(from: &str, to: &str) -> GraphEdge {
        GraphEdge::new(
            RelationKind::Calls,
            CanonicalId::from(from),
            CanonicalId::from(to),
        )
    }

    async fn seed(store: &MockStore, ops: Vec<DeltaOp>) {
        let delta = GraphDelta {
            ops,
            base_revision: None,
            target_revision: None,
        };
        store.apply_delta(&delta).await.expect("seed");
    }

    fn call_chain_ops() -> Vec<DeltaOp> {
        let mut ops = Vec::new();
        for i in 1..=5 {
            ops.push(DeltaOp::CreateNode(fn_node(
                &format!("n{i}"),
                &format!("f{i}"),
            )));
        }
        for i in 1..5 {
            ops.push(DeltaOp::CreateRelationship(calls(
                &format!("n{i}"),
                &format!("n{}", i + 1),
            )));
        }
        ops
    }

    #[tokio::test]
    async fn callers_respect_depth_and_max_depth() {
        let store = Arc::new(MockStore::new());
        seed(&store, call_chain_ops()).await;

        let default_api = GraphApi::with_defaults(store.clone());
        let n5 = CanonicalId::from("n5");

        let d0 = default_api.get_callers(&n5, 0, None).await.unwrap();
        assert!(d0.data.is_empty());
        assert!(!d0.truncated);

        let d1 = default_api.get_callers(&n5, 1, None).await.unwrap();
        assert_eq!(d1.data.len(), 1);
        assert_eq!(d1.data[0].depth, 1);
        assert_eq!(d1.data[0].edge.from.as_str(), "n4");
        assert_eq!(d1.data[0].edge.to.as_str(), "n5");

        let d2 = default_api.get_callers(&n5, 2, None).await.unwrap();
        assert_eq!(d2.data.len(), 2);
        assert_eq!(d2.data[1].depth, 2);
        assert_eq!(d2.data[1].edge.from.as_str(), "n3");

        let d99 = default_api.get_callers(&n5, 99, None).await.unwrap();
        assert_eq!(d99.data.len(), 3, "clamped to max_depth 3");
        assert!(d99.truncated, "deeper callers exist beyond max_depth");

        let deep_api = GraphApi::new(
            store.clone(),
            QueryLimits {
                max_depth: 10,
                max_results: 50,
                branch_set_limit: 10_000,
            },
        );
        let full = deep_api.get_callers(&n5, 99, None).await.unwrap();
        assert_eq!(full.data.len(), 4);
        assert!(!full.truncated);

        let shallow_api = GraphApi::new(
            store.clone(),
            QueryLimits {
                max_depth: 2,
                max_results: 50,
                branch_set_limit: 10_000,
            },
        );
        let clamped = shallow_api.get_callers(&n5, 99, None).await.unwrap();
        assert_eq!(clamped.data.len(), 2);
        assert!(clamped.truncated);
    }

    #[tokio::test]
    async fn callees_respect_depth() {
        let store = Arc::new(MockStore::new());
        seed(&store, call_chain_ops()).await;

        let api = GraphApi::with_defaults(store.clone());
        let n1 = CanonicalId::from("n1");

        let d1 = api.get_callees(&n1, 1, None).await.unwrap();
        assert_eq!(d1.data.len(), 1);
        assert_eq!(d1.data[0].edge.from.as_str(), "n1");
        assert_eq!(d1.data[0].edge.to.as_str(), "n2");

        let d3 = api.get_callees(&n1, 3, None).await.unwrap();
        assert_eq!(d3.data.len(), 3);
        let depths: Vec<u8> = d3.data.iter().map(|c| c.depth).collect();
        assert_eq!(depths, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn callers_enforce_max_results() {
        let store = Arc::new(MockStore::new());
        let mut ops = vec![DeltaOp::CreateNode(fn_node("root", "root_fn"))];
        for i in 0..60 {
            ops.push(DeltaOp::CreateNode(fn_node(
                &format!("c{i}"),
                &format!("caller{i}"),
            )));
            ops.push(DeltaOp::CreateRelationship(calls(&format!("c{i}"), "root")));
        }
        seed(&store, ops).await;

        let root = CanonicalId::from("root");

        let default_api = GraphApi::with_defaults(store.clone());
        let result = default_api.get_callers(&root, 1, None).await.unwrap();
        assert_eq!(result.data.len(), 50);
        assert!(result.truncated);

        let small_api = GraphApi::new(
            store.clone(),
            QueryLimits {
                max_depth: 3,
                max_results: 5,
                branch_set_limit: 10_000,
            },
        );
        let small = small_api.get_callers(&root, 1, None).await.unwrap();
        assert_eq!(small.data.len(), 5);
        assert!(small.truncated);
    }

    #[tokio::test]
    async fn empty_results_for_missing_entities() {
        let store = Arc::new(MockStore::new());
        let api = GraphApi::with_defaults(store.clone());

        let callers = api
            .get_callers(&CanonicalId::from("missing"), 3, None)
            .await
            .unwrap();
        assert!(callers.data.is_empty());
        assert!(!callers.truncated);
        assert!(callers.provenance.is_empty());

        let found = api
            .find_symbol("definitely_missing", None, 10, None)
            .await
            .unwrap();
        assert!(found.data.is_empty());

        let symbol = api
            .get_symbol(&CanonicalId::from("missing"), None)
            .await
            .unwrap();
        assert!(symbol.data.is_none());

        let deployment = api
            .get_deployment_state("org/missing", "production")
            .await
            .unwrap();
        assert!(deployment.data.deployments.is_empty());
        assert!(deployment.data.environment_node.is_none());

        let context = api.get_branch_context("org/missing", "main").await.unwrap();
        assert!(context.data.repository.is_none());
        assert!(context.data.branch.is_none());
        assert!(context.data.commit.is_none());
        assert!(context.data.state_sample.is_empty());
        assert!(
            !context.data.indexed,
            "missing branch must report indexed:false"
        );
    }

    #[tokio::test]
    async fn find_symbol_filters_by_repo_and_enforces_limit() {
        let store = Arc::new(MockStore::new());
        let mut ops = Vec::new();
        for (id, repo) in [("s1", "org/a"), ("s2", "org/b")] {
            ops.push(DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Function, CanonicalId::from(id), "shared")
                    .with_location(SourceLocation::new(repo, "sha", "src/lib.rs", 1, 0, 2, 0)),
            ));
        }
        for i in 0..60 {
            ops.push(DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Function,
                CanonicalId::from(format!("d{i}")),
                "dup",
            )));
        }
        seed(&store, ops).await;

        let api = GraphApi::with_defaults(store.clone());

        let all = api.find_symbol("shared", None, 10, None).await.unwrap();
        assert_eq!(all.data.len(), 2);

        let only_a = api
            .find_symbol("shared", Some("org/a"), 10, None)
            .await
            .unwrap();
        assert_eq!(only_a.data.len(), 1);
        assert_eq!(only_a.data[0].id.as_str(), "s1");

        let only_b = api
            .find_symbol("shared", Some("org/b"), 10, None)
            .await
            .unwrap();
        assert_eq!(only_b.data.len(), 1);
        assert_eq!(only_b.data[0].id.as_str(), "s2");

        let none = api
            .find_symbol("shared", Some("org/c"), 10, None)
            .await
            .unwrap();
        assert!(none.data.is_empty());

        let limited = api.find_symbol("dup", None, 1000, None).await.unwrap();
        assert_eq!(limited.data.len(), 50, "clamped to max_results 50");

        let tighter = api.find_symbol("dup", None, 5, None).await.unwrap();
        assert_eq!(tighter.data.len(), 5);
    }

    #[tokio::test]
    async fn store_failure_maps_to_api_error() {
        let store = Arc::new(MockStore::new());
        store.set_fail(true);
        let api = GraphApi::with_defaults(store.clone());

        let err = api.find_symbol("x", None, 10, None).await.unwrap_err();
        assert!(matches!(err, ApiError::Store(_)));

        let err = api
            .get_symbol(&CanonicalId::from("x"), None)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::Store(_)));
    }

    #[tokio::test]
    async fn invalid_arguments_rejected() {
        let store = Arc::new(MockStore::new());
        let api = GraphApi::with_defaults(store.clone());

        let err = api.find_symbol("", None, 10, None).await.unwrap_err();
        assert!(matches!(err, ApiError::InvalidArgument(_)));

        let err = api
            .get_symbol(&CanonicalId::from(""), None)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::InvalidArgument(_)));

        let err = api.get_code_context(&[], &[], 3, None).await.unwrap_err();
        assert!(matches!(err, ApiError::InvalidArgument(_)));
    }

    async fn seed_branch_graph(store: &MockStore) {
        let repo_id = CanonicalId::from("repo-org-pay");
        let ops = vec![
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Repository, repo_id.clone(), "org/pay")
                    .with_property("repository", serde_json::json!("org/pay"))
                    .with_provenance(Provenance::from(ProvenanceSource::Git)),
            ),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-main"),
                "main",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-dev"),
                "develop",
            )),
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Commit,
                    commit_id("org/pay", "sha-main"),
                    "sha-main",
                )
                .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, commit_id("org/pay", "sha-dev"), "sha-dev")
                    .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::HasBranch,
                repo_id,
                CanonicalId::from("br-main"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::HasBranch,
                CanonicalId::from("repo-org-pay"),
                CanonicalId::from("br-dev"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-main"),
                commit_id("org/pay", "sha-main"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-dev"),
                commit_id("org/pay", "sha-dev"),
            )),
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("sym-legacy"),
                    "legacy_parse",
                )
                .with_location(
                    SourceLocation::new("org/pay", "sha-main", "src/legacy.rs", 1, 0, 10, 0)
                        .with_branch("main"),
                ),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("sym-common"),
                    "common_util",
                )
                .with_location(
                    SourceLocation::new("org/pay", "sha-dev", "src/common.rs", 1, 0, 5, 0)
                        .with_branch("develop"),
                ),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Function,
                    CanonicalId::from("sym-parse"),
                    "parse_config",
                )
                .with_location(SourceLocation::new(
                    "org/pay",
                    "sha-dev",
                    "src/parse.rs",
                    1,
                    0,
                    20,
                    0,
                ))
                .with_provenance(Provenance::from(ProvenanceSource::TreeSitter)),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_id("org/pay", "sha-main"),
                CanonicalId::from("sym-legacy"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_id("org/pay", "sha-main"),
                CanonicalId::from("sym-common"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_id("org/pay", "sha-dev"),
                CanonicalId::from("sym-common"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_id("org/pay", "sha-dev"),
                CanonicalId::from("sym-parse"),
            )),
        ];
        seed(store, ops).await;
    }

    #[tokio::test]
    async fn branch_context_resolves_repository_branch_commit_and_sample() {
        let store = Arc::new(MockStore::new());
        seed_branch_graph(&store).await;
        let api = GraphApi::with_defaults(store.clone());

        let result = api.get_branch_context("org/pay", "main").await.unwrap();
        let context = &result.data;
        let repo = context.repository.as_ref().expect("repository");
        assert_eq!(repo.name, "org/pay");
        let branch = context.branch.as_ref().expect("branch");
        assert_eq!(branch.name, "main");
        let commit = context.commit.as_ref().expect("commit");
        assert_eq!(commit.name, "sha-main");
        assert_eq!(context.state_sample.len(), 2);
        assert!(context.indexed);
        assert!(!result.provenance.is_empty());

        let missing = api.get_branch_context("org/pay", "nope").await.unwrap();
        assert!(missing.data.branch.is_none());
        assert!(missing.data.commit.is_none());
        assert!(missing.data.state_sample.is_empty());
        assert!(
            !missing.data.indexed,
            "missing branch must report indexed:false"
        );
    }

    #[tokio::test]
    async fn compare_branch_state_reports_missing_branch_as_not_indexed() {
        let store = Arc::new(MockStore::new());
        seed_branch_graph(&store).await;
        let api = GraphApi::with_defaults(store.clone());

        let result = api
            .compare_branch_state("org/pay", "main", "nope")
            .await
            .unwrap();

        assert!(result.data.branch_a_indexed);
        assert!(
            !result.data.branch_b_indexed,
            "missing branch must report indexed:false"
        );
        assert!(result.data.only_in_b.is_empty());
    }

    #[tokio::test]
    async fn compare_branch_state_reports_intersection_and_differences() {
        let store = Arc::new(MockStore::new());
        seed_branch_graph(&store).await;
        let api = GraphApi::with_defaults(store.clone());

        let result = api
            .compare_branch_state("org/pay", "main", "develop")
            .await
            .unwrap();

        assert_eq!(result.data.common, vec!["common_util".to_string()]);
        assert_eq!(result.data.only_in_a, vec!["legacy_parse".to_string()]);
        assert_eq!(result.data.only_in_b, vec!["parse_config".to_string()]);
        assert!(result.data.branch_a_indexed);
        assert!(result.data.branch_b_indexed);
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn implementation_status_reports_per_branch_evidence() {
        let store = Arc::new(MockStore::new());
        seed_branch_graph(&store).await;
        let api = GraphApi::with_defaults(store.clone());

        let branches = vec!["develop".to_string(), "main".to_string()];
        let result = api
            .get_implementation_status("org/pay", "parse_config", &branches)
            .await
            .unwrap();

        assert_eq!(result.data.repository, "org/pay");
        assert_eq!(result.data.capability, "parse_config");
        assert_eq!(result.data.branches.len(), 2);

        let develop = &result.data.branches[0];
        assert_eq!(develop.branch, "develop");
        assert!(develop.indexed);
        assert!(develop.found);
        assert_eq!(develop.matches.len(), 1);
        assert_eq!(develop.matches[0].id.as_str(), "sym-parse");

        let main = &result.data.branches[1];
        assert_eq!(main.branch, "main");
        assert!(main.indexed, "main exists in the graph");
        assert!(!main.found);
        assert!(main.matches.is_empty());
    }

    #[tokio::test]
    async fn implementation_status_marks_missing_branch_not_indexed() {
        let store = Arc::new(MockStore::new());
        seed_branch_graph(&store).await;
        let api = GraphApi::with_defaults(store.clone());

        let branches = vec!["nope".to_string()];
        let result = api
            .get_implementation_status("org/pay", "parse_config", &branches)
            .await
            .unwrap();

        let evidence = &result.data.branches[0];
        assert_eq!(evidence.branch, "nope");
        assert!(
            !evidence.indexed,
            "missing branch must report indexed:false, not merely found:false"
        );
        assert!(!evidence.found);
        assert!(evidence.matches.is_empty());
    }

    #[tokio::test]
    async fn find_symbol_filters_by_branch() {
        let store = Arc::new(MockStore::new());
        let mut ops = Vec::new();
        for (id, branch) in [("s-main", "main"), ("s-dev", "develop")] {
            ops.push(DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Function, CanonicalId::from(id), "shared").with_location(
                    SourceLocation::new("org/r", "sha", "src/lib.rs", 1, 0, 2, 0)
                        .with_branch(branch),
                ),
            ));
        }
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let all = api.find_symbol("shared", None, 10, None).await.unwrap();
        assert_eq!(all.data.len(), 2);

        let on_main = api
            .find_symbol("shared", None, 10, Some("main"))
            .await
            .unwrap();
        assert_eq!(on_main.data.len(), 1);
        assert_eq!(on_main.data[0].id.as_str(), "s-main");

        let on_dev = api
            .find_symbol("shared", None, 10, Some("develop"))
            .await
            .unwrap();
        assert_eq!(on_dev.data.len(), 1);
        assert_eq!(on_dev.data[0].id.as_str(), "s-dev");

        let on_other = api
            .find_symbol("shared", None, 10, Some("release"))
            .await
            .unwrap();
        assert!(on_other.data.is_empty());
    }

    #[tokio::test]
    async fn get_callers_filters_by_branch() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Function, CanonicalId::from("callee"), "callee")
                    .with_location(
                        SourceLocation::new("org/r", "sha", "src/c.rs", 1, 0, 2, 0)
                            .with_branch("main"),
                    ),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Function, CanonicalId::from("caller-main"), "cm")
                    .with_location(
                        SourceLocation::new("org/r", "sha", "src/a.rs", 1, 0, 2, 0)
                            .with_branch("main"),
                    ),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Function, CanonicalId::from("caller-dev"), "cd")
                    .with_location(
                        SourceLocation::new("org/r", "sha", "src/b.rs", 1, 0, 2, 0)
                            .with_branch("develop"),
                    ),
            ),
            DeltaOp::CreateRelationship(calls("caller-main", "callee")),
            DeltaOp::CreateRelationship(calls("caller-dev", "callee")),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());
        let callee = CanonicalId::from("callee");

        let all = api.get_callers(&callee, 1, None).await.unwrap();
        assert_eq!(all.data.len(), 2);

        let on_main = api.get_callers(&callee, 1, Some("main")).await.unwrap();
        assert_eq!(on_main.data.len(), 1);
        assert_eq!(on_main.data[0].edge.from.as_str(), "caller-main");

        let on_dev = api.get_callers(&callee, 1, Some("develop")).await.unwrap();
        assert_eq!(on_dev.data.len(), 1);
        assert_eq!(on_dev.data[0].edge.from.as_str(), "caller-dev");

        let on_other = api.get_callers(&callee, 1, Some("release")).await.unwrap();
        assert!(on_other.data.is_empty());
    }

    #[tokio::test]
    async fn develop_has_symbol_main_does_not_and_production_not_deployed() {
        let store = Arc::new(MockStore::new());
        seed_branch_graph(&store).await;
        seed(
            &store,
            vec![DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::DeploymentEnvironment,
                    CanonicalId::from("env-prod-scenario"),
                    "production",
                )
                .with_property("is_production", serde_json::json!(true)),
            )],
        )
        .await;

        let api = GraphApi::with_defaults(store.clone());

        let branches = vec!["develop".to_string(), "main".to_string()];
        let status = api
            .get_implementation_status("org/pay", "parse_config", &branches)
            .await
            .unwrap();
        assert_eq!(status.data.repository, "org/pay");
        assert_eq!(status.data.capability, "parse_config");

        let develop = status
            .data
            .branches
            .iter()
            .find(|b| b.branch == "develop")
            .expect("develop evidence");
        assert!(develop.found, "develop must have parse_config");
        assert!(develop.indexed);
        assert_eq!(develop.matches.len(), 1);
        assert_eq!(develop.matches[0].name, "parse_config");
        assert!(!develop.provenance.is_empty());

        let main = status
            .data
            .branches
            .iter()
            .find(|b| b.branch == "main")
            .expect("main evidence");
        assert!(main.indexed);
        assert!(!main.found, "main must not have parse_config");
        assert!(main.matches.is_empty());

        let deployment = api
            .get_deployment_state("org/pay", "production")
            .await
            .unwrap();
        assert_eq!(deployment.data.repository, "org/pay");
        assert_eq!(deployment.data.environment, "production");
        let environment = deployment
            .data
            .environment_node
            .as_ref()
            .expect("production environment node exists");
        assert_eq!(environment.name, "production");
        assert!(
            deployment.data.deployments.is_empty(),
            "production must have no deployments for org/pay"
        );
    }

    #[tokio::test]
    async fn deployment_state_reports_only_deployed_commits_for_repo() {
        let store = Arc::new(MockStore::new());
        let env_prod = CanonicalId::from("env-prod");
        let env_staging = CanonicalId::from("env-staging");

        let ops = vec![
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::DeploymentEnvironment,
                    env_prod.clone(),
                    "production",
                )
                .with_property("is_production", serde_json::json!(true)),
            ),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::DeploymentEnvironment,
                env_staging.clone(),
                "staging",
            )),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, commit_id("org/pay", "sha1"), "sha1")
                    .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, commit_id("org/pay", "sha2"), "sha2")
                    .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, commit_id("org/other", "sha3"), "sha3")
                    .with_property("repository", serde_json::json!("org/other")),
            ),
            DeltaOp::CreateRelationship(
                GraphEdge::new(
                    RelationKind::DeployedTo,
                    commit_id("org/pay", "sha1"),
                    env_prod.clone(),
                )
                .with_property("status", serde_json::json!("SUCCEEDED"))
                .with_property("deployed_at", serde_json::json!("2026-01-01T00:00:00Z"))
                .with_property("release", serde_json::json!("v1.2.3"))
                .with_provenance(
                    Provenance::from(ProvenanceSource::DeploymentSystem)
                        .with_detail("github-actions"),
                ),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::DeployedTo,
                commit_id("org/other", "sha3"),
                env_prod,
            )),
        ];
        seed(&store, ops).await;

        let api = GraphApi::with_defaults(store.clone());

        let production = api
            .get_deployment_state("org/pay", "production")
            .await
            .unwrap();
        assert_eq!(production.data.repository, "org/pay");
        assert_eq!(production.data.environment, "production");
        let env_node = production.data.environment_node.as_ref().expect("env node");
        assert_eq!(env_node.name, "production");
        assert_eq!(production.data.deployments.len(), 1);
        let info = &production.data.deployments[0];
        assert_eq!(info.commit.name, "sha1");
        assert_eq!(info.status.as_deref(), Some("SUCCEEDED"));
        assert_eq!(info.deployed_at.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(info.release.as_deref(), Some("v1.2.3"));
        assert!(!production.provenance.is_empty());
        assert_eq!(
            production.provenance[0].source,
            ProvenanceSource::DeploymentSystem
        );

        let other_repo = api
            .get_deployment_state("org/other", "production")
            .await
            .unwrap();
        assert_eq!(other_repo.data.deployments.len(), 1);
        assert_eq!(other_repo.data.deployments[0].commit.name, "sha3");

        let nope = api
            .get_deployment_state("org/nope", "production")
            .await
            .unwrap();
        assert!(nope.data.deployments.is_empty());

        let staging = api
            .get_deployment_state("org/pay", "staging")
            .await
            .unwrap();
        assert_eq!(
            staging
                .data
                .environment_node
                .as_ref()
                .map(|n| n.name.as_str()),
            Some("staging")
        );
        assert!(
            staging.data.deployments.is_empty(),
            "no commit is deployed to staging"
        );
    }

    #[tokio::test]
    async fn change_impact_unions_callers_and_dependents() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(fn_node("root", "root_fn")),
            DeltaOp::CreateNode(fn_node("caller1", "caller_fn")),
            DeltaOp::CreateNode(fn_node("dep1", "dependent_mod")),
            DeltaOp::CreateRelationship(calls("caller1", "root")),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::DependsOn,
                CanonicalId::from("dep1"),
                CanonicalId::from("root"),
            )),
        ];
        seed(&store, ops).await;

        let api = GraphApi::with_defaults(store.clone());
        let result = api
            .get_change_impact(&CanonicalId::from("root"), 2, None)
            .await
            .unwrap();

        let impact = &result.data;
        assert_eq!(
            impact.root.as_ref().map(|n| n.name.as_str()),
            Some("root_fn")
        );
        assert_eq!(impact.callers.len(), 1);
        assert_eq!(impact.callers[0].edge.from.as_str(), "caller1");
        assert_eq!(impact.dependents.len(), 1);
        assert_eq!(impact.dependents[0].from.as_str(), "dep1");

        let impacted_ids: Vec<&str> = impact.impacted.iter().map(|n| n.id.as_str()).collect();
        assert!(impacted_ids.contains(&"caller1"));
        assert!(impacted_ids.contains(&"dep1"));
        assert!(!impacted_ids.contains(&"root"));
    }

    #[tokio::test]
    async fn change_impact_enforces_total_result_limit() {
        let store = Arc::new(MockStore::new());
        let mut ops = vec![DeltaOp::CreateNode(fn_node("root", "root_fn"))];
        for i in 0..40 {
            ops.push(DeltaOp::CreateNode(fn_node(
                &format!("c{i}"),
                &format!("c{i}"),
            )));
            ops.push(DeltaOp::CreateRelationship(calls(&format!("c{i}"), "root")));
        }
        for i in 0..40 {
            ops.push(DeltaOp::CreateNode(fn_node(
                &format!("d{i}"),
                &format!("d{i}"),
            )));
            ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::DependsOn,
                CanonicalId::from(format!("d{i}")),
                CanonicalId::from("root"),
            )));
        }
        seed(&store, ops).await;

        let api = GraphApi::with_defaults(store.clone());
        let result = api
            .get_change_impact(&CanonicalId::from("root"), 3, None)
            .await
            .unwrap();

        let total = result.data.callers.len() + result.data.dependents.len();
        assert!(total <= 50, "combined edges {total} exceed max_results");
        assert!(result.truncated);
    }

    #[tokio::test]
    async fn architecture_context_collects_neighbors_within_depth() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(
                GraphNode::new(
                    NodeKind::Service,
                    CanonicalId::from("svc"),
                    "payment-service",
                )
                .with_provenance(Provenance::from(ProvenanceSource::Manual)),
            ),
            DeltaOp::CreateNode(fn_node("fn1", "handler")),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Api,
                CanonicalId::from("api1"),
                "charge_api",
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::Exposes,
                CanonicalId::from("svc"),
                CanonicalId::from("api1"),
            )),
            DeltaOp::CreateRelationship(calls("svc", "fn1")),
        ];
        seed(&store, ops).await;

        let api = GraphApi::with_defaults(store.clone());

        let depth1 = api
            .get_architecture_context(&CanonicalId::from("svc"), 1, None)
            .await
            .unwrap();
        assert!(depth1.data.root.is_some());
        assert_eq!(depth1.data.edges.len(), 2);
        assert_eq!(depth1.data.nodes.len(), 2);
        assert!(!depth1.provenance.is_empty());

        let depth0 = api
            .get_architecture_context(&CanonicalId::from("svc"), 0, None)
            .await
            .unwrap();
        assert!(depth0.data.edges.is_empty());
        assert!(depth0.data.nodes.is_empty());
        assert!(!depth0.truncated);
    }

    #[tokio::test]
    async fn references_implementations_dependencies_and_dependents() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(fn_node("target", "target_fn")),
            DeltaOp::CreateNode(fn_node("user", "user_fn")),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Interface,
                CanonicalId::from("iface"),
                "Parser",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Class,
                CanonicalId::from("impl1"),
                "JsonParser",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Module,
                CanonicalId::from("mod-a"),
                "mod_a",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Module,
                CanonicalId::from("mod-b"),
                "mod_b",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Repository,
                CanonicalId::from("repo-x"),
                "org/x",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Repository,
                CanonicalId::from("repo-y"),
                "org/y",
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::References,
                CanonicalId::from("user"),
                CanonicalId::from("target"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::Implements,
                CanonicalId::from("impl1"),
                CanonicalId::from("iface"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::DependsOn,
                CanonicalId::from("mod-a"),
                CanonicalId::from("mod-b"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::Imports,
                CanonicalId::from("mod-a"),
                CanonicalId::from("target"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::DependsOn,
                CanonicalId::from("repo-y"),
                CanonicalId::from("repo-x"),
            )),
        ];
        seed(&store, ops).await;

        let api = GraphApi::with_defaults(store.clone());

        let refs = api
            .get_references(&CanonicalId::from("target"), 10, None)
            .await
            .unwrap();
        assert_eq!(refs.data.len(), 1);
        assert_eq!(refs.data[0].from.as_str(), "user");

        let impls = api
            .get_implementations(&CanonicalId::from("iface"), 10, None)
            .await
            .unwrap();
        assert_eq!(impls.data.len(), 1);
        assert_eq!(impls.data[0].from.as_str(), "impl1");

        let deps = api
            .get_dependencies(&CanonicalId::from("mod-a"), 10, None)
            .await
            .unwrap();
        assert_eq!(deps.data.len(), 2);
        assert_eq!(deps.data[0].kind, RelationKind::DependsOn);
        assert_eq!(deps.data[1].kind, RelationKind::Imports);

        let dependents = api
            .get_dependents(&CanonicalId::from("mod-b"), 10, None)
            .await
            .unwrap();
        assert_eq!(dependents.data.len(), 1);
        assert_eq!(dependents.data[0].from.as_str(), "mod-a");

        let repo_dependents = api
            .get_dependents(&CanonicalId::from("repo-x"), 10, None)
            .await
            .unwrap();
        assert_eq!(repo_dependents.data.len(), 1);
        assert_eq!(repo_dependents.data[0].from.as_str(), "repo-y");

        let capped = api
            .get_references(&CanonicalId::from("target"), 1000, None)
            .await
            .unwrap();
        assert_eq!(capped.data.len(), 1);
    }

    #[tokio::test]
    async fn code_context_resolves_ids_and_paths() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(fn_node("sym-main", "main")),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Function, CanonicalId::from("sym-lib"), "helper")
                    .with_qualified_name("src/lib.rs")
                    .with_location(SourceLocation::new(
                        "org/pay",
                        "sha",
                        "src/lib.rs",
                        3,
                        0,
                        9,
                        0,
                    )),
            ),
            DeltaOp::CreateRelationship(calls("sym-main", "sym-lib")),
        ];
        seed(&store, ops).await;

        let api = GraphApi::with_defaults(store.clone());

        let context = api
            .get_code_context(
                &[CanonicalId::from("sym-main")],
                &["src/lib.rs".to_string()],
                2,
                None,
            )
            .await
            .unwrap();

        assert_eq!(context.data.symbols.len(), 1);
        assert_eq!(context.data.symbols[0].name, "main");
        assert_eq!(context.data.contexts.len(), 1);
        assert_eq!(context.data.contexts[0].edges.len(), 1);
        assert_eq!(context.data.path_matches.len(), 1);
        assert_eq!(context.data.path_matches[0].id.as_str(), "sym-lib");

        let missing = api
            .get_code_context(&[CanonicalId::from("nope")], &[], 2, None)
            .await
            .unwrap();
        assert!(missing.data.symbols.is_empty());
        assert!(missing.data.contexts.is_empty());
    }

    #[tokio::test]
    async fn get_dependencies_and_dependents_include_service_dependency_kinds() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(fn_node("consumer", "consumer_fn")),
            DeltaOp::CreateNode(fn_node("dep-mod", "dep_mod")),
            DeltaOp::CreateNode(fn_node("target", "target_fn")),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Api,
                CanonicalId::from("api-p"),
                "charge_api",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Topic,
                CanonicalId::from("topic-1"),
                "orders.created",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Topic,
                CanonicalId::from("queue-1"),
                "billing.q",
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::DependsOn,
                CanonicalId::from("consumer"),
                CanonicalId::from("dep-mod"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::Imports,
                CanonicalId::from("consumer"),
                CanonicalId::from("target"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::CallsApi,
                CanonicalId::from("consumer"),
                CanonicalId::from("api-p"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PublishTo,
                CanonicalId::from("consumer"),
                CanonicalId::from("topic-1"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ConsumeFrom,
                CanonicalId::from("consumer"),
                CanonicalId::from("queue-1"),
            )),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());
        let consumer = CanonicalId::from("consumer");

        let deps = api.get_dependencies(&consumer, 10, None).await.unwrap();
        let kinds: Vec<RelationKind> = deps.data.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RelationKind::DependsOn,
                RelationKind::Imports,
                RelationKind::CallsApi,
                RelationKind::PublishTo,
                RelationKind::ConsumeFrom,
            ]
        );
        assert!(!deps.truncated);

        let capped = api.get_dependencies(&consumer, 2, None).await.unwrap();
        let capped_kinds: Vec<RelationKind> = capped.data.iter().map(|e| e.kind).collect();
        assert_eq!(
            capped_kinds,
            vec![RelationKind::DependsOn, RelationKind::Imports],
            "new kinds fill remaining budget after DependsOn and Imports"
        );
        assert!(capped.truncated);

        let partial = api.get_dependencies(&consumer, 4, None).await.unwrap();
        let partial_kinds: Vec<RelationKind> = partial.data.iter().map(|e| e.kind).collect();
        assert_eq!(
            partial_kinds,
            vec![
                RelationKind::DependsOn,
                RelationKind::Imports,
                RelationKind::CallsApi,
                RelationKind::PublishTo,
            ]
        );

        for (id, expected) in [
            ("dep-mod", RelationKind::DependsOn),
            ("target", RelationKind::Imports),
            ("api-p", RelationKind::CallsApi),
            ("topic-1", RelationKind::PublishTo),
            ("queue-1", RelationKind::ConsumeFrom),
        ] {
            let dependents = api
                .get_dependents(&CanonicalId::from(id), 10, None)
                .await
                .unwrap();
            assert_eq!(dependents.data.len(), 1, "dependents of {id}");
            assert_eq!(dependents.data[0].kind, expected);
            assert_eq!(dependents.data[0].from.as_str(), "consumer");
        }
    }

    #[tokio::test]
    async fn get_dependencies_includes_resource_dependency_kinds() {
        let store = Arc::new(MockStore::new());
        let mut ops = vec![DeltaOp::CreateNode(fn_node("app", "app_fn"))];
        let targets = [
            ("db-1", "orders-db", RelationKind::ConnectsTo),
            ("bucket-1", "orders-exports", RelationKind::ReadsFrom),
            ("bucket-2", "orders-raw", RelationKind::WritesTo),
            ("fn-1", "billing-worker", RelationKind::Invokes),
            ("ds-1", "analytics.orders", RelationKind::Queries),
        ];
        for (id, name, _) in targets {
            ops.push(DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Resource,
                CanonicalId::from(id),
                name,
            )));
        }
        for (id, _, kind) in targets {
            ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
                kind,
                CanonicalId::from("app"),
                CanonicalId::from(id),
            )));
        }
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let deps = api
            .get_dependencies(&CanonicalId::from("app"), 10, None)
            .await
            .unwrap();
        let kinds: Vec<RelationKind> = deps.data.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RelationKind::ReadsFrom,
                RelationKind::WritesTo,
                RelationKind::Invokes,
                RelationKind::ConnectsTo,
                RelationKind::Queries,
            ],
            "all five resource kinds are dependency kinds"
        );

        let capped = api
            .get_dependencies(&CanonicalId::from("app"), 2, None)
            .await
            .unwrap();
        assert_eq!(
            capped.data.iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![RelationKind::ReadsFrom, RelationKind::WritesTo],
            "resource kinds respect the total result limit"
        );
        assert!(capped.truncated);

        let impact = api
            .get_change_impact(&CanonicalId::from("bucket-1"), 1, None)
            .await
            .unwrap();
        let impacted: Vec<&str> = impact.data.impacted.iter().map(|n| n.id.as_str()).collect();
        assert!(
            impacted.contains(&"app"),
            "inbound ReadsFrom counts as change impact"
        );
    }

    async fn seed_service_edge_graph(store: &MockStore) {
        seed_branch_graph(store).await;
        let mut ops = vec![
            DeltaOp::CreateNode(fn_node("svc-consumer", "consumer_fn")),
            DeltaOp::CreateNode(fn_node("svc-provider", "provider_fn")),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Topic,
                CanonicalId::from("svc-topic"),
                "orders.created",
            )),
        ];
        for node_id in ["svc-consumer", "svc-provider", "svc-topic"] {
            for sha in ["sha-main", "sha-dev"] {
                ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
                    RelationKind::ContainsStateOf,
                    commit_id("org/pay", sha),
                    CanonicalId::from(node_id),
                )));
            }
        }
        ops.push(DeltaOp::CreateRelationship(
            GraphEdge::new(
                RelationKind::CallsApi,
                CanonicalId::from("svc-consumer"),
                CanonicalId::from("svc-provider"),
            )
            .with_property("branch_pairs", serde_json::json!(["main"])),
        ));
        ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
            RelationKind::PublishTo,
            CanonicalId::from("svc-consumer"),
            CanonicalId::from("svc-topic"),
        )));
        seed(store, ops).await;
    }

    #[tokio::test]
    async fn branch_pairs_filters_service_edges_per_branch() {
        let store = Arc::new(MockStore::new());
        seed_service_edge_graph(&store).await;
        let api = GraphApi::with_defaults(store.clone());
        let consumer = CanonicalId::from("svc-consumer");
        let provider = CanonicalId::from("svc-provider");

        let consumer_node = store.get_node(&consumer).await.unwrap().unwrap();
        assert!(
            api.node_in_branch(&consumer_node, "main").await.unwrap(),
            "endpoint passes node_in_branch for main"
        );
        assert!(
            api.node_in_branch(&consumer_node, "develop").await.unwrap(),
            "endpoint passes node_in_branch for develop"
        );

        let raw = store
            .get_edges_from(&consumer, Some(RelationKind::CallsApi), 10)
            .await
            .unwrap();
        assert_eq!(raw.len(), 1, "mock store round-trips service edges");
        assert_eq!(
            raw[0].properties.get("branch_pairs"),
            Some(&serde_json::json!(["main"])),
            "mock store preserves edge properties for branch_pairs"
        );

        let all = api.get_dependencies(&consumer, 10, None).await.unwrap();
        assert_eq!(all.data.len(), 2, "no branch filter unions all edges");

        let on_main = api
            .get_dependencies(&consumer, 10, Some("main"))
            .await
            .unwrap();
        assert_eq!(on_main.data.len(), 2, "branch_pairs contains main");

        let on_dev = api
            .get_dependencies(&consumer, 10, Some("develop"))
            .await
            .unwrap();
        assert_eq!(on_dev.data.len(), 1, "branch_pairs excludes develop");
        assert_eq!(on_dev.data[0].kind, RelationKind::PublishTo);

        let dependents_main = api
            .get_dependents(&provider, 10, Some("main"))
            .await
            .unwrap();
        assert_eq!(dependents_main.data.len(), 1);

        let dependents_dev = api
            .get_dependents(&provider, 10, Some("develop"))
            .await
            .unwrap();
        assert!(
            dependents_dev.data.is_empty(),
            "CALLS_API edge without develop in branch_pairs is filtered"
        );
    }

    #[tokio::test]
    async fn topic_node_in_branch_via_contains_state_of_walk() {
        let store = Arc::new(MockStore::new());
        let topic_id = CanonicalId::from("topic-orders");
        let ops = vec![
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-main"),
                "main",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Branch,
                CanonicalId::from("br-dev"),
                "develop",
            )),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, commit_id("org/pay", "sha1"), "sha1")
                    .with_property("repository", serde_json::json!("org/pay")),
            ),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Topic,
                topic_id.clone(),
                "orders.created",
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-main"),
                commit_id("org/pay", "sha1"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                commit_id("org/pay", "sha1"),
                topic_id.clone(),
            )),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let topic = store.get_node(&topic_id).await.unwrap().unwrap();
        assert_eq!(topic.location, None, "topic nodes carry no location");
        assert!(
            api.node_in_branch(&topic, "main").await.unwrap(),
            "topic reachable from main via ContainsStateOf walk"
        );
        assert!(
            !api.node_in_branch(&topic, "develop").await.unwrap(),
            "no develop linkage for the topic"
        );

        let on_main = api
            .find_symbol("orders.created", None, 10, Some("main"))
            .await
            .unwrap();
        assert_eq!(on_main.data.len(), 1);

        let on_dev = api
            .find_symbol("orders.created", None, 10, Some("develop"))
            .await
            .unwrap();
        assert!(on_dev.data.is_empty());
    }

    #[tokio::test]
    async fn change_impact_includes_inbound_service_dependencies() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(fn_node("handler", "handler_fn")),
            DeltaOp::CreateNode(fn_node("remote-consumer", "remote_fn")),
            DeltaOp::CreateNode(fn_node("subscriber", "subscriber_fn")),
            DeltaOp::CreateRelationship(
                GraphEdge::new(
                    RelationKind::CallsApi,
                    CanonicalId::from("remote-consumer"),
                    CanonicalId::from("handler"),
                )
                .with_property("branch_pairs", serde_json::json!(["main"])),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ConsumeFrom,
                CanonicalId::from("subscriber"),
                CanonicalId::from("handler"),
            )),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let result = api
            .get_change_impact(&CanonicalId::from("handler"), 1, None)
            .await
            .unwrap();
        let kinds: Vec<RelationKind> = result.data.dependents.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&RelationKind::CallsApi));
        assert!(kinds.contains(&RelationKind::ConsumeFrom));
        let impacted: Vec<&str> = result.data.impacted.iter().map(|n| n.id.as_str()).collect();
        assert!(impacted.contains(&"remote-consumer"));
        assert!(impacted.contains(&"subscriber"));
    }

    #[tokio::test]
    async fn architecture_context_includes_service_dependency_edges() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Service,
                CanonicalId::from("svc-a"),
                "billing",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Service,
                CanonicalId::from("svc-b"),
                "ledger",
            )),
            DeltaOp::CreateNode(GraphNode::new(
                NodeKind::Topic,
                CanonicalId::from("topic-a"),
                "ledger.posted",
            )),
            DeltaOp::CreateRelationship(
                GraphEdge::new(
                    RelationKind::CallsApi,
                    CanonicalId::from("svc-a"),
                    CanonicalId::from("svc-b"),
                )
                .with_property("branch_pairs", serde_json::json!(["main", "develop"])),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PublishTo,
                CanonicalId::from("svc-a"),
                CanonicalId::from("topic-a"),
            )),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let context = api
            .get_architecture_context(&CanonicalId::from("svc-a"), 1, None)
            .await
            .unwrap();
        let kinds: Vec<RelationKind> = context.data.edges.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&RelationKind::CallsApi));
        assert!(kinds.contains(&RelationKind::PublishTo));
        assert_eq!(context.data.nodes.len(), 2);
    }

    fn resource_node(id: &str, name: &str) -> GraphNode {
        GraphNode::new(NodeKind::Resource, CanonicalId::from(id), name)
    }

    fn resource_edge(from: &str, to: &str, kind: RelationKind) -> GraphEdge {
        GraphEdge::new(kind, CanonicalId::from(from), CanonicalId::from(to))
    }

    #[tokio::test]
    async fn blast_radius_reports_transitive_dependents_with_distances() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(fn_node("app", "app_fn")),
            DeltaOp::CreateNode(fn_node("worker", "worker_fn")),
            DeltaOp::CreateNode(resource_node("db-1", "postgres://orders-db")),
            DeltaOp::CreateNode(resource_node("bucket-1", "s3://unrelated")),
            DeltaOp::CreateRelationship(resource_edge("app", "db-1", RelationKind::ConnectsTo)),
            DeltaOp::CreateRelationship(resource_edge("worker", "app", RelationKind::DependsOn)),
            DeltaOp::CreateRelationship(resource_edge("bucket-1", "db-1", RelationKind::Queries)),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let result = api
            .get_dependency_analytics(
                Some(&CanonicalId::from("db-1")),
                AnalyticsMode::BlastRadius,
                Some(3),
                None,
                None,
            )
            .await
            .unwrap();
        let rows = &result.data.rows;
        assert_eq!(rows.len(), 3, "app, worker and bucket-1 all depend on db-1");
        assert_eq!(rows[0].id.as_str(), "app");
        assert_eq!(rows[0].distance, Some(1));
        assert_eq!(rows[1].id.as_str(), "bucket-1");
        assert_eq!(rows[1].distance, Some(1));
        assert_eq!(rows[2].id.as_str(), "worker");
        assert_eq!(rows[2].distance, Some(2));
        assert_eq!(result.data.engine, AnalyticsEngine::Cypher);
        assert!(result.data.degraded);
        assert!(result.data.note.is_some());

        let shallow = api
            .get_dependency_analytics(
                Some(&CanonicalId::from("db-1")),
                AnalyticsMode::BlastRadius,
                Some(1),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(shallow.data.rows.len(), 2, "depth 1 stops before worker");
    }

    #[tokio::test]
    async fn critical_resources_ranks_by_dependency_degree() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(fn_node("a", "a_fn")),
            DeltaOp::CreateNode(fn_node("b", "b_fn")),
            DeltaOp::CreateNode(resource_node("db-1", "orders-db")),
            DeltaOp::CreateNode(resource_node("bucket-1", "orders-exports")),
            DeltaOp::CreateRelationship(resource_edge("a", "db-1", RelationKind::ConnectsTo)),
            DeltaOp::CreateRelationship(resource_edge("a", "db-1", RelationKind::ReadsFrom)),
            DeltaOp::CreateRelationship(resource_edge("b", "db-1", RelationKind::WritesTo)),
            DeltaOp::CreateRelationship(resource_edge("b", "bucket-1", RelationKind::WritesTo)),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let result = api
            .get_dependency_analytics(None, AnalyticsMode::CriticalResources, None, None, None)
            .await
            .unwrap();
        let rows = &result.data.rows;
        assert_eq!(rows.len(), 2, "only Resource nodes are ranked");
        assert_eq!(rows[0].id.as_str(), "db-1");
        assert_eq!(rows[0].score, 3.0);
        assert_eq!(rows[1].id.as_str(), "bucket-1");
        assert_eq!(rows[1].score, 1.0);
        assert!(result.data.degraded);
    }

    #[tokio::test]
    async fn analytics_validates_id_per_mode() {
        let store = Arc::new(MockStore::new());
        let api = GraphApi::with_defaults(store.clone());

        let err = api
            .get_dependency_analytics(None, AnalyticsMode::BlastRadius, None, None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("id is required"));

        let err = api
            .get_dependency_analytics(
                Some(&CanonicalId::from("db-1")),
                AnalyticsMode::Clusters,
                None,
                None,
                None,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not valid"));
    }

    #[tokio::test]
    async fn bridges_and_clusters_skip_without_gds() {
        let store = Arc::new(MockStore::new());
        seed(&store, vec![DeltaOp::CreateNode(fn_node("a", "a_fn"))]).await;
        let api = GraphApi::with_defaults(store.clone());

        for mode in [AnalyticsMode::Bridges, AnalyticsMode::Clusters] {
            let result = api
                .get_dependency_analytics(None, mode, None, None, None)
                .await
                .unwrap();
            assert_eq!(result.data.mode, mode);
            assert_eq!(result.data.engine, AnalyticsEngine::Gds);
            assert!(result.data.degraded);
            assert!(result.data.rows.is_empty());
            assert!(result.data.note.is_some());
        }
    }

    #[tokio::test]
    async fn analytics_branch_filter_drops_nodes_outside_branch() {
        let store = Arc::new(MockStore::new());
        let ops = vec![
            DeltaOp::CreateNode(
                fn_node("on-main", "main_fn").with_property("branch", serde_json::json!("main")),
            ),
            DeltaOp::CreateNode(
                fn_node("on-dev", "dev_fn").with_property("branch", serde_json::json!("develop")),
            ),
            DeltaOp::CreateRelationship(
                GraphEdge::new(
                    RelationKind::DependsOn,
                    CanonicalId::from("on-main"),
                    CanonicalId::from("db-1"),
                )
                .with_property("branch_pairs", serde_json::json!(["main"])),
            ),
            DeltaOp::CreateRelationship(
                GraphEdge::new(
                    RelationKind::DependsOn,
                    CanonicalId::from("on-dev"),
                    CanonicalId::from("db-1"),
                )
                .with_property("branch_pairs", serde_json::json!(["develop"])),
            ),
        ];
        seed(&store, ops).await;
        let api = GraphApi::with_defaults(store.clone());

        let unfiltered = api
            .get_dependency_analytics(
                Some(&CanonicalId::from("db-1")),
                AnalyticsMode::BlastRadius,
                Some(1),
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(unfiltered.data.rows.len(), 2);

        let filtered = api
            .get_dependency_analytics(
                Some(&CanonicalId::from("db-1")),
                AnalyticsMode::BlastRadius,
                Some(1),
                None,
                Some("main"),
            )
            .await
            .unwrap();
        let ids: Vec<&str> = filtered.data.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["on-main"]);
    }
}
