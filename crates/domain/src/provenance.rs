use serde::{Deserialize, Serialize};

use crate::entity::ProcessingStatus;

/// Which analyzer/system produced a graph fact.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProvenanceSource {
    Git,
    TreeSitter,
    Scip,
    Joern,
    CiCd,
    DeploymentSystem,
    Manual,
    Other(String),
}

/// Provenance attached to a graph fact: source, analyzer version, status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source: ProvenanceSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyzer_version: Option<String>,
    pub status: ProcessingStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default = "chrono_free_timestamp")]
    pub recorded_at: String,
}

fn chrono_free_timestamp() -> String {
    // Avoid chrono dependency in domain for MVP; RFC3339 from system time.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

impl Provenance {
    pub fn from(source: ProvenanceSource) -> Self {
        Self {
            source,
            analyzer_version: None,
            status: ProcessingStatus::Complete,
            detail: None,
            recorded_at: chrono_free_timestamp(),
        }
    }

    pub fn tree_sitter(version: impl Into<String>) -> Self {
        Self::from(ProvenanceSource::TreeSitter).with_version(version)
    }

    pub fn scip(version: impl Into<String>) -> Self {
        Self::from(ProvenanceSource::Scip).with_version(version)
    }

    pub fn joern(version: impl Into<String>) -> Self {
        Self::from(ProvenanceSource::Joern).with_version(version)
    }

    pub fn git() -> Self {
        Self::from(ProvenanceSource::Git)
    }

    pub fn with_version(mut self, v: impl Into<String>) -> Self {
        self.analyzer_version = Some(v.into());
        self
    }

    pub fn with_status(mut self, s: ProcessingStatus) -> Self {
        self.status = s;
        self
    }

    pub fn with_detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }
}
