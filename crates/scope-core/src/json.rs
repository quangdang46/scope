use serde::Serialize;

use crate::ScopeError;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonStatus {
    Ok,
    Stub,
    Error,
}

#[derive(Debug, Serialize)]
pub struct JsonEnvelope<T> {
    pub schema_version: u32,
    pub command: &'static str,
    pub status: JsonStatus,
    pub data: T,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ErrorData {
    pub kind: &'static str,
    pub message: String,
}

impl<T> JsonEnvelope<T> {
    pub fn success(command: &'static str, data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            status: JsonStatus::Ok,
            data,
            warnings: Vec::new(),
        }
    }

    pub fn stub(command: &'static str, data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            status: JsonStatus::Stub,
            data,
            warnings: vec!["Command is scaffolded but not implemented yet".to_string()],
        }
    }
}

impl JsonEnvelope<ErrorData> {
    pub fn error(command: &'static str, error: &ScopeError) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            command,
            status: JsonStatus::Error,
            data: ErrorData {
                kind: error.kind(),
                message: error.to_string(),
            },
            warnings: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_envelope_uses_stable_machine_fields() {
        let error = ScopeError::InvalidInput("missing target".to_string());
        let envelope = JsonEnvelope::error("cli", &error);

        assert_eq!(envelope.schema_version, SCHEMA_VERSION);
        assert_eq!(envelope.command, "cli");
        assert!(matches!(envelope.status, JsonStatus::Error));
        assert_eq!(envelope.data.kind, "invalid_input");
        assert_eq!(envelope.data.message, "invalid command input: missing target");
        assert!(envelope.warnings.is_empty());
    }
}
