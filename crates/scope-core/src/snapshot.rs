use std::io::Cursor;

use crate::{ScopeError, ScopeResult, SnapshotGraph, SnapshotStoredRecord};

pub const SNAPSHOT_VERSION: u32 = 1;
pub const SNAPSHOT_COMPRESSION_LEVEL: i32 = 3;

pub fn encode_snapshot(record: &SnapshotStoredRecord) -> ScopeResult<Vec<u8>> {
    let json =
        serde_json::to_vec(record).map_err(|error| ScopeError::Serialization(error.to_string()))?;
    zstd::stream::encode_all(Cursor::new(json), SNAPSHOT_COMPRESSION_LEVEL)
        .map_err(|error| ScopeError::Serialization(error.to_string()))
}

pub fn decode_snapshot(bytes: &[u8]) -> ScopeResult<SnapshotStoredRecord> {
    let json = zstd::stream::decode_all(Cursor::new(bytes))
        .map_err(|error| ScopeError::Serialization(error.to_string()))?;
    serde_json::from_slice(&json).map_err(|error| ScopeError::Serialization(error.to_string()))
}

pub fn snapshot_summary(graph: &SnapshotGraph) -> crate::SnapshotDiffSummary {
    crate::SnapshotDiffSummary {
        files: graph.files.len(),
        symbols: graph.symbols.len(),
        file_edges: graph.file_edges.len(),
        symbol_edges: graph.symbol_edges.len(),
    }
}
