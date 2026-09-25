mod config;
mod error;

pub use config::{IndexingConfig, RepositoryConfig, WorkspaceConfig, default_analyzers};
pub use error::ConfigError;

pub type Result<T, E = ConfigError> = std::result::Result<T, E>;
