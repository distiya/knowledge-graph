use serde::{Deserialize, Serialize};

/// Precise source location for a code entity (1-based lines/columns as from tree-sitter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    pub repository: String,
    pub commit_sha: String,
    pub path: String,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

impl SourceLocation {
    pub fn new(
        repository: impl Into<String>,
        commit_sha: impl Into<String>,
        path: impl Into<String>,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    ) -> Self {
        Self {
            repository: repository.into(),
            commit_sha: commit_sha.into(),
            path: path.into(),
            start_line,
            start_column,
            end_line,
            end_column,
            branch: None,
        }
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }
}
