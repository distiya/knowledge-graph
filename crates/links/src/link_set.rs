use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ckg_domain::{
    CanonicalId, ContractBatch, GraphEdge, GraphNode, NodeKind, Provenance, ProvenanceSource,
    RelationKind,
};
use ckg_graph_delta::relation_kind_str;
use serde::{Deserialize, Serialize};

/// Path-safe segment for cache sub-directories (mirrors `state::maps_path`).
pub fn safe_segment(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Per repo+branch contract storage path: `{cache}/contracts/{repo}/{branch}.json`.
pub fn contracts_path(cache_dir: &Path, repo: &str, branch: &str) -> PathBuf {
    cache_dir
        .join("contracts")
        .join(safe_segment(repo))
        .join(format!("{}.json", safe_segment(branch)))
}

pub fn load_contracts(path: &Path) -> Result<Option<ContractBatch>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read contracts {}", path.display()))?;
    let batch: ContractBatch = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse contracts {}", path.display()))?;
    Ok(Some(batch))
}

pub fn save_contracts(path: &Path, batch: &ContractBatch) -> Result<()> {
    write_json(path, batch, "contracts")
}

/// Workspace-level link state: `{cache}/links.json`.
pub fn link_set_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("links.json")
}

pub fn load_link_set(path: &Path) -> Result<LinkSet> {
    if !path.exists() {
        return Ok(LinkSet::default());
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read link set {}", path.display()))?;
    let mut set: LinkSet = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse link set {}", path.display()))?;
    set.sort();
    Ok(set)
}

pub fn save_link_set(path: &Path, set: &LinkSet) -> Result<()> {
    write_json(path, set, "link set")
}

fn write_json<T: Serialize>(path: &Path, value: &T, what: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize {what}"))?;
    std::fs::write(path, raw)
        .with_context(|| format!("failed to write {what} {}", path.display()))?;
    Ok(())
}

/// Shared provenance stamped on every fact produced by the link pass.
pub fn link_provenance() -> Provenance {
    Provenance::from(ProvenanceSource::Manual).with_detail("workspace link pass")
}

/// One cross-repository service relationship (CallsApi / PublishTo /
/// ConsumeFrom), persisted in [`LinkSet`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEdge {
    pub kind: RelationKind,
    pub from: CanonicalId,
    pub to: CanonicalId,
    /// Edge properties excluding `branch_pairs` (those live in
    /// [`LinkEdge::branch_pairs`]; `content_hash` covers this map only).
    #[serde(default)]
    pub properties: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub branch_pairs: Vec<String>,
}

impl LinkEdge {
    pub fn key(&self) -> (String, String, String) {
        (
            relation_kind_str(self.kind),
            self.from.to_string(),
            self.to.to_string(),
        )
    }

    /// Deterministic hash over `properties` (without `branch_pairs`).
    pub fn content_hash(&self) -> String {
        let raw = serde_json::to_vec(&self.properties).unwrap_or_default();
        ckg_domain::content_hash(&raw)
    }

    pub fn to_graph_edge(&self) -> GraphEdge {
        let mut edge = GraphEdge::new(self.kind, self.from.clone(), self.to.clone())
            .with_content_hash(self.content_hash())
            .with_provenance(link_provenance());
        edge.properties = self.graph_properties();
        edge
    }

    /// Full property map as stored on the graph edge: `properties` plus
    /// `branch_pairs`.
    pub fn graph_properties(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut props = self.properties.clone();
        props.insert(
            "branch_pairs".to_string(),
            serde_json::Value::Array(
                self.branch_pairs
                    .iter()
                    .map(|p| serde_json::Value::String(p.clone()))
                    .collect(),
            ),
        );
        props
    }
}

/// Shared messaging hub (topic/queue) referenced by one or more branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicState {
    pub id: CanonicalId,
    #[serde(default)]
    pub broker: String,
    pub channel: String,
    #[serde(default)]
    pub channel_type: String,
    /// Commit ids (from `BranchHead::commit_id`) that contain this topic.
    #[serde(default)]
    pub commits: Vec<String>,
}

