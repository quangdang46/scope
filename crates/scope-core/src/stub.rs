use serde::Serialize;

use crate::{
    json::JsonEnvelope,
    model::{
        Certainty, DependencyRecord, EdgeKind, NodeKind, RepoPath, Span, SymbolKind, SymbolRecord,
        TraversalRecord, Visibility,
    },
    DatabaseInfo,
};

#[derive(Debug, Serialize)]
pub struct IndexData {
    pub repo_root: RepoPath,
    pub no_git: bool,
    pub watch: bool,
    pub database: DatabaseInfo,
}

#[derive(Debug, Serialize)]
pub struct DepsData {
    pub target: RepoPath,
    pub reverse: bool,
    pub transitive: bool,
    pub depth: Option<usize>,
    pub dependencies: Vec<DependencyRecord>,
}

#[derive(Debug, Serialize)]
pub struct SymbolsData {
    pub target: RepoPath,
    pub public_only: bool,
    pub kind: Option<SymbolKind>,
    pub symbols: Vec<SymbolRecord>,
}

#[derive(Debug, Serialize)]
pub struct CallsData {
    pub symbol: String,
    pub transitive: bool,
    pub traversals: Vec<TraversalRecord>,
}

#[derive(Debug, Serialize)]
pub struct ImpactData {
    pub target: String,
    pub change_type: String,
    pub depth: Option<usize>,
    pub impacted: Vec<TraversalRecord>,
    pub risk: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ExplainData {
    pub target: String,
    pub to: Option<String>,
    pub depth: Option<usize>,
    pub traversals: Vec<TraversalRecord>,
}

#[derive(Debug, Serialize)]
pub struct DoctorData {
    pub fix: bool,
    pub checks: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BenchmarkData {
    pub fixture: Option<String>,
    pub iterations: Option<u32>,
    pub benchmarks: Vec<String>,
}

pub fn index(
    repo_root: String,
    no_git: bool,
    watch: bool,
    database: DatabaseInfo,
) -> JsonEnvelope<IndexData> {
    JsonEnvelope::success(
        "index",
        IndexData {
            repo_root: RepoPath::from(repo_root),
            no_git,
            watch,
            database,
        },
    )
}

pub fn deps(
    target: String,
    reverse: bool,
    transitive: bool,
    depth: Option<usize>,
) -> JsonEnvelope<DepsData> {
    JsonEnvelope::stub(
        "deps",
        DepsData {
            target: RepoPath::from(target),
            reverse,
            transitive,
            depth,
            dependencies: Vec::new(),
        },
    )
}

pub fn symbols(
    target: String,
    public_only: bool,
    kind: Option<SymbolKind>,
) -> JsonEnvelope<SymbolsData> {
    JsonEnvelope::stub(
        "symbols",
        SymbolsData {
            target: RepoPath::from(target),
            public_only,
            kind,
            symbols: Vec::new(),
        },
    )
}

pub fn calls(symbol: String, transitive: bool) -> JsonEnvelope<CallsData> {
    JsonEnvelope::stub(
        "calls",
        CallsData {
            symbol,
            transitive,
            traversals: Vec::new(),
        },
    )
}

pub fn callers(symbol: String, transitive: bool) -> JsonEnvelope<CallsData> {
    JsonEnvelope::stub(
        "callers",
        CallsData {
            symbol,
            transitive,
            traversals: Vec::new(),
        },
    )
}

pub fn impact(
    target: String,
    change_type: String,
    depth: Option<usize>,
) -> JsonEnvelope<ImpactData> {
    JsonEnvelope::stub(
        "impact",
        ImpactData {
            target,
            change_type,
            depth,
            impacted: Vec::new(),
            risk: "unknown",
        },
    )
}

pub fn explain(
    target: String,
    to: Option<String>,
    depth: Option<usize>,
) -> JsonEnvelope<ExplainData> {
    JsonEnvelope::stub(
        "explain",
        ExplainData {
            target,
            to,
            depth,
            traversals: Vec::new(),
        },
    )
}

pub fn doctor(fix: bool) -> JsonEnvelope<DoctorData> {
    JsonEnvelope::stub(
        "doctor",
        DoctorData {
            fix,
            checks: Vec::new(),
        },
    )
}

pub fn benchmark(fixture: Option<String>, iterations: Option<u32>) -> JsonEnvelope<BenchmarkData> {
    JsonEnvelope::stub(
        "benchmark",
        BenchmarkData {
            fixture,
            iterations,
            benchmarks: Vec::new(),
        },
    )
}

pub fn mcp_stub_message() -> JsonEnvelope<TraversalRecord> {
    JsonEnvelope::stub(
        "scope-mcp",
        TraversalRecord {
            kind: NodeKind::Symbol,
            path: None,
            qualname: Some("scope-mcp".to_string()),
            edge_kind: EdgeKind::Dynamic,
            certainty: Certainty::Dynamic,
            reason: "scope-mcp is scaffolded but not implemented yet".to_string(),
            distance: 0,
        },
    )
}

pub fn placeholder_symbol(path: RepoPath, name: &str, kind: SymbolKind) -> SymbolRecord {
    SymbolRecord {
        file: path,
        name: name.to_string(),
        qualname: name.to_string(),
        kind,
        visibility: Visibility::Unknown,
        exported: false,
        span: Span {
            start_byte: 0,
            end_byte: 0,
            start_line: 0,
            end_line: 0,
        },
    }
}
