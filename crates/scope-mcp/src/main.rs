use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use scope_core::{
    adapter_for_language, arch_check, load_arch_config, scan_repo, BootstrapOptions, DatabaseInfo,
    RepoPath, RiskSort, ScanConfig, Store, SymbolKind, Verbosity,
};
use serde_json::{json, Value};

fn main() {
    if let Err(error) = run() {
        let _ = writeln!(io::stderr(), "scope-mcp fatal error: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    while let Some(request) = read_message(&mut reader)? {
        let response = handle_message(&request);
        if let Some(response) = response {
            write_message(&mut writer, &response)?;
            writer.flush()?;
        }
    }

    Ok(())
}

fn handle_message(request: &Value) -> Option<Value> {
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return request
            .get("id")
            .cloned()
            .map(|id| jsonrpc_error(id, -32600, "invalid request"));
    };

    let id = request.get("id").cloned();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => id.map(|id| initialize_response(id)),
        "ping" => id.map(|id| jsonrpc_result(id, json!({}))),
        "tools/list" => id.map(|id| jsonrpc_result(id, json!({ "tools": tool_registry() }))),
        "tools/call" => {
            let id = id?;
            let tool_name = params.get("name").and_then(Value::as_str);
            let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match tool_name {
                Some(name) => match dispatch_tool(name, &arguments) {
                    Ok(output) => Some(jsonrpc_result(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": output
                                }
                            ]
                        }),
                    )),
                    Err(DispatchError::Transport(message)) => {
                        Some(jsonrpc_error(id, -32602, &message))
                    }
                },
                None => Some(jsonrpc_error(id, -32602, "missing tool name")),
            }
        }
        "notifications/initialized" => None,
        "exit" => None,
        _ => id.map(|id| jsonrpc_error(id, -32601, "method not found")),
    }
}

fn initialize_response(id: Value) -> Value {
    jsonrpc_result(
        id,
        json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": {
                "name": "scope-mcp",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            }
        }),
    )
}

fn jsonrpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(None);
        }
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse::<usize>().ok();
            }
        }
    }

    let Some(content_length) = content_length else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        ));
    };

    let mut buffer = vec![0; content_length];
    reader.read_exact(&mut buffer)?;
    let value = serde_json::from_slice(&buffer)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some(value))
}

fn write_message(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)
}

