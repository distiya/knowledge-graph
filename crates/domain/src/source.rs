use serde::{Deserialize, Serialize};

use crate::contracts::ContractBatch;
use crate::entity::NodeKind;
use crate::identity::CanonicalId;
use crate::location::SourceLocation;
use crate::provenance::Provenance;
use crate::relationship::RelationKind;

/// Intermediate structural fact emitted by analyzers before normalization.
///
/// Analyzer adapters produce these; the normalizer merges them into GraphNode/GraphEdge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSymbol {
    pub canonical_id: CanonicalId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub properties: serde_json::Map<String, serde_json::Value>,
    pub provenance: Provenance,
}

/// Intermediate relationship fact emitted by analyzers before normalization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRelation {
    pub kind: RelationKind,
    pub from: CanonicalId,
    pub to: CanonicalId,
    #[serde(default)]
    pub properties: serde_json::Map<String, serde_json::Value>,
    pub provenance: Provenance,
}

/// Full analyzer output for a single file or analysis unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AnalyzerOutput {
    pub symbols: Vec<RawSymbol>,
    pub relations: Vec<RawRelation>,
    pub provenance: Vec<Provenance>,
    #[serde(default)]
    pub errors: Vec<String>,
    /// Service-dependency contracts (routes, channels, rpc endpoints)
    /// extracted from this file; carried to the workspace link pass.
    #[serde(default)]
    pub contracts: ContractBatch,
}

impl AnalyzerOutput {
    pub fn merge(&mut self, other: AnalyzerOutput) {
        self.symbols.extend(other.symbols);
        self.relations.extend(other.relations);
        self.provenance.extend(other.provenance);
        self.errors.extend(other.errors);
        self.contracts.merge(other.contracts);
    }

    pub fn is_failed(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Trait all analyzer adapters implement (tree-sitter, scip, joern).
pub trait Analyzer: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> &str;
    fn analyze_file(
        &self,
        repository: &str,
        commit_sha: &str,
        path: &str,
        content: &str,
        language: &str,
    ) -> AnalyzerOutput;
}
