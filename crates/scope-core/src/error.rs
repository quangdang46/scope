use std::path::PathBuf;

use rusqlite::Error as SqliteError;
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
    #[error("filesystem error at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("database error at {path}: {message}")]
    Database { path: PathBuf, message: String },
    #[error("database migration failed: {0}")]
    Migration(String),
    #[error("tracing initialization failed: {0}")]
    Tracing(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] SqliteError),
    #[error("internal error: {0}")]
    Internal(String),
}

impl ScopeError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::IndexNotFound => "index_not_found",
            Self::NotFound { .. } => "not_found",
            Self::UnsupportedLanguage { .. } => "unsupported_language",
            Self::ParseFailed { .. } => "parse_failed",
            Self::Io { .. } => "io",
            Self::Database { .. } => "database",
            Self::Migration(_) => "migration",
            Self::Tracing(_) => "tracing",
            Self::Serialization(_) => "serialization",
            Self::Sqlite(_) => "sqlite",
            Self::Internal(_) => "internal",
        }
    }

    pub fn io(path: impl Into<PathBuf>, error: impl std::fmt::Display) -> Self {
        Self::Io {
            path: path.into(),
            message: error.to_string(),
        }
    }

    pub fn database(path: impl Into<PathBuf>, error: impl std::fmt::Display) -> Self {
        Self::Database {
            path: path.into(),
            message: error.to_string(),
        }
    }
}
