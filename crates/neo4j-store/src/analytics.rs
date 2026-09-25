use serde::{Deserialize, Serialize};

use ckg_domain::{CanonicalId, NodeKind};

/// Analysis modes exposed by the store analytics layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyticsMode {
    /// Nodes that transitively depend on a given node (impact of a change).
    BlastRadius,
    /// External resources ranked by how much the codebase depends on them.
    CriticalResources,
    /// Nodes with the highest betweenness (bridges between graph regions).
    Bridges,
    /// Weakly connected components of the dependency graph.
    Clusters,
}

impl AnalyticsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AnalyticsMode::BlastRadius => "blast_radius",
            AnalyticsMode::CriticalResources => "critical_resources",
            AnalyticsMode::Bridges => "bridges",
            AnalyticsMode::Clusters => "clusters",
        }
    }
}

/// Engine that produced (or was intended to produce) an analytics result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalyticsEngine {
    Cypher,
    Apoc,
    Gds,
}

/// A bounded analytics request. `id` is required for blast radius; `depth`
/// and `limit` are clamped by the store before the query runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsSpec {
    pub mode: AnalyticsMode,
    #[serde(default)]
    pub id: Option<CanonicalId>,
    #[serde(default)]
    pub depth: u8,
    #[serde(default)]
    pub limit: u32,
}

/// One row of an analytics result.
///
/// `score` carries PageRank/betweenness/degree/component size depending on
/// the mode; `distance` is populated for blast radius and `component` for
/// clusters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsRow {
    pub id: CanonicalId,
    pub name: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<i64>,
}

/// Result of an analytics request.
///
/// `engine` is the engine that produced `rows`; when the preferred engine
/// was unavailable and the analysis was skipped, `engine` names the engine
/// that was requested, `rows` is empty, and `degraded`/`note` explain why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsReport {
    pub mode: AnalyticsMode,
    pub engine: AnalyticsEngine,
    pub rows: Vec<AnalyticsRow>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl AnalyticsReport {
    pub fn skipped(mode: AnalyticsMode, engine: AnalyticsEngine, note: impl Into<String>) -> Self {
        Self {
            mode,
            engine,
            rows: Vec::new(),
            truncated: false,
            degraded: true,
            note: Some(note.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&AnalyticsMode::BlastRadius).unwrap(),
            "\"blast_radius\""
        );
        assert_eq!(
            serde_json::to_string(&AnalyticsMode::CriticalResources).unwrap(),
            "\"critical_resources\""
        );
        let mode: AnalyticsMode = serde_json::from_str("\"clusters\"").unwrap();
        assert_eq!(mode, AnalyticsMode::Clusters);
        assert_eq!(AnalyticsMode::Bridges.as_str(), "bridges");
    }

    #[test]
    fn report_skips_optional_fields() {
        let report =
            AnalyticsReport::skipped(AnalyticsMode::Bridges, AnalyticsEngine::Gds, "no gds");
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["degraded"], serde_json::json!(true));
        assert!(json.get("note").is_some());
        assert_eq!(json["rows"], serde_json::json!([]));

        let row = AnalyticsRow {
            id: CanonicalId::from("res-1"),
            name: "orders-db".into(),
            kind: NodeKind::Resource,
            score: 1.5,
            distance: None,
            component: None,
        };
        let json = serde_json::to_value(&row).unwrap();
        assert!(json.get("distance").is_none());
        assert!(json.get("component").is_none());
        assert_eq!(json["score"], serde_json::json!(1.5));
    }

    #[test]
    fn spec_roundtrips() {
        let spec = AnalyticsSpec {
            mode: AnalyticsMode::BlastRadius,
            id: Some(CanonicalId::from("n1")),
            depth: 3,
            limit: 25,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: AnalyticsSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
    }
}
