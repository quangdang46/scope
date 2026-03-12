use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::{RepoPath, ScopeError, ScopeResult, Store, SymbolRecord, TraversalRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuerySource {
    File(RepoPath),
    Symbol(String),
    AllFiles,
    AllSymbols,
    Var(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryStep {
    Deps,
    Reverse,
    Symbols,
    Callers,
    Callees,
    Unique,
    Count,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryExpr {
    pub source: QuerySource,
    pub steps: Vec<QueryStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryStatement {
    Expr(QueryExpr),
    Let { name: String, expr: QueryExpr },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryValue {
    Files(Vec<RepoPath>),
    Symbols(Vec<SymbolRecord>),
    Traversals(Vec<TraversalRecord>),
    Number(u64),
}

#[derive(Debug, Default)]
pub struct QuerySession {
    bindings: HashMap<String, QueryValue>,
}

impl QuerySession {
    pub fn binding_names(&self) -> Vec<String> {
        let mut names = self.bindings.keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }
}

pub fn execute_query(input: &str, store: &Store, session: &mut QuerySession) -> ScopeResult<QueryValue> {
    let statement = parse_query_statement(input)?;
    evaluate_statement(&statement, store, session)
}

pub fn parse_query_statement(input: &str) -> ScopeResult<QueryStatement> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ScopeError::InvalidInput(
            "query expression cannot be empty".to_string(),
        ));
    }

    if let Some(rest) = trimmed.strip_prefix("let ") {
        let (name, expr) = rest.split_once('=').ok_or_else(|| {
            ScopeError::InvalidInput(
                "let bindings must use `let <name> = <expr>`".to_string(),
            )
        })?;
        let name = name.trim();
        validate_binding_name(name)?;
        return Ok(QueryStatement::Let {
            name: name.to_string(),
            expr: parse_query_expr(expr.trim())?,
        });
    }

    Ok(QueryStatement::Expr(parse_query_expr(trimmed)?))
}

fn parse_query_expr(input: &str) -> ScopeResult<QueryExpr> {
    let mut segments = input
        .split('|')
        .map(str::trim)
        .filter(|segment| !segment.is_empty());

    let source_segment = segments.next().ok_or_else(|| {
        ScopeError::InvalidInput("query expression is missing a source".to_string())
    })?;
    let source = parse_source(source_segment)?;
    let mut steps = Vec::new();
    for segment in segments {
        steps.push(parse_step(segment)?);
    }

    Ok(QueryExpr { source, steps })
}

fn parse_source(input: &str) -> ScopeResult<QuerySource> {
    if let Some(value) = parse_quoted(input, "file")? {
        return Ok(QuerySource::File(RepoPath::from(value)));
    }
    if let Some(value) = parse_quoted(input, "symbol")? {
        return Ok(QuerySource::Symbol(value));
    }
    if input == "all-files" {
        return Ok(QuerySource::AllFiles);
    }
    if input == "all-symbols" {
        return Ok(QuerySource::AllSymbols);
    }
    if let Some(name) = input.strip_prefix('$') {
        validate_binding_name(name)?;
        return Ok(QuerySource::Var(name.to_string()));
    }

    Err(ScopeError::InvalidInput(format!(
        "unsupported query source `{input}`; expected `file \"...\"`, `symbol \"...\"`, `all-files`, `all-symbols`, or `$name`"
    )))
}

fn parse_step(input: &str) -> ScopeResult<QueryStep> {
    match input {
        ".deps" => Ok(QueryStep::Deps),
        ".reverse" => Ok(QueryStep::Reverse),
        ".symbols" => Ok(QueryStep::Symbols),
        ".callers" => Ok(QueryStep::Callers),
        ".callees" => Ok(QueryStep::Callees),
        "unique" | ".unique" => Ok(QueryStep::Unique),
        "count" | ".count" => Ok(QueryStep::Count),
        _ => Err(ScopeError::InvalidInput(format!(
            "unsupported query step `{input}`; supported steps are .deps, .reverse, .symbols, .callers, .callees, unique, and count"
        ))),
    }
}

fn parse_quoted(input: &str, keyword: &str) -> ScopeResult<Option<String>> {
    let prefix = format!("{keyword} ");
    let Some(rest) = input.strip_prefix(&prefix) else {
        return Ok(None);
    };
    let rest = rest.trim();
    if !(rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2) {
        return Err(ScopeError::InvalidInput(format!(
            "{keyword} selectors must use a quoted string"
        )));
    }
    Ok(Some(rest[1..rest.len() - 1].to_string()))
}

fn validate_binding_name(name: &str) -> ScopeResult<()> {
    if name.is_empty() {
        return Err(ScopeError::InvalidInput(
            "binding names cannot be empty".to_string(),
        ));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        return Err(ScopeError::InvalidInput(format!(
            "binding name `{name}` may only use ASCII letters, digits, and underscores"
        )));
    }
    Ok(())
}

fn evaluate_statement(
    statement: &QueryStatement,
    store: &Store,
    session: &mut QuerySession,
) -> ScopeResult<QueryValue> {
    match statement {
        QueryStatement::Expr(expr) => evaluate_expr(expr, store, session),
        QueryStatement::Let { name, expr } => {
            let value = evaluate_expr(expr, store, session)?;
            session.bindings.insert(name.clone(), value.clone());
            Ok(value)
        }
    }
}

fn evaluate_expr(expr: &QueryExpr, store: &Store, session: &QuerySession) -> ScopeResult<QueryValue> {
    let mut value = match &expr.source {
        QuerySource::File(path) => {
            let path = path.clone();
            if store.query_deps(&path)?.is_empty() && store.query_reverse_deps(&path)?.is_empty() {
                let symbols = store.query_symbols(&path, false, None)?;
                if symbols.is_empty() {
                    return Err(ScopeError::InvalidInput(format!(
                        "query file target `{}` is not indexed",
                        path.0
                    )));
                }
            }
            QueryValue::Files(vec![path])
        }
        QuerySource::Symbol(symbol) => {
            let record = store.resolve_query_symbol(symbol)?.ok_or_else(|| {
                ScopeError::InvalidInput(format!("query symbol target `{symbol}` is not indexed"))
            })?;
            QueryValue::Symbols(vec![record])
        }
        QuerySource::AllFiles => QueryValue::Files(store.list_indexed_files()?),
        QuerySource::AllSymbols => QueryValue::Symbols(store.list_indexed_symbols()?),
        QuerySource::Var(name) => session.bindings.get(name).cloned().ok_or_else(|| {
            ScopeError::InvalidInput(format!("unknown query binding `${name}`"))
        })?,
    };

    for step in &expr.steps {
        value = apply_step(value, step, store)?;
    }

    Ok(value)
}

fn apply_step(value: QueryValue, step: &QueryStep, store: &Store) -> ScopeResult<QueryValue> {
    match step {
        QueryStep::Deps => {
            let files = file_paths(&value, ".deps")?;
            let mut next = Vec::new();
            for path in files {
                next.extend(store.query_deps(&path)?.into_iter().map(|record| record.path));
            }
            Ok(QueryValue::Files(next))
        }
        QueryStep::Reverse => {
            let files = file_paths(&value, ".reverse")?;
            let mut next = Vec::new();
            for path in files {
                next.extend(
                    store
                        .query_reverse_deps(&path)?
                        .into_iter()
                        .map(|record| record.path),
                );
            }
            Ok(QueryValue::Files(next))
        }
        QueryStep::Symbols => {
            let files = file_paths(&value, ".symbols")?;
            let mut next = Vec::new();
            for path in files {
                next.extend(store.query_symbols(&path, false, None)?);
            }
            Ok(QueryValue::Symbols(next))
        }
        QueryStep::Callers => {
            let symbols = symbol_names(&value, ".callers")?;
            let mut next = Vec::new();
            for symbol in symbols {
                next.extend(store.query_callers(&symbol, false)?);
            }
            Ok(QueryValue::Traversals(next))
        }
        QueryStep::Callees => {
            let symbols = symbol_names(&value, ".callees")?;
            let mut next = Vec::new();
            for symbol in symbols {
                next.extend(store.query_callees(&symbol, false)?);
            }
            Ok(QueryValue::Traversals(next))
        }
        QueryStep::Unique => Ok(unique_value(value)),
        QueryStep::Count => Ok(QueryValue::Number(count_value(&value) as u64)),
    }
}

fn file_paths(value: &QueryValue, step: &str) -> ScopeResult<Vec<RepoPath>> {
    match value {
        QueryValue::Files(files) => Ok(files.clone()),
        QueryValue::Traversals(records) => Ok(records
            .iter()
            .filter_map(|record| record.path.clone())
            .collect()),
        QueryValue::Symbols(_) => Err(ScopeError::InvalidInput(format!(
            "{step} requires file results, not symbol results"
        ))),
        QueryValue::Number(_) => Err(ScopeError::InvalidInput(format!(
            "{step} cannot be applied after count"
        ))),
    }
}

fn symbol_names(value: &QueryValue, step: &str) -> ScopeResult<Vec<String>> {
    match value {
        QueryValue::Symbols(symbols) => Ok(symbols.iter().map(|symbol| symbol.qualname.clone()).collect()),
        QueryValue::Traversals(records) => Ok(records
            .iter()
            .filter_map(|record| record.qualname.clone())
            .collect()),
        QueryValue::Files(_) => Err(ScopeError::InvalidInput(format!(
            "{step} requires symbol results; try `.symbols` first"
        ))),
        QueryValue::Number(_) => Err(ScopeError::InvalidInput(format!(
            "{step} cannot be applied after count"
        ))),
    }
}

fn unique_value(value: QueryValue) -> QueryValue {
    match value {
        QueryValue::Files(files) => {
            let mut seen = HashSet::new();
            QueryValue::Files(
                files
                    .into_iter()
                    .filter(|path| seen.insert(path.clone()))
                    .collect(),
            )
        }
        QueryValue::Symbols(symbols) => {
            let mut seen = HashSet::new();
            QueryValue::Symbols(
                symbols
                    .into_iter()
                    .filter(|symbol| seen.insert(symbol.qualname.clone()))
                    .collect(),
            )
        }
        QueryValue::Traversals(records) => {
            let mut seen = HashSet::new();
            QueryValue::Traversals(
                records
                    .into_iter()
                    .filter(|record| {
                        seen.insert((
                            record.kind.clone(),
                            record.path.clone(),
                            record.qualname.clone(),
                            record.edge_kind.clone(),
                            record.distance,
                        ))
                    })
                    .collect(),
            )
        }
        QueryValue::Number(value) => QueryValue::Number(value),
    }
}

fn count_value(value: &QueryValue) -> usize {
    match value {
        QueryValue::Files(files) => files.len(),
        QueryValue::Symbols(symbols) => symbols.len(),
        QueryValue::Traversals(records) => records.len(),
        QueryValue::Number(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_file_pipeline_and_count() {
        let statement = parse_query_statement("file \"src/lib.rs\" | .deps | count").unwrap();
        assert_eq!(
            statement,
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::File(RepoPath::from("src/lib.rs")),
                steps: vec![QueryStep::Deps, QueryStep::Count],
            })
        );
    }

    #[test]
    fn parse_let_binding() {
        let statement = parse_query_statement("let auth = symbol \"auth::login\" | .callers").unwrap();
        assert_eq!(
            statement,
            QueryStatement::Let {
                name: "auth".to_string(),
                expr: QueryExpr {
                    source: QuerySource::Symbol("auth::login".to_string()),
                    steps: vec![QueryStep::Callers],
                },
            }
        );
    }

    #[test]
    fn reject_unknown_step() {
        let error = parse_query_statement("file \"src/lib.rs\" | .impact").unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(error.to_string().contains("unsupported query step"));
    }
}
