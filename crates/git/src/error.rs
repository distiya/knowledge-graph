use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("failed to run git: {0}")]
    Io(#[from] std::io::Error),

    #[error("git command failed: {command}: {stderr}")]
    CommandFailed { command: String, stderr: String },

    #[error("not a git repository: {0}")]
    NotARepository(PathBuf),

    #[error("clone target exists and is not an empty directory: {0}")]
    TargetOccupied(PathBuf),

    #[error("{0}")]
    Other(String),
}
