use serde::{Deserialize, Serialize};

use crate::identity::CanonicalId;
use crate::location::SourceLocation;
use crate::provenance::Provenance;

/// Canonical node kinds in the enterprise knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NodeKind {
    Organization,
    Repository,
    Branch,
    Commit,
    File,
    Package,
    Module,
    Class,
    Interface,
    Function,
    Method,
    Symbol,
    Variable,
    Api,
    Service,
    Database,
    DeploymentEnvironment,
    Deployment,
    Release,
    AnalyzerRun,
    Topic,
    Resource,
}

/// A canonical graph node produced by normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: CanonicalId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub properties: serde_json::Map<String, serde_json::Value>,
    pub provenance: Vec<Provenance>,
}

impl GraphNode {
    pub fn new(kind: NodeKind, id: CanonicalId, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            qualified_name: name.clone(),
            id,
            kind,
            name,
            language: None,
            location: None,
            content_hash: None,
            properties: serde_json::Map::new(),
            provenance: Vec::new(),
        }
    }

    pub fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    pub fn with_location(mut self, location: SourceLocation) -> Self {
        self.location = Some(location);
        self
    }

    pub fn with_content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
        self
    }

    pub fn with_qualified_name(mut self, qn: impl Into<String>) -> Self {
        self.qualified_name = qn.into();
        self
    }

    pub fn with_property(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.properties.insert(key.into(), value);
        self
    }

    pub fn with_provenance(mut self, p: Provenance) -> Self {
        self.provenance.push(p);
        self
    }
}

/// Status of an indexing/analysis artifact for provenance tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProcessingStatus {
    Complete,
    Partial,
    Failed,
    Skipped,
}
