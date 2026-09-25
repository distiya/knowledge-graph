#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse toml: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("failed to serialize toml: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("failed to parse json: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("duplicate repository id: {0}")]
    DuplicateId(String),

    #[error("repository not found: {0}")]
    NotFound(String),

    #[error("repository '{id}' has an empty github field")]
    EmptyGithub { id: String },

    #[error("enabled repository '{id}' must monitor at least one branch")]
    NoBranches { id: String },

    #[error("unsupported config format: {0}")]
    UnsupportedFormat(String),

    #[error("validation failed: {0}")]
    Validation(String),
}
