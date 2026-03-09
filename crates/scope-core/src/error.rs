use thiserror::Error;

pub type ScopeResult<T> = Result<T, ScopeError>;

#[derive(Debug, Error)]
pub enum ScopeError {
    #[error("invalid command input: {0}")]
    InvalidInput(String),
    #[error("index not found")]
    IndexNotFound,
    #[error("{kind} not found: {value}")]
    NotFound { kind: &'static str, value: String },
    #[error("unsupported language for path: {path}")]
    UnsupportedLanguage { path: String },
    #[error("parse failed for {path}: {message}")]
    ParseFailed { path: String, message: String },
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("internal error: {0}")]
    Internal(String),
}