fn tool_registry() -> Vec<Value> {
    vec![
        tool_definition(
            "index",
            "Refresh the repository index for the supplied repo root.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "no_git": { "type": "boolean" },
                    "watch": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "deps",
            "Query forward or reverse file dependencies.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "file": { "type": "string" },
                    "reverse": { "type": "boolean" },
                    "transitive": { "type": "boolean" },
                    "depth": { "type": "integer", "minimum": 0 }
                },
                "required": ["file"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "symbols",
            "Query indexed symbols defined in a file.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "file": { "type": "string" },
                    "public_only": { "type": "boolean" },
                    "kind": { "type": "string" }
                },
                "required": ["file"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "calls",
            "Query what a symbol calls.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "symbol": { "type": "string" },
                    "transitive": { "type": "boolean" }
                },
                "required": ["symbol"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "callers",
            "Query what calls a symbol.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "symbol": { "type": "string" },
                    "transitive": { "type": "boolean" }
                },
                "required": ["symbol"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "impact",
            "Estimate static impact for a change target.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "change_type": { "type": "string" },
                    "depth": { "type": "integer", "minimum": 0 }
                },
                "required": ["target", "change_type"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "explain",
            "Explain why a file or symbol appears in impact results.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "to": { "type": "string" },
                    "depth": { "type": "integer", "minimum": 0 }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "why",
            "Explain the shortest indexed path between two files or symbols.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "depth": { "type": "integer", "minimum": 0 }
                },
                "required": ["from", "to"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "context",
            "Recommend the minimum file set to read before making a change.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "targets": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "change_type": { "type": "string" },
                    "budget": { "type": "integer", "minimum": 0 }
                },
                "required": ["targets", "change_type"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "arch_check",
            "Check architecture rules against indexed file dependencies.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "stability",
            "Report Martin instability scores from indexed file dependencies.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "file": { "type": "string" },
                    "flag_threshold": { "type": "number" },
                    "sort": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "risk",
            "Report churn-weighted risk scores from indexed file dependencies.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "file": { "type": "string" },
                    "days": { "type": "integer", "minimum": 1 },
                    "threshold": { "type": "number" },
                    "top": { "type": "integer", "minimum": 0 },
                    "sort": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "surface",
            "Query the public API surface for a file or symbol target.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "surface_diff",
            "Compare public API surface between two indexed files or symbol targets.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "before": { "type": "string" },
                    "after": { "type": "string" }
                },
                "required": ["before", "after"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "test_map_build",
            "Detect test files and summarize static test coverage topology.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "test_map_covers",
            "Show which tests statically cover a source file.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "test_map_covered_by",
            "Show which source files are statically covered by a test file.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "test_map_uncovered",
            "List indexed non-test files with no static test coverage.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "unused",
            "Report exported symbols with no indexed inbound references.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "cycles",
            "Report circular file dependency chains.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "severity": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "diff",
            "Report blast radius for files changed relative to a git branch/ref.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "branch": { "type": "string" }
                },
                "required": ["branch"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "tree",
            "Render a recursive dependency tree for an indexed file.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "reverse": { "type": "boolean" },
                    "depth": { "type": "integer", "minimum": 0 }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "snapshot_save",
            "Save the current indexed graph as a named snapshot.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "name": { "type": "string" },
                    "commit": { "type": "string" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "snapshot_list",
            "List saved snapshots.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "snapshot_delete",
            "Delete a saved snapshot by name.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "name": { "type": "string" }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "diff_snapshot",
            "Compare two saved architectural snapshots.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "before": { "type": "string" },
                    "after": { "type": "string" }
                },
                "required": ["before", "after"],
                "additionalProperties": false
            }),
        ),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
    })
}

#[derive(Debug)]
enum DispatchError {
    Transport(String),
}

fn dispatch_tool(name: &str, arguments: &Value) -> Result<String, DispatchError> {
    let output = match name {
        "index" => dispatch_index(arguments),
        "deps" => dispatch_deps(arguments),
        "symbols" => dispatch_symbols(arguments),
        "calls" => dispatch_calls(arguments),
        "callers" => dispatch_callers(arguments),
        "impact" => dispatch_impact(arguments),
        "explain" => dispatch_explain(arguments),
        "why" => dispatch_why(arguments),
        "context" => dispatch_context(arguments),
        "arch_check" => dispatch_arch_check(arguments),
        "stability" => dispatch_stability(arguments),
        "risk" => dispatch_risk(arguments),
        "surface" => dispatch_surface(arguments),
        "surface_diff" => dispatch_surface_diff(arguments),
        "test_map_build" => dispatch_test_map_build(arguments),
        "test_map_covers" => dispatch_test_map_covers(arguments),
        "test_map_covered_by" => dispatch_test_map_covered_by(arguments),
        "test_map_uncovered" => dispatch_test_map_uncovered(arguments),
        "unused" => dispatch_unused(arguments),
        "cycles" => dispatch_cycles(arguments),
        "diff" => dispatch_diff(arguments),
        "tree" => dispatch_tree(arguments),
        "snapshot_save" => dispatch_snapshot_save(arguments),
        "snapshot_list" => dispatch_snapshot_list(arguments),
        "snapshot_delete" => dispatch_snapshot_delete(arguments),
        "diff_snapshot" => dispatch_diff_snapshot(arguments),
        other => return Err(DispatchError::Transport(format!("unknown tool: {other}"))),
    };

    Ok(output)
}

fn dispatch_index(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let no_git = optional_bool(arguments, "no_git").unwrap_or(false);
        let watch = optional_bool(arguments, "watch").unwrap_or(false);
        let indexed = index_repo(&context.paths.repo_root, &context.store)?;
        if no_git {
            context.store.clear_file_churn()?;
        }
        let database = DatabaseInfo {
            path: context.paths.db_path.display().to_string(),
            schema_version: context.store.schema_version()?,
        };
        serialize_json(&scope_core::stub::index(
            context.paths.repo_root.display().to_string(),
            no_git,
            watch,
            database,
            indexed.affected_files,
        ))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("index", &error),
    }
}