impl TopicState {
    pub fn to_graph_node(&self) -> GraphNode {
        let qualified = if self.broker.is_empty() {
            self.channel.clone()
        } else {
            format!("{}/{}", self.broker, self.channel)
        };
        let mut node = GraphNode::new(NodeKind::Topic, self.id.clone(), self.channel.clone())
            .with_qualified_name(qualified)
            .with_provenance(link_provenance())
            .with_property("channel", serde_json::Value::String(self.channel.clone()));
        if !self.broker.is_empty() {
            node = node.with_property("broker", serde_json::Value::String(self.broker.clone()));
        }
        if !self.channel_type.is_empty() {
            node = node.with_property(
                "channel_type",
                serde_json::Value::String(self.channel_type.clone()),
            );
        }
        node
    }

    /// Whether the node-visible content (properties) differs from `other`.
    pub fn content_differs(&self, other: &TopicState) -> bool {
        self.broker != other.broker
            || self.channel_type != other.channel_type
            || self.channel != other.channel
    }
}

/// External resource (database, bucket, function, dataset, table, api)
/// referenced by one or more branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceState {
    pub id: CanonicalId,
    /// Resource class: database | bucket | function | dataset | table | api.
    pub resource_type: String,
    /// Canonical identity (normalized: credentials stripped, host lowered).
    pub identity: String,
    /// Provider/technology discriminator (s3 | postgres | lambda | ...).
    #[serde(default)]
    pub mechanism: String,
    /// Commit ids (from `BranchHead::commit_id`) that reference this resource.
    #[serde(default)]
    pub commits: Vec<String>,
}

impl ResourceState {
    pub fn to_graph_node(&self) -> GraphNode {
        let mut node = GraphNode::new(NodeKind::Resource, self.id.clone(), self.identity.clone())
            .with_qualified_name(format!("{}:{}", self.resource_type, self.identity))
            .with_provenance(link_provenance())
            .with_property(
                "resource_type",
                serde_json::Value::String(self.resource_type.clone()),
            );
        if !self.mechanism.is_empty() {
            node = node.with_property(
                "mechanism",
                serde_json::Value::String(self.mechanism.clone()),
            );
        }
        node
    }

    /// Whether the node-visible content (properties) differs from `other`.
    pub fn content_differs(&self, other: &ResourceState) -> bool {
        self.resource_type != other.resource_type
            || self.identity != other.identity
            || self.mechanism != other.mechanism
    }
}

/// Persisted result of the workspace link pass (`{cache}/links.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkSet {
    #[serde(default)]
    pub edges: Vec<LinkEdge>,
    #[serde(default)]
    pub topics: Vec<TopicState>,
    #[serde(default)]
    pub resources: Vec<ResourceState>,
}

impl LinkSet {
    pub fn sort(&mut self) {
        self.edges.sort_by_key(|edge| edge.key());
        self.topics.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        self.resources
            .sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty() && self.topics.is_empty() && self.resources.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contracts_path_sanitizes_repo_and_branch() {
        let path = contracts_path(Path::new("/cache"), "org/payments", "feature/x");
        assert_eq!(
            path,
            Path::new("/cache/contracts/org_payments/feature_x.json")
        );
    }

    #[test]
    fn contracts_roundtrip_and_missing_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let path = contracts_path(tmp.path(), "org/repo", "main");
        assert!(load_contracts(&path).unwrap().is_none());

        let mut batch = ContractBatch::default();
        batch.evidence.push(ckg_domain::EvidenceRecord {
            file: ".env".to_string(),
            kind: ckg_domain::EvidenceKind::EnvFile,
            value: "https://api.orders.internal".to_string(),
            detail: String::new(),
            target_repo: String::new(),
            channel: String::new(),
            broker: String::new(),
            line: 1,
        });
        save_contracts(&path, &batch).unwrap();
        let loaded = load_contracts(&path).unwrap().expect("saved batch");
        assert_eq!(loaded, batch);
    }

    #[test]
    fn link_set_roundtrip_and_missing_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = link_set_path(tmp.path());
        let mut set = load_link_set(&path).unwrap();
        assert!(set.is_empty());

