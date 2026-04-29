//! Error types for the group engine.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GroupError {
    #[error("config error: {0}")]
    Config(String),
    #[error("manifest error: {0}")]
    Manifest(String),
    #[error("shard error: {0}")]
    Shard(String),
    #[error("aggregate error: {0}")]
    Aggregate(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde error: {0}")]
    Serde(String),
}

impl From<serde_yaml::Error> for GroupError {
    fn from(e: serde_yaml::Error) -> Self {
        Self::Serde(format!("yaml: {e}"))
    }
}

impl From<serde_json::Error> for GroupError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serde(format!("json: {e}"))
    }
}

pub type GroupResult<T> = Result<T, GroupError>;