fn dispatch_deps(arguments: &Value) -> String {
    let file = required_string(arguments, "file");
    match file.and_then(|file| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let reverse = optional_bool(arguments, "reverse").unwrap_or(false);
            let transitive = optional_bool(arguments, "transitive").unwrap_or(false);
            let depth = optional_usize(arguments, "depth")?;
            let dependencies = if reverse {
                context.store.query_reverse_deps(&RepoPath::from(file.clone()))?
            } else {
                context.store.query_deps(&RepoPath::from(file.clone()))?
            };
            serialize_json(&scope_core::stub::deps(
                file,
                reverse,
                transitive,
                depth,
                dependencies,
            ))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("deps", &error),
    }
}

fn dispatch_symbols(arguments: &Value) -> String {
    let file = required_string(arguments, "file");
    match file.and_then(|file| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let public_only = optional_bool(arguments, "public_only").unwrap_or(false);
            let kind = optional_symbol_kind(arguments, "kind")?;
            let symbols = context
                .store
                .query_symbols(&RepoPath::from(file.clone()), public_only, kind.clone())?;
            serialize_json(&scope_core::stub::symbols(file, public_only, kind, symbols))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("symbols", &error),
    }
}

fn dispatch_calls(arguments: &Value) -> String {
    let symbol = required_string(arguments, "symbol");
    match symbol.and_then(|symbol| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let transitive = optional_bool(arguments, "transitive").unwrap_or(false);
            let traversals = context.store.query_callees(&symbol, transitive)?;
            serialize_json(&scope_core::stub::calls(symbol, transitive, traversals))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("calls", &error),
    }
}

fn dispatch_callers(arguments: &Value) -> String {
    let symbol = required_string(arguments, "symbol");
    match symbol.and_then(|symbol| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let transitive = optional_bool(arguments, "transitive").unwrap_or(false);
            let traversals = context.store.query_callers(&symbol, transitive)?;
            serialize_json(&scope_core::stub::callers(symbol, transitive, traversals))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("callers", &error),
    }
}

