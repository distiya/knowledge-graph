use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("neo4j error: {0}")]
    Neo4j(#[from] neo4j::Neo4jError),

    #[error("invalid neo4j uri: {0}")]
    InvalidUri(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("data error: {0}")]
    Data(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}
