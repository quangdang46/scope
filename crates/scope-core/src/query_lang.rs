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
    CallersTransitive,
    Callees,
    CalleesTransitive,
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

pub fn execute_query(
    input: &str,
    store: &Store,
    session: &mut QuerySession,
) -> ScopeResult<QueryValue> {
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
            ScopeError::InvalidInput("let bindings must use `let <name> = <expr>`".to_string())
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
    let segments = split_pipeline_segments(input)?;
    if segments.is_empty() || segments[0].is_empty() {
        return Err(ScopeError::InvalidInput(
            "query expression is missing a source".to_string(),
        ));
    }

    let source = parse_source(&segments[0])?;
    let mut steps = Vec::new();
    for segment in &segments[1..] {
        steps.push(parse_step(segment)?);
    }

    Ok(QueryExpr { source, steps })
}

fn split_pipeline_segments(input: &str) -> ScopeResult<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;

    for ch in input.chars() {
        if in_quotes && escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' if in_quotes => {
                current.push(ch);
                escaped = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            '|' if !in_quotes => {
                let segment = current.trim();
                if segment.is_empty() {
                    return Err(ScopeError::InvalidInput(
                        "query expression contains an empty pipeline segment".to_string(),
                    ));
                }
                segments.push(segment.to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if in_quotes {
        return Err(ScopeError::InvalidInput(
            "query expression contains an unterminated quoted string".to_string(),
        ));
    }

    let tail = current.trim();
    if tail.is_empty() {
        if !segments.is_empty() {
            return Err(ScopeError::InvalidInput(
                "query expression contains an empty pipeline segment".to_string(),
            ));
        }
        return Ok(Vec::new());
    }
    segments.push(tail.to_string());
    Ok(segments)
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
        ".callers_transitive" => Ok(QueryStep::CallersTransitive),
        ".callees" => Ok(QueryStep::Callees),
        ".callees_transitive" => Ok(QueryStep::CalleesTransitive),
        "unique" | ".unique" => Ok(QueryStep::Unique),
        "count" | ".count" => Ok(QueryStep::Count),
        _ => Err(ScopeError::InvalidInput(format!(
            "unsupported query step `{input}`; supported steps are .deps, .reverse, .symbols, .callers, .callees, unique, and count; plus .callers_transitive and .callees_transitive"
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
    decode_quoted_selector(&rest[1..rest.len() - 1]).map(Some)
}

fn decode_quoted_selector(raw: &str) -> ScopeResult<String> {
    let mut decoded = String::new();
    let mut chars = raw.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        match chars.next() {
            Some('"') => decoded.push('"'),
            Some('\\') => decoded.push('\\'),
            Some(other) => {
                decoded.push('\\');
                decoded.push(other);
            }
            None => {
                return Err(ScopeError::InvalidInput(
                    "query expression contains an unterminated escape sequence".to_string(),
                ));
            }
        }
    }

    Ok(decoded)
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

fn evaluate_expr(
    expr: &QueryExpr,
    store: &Store,
    session: &QuerySession,
) -> ScopeResult<QueryValue> {
    let mut value = match &expr.source {
        QuerySource::File(path) => {
            let path = path.clone();
            if store.query_deps(&path)?.is_empty() && store.query_reverse_deps(&path)?.is_empty() {
                let symbols = store.query_symbols(&path, false, None)?;
                let indexed = store
                    .list_indexed_files()?
                    .into_iter()
                    .any(|candidate| candidate == path);
                if symbols.is_empty() && !indexed {
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
        QuerySource::Var(name) => {
            session.bindings.get(name).cloned().ok_or_else(|| {
                ScopeError::InvalidInput(format!("unknown query binding `${name}`"))
            })?
        }
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
                next.extend(
                    store
                        .query_deps(&path)?
                        .into_iter()
                        .map(|record| record.path),
                );
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
        QueryStep::CallersTransitive => {
            let symbols = symbol_names(&value, ".callers_transitive")?;
            let mut next = Vec::new();
            for symbol in symbols {
                next.extend(store.query_callers(&symbol, true)?);
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
        QueryStep::CalleesTransitive => {
            let symbols = symbol_names(&value, ".callees_transitive")?;
            let mut next = Vec::new();
            for symbol in symbols {
                next.extend(store.query_callees(&symbol, true)?);
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
        QueryValue::Symbols(symbols) => Ok(symbols
            .iter()
            .map(|symbol| symbol.qualname.clone())
            .collect()),
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
        let statement =
            parse_query_statement("let auth = symbol \"auth::login\" | .callers").unwrap();
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
    fn parse_all_sources_and_var_reference() {
        assert_eq!(
            parse_query_statement("all-files | count").unwrap(),
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::AllFiles,
                steps: vec![QueryStep::Count],
            })
        );
        assert_eq!(
            parse_query_statement("all-symbols | count").unwrap(),
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::AllSymbols,
                steps: vec![QueryStep::Count],
            })
        );
        assert_eq!(
            parse_query_statement("$roots | unique").unwrap(),
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::Var("roots".to_string()),
                steps: vec![QueryStep::Unique],
            })
        );
    }

    #[test]
    fn parse_transitive_call_steps() {
        assert_eq!(
            parse_query_statement(
                "symbol \"parser::parse\" | .callers_transitive | .callees_transitive"
            )
            .unwrap(),
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::Symbol("parser::parse".to_string()),
                steps: vec![QueryStep::CallersTransitive, QueryStep::CalleesTransitive],
            })
        );
    }

    #[test]
    fn binding_names_are_sorted() {
        let mut session = QuerySession::default();
        session
            .bindings
            .insert("zeta".to_string(), QueryValue::Number(1));
        session
            .bindings
            .insert("alpha".to_string(), QueryValue::Number(2));
        session
            .bindings
            .insert("middle".to_string(), QueryValue::Number(3));

        assert_eq!(
            session.binding_names(),
            vec![
                "alpha".to_string(),
                "middle".to_string(),
                "zeta".to_string()
            ]
        );
    }

    #[test]
    fn reject_unknown_step() {
        let error = parse_query_statement("file \"src/lib.rs\" | .impact").unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(error.to_string().contains("unsupported query step"));
    }

    #[test]
    fn reject_trailing_pipe_segment() {
        let error = parse_query_statement("file \"src/lib.rs\" | .deps |").unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(error.to_string().contains("empty pipeline segment"));
    }

    #[test]
    fn reject_repeated_pipe_segment() {
        let error = parse_query_statement("file \"src/lib.rs\" || .deps").unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(error.to_string().contains("empty pipeline segment"));
    }

    #[test]
    fn allow_pipe_inside_quoted_selector() {
        let statement = parse_query_statement("file \"src/a|b.rs\" | .deps").unwrap();
        assert_eq!(
            statement,
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::File(RepoPath::from("src/a|b.rs")),
                steps: vec![QueryStep::Deps],
            })
        );
    }

    #[test]
    fn reject_unterminated_quoted_selector() {
        let error = parse_query_statement("file \"src/lib.rs | .deps").unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(error.to_string().contains("unterminated quoted string"));
    }

    #[test]
    fn reject_unquoted_selector_and_invalid_binding_names() {
        let unquoted = parse_query_statement("file src/lib.rs | .deps").unwrap_err();
        assert_eq!(unquoted.kind(), "invalid_input");
        assert!(unquoted.to_string().contains("quoted string"));

        let invalid_let = parse_query_statement("let bad-name = all-files | count").unwrap_err();
        assert_eq!(invalid_let.kind(), "invalid_input");
        assert!(invalid_let
            .to_string()
            .contains("may only use ASCII letters"));

        let invalid_var = parse_query_statement("$bad-name | count").unwrap_err();
        assert_eq!(invalid_var.kind(), "invalid_input");
        assert!(invalid_var
            .to_string()
            .contains("may only use ASCII letters"));
    }

    #[test]
    fn reject_missing_source_and_unsupported_source() {
        let missing_source = parse_query_statement("| .deps").unwrap_err();
        assert_eq!(missing_source.kind(), "invalid_input");
        assert!(missing_source
            .to_string()
            .contains("empty pipeline segment"));

        let unsupported = parse_query_statement("module \"src/lib.rs\" | count").unwrap_err();
        assert_eq!(unsupported.kind(), "invalid_input");
        assert!(unsupported.to_string().contains("unsupported query source"));
    }

    #[test]
    fn allow_escaped_quote_and_pipe_inside_quoted_selector() {
        let statement = parse_query_statement("file \"src/a\\\"|b.rs\" | .deps").unwrap();
        assert_eq!(
            statement,
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::File(RepoPath::from("src/a\"|b.rs")),
                steps: vec![QueryStep::Deps],
            })
        );
    }

    #[test]
    fn preserve_literal_backslashes_in_quoted_selector() {
        let statement = parse_query_statement("file \"src\\path\\file.rs\" | .deps").unwrap();
        assert_eq!(
            statement,
            QueryStatement::Expr(QueryExpr {
                source: QuerySource::File(RepoPath::from("src\\path\\file.rs")),
                steps: vec![QueryStep::Deps],
            })
        );
    }

    #[test]
    fn reject_unterminated_escape_in_quoted_selector() {
        let error = decode_quoted_selector("src/lib\\").unwrap_err();
        assert_eq!(error.kind(), "invalid_input");
        assert!(error.to_string().contains("unterminated escape sequence"));
    }

    #[test]
    fn file_steps_reject_symbol_and_number_values() {
        let symbol_value = QueryValue::Symbols(vec![SymbolRecord {
            file: RepoPath::from("src/lib.rs"),
            name: "parse".to_string(),
            qualname: "crate::parse".to_string(),
            kind: crate::SymbolKind::Function,
            visibility: crate::Visibility::Public,
            exported: true,
            span: crate::Span {
                start_byte: 0,
                end_byte: 32,
                start_line: 1,
                end_line: 3,
            },
        }]);
        let deps_error = file_paths(&symbol_value, ".deps").unwrap_err();
        assert_eq!(deps_error.kind(), "invalid_input");
        assert!(deps_error
            .to_string()
            .contains(".deps requires file results, not symbol results"));

        let count_value = QueryValue::Number(2);
        let reverse_error = file_paths(&count_value, ".reverse").unwrap_err();
        assert_eq!(reverse_error.kind(), "invalid_input");
        assert!(reverse_error
            .to_string()
            .contains(".reverse cannot be applied after count"));
    }

    #[test]
    fn symbol_steps_reject_file_and_number_values() {
        let file_value = QueryValue::Files(vec![RepoPath::from("src/lib.rs")]);
        let callers_error = symbol_names(&file_value, ".callers").unwrap_err();
        assert_eq!(callers_error.kind(), "invalid_input");
        assert!(callers_error
            .to_string()
            .contains(".callers requires symbol results; try `.symbols` first"));

        let count_value = QueryValue::Number(1);
        let callees_error = symbol_names(&count_value, ".callees").unwrap_err();
        assert_eq!(callees_error.kind(), "invalid_input");
        assert!(callees_error
            .to_string()
            .contains(".callees cannot be applied after count"));
    }

    #[test]
    fn unique_value_deduplicates_traversals_and_preserves_numbers() {
        let traversal = TraversalRecord {
            kind: crate::NodeKind::Symbol,
            path: Some(RepoPath::from("src/lib.rs")),
            qualname: Some("crate::parse".to_string()),
            edge_kind: crate::EdgeKind::Call,
            certainty: crate::Certainty::Exact,
            reason: "direct call".to_string(),
            distance: 1,
        };
        let distinct = TraversalRecord {
            kind: crate::NodeKind::Symbol,
            path: Some(RepoPath::from("src/parser.rs")),
            qualname: Some("crate::parser::parse".to_string()),
            edge_kind: crate::EdgeKind::Call,
            certainty: crate::Certainty::Resolved,
            reason: "indirect call".to_string(),
            distance: 2,
        };

        let unique = unique_value(QueryValue::Traversals(vec![
            traversal.clone(),
            traversal,
            distinct.clone(),
        ]));
        assert_eq!(
            unique,
            QueryValue::Traversals(vec![
                TraversalRecord {
                    kind: crate::NodeKind::Symbol,
                    path: Some(RepoPath::from("src/lib.rs")),
                    qualname: Some("crate::parse".to_string()),
                    edge_kind: crate::EdgeKind::Call,
                    certainty: crate::Certainty::Exact,
                    reason: "direct call".to_string(),
                    distance: 1,
                },
                distinct,
            ])
        );

        assert_eq!(unique_value(QueryValue::Number(7)), QueryValue::Number(7));
    }

    #[test]
    fn count_value_counts_number_as_single_result() {
        assert_eq!(count_value(&QueryValue::Files(vec![])), 0);
        assert_eq!(count_value(&QueryValue::Number(99)), 1);
    }
}
