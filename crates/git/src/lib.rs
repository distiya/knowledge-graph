mod clone;
mod cmd;
mod error;
mod language;
mod repo;

pub use clone::{
    CloneConfig, clone_or_open, clone_repo, fetch_updates, open_and_fetch, repo_name_from_url,
};
pub use error::GitError;
pub use language::detect_language;
pub use repo::{FileDiff, RepoHandle};

pub type Result<T, E = GitError> = std::result::Result<T, E>;