fn dispatch_impact(arguments: &Value) -> String {
    let target = required_string(arguments, "target");
    let change_type = required_string(arguments, "change_type");
    match target.and_then(|target| {
        change_type.and_then(|change_type| {
            bootstrap_from_arguments(arguments).and_then(|context| {
                let depth = optional_usize(arguments, "depth")?;
                let impacted = context.store.query_impact(&target, &change_type, depth)?;
                serialize_json(&scope_core::stub::impact(target, change_type, depth, impacted))
            })
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("impact", &error),
    }
}

fn dispatch_explain(arguments: &Value) -> String {
    let target = required_string(arguments, "target");
    match target.and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let to = optional_string(arguments, "to");
            let depth = optional_usize(arguments, "depth")?;
            let traversals = context.store.query_explain(&target, to.as_deref(), depth)?;
            serialize_json(&scope_core::stub::explain(target, to, depth, traversals))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("explain", &error),
    }
}

fn dispatch_why(arguments: &Value) -> String {
    let from = required_string(arguments, "from");
    let to = required_string(arguments, "to");
    match from.and_then(|from| {
        to.and_then(|to| {
            bootstrap_from_arguments(arguments).and_then(|context| {
                let depth = optional_usize(arguments, "depth")?;
                let path = context.store.query_why(&from, &to, depth)?;
                serialize_json(&scope_core::stub::why(from, to, depth, path))
            })
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("why", &error),
    }
}

fn dispatch_context(arguments: &Value) -> String {
    match required_string_array(arguments, "targets").and_then(|targets| {
        required_string(arguments, "change_type").and_then(|change_type| {
            bootstrap_from_arguments(arguments).and_then(|context| {
                let budget = optional_usize(arguments, "budget")?;
                let result = context.store.query_context(&targets, &change_type, budget)?;
                serialize_json(&scope_core::stub::context(result))
            })
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("context", &error),
    }
}

fn dispatch_arch_check(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let config = load_arch_config(&context.paths.repo_root)?;
        let result = arch_check(&context.store, &config)?;
        serialize_json(&scope_core::stub::arch_check(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("arch-check", &error),
    }
}

fn dispatch_stability(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let file = optional_string(arguments, "file").map(RepoPath::from);
        let flag_threshold = optional_f64(arguments, "flag_threshold")?;
        let sort = optional_stability_sort(arguments, "sort")?;
        let result = context
            .store
            .query_stability(file.as_ref(), flag_threshold, sort)?;
        serialize_json(&scope_core::stub::stability(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("stability", &error),
    }
}

fn dispatch_risk(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let file = optional_string(arguments, "file").map(RepoPath::from);
        let days = optional_u32(arguments, "days")?.unwrap_or(90);
        let threshold = optional_f64(arguments, "threshold")?;
        let top = optional_usize(arguments, "top")?;
        let sort = optional_risk_sort(arguments, "sort")?;
        let result = context
            .store
            .query_risk(file.as_ref(), days, threshold, top, sort)?;
        serialize_json(&scope_core::stub::risk(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("risk", &error),
    }
}

fn dispatch_surface(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let path = context.store.resolve_surface_target(&target)?;
            let surface = context.store.query_public_surface(&path)?;
            serialize_json(&scope_core::stub::surface(path, surface))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("surface", &error),
    }
}

fn dispatch_surface_diff(arguments: &Value) -> String {
    let before = required_string(arguments, "before");
    let after = required_string(arguments, "after");
    match before.and_then(|before| {
        after.and_then(|after| {
            bootstrap_from_arguments(arguments).and_then(|context| {
                let before_path = context.store.resolve_surface_target(&before)?;
                let after_path = context.store.resolve_surface_target(&after)?;
                let diff = context.store.diff_public_surface(&before_path, &after_path)?;
                serialize_json(&scope_core::stub::surface_diff(before_path, after_path, diff))
            })
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("surface-diff", &error),
    }
}

fn dispatch_test_map_build(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let config = load_arch_config(&context.paths.repo_root)?;
        let result = context.store.build_test_map(&config.tests)?;
        serialize_json(&scope_core::stub::test_map_build(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("test-map-build", &error),
    }
}

fn dispatch_test_map_covers(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let config = load_arch_config(&context.paths.repo_root)?;
            let result = context
                .store
                .query_tests_covering(&RepoPath::from(target), &config.tests)?;
            serialize_json(&scope_core::stub::test_map_covers(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("test-map-covers", &error),
    }
}

fn dispatch_test_map_covered_by(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let config = load_arch_config(&context.paths.repo_root)?;
            let result = context
                .store
                .query_test_coverage(&RepoPath::from(target), &config.tests)?;
            serialize_json(&scope_core::stub::test_map_covered_by(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("test-map-covered-by", &error),
    }
}

fn dispatch_test_map_uncovered(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let config = load_arch_config(&context.paths.repo_root)?;
        let result = context.store.query_uncovered_files(&config.tests)?;
        serialize_json(&scope_core::stub::test_map_uncovered(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("test-map-uncovered", &error),
    }
}

fn dispatch_unused(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let result = context.store.query_unused()?;
        serialize_json(&scope_core::stub::unused(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("unused", &error),
    }
}

fn dispatch_cycles(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let severity = optional_cycle_severity(arguments, "severity")?;
        let result = context.store.query_cycles(severity)?;
        serialize_json(&scope_core::stub::cycles(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("cycles", &error),
    }
}

fn dispatch_diff(arguments: &Value) -> String {
    match required_string(arguments, "branch").and_then(|branch| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let result = context.store.query_branch_diff(&context.paths.repo_root, &branch)?;
            serialize_json(&scope_core::stub::diff(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("diff", &error),
    }
}

fn dispatch_tree(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let reverse = optional_bool(arguments, "reverse").unwrap_or(false);
            let depth = optional_usize(arguments, "depth")?;
            let result = context
                .store
                .query_tree(&RepoPath::from(target), reverse, depth)?;
            serialize_json(&scope_core::stub::tree(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("tree", &error),
    }
}

fn dispatch_snapshot_save(arguments: &Value) -> String {
    match required_string(arguments, "name").and_then(|name| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let commit = optional_string(arguments, "commit");
            let result = context.store.save_snapshot(&name, commit)?;
            serialize_json(&scope_core::stub::snapshot_save(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("snapshot-save", &error),
    }
}

fn dispatch_snapshot_list(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let result = context.store.list_snapshots()?;
        serialize_json(&scope_core::stub::snapshot_list(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("snapshot-list", &error),
    }
}

fn dispatch_snapshot_delete(arguments: &Value) -> String {
    match required_string(arguments, "name").and_then(|name| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let result = context.store.delete_snapshot(&name)?;
            serialize_json(&scope_core::stub::snapshot_delete(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("snapshot-delete", &error),
    }
}

fn dispatch_diff_snapshot(arguments: &Value) -> String {
    let before = required_string(arguments, "before");
    let after = required_string(arguments, "after");
    match before.and_then(|before| {
        after.and_then(|after| {
            bootstrap_from_arguments(arguments).and_then(|context| {
                let config = load_arch_config(&context.paths.repo_root)?;
                let result = context.store.diff_snapshot(&before, &after, &config)?;
                serialize_json(&scope_core::stub::diff_snapshot(result))
            })
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("diff-snapshot", &error),
    }
}

fn bootstrap_from_arguments(arguments: &Value) -> Result<scope_core::AppContext, scope_core::ScopeError> {
    let repo_root_override = optional_string(arguments, "repo_root").map(PathBuf::from);
    let cwd = if let Some(repo_root) = repo_root_override.as_ref() {
        repo_root.clone()
    } else {
        env::current_dir().map_err(|error| scope_core::ScopeError::io(".", error))?
    };
    let options = BootstrapOptions {
        repo_root_override,
        db_override: optional_string(arguments, "db_path").map(PathBuf::from),
    };
    scope_core::bootstrap(&cwd, &options, Verbosity::Quiet)
}

fn required_string(arguments: &Value, key: &str) -> Result<String, scope_core::ScopeError> {
    optional_string(arguments, key).ok_or_else(|| {
        scope_core::ScopeError::InvalidInput(format!("mcp tool arguments require `{key}`"))
    })
}

fn optional_string(arguments: &Value, key: &str) -> Option<String> {
    arguments.get(key).and_then(Value::as_str).map(ToString::to_string)
}

fn required_string_array(arguments: &Value, key: &str) -> Result<Vec<String>, scope_core::ScopeError> {
    let Some(values) = arguments.get(key).and_then(Value::as_array) else {
        return Err(scope_core::ScopeError::InvalidInput(format!(
            "mcp tool arguments require `{key}` as an array of strings"
        )));
    };
    let mut parsed = Vec::with_capacity(values.len());
    for value in values {
        let Some(value) = value.as_str() else {
            return Err(scope_core::ScopeError::InvalidInput(format!(
                "mcp tool argument `{key}` must contain only strings"
            )));
        };
        parsed.push(value.to_string());
    }
    Ok(parsed)
}

fn optional_bool(arguments: &Value, key: &str) -> Option<bool> {
    arguments.get(key).and_then(Value::as_bool)
}

fn optional_usize(arguments: &Value, key: &str) -> Result<Option<usize>, scope_core::ScopeError> {
    arguments
        .get(key)
        .map(|value| {
            value.as_u64().map(|value| value as usize).ok_or_else(|| {
                scope_core::ScopeError::InvalidInput(format!(
                    "mcp tool argument `{key}` must be a non-negative integer"
                ))
            })
        })
        .transpose()
}

fn optional_u32(arguments: &Value, key: &str) -> Result<Option<u32>, scope_core::ScopeError> {
    arguments
        .get(key)
        .map(|value| {
            value.as_u64().map(|value| value as u32).ok_or_else(|| {
                scope_core::ScopeError::InvalidInput(format!(
                    "mcp tool argument `{key}` must be a non-negative integer"
                ))
            })
        })
        .transpose()
}

fn optional_f64(arguments: &Value, key: &str) -> Result<Option<f64>, scope_core::ScopeError> {
    arguments
        .get(key)
        .map(|value| {
            value.as_f64().ok_or_else(|| {
                scope_core::ScopeError::InvalidInput(format!(
                    "mcp tool argument `{key}` must be a number"
                ))
            })
        })
        .transpose()
}

fn optional_symbol_kind(arguments: &Value, key: &str) -> Result<Option<SymbolKind>, scope_core::ScopeError> {
    optional_string(arguments, key)
        .map(|kind| match kind.as_str() {
            "function" => Ok(SymbolKind::Function),
            "struct" => Ok(SymbolKind::Struct),
            "enum" => Ok(SymbolKind::Enum),
            "trait" => Ok(SymbolKind::Trait),
            "method" => Ok(SymbolKind::Method),
            "module" => Ok(SymbolKind::Module),
            "constant" => Ok(SymbolKind::Constant),
            "variable" => Ok(SymbolKind::Variable),
            _ => Err(scope_core::ScopeError::InvalidInput(format!(
                "unsupported symbol kind: {kind}"
            ))),
        })
        .transpose()
}

fn optional_cycle_severity(arguments: &Value, key: &str) -> Result<Option<scope_core::CycleSeverity>, scope_core::ScopeError> {
    optional_string(arguments, key)
        .map(|severity| match severity.as_str() {
            "low" => Ok(scope_core::CycleSeverity::Low),
            "medium" => Ok(scope_core::CycleSeverity::Medium),
            "high" => Ok(scope_core::CycleSeverity::High),
            _ => Err(scope_core::ScopeError::InvalidInput(format!(
                "unsupported cycle severity: {severity}"
            ))),
        })
        .transpose()
}

fn optional_stability_sort(arguments: &Value, key: &str) -> Result<scope_core::StabilitySort, scope_core::ScopeError> {
    match optional_string(arguments, key).as_deref() {
        None | Some("instability") => Ok(scope_core::StabilitySort::Instability),
        Some("fan_in") => Ok(scope_core::StabilitySort::FanIn),
        Some("fan_out") => Ok(scope_core::StabilitySort::FanOut),
        Some("path") => Ok(scope_core::StabilitySort::Path),
        Some(other) => Err(scope_core::ScopeError::InvalidInput(format!(
            "unsupported stability sort: {other}"
        ))),
    }
}

fn optional_risk_sort(arguments: &Value, key: &str) -> Result<RiskSort, scope_core::ScopeError> {
    match optional_string(arguments, key).as_deref() {
        None | Some("score") => Ok(RiskSort::Score),
        Some("churn") => Ok(RiskSort::Churn),
        Some("dependents") => Ok(RiskSort::Dependents),
        Some("path") => Ok(RiskSort::Path),
        Some(other) => Err(scope_core::ScopeError::InvalidInput(format!(
            "unsupported risk sort: {other}"
        ))),
    }
}

fn serialize_json<T: serde::Serialize>(value: &T) -> Result<String, scope_core::ScopeError> {
    serde_json::to_string_pretty(value).map_err(|error| {
        scope_core::ScopeError::InvalidInput(format!("failed to serialize MCP result: {error}"))
    })
}

fn render_domain_error(command: &'static str, error: &scope_core::ScopeError) -> String {
    serialize_json(&scope_core::JsonEnvelope::error(command, error))
        .unwrap_or_else(|_| {
            "{\n  \"schema_version\": 1,\n  \"command\": \"mcp\",\n  \"status\": \"error\",\n  \"data\": {\n    \"kind\": \"serialization\",\n    \"message\": \"failed to serialize error envelope\"\n  },\n  \"warnings\": []\n}".to_string()
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexRunStats {
    indexed_files: usize,
    changed_files: usize,
    deleted_files: usize,
    affected_files: usize,
}

fn index_repo(repo_root: &Path, store: &Store) -> Result<IndexRunStats, scope_core::ScopeError> {
    let entries = scan_repo(repo_root, &ScanConfig::default())?;
    let extracts: Vec<_> = entries
        .into_iter()
        .filter_map(|entry| {
            let adapter = adapter_for_language(entry.language)?;
            if !scope_core::adapters::supports_path(adapter, &entry.absolute_path) {
                return None;
            }
            let source = fs::read_to_string(&entry.absolute_path).ok()?;
            let mut extract = adapter.extract(&entry, &source);
            let metadata = fs::metadata(&entry.absolute_path).ok()?;
            let modified = metadata.modified().ok()?;
            let modified_seconds = modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs() as i64);
            extract.file.content_hash = Some(blake3::hash(source.as_bytes()).to_hex().to_string());
            extract.file.mtime_unix_seconds = modified_seconds;
            extract.file.size_bytes = Some(metadata.len() as i64);
            Some(extract)
        })
        .collect();

    let extract_map: HashMap<RepoPath, scope_core::ExtractResult> = extracts
        .into_iter()
        .map(|extract| (extract.file.path.clone(), extract))
        .collect();
    let scanned_paths: HashSet<_> = extract_map.keys().cloned().collect();
    let indexed_paths = store.list_indexed_files()?;
    if indexed_paths.is_empty() {
        let mut all_extracts: Vec<_> = extract_map.into_values().collect();
        all_extracts.sort_by(|left, right| left.file.path.cmp(&right.file.path));
        let indexed_files = all_extracts.len();
        store.persist_extract_results(&all_extracts)?;
        return Ok(IndexRunStats {
            indexed_files,
            changed_files: indexed_files,
            deleted_files: 0,
            affected_files: indexed_files,
        });
    }

    let mut changed_or_new = Vec::new();
    for extract in extract_map.values() {
        match store.classify_file_change(&extract.file)? {
            None | Some(true) => changed_or_new.push(extract.file.path.clone()),
            Some(false) => {}
        }
    }

    let deleted_paths: Vec<_> = indexed_paths
        .into_iter()
        .filter(|path| !scanned_paths.contains(path))
        .collect();

    let mut affected_paths: HashSet<_> = changed_or_new.iter().cloned().collect();
    let mut closure_seeds = changed_or_new;
    closure_seeds.extend(deleted_paths.iter().cloned());

    for dependent in store.reverse_dependency_closure(&closure_seeds)? {
        affected_paths.insert(dependent);
    }

    for path in &deleted_paths {
        let _ = store.delete_file(path)?;
    }

    let mut affected_extracts: Vec<_> = affected_paths
        .into_iter()
        .filter_map(|path| extract_map.get(&path).cloned())
        .collect();
    affected_extracts.sort_by(|left, right| left.file.path.cmp(&right.file.path));

    for extract in &affected_extracts {
        store.upsert_file(&extract.file)?;
    }
    for extract in &affected_extracts {
        store.persist_extract_result(extract)?;
    }
    for extract in &affected_extracts {
        store.refresh_call_edges(extract)?;
    }

    Ok(IndexRunStats {
        indexed_files: extract_map.len(),
        changed_files: closure_seeds.len().saturating_sub(deleted_paths.len()),
        deleted_files: deleted_paths.len(),
        affected_files: affected_extracts.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root should resolve")
    }

    fn fixture_root(name: &str) -> PathBuf {
        repo_root().join("fixtures").join(name)
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("scope-mcp-{prefix}-{nanos}-{counter}"))
    }

    fn copy_dir_recursive(root: &Path, src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir_recursive(root, &src_path, &dst_path);
            } else {
                if src_path
                    .strip_prefix(root)
                    .ok()
                    .and_then(|relative| relative.to_str())
                    == Some(".scope/index.db")
                {
                    continue;
                }
                fs::copy(&src_path, &dst_path).unwrap();
            }
        }
    }

    fn prepare_fixture_copy(name: &str) -> PathBuf {
        let src = fixture_root(name);
        let dst = unique_temp_dir(name);
        copy_dir_recursive(&src, &src, &dst);
        dst
    }

    #[test]
    fn tools_list_reports_expected_initial_wrapper_scope() {
        let tools = tool_registry();
        assert!(tools.iter().any(|tool| tool["name"] == "deps"));
        assert!(tools.iter().any(|tool| tool["name"] == "impact"));
        assert!(tools.iter().any(|tool| tool["name"] == "arch_check"));
        assert!(tools.iter().any(|tool| tool["name"] == "surface"));
        assert!(tools.iter().any(|tool| tool["name"] == "surface_diff"));
        assert!(tools.iter().any(|tool| tool["name"] == "test_map_covers"));
        assert!(tools.iter().any(|tool| tool["name"] == "snapshot_save"));
        assert!(tools.iter().any(|tool| tool["name"] == "snapshot_list"));
        assert!(tools.iter().any(|tool| tool["name"] == "snapshot_delete"));
        assert!(tools.iter().any(|tool| tool["name"] == "diff_snapshot"));
    }

    #[test]
    fn dispatch_deps_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("rust_small");
        let index_output = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();
        let index_value: Value = serde_json::from_str(&index_output).unwrap();
        assert_eq!(index_value["command"], "index");
        assert_eq!(index_value["status"], "ok");

        let output = dispatch_tool(
            "deps",
            &json!({
                "repo_root": repo.display().to_string(),
                "file": "src/lib.rs"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "deps");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/lib.rs");
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_unknown_tool_returns_transport_error() {
        let error = dispatch_tool("does_not_exist", &json!({})).unwrap_err();
        match error {
            DispatchError::Transport(message) => assert!(message.contains("unknown tool")),
        }
    }

    #[test]
    fn dispatch_surface_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("ts_small");
        let index_output = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();
        let index_value: Value = serde_json::from_str(&index_output).unwrap();
        assert_eq!(index_value["command"], "index");

        let output = dispatch_tool(
            "surface",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/auth/middleware.ts"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "surface");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/auth/middleware.ts");
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_test_map_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("test_map_ts");
        let index_output = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();
        let index_value: Value = serde_json::from_str(&index_output).unwrap();
        assert_eq!(index_value["command"], "index");

        let output = dispatch_tool(
            "test_map_covers",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/auth/middleware.ts"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "test-map-covers");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["source_file"], "src/auth/middleware.ts");
        assert_eq!(value["data"]["result"]["summary"]["covering_tests"], 3);
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_test_map_invalid_target_returns_domain_error_envelope() {
        let repo = prepare_fixture_copy("test_map_ts");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "test_map_covered_by",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/auth/middleware.ts"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "test-map-covered-by");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_snapshot_round_trip_returns_scope_json_envelopes() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let save_output = dispatch_tool(
            "snapshot_save",
            &json!({
                "repo_root": repo.display().to_string(),
                "name": "baseline",
                "commit": "HEAD"
            }),
        )
        .unwrap();
        let save_value: Value = serde_json::from_str(&save_output).unwrap();
        assert_eq!(save_value["command"], "snapshot-save");
        assert_eq!(save_value["status"], "ok");
        assert_eq!(save_value["data"]["result"]["snapshot"]["name"], "baseline");

        let list_output = dispatch_tool(
            "snapshot_list",
            &json!({ "repo_root": repo.display().to_string() }),
        )
        .unwrap();
        let list_value: Value = serde_json::from_str(&list_output).unwrap();
        assert_eq!(list_value["command"], "snapshot-list");
        assert_eq!(list_value["status"], "ok");
        assert_eq!(list_value["data"]["result"]["summary"]["snapshot_count"], 1);

        let diff_output = dispatch_tool(
            "diff_snapshot",
            &json!({
                "repo_root": repo.display().to_string(),
                "before": "baseline",
                "after": "baseline"
            }),
        )
        .unwrap();
        let diff_value: Value = serde_json::from_str(&diff_output).unwrap();
        assert_eq!(diff_value["command"], "diff-snapshot");
        assert_eq!(diff_value["status"], "ok");
        assert_eq!(diff_value["data"]["result"]["summary"]["files_changed"], 0);

        let delete_output = dispatch_tool(
            "snapshot_delete",
            &json!({
                "repo_root": repo.display().to_string(),
                "name": "baseline"
            }),
        )
        .unwrap();
        let delete_value: Value = serde_json::from_str(&delete_output).unwrap();
        assert_eq!(delete_value["command"], "snapshot-delete");
        assert_eq!(delete_value["status"], "ok");
        assert_eq!(delete_value["data"]["result"]["deleted"], true);
        fs::remove_dir_all(repo).unwrap();
    }
}
