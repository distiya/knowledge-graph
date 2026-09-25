use std::path::{Path, PathBuf};

use ckg_domain::{
    CanonicalId, CanonicalIdBuilder, GraphEdge, GraphNode, NodeKind, Provenance, ProvenanceSource,
    RelationKind,
};
use ckg_graph_delta::{DeltaOp, GraphDelta};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DeploymentStatus {
    #[serde(alias = "Succeeded")]
    Succeeded,
    #[serde(alias = "Failed")]
    Failed,
    #[serde(alias = "InProgress")]
    InProgress,
    #[serde(alias = "Unknown")]
    Unknown,
}

impl DeploymentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "SUCCEEDED",
            Self::Failed => "FAILED",
            Self::InProgress => "IN_PROGRESS",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentEnvironment {
    pub name: String,
    pub is_production: bool,
}

impl DeploymentEnvironment {
    pub fn new(name: impl Into<String>, is_production: bool) -> Self {
        Self {
            name: name.into(),
            is_production,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentRecord {
    pub environment: String,
    pub repository: String,
    pub commit_sha: String,
    pub deployed_at: String,
    pub status: DeploymentStatus,
    pub source: String,
    #[serde(default)]
    pub release: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum DeploymentError {
    #[error("failed to read deployment file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse deployment file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub trait DeploymentImporter {
    fn load(&self) -> Result<Vec<DeploymentRecord>, DeploymentError>;
}

#[derive(Debug, Clone)]
pub struct InMemoryDeploymentSource {
    records: Vec<DeploymentRecord>,
}

impl InMemoryDeploymentSource {
    pub fn new(records: Vec<DeploymentRecord>) -> Self {
        Self { records }
    }

    pub fn from_records(records: impl IntoIterator<Item = DeploymentRecord>) -> Self {
        Self {
            records: records.into_iter().collect(),
        }
    }
}

impl DeploymentImporter for InMemoryDeploymentSource {
    fn load(&self) -> Result<Vec<DeploymentRecord>, DeploymentError> {
        Ok(self.records.clone())
    }
}

#[derive(Debug, Clone)]
pub struct JsonDeploymentSource {
    path: PathBuf,
}

impl JsonDeploymentSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl DeploymentImporter for JsonDeploymentSource {
    fn load(&self) -> Result<Vec<DeploymentRecord>, DeploymentError> {
        let text = std::fs::read_to_string(&self.path).map_err(|source| DeploymentError::Io {
            path: self.path.clone(),
            source,
        })?;
        serde_json::from_str(&text).map_err(|source| DeploymentError::Parse {
            path: self.path.clone(),
            source,
        })
    }
}

pub fn environment_id(name: &str) -> CanonicalId {
    CanonicalIdBuilder::new()
        .namespace("deployment-environment")
        .symbol(name)
        .build()
}

pub fn commit_id(repository: &str, commit_sha: &str) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(repository)
        .container("Commit")
        .symbol(commit_sha)
        .build()
}

pub fn deployment_record_id(record: &DeploymentRecord) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(record.repository.as_str())
        .namespace("deployment")
        .symbol(record.environment.as_str())
        .signature(format!("{}@{}", record.commit_sha, record.deployed_at))
        .build()
}

fn record_provenance(record: &DeploymentRecord) -> Provenance {
    Provenance::from(ProvenanceSource::DeploymentSystem).with_detail(record.source.clone())
}

pub fn environment_node(environment: &DeploymentEnvironment) -> GraphNode {
    GraphNode::new(
        NodeKind::DeploymentEnvironment,
        environment_id(&environment.name),
        environment.name.clone(),
    )
    .with_qualified_name(environment.name.clone())
    .with_property("name", serde_json::Value::String(environment.name.clone()))
    .with_property(
        "is_production",
        serde_json::Value::Bool(environment.is_production),
    )
    .with_provenance(
        Provenance::from(ProvenanceSource::DeploymentSystem).with_detail("deployment importer"),
    )
}

pub fn deployment_node(record: &DeploymentRecord) -> GraphNode {
    let name = format!("{}@{}", record.repository, record.environment);
    let qualified_name = format!(
        "{}@{}:{}",
        record.repository, record.environment, record.commit_sha
    );
    let mut node = GraphNode::new(NodeKind::Deployment, deployment_record_id(record), name)
        .with_qualified_name(qualified_name)
        .with_property(
            "repository",
            serde_json::Value::String(record.repository.clone()),
        )
        .with_property(
            "environment",
            serde_json::Value::String(record.environment.clone()),
        )
        .with_property(
            "commit_sha",
            serde_json::Value::String(record.commit_sha.clone()),
        )
        .with_property(
            "deployed_at",
            serde_json::Value::String(record.deployed_at.clone()),
        )
        .with_property(
            "status",
            serde_json::Value::String(record.status.as_str().to_string()),
        )
        .with_property("source", serde_json::Value::String(record.source.clone()));
    if let Some(release) = &record.release {
        node = node.with_property("release", serde_json::Value::String(release.clone()));
    }
    node.with_provenance(record_provenance(record))
}

pub fn deployment_edge(record: &DeploymentRecord) -> GraphEdge {
    let mut edge = GraphEdge::new(
        RelationKind::DeployedTo,
        commit_id(&record.repository, &record.commit_sha),
        environment_id(&record.environment),
    )
    .with_property(
        "status",
        serde_json::Value::String(record.status.as_str().to_string()),
    )
    .with_property(
        "deployed_at",
        serde_json::Value::String(record.deployed_at.clone()),
    )
    .with_property("source", serde_json::Value::String(record.source.clone()))
    .with_property(
        "repository",
        serde_json::Value::String(record.repository.clone()),
    )
    .with_property(
        "commit_sha",
        serde_json::Value::String(record.commit_sha.clone()),
    );
    if let Some(release) = &record.release {
        edge = edge.with_property("release", serde_json::Value::String(release.clone()));
    }
    edge.with_provenance(record_provenance(record))
}

pub fn deployment_delta(
    records: &[DeploymentRecord],
    environments: &[DeploymentEnvironment],
) -> GraphDelta {
    let mut ops = Vec::new();
    let mut seen_environments: Vec<String> = Vec::new();

    for environment in environments {
        if !seen_environments.contains(&environment.name) {
            seen_environments.push(environment.name.clone());
            ops.push(DeltaOp::CreateNode(environment_node(environment)));
        }
    }

    for record in records {
        if !seen_environments.contains(&record.environment) {
            seen_environments.push(record.environment.clone());
            ops.push(DeltaOp::CreateNode(environment_node(
                &DeploymentEnvironment {
                    name: record.environment.clone(),
                    is_production: false,
                },
            )));
        }
        ops.push(DeltaOp::CreateNode(deployment_node(record)));
        ops.push(DeltaOp::CreateRelationship(deployment_edge(record)));
    }

    GraphDelta {
        ops,
        base_revision: None,
        target_revision: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str, is_production: bool) -> DeploymentEnvironment {
        DeploymentEnvironment::new(name, is_production)
    }

    fn record(environment: &str, commit_sha: &str) -> DeploymentRecord {
        DeploymentRecord {
            environment: environment.to_string(),
            repository: "org/payments".to_string(),
            commit_sha: commit_sha.to_string(),
            deployed_at: "2026-01-01T00:00:00Z".to_string(),
            status: DeploymentStatus::Succeeded,
            source: "github-actions".to_string(),
            release: Some("v1.2.3".to_string()),
        }
    }

    fn create_node_ops(delta: &GraphDelta, kind: NodeKind) -> Vec<GraphNode> {
        delta
            .ops
            .iter()
            .filter_map(|op| match op {
                DeltaOp::CreateNode(node) if node.kind == kind => Some(node.clone()),
                _ => None,
            })
            .collect()
    }

    fn relationship_ops(delta: &GraphDelta) -> Vec<GraphEdge> {
        delta
            .ops
            .iter()
            .filter_map(|op| match op {
                DeltaOp::CreateRelationship(edge) => Some(edge.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn records_produce_environment_deployment_and_edge_ops() {
        let environments = vec![env("production", true), env("staging", false)];
        let records = vec![record("production", "abc123"), record("staging", "def456")];

        let delta = deployment_delta(&records, &environments);

        let environment_nodes = create_node_ops(&delta, NodeKind::DeploymentEnvironment);
        let deployment_nodes = create_node_ops(&delta, NodeKind::Deployment);
        let edges = relationship_ops(&delta);

        assert_eq!(delta.ops.len(), 6);
        assert_eq!(environment_nodes.len(), 2);
        assert_eq!(deployment_nodes.len(), 2);
        assert_eq!(edges.len(), 2);
        assert!(delta.base_revision.is_none());
        assert!(delta.target_revision.is_none());

        for edge in &edges {
            assert_eq!(edge.kind, RelationKind::DeployedTo);
            assert_eq!(
                edge.to,
                environment_id(
                    if edge.properties.get("commit_sha").and_then(|v| v.as_str()) == Some("abc123")
                    {
                        "production"
                    } else {
                        "staging"
                    }
                )
            );
        }

        let edge_abc = edges
            .iter()
            .find(|e| e.properties.get("commit_sha").and_then(|v| v.as_str()) == Some("abc123"))
            .expect("edge for abc123");
        assert_eq!(edge_abc.from, commit_id("org/payments", "abc123"));
        assert_eq!(
            edge_abc.properties.get("status").and_then(|v| v.as_str()),
            Some("SUCCEEDED")
        );
        assert_eq!(
            edge_abc.properties.get("source").and_then(|v| v.as_str()),
            Some("github-actions")
        );
        assert_eq!(
            edge_abc.properties.get("release").and_then(|v| v.as_str()),
            Some("v1.2.3")
        );
        assert!(!edge_abc.provenance.is_empty());
        assert_eq!(
            edge_abc.provenance[0].source,
            ProvenanceSource::DeploymentSystem
        );

        let dep = &deployment_nodes[0];
        assert_eq!(
            dep.properties.get("repository").and_then(|v| v.as_str()),
            Some("org/payments")
        );
        assert!(!dep.provenance.is_empty());
    }

    #[test]
    fn production_flag_is_preserved_on_environment_nodes() {
        let environments = vec![env("production", true), env("staging", false)];
        let delta = deployment_delta(&[], &environments);

        let nodes = create_node_ops(&delta, NodeKind::DeploymentEnvironment);
        assert_eq!(nodes.len(), 2);

        let production = nodes
            .iter()
            .find(|n| n.name == "production")
            .expect("production env node");
        let staging = nodes
            .iter()
            .find(|n| n.name == "staging")
            .expect("staging env node");

        assert_eq!(
            production.properties.get("is_production"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            staging.properties.get("is_production"),
            Some(&serde_json::Value::Bool(false))
        );
        assert_eq!(production.kind, NodeKind::DeploymentEnvironment);
        assert_ne!(production.id, staging.id);
    }

    #[test]
    fn two_environments_are_distinguishable() {
        let environments = vec![env("production", true), env("staging", false)];
        let records = vec![record("production", "abc123"), record("staging", "def456")];

        let delta = deployment_delta(&records, &environments);

        let edges = relationship_ops(&delta);
        assert_eq!(edges.len(), 2);

        let to_production = environment_id("production");
        let to_staging = environment_id("staging");
        assert_ne!(to_production, to_staging);

        let edge_abc = edges
            .iter()
            .find(|e| e.from == commit_id("org/payments", "abc123"))
            .expect("abc123 edge");
        let edge_def = edges
            .iter()
            .find(|e| e.from == commit_id("org/payments", "def456"))
            .expect("def456 edge");
        assert_eq!(edge_abc.to, to_production);
        assert_eq!(edge_def.to, to_staging);

        let deployments = create_node_ops(&delta, NodeKind::Deployment);
        let environments_in_deployments: Vec<&str> = deployments
            .iter()
            .map(|d| {
                d.properties
                    .get("environment")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
            })
            .collect();
        assert!(environments_in_deployments.contains(&"production"));
        assert!(environments_in_deployments.contains(&"staging"));
    }

    #[test]
    fn record_referencing_unknown_environment_still_emits_edge() {
        let delta = deployment_delta(&[record("canary", "abc123")], &[]);

        let environment_nodes = create_node_ops(&delta, NodeKind::DeploymentEnvironment);
        assert_eq!(environment_nodes.len(), 1);
        assert_eq!(environment_nodes[0].name, "canary");
        assert_eq!(
            environment_nodes[0].properties.get("is_production"),
            Some(&serde_json::Value::Bool(false))
        );
        let edges = relationship_ops(&delta);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, environment_id("canary"));
    }

    #[test]
    fn in_memory_source_loads_records() {
        let source = InMemoryDeploymentSource::new(vec![record("production", "abc123")]);
        let importer: &dyn DeploymentImporter = &source;
        let loaded = importer.load().expect("in-memory load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].environment, "production");
        assert_eq!(loaded[0].status, DeploymentStatus::Succeeded);
    }

    #[test]
    fn json_source_loads_records_from_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("deployments.json");
        let json = r#"[
            {
                "environment": "production",
                "repository": "org/payments",
                "commit_sha": "abc123",
                "deployed_at": "2026-01-01T00:00:00Z",
                "status": "SUCCEEDED",
                "source": "github-actions",
                "release": "v1.0.0"
            },
            {
                "environment": "staging",
                "repository": "org/payments",
                "commit_sha": "def456",
                "deployed_at": "2026-01-02T00:00:00Z",
                "status": "IN_PROGRESS",
                "source": "github-actions"
            },
            {
                "environment": "production",
                "repository": "org/payments",
                "commit_sha": "ghi789",
                "deployed_at": "2026-01-03T00:00:00Z",
                "status": "Failed",
                "source": "argo-cd"
            }
        ]"#;
        std::fs::write(&path, json).expect("write json");

        let source = JsonDeploymentSource::new(&path);
        let loaded = source.load().expect("json load");

        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].status, DeploymentStatus::Succeeded);
        assert_eq!(loaded[0].release.as_deref(), Some("v1.0.0"));
        assert_eq!(loaded[1].status, DeploymentStatus::InProgress);
        assert!(loaded[1].release.is_none());
        assert_eq!(loaded[2].status, DeploymentStatus::Failed);
        assert_eq!(loaded[2].source, "argo-cd");
    }

    #[test]
    fn json_source_reports_missing_file() {
        let source = JsonDeploymentSource::new("/nonexistent/deployments.json");
        let err = source.load().expect_err("should fail");
        assert!(matches!(err, DeploymentError::Io { .. }));
    }

    #[test]
    fn json_source_reports_invalid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").expect("write");
        let source = JsonDeploymentSource::new(&path);
        let err = source.load().expect_err("should fail");
        assert!(matches!(err, DeploymentError::Parse { .. }));
    }

    #[test]
    fn status_serializes_as_screaming_snake_case() {
        assert_eq!(
            serde_json::to_value(DeploymentStatus::Succeeded).unwrap(),
            serde_json::json!("SUCCEEDED")
        );
        assert_eq!(
            serde_json::to_value(DeploymentStatus::InProgress).unwrap(),
            serde_json::json!("IN_PROGRESS")
        );
        let status: DeploymentStatus =
            serde_json::from_value(serde_json::json!("Unknown")).expect("deserialize alias");
        assert_eq!(status, DeploymentStatus::Unknown);
    }

    #[test]
    fn delta_ids_are_deterministic() {
        let environments = vec![env("production", true)];
        let records = vec![record("production", "abc123")];
        let first = deployment_delta(&records, &environments);
        let second = deployment_delta(&records, &environments);
        assert_eq!(first.ops.len(), second.ops.len());
        for (a, b) in first.ops.iter().zip(second.ops.iter()) {
            match (a, b) {
                (DeltaOp::CreateNode(na), DeltaOp::CreateNode(nb)) => {
                    assert_eq!(na.id, nb.id)
                }
                (DeltaOp::CreateRelationship(ea), DeltaOp::CreateRelationship(eb)) => {
                    assert_eq!(ea.from, eb.from);
                    assert_eq!(ea.to, eb.to);
                }
                _ => panic!("mismatched op kinds"),
            }
        }
    }
}
