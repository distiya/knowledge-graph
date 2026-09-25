use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ckg_domain::{GraphEdge, GraphNode};
use ckg_graph_delta::{EdgeKey, EdgeMap, NodeMap};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchIndexState {
    pub sha: String,
    #[serde(default)]
    pub indexed_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexState {
    #[serde(default)]
    pub repositories: BTreeMap<String, BTreeMap<String, BranchIndexState>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMaps {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl IndexState {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read index state {}", path.display()))?;
        let state: Self = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse index state {}", path.display()))?;
        Ok(state)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
        }
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(path, raw)
            .with_context(|| format!("failed to write index state {}", path.display()))?;
        Ok(())
    }

    pub fn last_sha(&self, repo: &str, branch: &str) -> Option<&str> {
        self.repositories
            .get(repo)
            .and_then(|branches| branches.get(branch))
            .map(|state| state.sha.as_str())
    }

    pub fn record(&mut self, repo: &str, branch: &str, sha: String, indexed_at: String) {
        self.repositories
            .entry(repo.to_string())
            .or_default()
            .insert(branch.to_string(), BranchIndexState { sha, indexed_at });
    }
}

pub fn state_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("index-state.json")
}

pub fn maps_path(cache_dir: &Path, repo: &str, branch: &str) -> PathBuf {
    cache_dir
        .join("maps")
        .join(safe_segment(repo))
        .join(format!("{}.json", safe_segment(branch)))
}

fn safe_segment(value: &str) -> String {
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

pub fn load_maps(path: &Path) -> Result<Option<(NodeMap, EdgeMap)>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read maps {}", path.display()))?;
    let stored: StoredMaps = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse maps {}", path.display()))?;
    Ok(Some(maps_from_stored(stored)))
}

pub fn save_maps(path: &Path, nodes: &NodeMap, edges: &EdgeMap) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let stored = StoredMaps {
        nodes: nodes.values().cloned().collect(),
        edges: edges.values().cloned().collect(),
    };
    let raw = serde_json::to_string(&stored)?;
    std::fs::write(path, raw)
        .with_context(|| format!("failed to write maps {}", path.display()))?;
    Ok(())
}

pub fn maps_from_stored(stored: StoredMaps) -> (NodeMap, EdgeMap) {
    let nodes: NodeMap = stored
        .nodes
        .iter()
        .map(|node| (node.id.to_string(), node.clone()))
        .collect();
    let edges: EdgeMap = stored
        .edges
        .iter()
        .map(|edge| (EdgeKey::from_edge(edge), edge.clone()))
        .collect();
    (nodes, edges)
}

pub fn now_label() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::{CanonicalId, NodeKind};

    #[test]
    fn missing_state_file_loads_default() {
        let tmp = tempfile::tempdir().unwrap();
        let state = IndexState::load(&tmp.path().join("nope.json")).unwrap();
        assert!(state.repositories.is_empty());
        assert!(state.last_sha("r", "main").is_none());
    }

    #[test]
    fn state_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = state_path(tmp.path());
        let mut state = IndexState::default();
        state.record("payments", "main", "abc".into(), now_label());
        state.save(&path).unwrap();

        let loaded = IndexState::load(&path).unwrap();
        assert_eq!(loaded.last_sha("payments", "main"), Some("abc"));
        assert!(loaded.last_sha("payments", "release").is_none());
    }

    #[test]
    fn maps_roundtrip_preserves_nodes_and_edges() {
        let tmp = tempfile::tempdir().unwrap();
        let path = maps_path(tmp.path(), "org/repo", "feature/x");
        let mut nodes = NodeMap::new();
        nodes.insert(
            "n1".into(),
            GraphNode::new(NodeKind::Function, CanonicalId::from("n1"), "f"),
        );
        let mut edges = EdgeMap::new();
        let edge = GraphEdge::new(
            ckg_domain::RelationKind::Calls,
            CanonicalId::from("n1"),
            CanonicalId::from("n2"),
        );
        edges.insert(EdgeKey::from_edge(&edge), edge);

        save_maps(&path, &nodes, &edges).unwrap();
        let (loaded_nodes, loaded_edges) = load_maps(&path).unwrap().expect("maps present");
        assert_eq!(loaded_nodes, nodes);
        assert_eq!(loaded_edges, edges);
    }
}
