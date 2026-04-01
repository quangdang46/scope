use serde::Serialize;
use serde_json::Value;

use crate::{stub, JsonEnvelope, RepoPath, ScopeError, ScopeResult, Store, SymbolKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepsQuery {
    pub file: String,
    pub reverse: bool,
    pub transitive: bool,
    pub depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolsQuery {
    pub file: String,
    pub public_only: bool,
    pub kind: Option<SymbolKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallsQuery {
    pub symbol: String,
    pub transitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactQuery {
    pub target: String,
    pub change_type: String,
    pub depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainQuery {
    pub target: String,
    pub to: Option<String>,
    pub depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhyQuery {
    pub from: String,
    pub to: String,
    pub depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextQuery {
    pub targets: Vec<String>,
    pub change_type: String,
    pub budget: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryRequest {
    Deps(DepsQuery),
    Symbols(SymbolsQuery),
    Calls(CallsQuery),
    Callers(CallsQuery),
    Impact(ImpactQuery),
    Explain(ExplainQuery),
    Why(WhyQuery),
    Context(ContextQuery),
}

impl QueryRequest {
    pub fn command(&self) -> &'static str {
        match self {
            Self::Deps(_) => "deps",
            Self::Symbols(_) => "symbols",
            Self::Calls(_) => "calls",
            Self::Callers(_) => "callers",
            Self::Impact(_) => "impact",
            Self::Explain(_) => "explain",
            Self::Why(_) => "why",
            Self::Context(_) => "context",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QueryEngine<'a> {
    store: &'a Store,
}

#[derive(Debug, Clone)]
pub struct QueryOutput {
    value: Value,
    pretty: String,
    compact: String,
}

impl QueryOutput {
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    pub fn into_value(self) -> Value {
        self.value
    }

    pub fn render(&self, compact: bool) -> &str {
        if compact {
            &self.compact
        } else {
            &self.pretty
        }
    }
}

impl<'a> QueryEngine<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn execute(&self, request: QueryRequest) -> ScopeResult<QueryOutput> {
        match request {
            QueryRequest::Deps(query) => {
                let target = RepoPath::from(query.file.clone());
                let dependencies = if query.transitive {
                    self.store
                        .query_deps_transitive(&target, query.reverse, query.depth)?
                } else if query.reverse {
                    self.store.query_reverse_deps(&target)?
                } else {
                    self.store.query_deps(&target)?
                };
                serialize_envelope(stub::deps(
                    query.file,
                    query.reverse,
                    query.transitive,
                    query.depth,
                    dependencies,
                ))
            }
            QueryRequest::Symbols(query) => {
                let symbols = self.store.query_symbols(
                    &RepoPath::from(query.file.clone()),
                    query.public_only,
                    query.kind.clone(),
                )?;
                serialize_envelope(stub::symbols(
                    query.file,
                    query.public_only,
                    query.kind,
                    symbols,
                ))
            }
            QueryRequest::Calls(query) => {
                let traversals = self.store.query_callees(&query.symbol, query.transitive)?;
                serialize_envelope(stub::calls(query.symbol, query.transitive, traversals))
            }
            QueryRequest::Callers(query) => {
                let traversals = self.store.query_callers(&query.symbol, query.transitive)?;
                serialize_envelope(stub::callers(query.symbol, query.transitive, traversals))
            }
            QueryRequest::Impact(query) => {
                let impacted =
                    self.store
                        .query_impact(&query.target, &query.change_type, query.depth)?;
                serialize_envelope(stub::impact(
                    query.target,
                    query.change_type,
                    query.depth,
                    impacted,
                ))
            }
            QueryRequest::Explain(query) => {
                let traversals =
                    self.store
                        .query_explain(&query.target, query.to.as_deref(), query.depth)?;
                serialize_envelope(stub::explain(
                    query.target,
                    query.to,
                    query.depth,
                    traversals,
                ))
            }
            QueryRequest::Why(query) => {
                let path = self.store.query_why(&query.from, &query.to, query.depth)?;
                serialize_envelope(stub::why(query.from, query.to, query.depth, path))
            }
            QueryRequest::Context(query) => {
                let result =
                    self.store
                        .query_context(&query.targets, &query.change_type, query.budget)?;
                serialize_envelope(stub::context(result))
            }
        }
    }
}

fn serialize_envelope<T: Serialize>(envelope: JsonEnvelope<T>) -> ScopeResult<QueryOutput> {
    let pretty = serde_json::to_string_pretty(&envelope).map_err(|error| {
        ScopeError::Internal(format!("failed to serialize query envelope: {error}"))
    })?;
    let value = serde_json::to_value(envelope).map_err(|error| {
        ScopeError::Internal(format!("failed to serialize query envelope: {error}"))
    })?;
    let mut compact = value.clone();
    compact_json_value(&mut compact);
    let compact = serde_json::to_string(&compact).map_err(|error| {
        ScopeError::Internal(format!("failed to serialize query envelope: {error}"))
    })?;
    Ok(QueryOutput {
        value,
        pretty,
        compact,
    })
}

fn compact_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for value in map.values_mut() {
                compact_json_value(value);
            }
            map.retain(|_, value| !should_prune_compact_value(value));
        }
        Value::Array(values) => {
            for value in values {
                compact_json_value(value);
            }
        }
        _ => {}
    }
}

fn should_prune_compact_value(value: &Value) -> bool {
    matches!(value, Value::Null)
        || matches!(value, Value::Array(values) if values.is_empty())
        || matches!(value, Value::Object(map) if map.is_empty())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{adapter_for_language, scan_repo, ScanConfig};

    use super::*;

    static DIR_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let count = DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("scope-query-runtime-{prefix}-{nanos}-{count}"))
    }

    fn write_fixture_repo(root: &Path) {
        if root.exists() {
            fs::remove_dir_all(root).expect("fixture root should be removable");
        }
        fs::create_dir_all(root.join("src")).expect("fixture src directory should exist");
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"query_runtime_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("fixture manifest should be written");
        fs::write(
            root.join("src/lib.rs"),
            "mod parser;\npub fn parse(input: &str) -> Vec<&str> { parser::parse(input) }\n",
        )
        .expect("fixture lib should be written");
        fs::write(
            root.join("src/parser.rs"),
            "pub fn parse(input: &str) -> Vec<&str> { input.split(',').collect() }\n",
        )
        .expect("fixture parser should be written");
    }

    fn index_fixture(root: &Path) -> Store {
        let store = Store::open(&root.join(".scope/index.db")).expect("fixture store should open");
        let entries = scan_repo(root, &ScanConfig::default()).expect("fixture scan should succeed");
        let extracts: Vec<_> = entries
            .into_iter()
            .filter_map(|entry| {
                let adapter = adapter_for_language(entry.language)?;
                if !crate::adapters::supports_path(adapter, &entry.absolute_path) {
                    return None;
                }
                let source = fs::read_to_string(&entry.absolute_path)
                    .expect("fixture source should be readable");
                let mut extract = adapter.extract(&entry, &source);
                extract.file.content_hash =
                    Some(blake3::hash(source.as_bytes()).to_hex().to_string());
                Some(extract)
            })
            .collect();
        store
            .persist_extract_results(&extracts)
            .expect("fixture extracts should persist");
        store
    }

    #[test]
    fn deps_request_returns_shared_stub_envelope() {
        let root = unique_temp_dir("deps");
        write_fixture_repo(&root);
        let store = index_fixture(&root);
        let engine = QueryEngine::new(&store);

        let output = engine
            .execute(QueryRequest::Deps(DepsQuery {
                file: "src/lib.rs".to_string(),
                reverse: false,
                transitive: false,
                depth: None,
            }))
            .expect("deps request should succeed");

        let value = output.as_value();
        assert_eq!(value["command"], "deps");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/lib.rs");
        assert_eq!(value["data"]["dependencies"][0]["path"], "src/parser.rs");
    }

    #[test]
    fn context_request_normalizes_shared_query_output() {
        let root = unique_temp_dir("context");
        write_fixture_repo(&root);
        let store = index_fixture(&root);
        let engine = QueryEngine::new(&store);

        let output = engine
            .execute(QueryRequest::Context(ContextQuery {
                targets: vec!["src/lib.rs".to_string()],
                change_type: "body".to_string(),
                budget: Some(400),
            }))
            .expect("context request should succeed");

        let value = output.as_value();
        assert_eq!(value["command"], "context");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["result"]["must_read"].is_array());
        assert!(value["data"]["result"]["should_read"].is_array());
        assert!(value["data"]["result"]["summary"].is_object());
    }
}