        set.edges.push(LinkEdge {
            kind: RelationKind::CallsApi,
            from: CanonicalId::from("caller"),
            to: CanonicalId::from("handler"),
            properties: Default::default(),
            branch_pairs: vec!["a@main -> b@main".to_string()],
        });
        set.topics.push(TopicState {
            id: CanonicalId::from("topic-1"),
            broker: "kafka".to_string(),
            channel: "orders.created".to_string(),
            channel_type: "topic".to_string(),
            commits: vec!["c1".to_string()],
        });
        set.resources.push(ResourceState {
            id: CanonicalId::from("resource-1"),
            resource_type: "bucket".to_string(),
            identity: "orders-exports".to_string(),
            mechanism: "s3".to_string(),
            commits: vec!["c1".to_string()],
        });
        save_link_set(&path, &set).unwrap();

        let loaded = load_link_set(&path).unwrap();
        assert_eq!(loaded, set);
    }

    #[test]
    fn resource_node_projection_has_provenance_and_props() {
        let resource = ResourceState {
            id: CanonicalId::from("r1"),
            resource_type: "database".to_string(),
            identity: "db.internal/app".to_string(),
            mechanism: "postgres".to_string(),
            commits: vec![],
        };
        let node = resource.to_graph_node();
        assert_eq!(node.kind, NodeKind::Resource);
        assert_eq!(node.name, "db.internal/app");
        assert_eq!(node.qualified_name, "database:db.internal/app");
        assert_eq!(
            node.properties.get("resource_type"),
            Some(&serde_json::json!("database"))
        );
        assert_eq!(
            node.properties.get("mechanism"),
            Some(&serde_json::json!("postgres"))
        );
        assert_eq!(node.language, None);
        assert_eq!(node.location, None);
        assert_eq!(node.provenance.len(), 1);
        assert_eq!(node.provenance[0].source, ProvenanceSource::Manual);
        assert_eq!(
            node.provenance[0].detail.as_deref(),
            Some("workspace link pass")
        );
    }

    #[test]
    fn resource_content_differs_tracks_visible_properties() {
        let base = ResourceState {
            id: CanonicalId::from("r1"),
            resource_type: "bucket".to_string(),
            identity: "orders".to_string(),
            mechanism: "s3".to_string(),
            commits: vec!["c1".to_string()],
        };
        assert!(!base.content_differs(&base.clone()));
        let mut changed = base.clone();
        changed.mechanism = "gcs".to_string();
        assert!(base.content_differs(&changed));
        let mut same_content = base.clone();
        same_content.commits = vec!["c2".to_string()];
        assert!(!base.content_differs(&same_content));
    }

    #[test]
    fn content_hash_is_deterministic() {
        let mut edge = LinkEdge {
            kind: RelationKind::CallsApi,
            from: CanonicalId::from("a"),
            to: CanonicalId::from("b"),
            properties: serde_json::Map::new(),
            branch_pairs: vec!["x@main -> y@main".to_string()],
        };
        edge.properties
            .insert("confidence".to_string(), serde_json::json!("high"));
        let hash1 = edge.content_hash();
        let hash2 = edge.content_hash();
        assert_eq!(hash1, hash2, "content_hash must be deterministic");
    }

    #[test]
    fn topic_node_projection_has_provenance_and_props() {
        let topic = TopicState {
            id: CanonicalId::from("t1"),
            broker: "kafka".to_string(),
            channel: "orders.created".to_string(),
            channel_type: "topic".to_string(),
            commits: vec![],
        };
        let node = topic.to_graph_node();
        assert_eq!(node.kind, NodeKind::Topic);
        assert_eq!(node.name, "orders.created");
        assert_eq!(node.qualified_name, "kafka/orders.created");
        assert_eq!(
            node.properties.get("broker"),
            Some(&serde_json::json!("kafka"))
        );
        assert_eq!(node.language, None);
        assert_eq!(node.location, None);
        assert_eq!(node.provenance.len(), 1);
        assert_eq!(node.provenance[0].source, ProvenanceSource::Manual);
        assert_eq!(
            node.provenance[0].detail.as_deref(),
            Some("workspace link pass")
        );
    }
}
