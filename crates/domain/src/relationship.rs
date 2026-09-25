use serde::{Deserialize, Serialize};

use crate::identity::CanonicalId;
use crate::provenance::Provenance;

/// Canonical relationship types in the enterprise knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationKind {
    HasBranch,
    PointsTo,
    ContainsStateOf,
    DefinedIn,
    Calls,
    References,
    Implements,
    Extends,
    Imports,
    DependsOn,
    Exposes,
    ConsumedBy,
    Uses,
    DeployedTo,
    HasCommit,
    HasFile,
    PartOf,
    CallsApi,
    PublishTo,
    ConsumeFrom,
    ReadsFrom,
    WritesTo,
    Invokes,
    ConnectsTo,
    Queries,
}

/// Relation kinds that express a dependency from one node to another.
/// Traversal order is significant: structural kinds rank before service and
/// external-resource kinds.
pub const DEPENDENCY_KINDS: [RelationKind; 10] = [
    RelationKind::DependsOn,
    RelationKind::Imports,
    RelationKind::CallsApi,
    RelationKind::PublishTo,
    RelationKind::ConsumeFrom,
    RelationKind::ReadsFrom,
    RelationKind::WritesTo,
    RelationKind::Invokes,
    RelationKind::ConnectsTo,
    RelationKind::Queries,
];

/// A canonical graph edge produced by normalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub kind: RelationKind,
    pub from: CanonicalId,
    pub to: CanonicalId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub properties: serde_json::Map<String, serde_json::Value>,
    pub provenance: Vec<Provenance>,
}

impl GraphEdge {
    pub fn new(kind: RelationKind, from: CanonicalId, to: CanonicalId) -> Self {
        Self {
            kind,
            from,
            to,
            content_hash: None,
            properties: serde_json::Map::new(),
            provenance: Vec::new(),
        }
    }

    pub fn with_content_hash(mut self, hash: impl Into<String>) -> Self {
        self.content_hash = Some(hash.into());
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
