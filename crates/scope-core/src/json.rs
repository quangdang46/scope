use serde::Serialize;

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
