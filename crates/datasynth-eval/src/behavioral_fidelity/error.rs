//! Errors emitted by the behavioral-fidelity module.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BehavioralFidelityError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parquet error: {0}")]
    Parquet(String),
    #[error("schema error: {0}")]
    Schema(String),
    #[error("computation error: {0}")]
    Computation(String),
    #[error("serde error: {0}")]
    Serde(String),
}

impl From<arrow::error::ArrowError> for BehavioralFidelityError {
    fn from(value: arrow::error::ArrowError) -> Self {
        BehavioralFidelityError::Parquet(value.to_string())
    }
}

impl From<parquet::errors::ParquetError> for BehavioralFidelityError {
    fn from(value: parquet::errors::ParquetError) -> Self {
        BehavioralFidelityError::Parquet(value.to_string())
    }
}

impl From<serde_json::Error> for BehavioralFidelityError {
    fn from(value: serde_json::Error) -> Self {
        BehavioralFidelityError::Serde(value.to_string())
    }
}

pub type BehavioralFidelityResult<T> = std::result::Result<T, BehavioralFidelityError>;
