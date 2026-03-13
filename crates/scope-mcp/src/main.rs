use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::{Instant, UNIX_EPOCH},
};

use scope_core::{
    adapter_for_language, arch_check, execute_query, load_arch_config, scan_repo,
    BootstrapOptions, CochangeSort, DatabaseInfo, QuerySession, RepoPath, RiskSort, ScanConfig,
    Store, SymbolKind, Verbosity,
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
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
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
            "pack",
            "Generate a budgeted plain-text context pack for an agent.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "change_type": { "type": "string" },
                    "budget": { "type": "integer", "minimum": 0 }
                },
                "required": ["target", "budget"],
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
            "audit",
            "Trace entry points that can reach a configured sensitive capability.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "capability": { "type": "string" }
                },
                "required": ["capability"],
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
            "cochange",
            "Report files that frequently change with a target file across recent commits.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "days": { "type": "integer", "minimum": 1 },
                    "min_shared_commits": { "type": "integer", "minimum": 1 },
                    "top": { "type": "integer", "minimum": 0 },
                    "sort": { "type": "string" }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "report",
            "Return repository health metrics and architectural findings.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "compare": { "type": "string" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "gate",
            "Evaluate configured CI-style quality gates against repository health.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "compare": { "type": "string" },
                    "strict": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "query",
            "Run a composable graph query over indexed files and symbols.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "expr": { "type": "string" }
                },
                "required": ["expr"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "simulate_extract",
            "Simulate extracting symbols into a new file without persisting changes.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "symbols": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1
                    },
                    "into_file": { "type": "string" }
                },
                "required": ["symbols", "into_file"],
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
            "rename_plan",
            "Build a conservative rename execution plan for a file or symbol target.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "to": { "type": "string" },
                    "apply": { "type": "boolean" },
                    "force": { "type": "boolean" }
                },
                "required": ["target", "to"],
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
            "split",
            "Suggest decomposition clusters for a large indexed file.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "clusters": { "type": "integer", "minimum": 1 }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "mirror",
            "Compare a file against structurally similar indexed files.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "target": { "type": "string" },
                    "other": { "type": "string" },
                    "threshold": { "type": "integer", "minimum": 0, "maximum": 100 },
                    "top": { "type": "integer", "minimum": 1 }
                },
                "required": ["target"],
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "entry_list",
            "List detected entry points.",
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
            "entry_cone",
            "Show files reachable from a detected entry point.",
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
            "entry_reaches",
            "Show which entry points can reach a target file.",
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
            "entry_unreachable",
            "Show indexed files reachable from no detected entry point.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "min_age_days": { "type": "integer", "minimum": 0 }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "doctor",
            "Inspect repository and index health.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "fix": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        ),
        tool_definition(
            "benchmark",
            "Benchmark full versus incremental indexing on an isolated repo copy.",
            json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "db_path": { "type": "string" },
                    "fixture": { "type": "string" },
                    "iterations": { "type": "integer", "minimum": 1 }
                },
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
        "pack" => dispatch_pack(arguments),
        "arch_check" => dispatch_arch_check(arguments),
        "audit" => dispatch_audit(arguments),
        "stability" => dispatch_stability(arguments),
        "risk" => dispatch_risk(arguments),
        "cochange" => dispatch_cochange(arguments),
        "report" => dispatch_report(arguments),
        "gate" => dispatch_gate(arguments),
        "query" => dispatch_query(arguments),
        "simulate_extract" => dispatch_simulate_extract(arguments),
        "surface" => dispatch_surface(arguments),
        "surface_diff" => dispatch_surface_diff(arguments),
        "test_map_build" => dispatch_test_map_build(arguments),
        "test_map_covers" => dispatch_test_map_covers(arguments),
        "test_map_covered_by" => dispatch_test_map_covered_by(arguments),
        "test_map_uncovered" => dispatch_test_map_uncovered(arguments),
        "rename_plan" => dispatch_rename_plan(arguments),
        "unused" => dispatch_unused(arguments),
        "cycles" => dispatch_cycles(arguments),
        "diff" => dispatch_diff(arguments),
        "tree" => dispatch_tree(arguments),
        "split" => dispatch_split(arguments),
        "mirror" => dispatch_mirror(arguments),
        "entry_list" => dispatch_entry_list(arguments),
        "entry_cone" => dispatch_entry_cone(arguments),
        "entry_reaches" => dispatch_entry_reaches(arguments),
        "entry_unreachable" => dispatch_entry_unreachable(arguments),
        "doctor" => dispatch_doctor(arguments),
        "benchmark" => dispatch_benchmark(arguments),
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
                context
                    .store
                    .query_reverse_deps(&RepoPath::from(file.clone()))?
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
            let symbols = context.store.query_symbols(
                &RepoPath::from(file.clone()),
                public_only,
                kind.clone(),
            )?;
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
                serialize_json(&scope_core::stub::impact(
                    target,
                    change_type,
                    depth,
                    impacted,
                ))
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
                let result = context
                    .store
                    .query_context(&targets, &change_type, budget)?;
                serialize_json(&scope_core::stub::context(result))
            })
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("context", &error),
    }
}

fn dispatch_pack(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let change_type =
                optional_string(arguments, "change_type").unwrap_or_else(|| "body".to_string());
            let budget = optional_usize(arguments, "budget")?.ok_or_else(|| {
                scope_core::ScopeError::InvalidInput(
                    "mcp tool arguments require `budget`".to_string(),
                )
            })?;
            let pack = build_context_pack(&context.store, &target, &change_type, budget)?;
            Ok(pack)
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("pack", &error),
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

fn dispatch_audit(arguments: &Value) -> String {
    match required_string(arguments, "capability").and_then(|capability| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let config = load_arch_config(&context.paths.repo_root)?;
            let result = context.store.query_audit(&config, &capability)?;
            serialize_json(&scope_core::stub::audit(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("audit", &error),
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

fn dispatch_cochange(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let days = optional_u32(arguments, "days")?.unwrap_or(90);
            let min_shared_commits = optional_usize(arguments, "min_shared_commits")?.unwrap_or(1);
            let top = optional_usize(arguments, "top")?;
            let sort = optional_cochange_sort(arguments, "sort")?;
            let result = context.store.query_cochange(
                &RepoPath::from(target),
                days,
                min_shared_commits,
                top,
                sort,
            )?;
            serialize_json(&scope_core::stub::cochange(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("cochange", &error),
    }
}

fn dispatch_report(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let compare = optional_string_arg(arguments, "compare")?;
        let config = load_arch_config(&context.paths.repo_root)?;
        let result = context.store.query_report(&config, compare.as_deref())?;
        serialize_json(&scope_core::stub::report(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("report", &error),
    }
}

fn dispatch_gate(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let compare = optional_string_arg(arguments, "compare")?;
        let strict = optional_bool_arg(arguments, "strict")?.unwrap_or(false);
        let config = load_arch_config(&context.paths.repo_root)?;
        let result = context.store.query_gate(&config, compare.as_deref(), strict)?;
        serialize_json(&scope_core::stub::gate(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("gate", &error),
    }
}

fn dispatch_query(arguments: &Value) -> String {
    match required_string_arg(arguments, "expr").and_then(|expr| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let mut session = QuerySession::default();
            let result = execute_query(&expr, &context.store, &mut session)?;
            serialize_json(&scope_core::stub::query(expr, result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("query", &error),
    }
}

fn dispatch_simulate_extract(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let symbols = required_string_array(arguments, "symbols")?;
        let into_file = required_string(arguments, "into_file")?;
        let config = load_arch_config(&context.paths.repo_root)?;
        let result = context
            .store
            .simulate_extract(&symbols, &RepoPath::from(into_file), &config)?;
        serialize_json(&scope_core::stub::simulate_extract(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("simulate-extract", &error),
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
                let diff = context
                    .store
                    .diff_public_surface(&before_path, &after_path)?;
                serialize_json(&scope_core::stub::surface_diff(
                    before_path,
                    after_path,
                    diff,
                ))
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

fn dispatch_rename_plan(arguments: &Value) -> String {
    let target = required_string(arguments, "target");
    let new_name = required_string(arguments, "to");
    match target.and_then(|target| {
        new_name.and_then(|new_name| {
            bootstrap_from_arguments(arguments).and_then(|context| {
                validate_new_name(&new_name)?;
                let apply = optional_bool(arguments, "apply").unwrap_or(false);
                let force = optional_bool(arguments, "force").unwrap_or(false);
                let plan = context.store.build_rename_plan(
                    &context.paths.repo_root,
                    &target,
                    &new_name,
                    apply,
                    force,
                )?;
                serialize_json(&scope_core::stub::rename_plan(plan))
            })
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("rename-plan", &error),
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
            let result = context
                .store
                .query_branch_diff(&context.paths.repo_root, &branch)?;
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

fn dispatch_split(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let clusters = optional_usize(arguments, "clusters")?;
            let result = context.store.query_split(&RepoPath::from(target), clusters)?;
            serialize_json(&scope_core::stub::split(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("split", &error),
    }
}

fn dispatch_mirror(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let other = optional_string(arguments, "other").map(RepoPath::from);
            let threshold = optional_u32(arguments, "threshold")?;
            let top = optional_usize(arguments, "top")?;
            let result = context
                .store
                .query_mirror(&RepoPath::from(target), other.as_ref(), threshold, top)?;
            serialize_json(&scope_core::stub::mirror(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("mirror", &error),
    }
}

fn dispatch_entry_list(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let config = load_arch_config(&context.paths.repo_root)?;
        let result = context.store.query_entry_list(&config)?;
        serialize_json(&scope_core::stub::entry_list(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("entry-list", &error),
    }
}

fn dispatch_entry_cone(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let config = load_arch_config(&context.paths.repo_root)?;
            let result = context
                .store
                .query_entry_cone(&config, &RepoPath::from(target))?;
            serialize_json(&scope_core::stub::entry_cone(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("entry-cone", &error),
    }
}

fn dispatch_entry_reaches(arguments: &Value) -> String {
    match required_string(arguments, "target").and_then(|target| {
        bootstrap_from_arguments(arguments).and_then(|context| {
            let config = load_arch_config(&context.paths.repo_root)?;
            let result = context
                .store
                .query_entry_reaches(&config, &RepoPath::from(target))?;
            serialize_json(&scope_core::stub::entry_reaches(result))
        })
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("entry-reaches", &error),
    }
}

fn dispatch_entry_unreachable(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let config = load_arch_config(&context.paths.repo_root)?;
        let min_age_days = optional_u64(arguments, "min_age_days")?;
        let result = context
            .store
            .query_entry_unreachable(&config, min_age_days)?;
        serialize_json(&scope_core::stub::entry_unreachable(result))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("entry-unreachable", &error),
    }
}

fn dispatch_doctor(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let fix = optional_bool(arguments, "fix").unwrap_or(false);
        let stats = context.store.index_health_stats()?;
        serialize_json(&scope_core::stub::doctor(fix, stats))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("doctor", &error),
    }
}

fn dispatch_benchmark(arguments: &Value) -> String {
    match bootstrap_from_arguments(arguments).and_then(|context| {
        let fixture = optional_string(arguments, "fixture");
        let iterations = optional_u32(arguments, "iterations")?;
        let summary = run_benchmark(
            &context.paths.repo_root,
            fixture.as_deref(),
            iterations.unwrap_or(1),
        )?;
        serialize_json(&scope_core::stub::benchmark(fixture, iterations, summary))
    }) {
        Ok(output) => output,
        Err(error) => render_domain_error("benchmark", &error),
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

fn bootstrap_from_arguments(
    arguments: &Value,
) -> Result<scope_core::AppContext, scope_core::ScopeError> {
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
    required_string_arg(arguments, key)
}

fn required_string_arg(arguments: &Value, key: &str) -> Result<String, scope_core::ScopeError> {
    optional_string_arg(arguments, key)?.ok_or_else(|| {
        scope_core::ScopeError::InvalidInput(format!("mcp tool arguments require `{key}`"))
    })
}

fn optional_string(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn optional_string_arg(
    arguments: &Value,
    key: &str,
) -> Result<Option<String>, scope_core::ScopeError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(scope_core::ScopeError::InvalidInput(format!(
            "mcp tool argument `{key}` must be a string when provided"
        ))),
    }
}

fn required_string_array(
    arguments: &Value,
    key: &str,
) -> Result<Vec<String>, scope_core::ScopeError> {
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

fn optional_bool_arg(
    arguments: &Value,
    key: &str,
) -> Result<Option<bool>, scope_core::ScopeError> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(scope_core::ScopeError::InvalidInput(format!(
            "mcp tool argument `{key}` must be a boolean when provided"
        ))),
    }
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

fn optional_u64(arguments: &Value, key: &str) -> Result<Option<u64>, scope_core::ScopeError> {
    arguments
        .get(key)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
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

fn optional_symbol_kind(
    arguments: &Value,
    key: &str,
) -> Result<Option<SymbolKind>, scope_core::ScopeError> {
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

fn optional_cycle_severity(
    arguments: &Value,
    key: &str,
) -> Result<Option<scope_core::CycleSeverity>, scope_core::ScopeError> {
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

fn optional_stability_sort(
    arguments: &Value,
    key: &str,
) -> Result<scope_core::StabilitySort, scope_core::ScopeError> {
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

fn optional_cochange_sort(
    arguments: &Value,
    key: &str,
) -> Result<CochangeSort, scope_core::ScopeError> {
    match optional_string(arguments, key).as_deref() {
        None | Some("score") => Ok(CochangeSort::Score),
        Some("shared_commits") => Ok(CochangeSort::SharedCommits),
        Some("path") => Ok(CochangeSort::Path),
        Some(other) => Err(scope_core::ScopeError::InvalidInput(format!(
            "unsupported cochange sort: {other}"
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

fn build_context_pack(
    store: &scope_core::Store,
    target: &str,
    change_type: &str,
    budget: usize,
) -> Result<String, scope_core::ScopeError> {
    let context = store.query_context(&[target.to_string()], change_type, Some(budget))?;
    let mut sections = Vec::new();

    let public_surface = format_public_surface(store, target)?;
    if !public_surface.is_empty() {
        sections.push(public_surface);
    }

    let direct_callers = format_direct_callers(store, target)?;
    if !direct_callers.is_empty() {
        sections.push(direct_callers);
    }

    let direct_callees = format_direct_callees(store, target)?;
    if !direct_callees.is_empty() {
        sections.push(direct_callees);
    }

    let transitive_callers = format_transitive_callers(&context.should_read);
    if !transitive_callers.is_empty() {
        sections.push(transitive_callers);
    }

    let change_section = format_change_specific_section(store, target, change_type)?;
    if !change_section.is_empty() {
        sections.push(change_section);
    }

    let header_without_used = vec![
        "=== SCOPE CONTEXT PACK ===".to_string(),
        format!("Target:      {target}"),
        format!("Change type: {change_type}"),
        format!("Budget:      {budget} tokens (approx)"),
        "Used:        0 tokens (approx)".to_string(),
        format!("Schema:      {}", scope_core::SCHEMA_VERSION),
        String::new(),
    ]
    .join("\n");
    let base_overhead = estimate_text_tokens(&header_without_used)
        + estimate_text_tokens(&format!(
            "END SCOPE PACK | schema: {} | truncated: yes",
            scope_core::SCHEMA_VERSION
        ));

    let mut body = String::new();
    let mut body_used = 0usize;
    let mut truncated = context.summary.truncated || base_overhead > budget;
    for section in sections {
        let section_tokens = estimate_text_tokens(&section);
        if base_overhead + body_used + section_tokens > budget {
            truncated = true;
            break;
        }
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&section);
        body_used += section_tokens;
    }

    let footer = format!(
        "END SCOPE PACK | schema: {} | truncated: {}",
        scope_core::SCHEMA_VERSION,
        if truncated { "yes" } else { "no" }
    );

    let header = vec![
        "=== SCOPE CONTEXT PACK ===".to_string(),
        format!("Target:      {target}"),
        format!("Change type: {change_type}"),
        format!("Budget:      {budget} tokens (approx)"),
        format!(
            "Used:        {} tokens (approx)",
            estimate_text_tokens(&header_without_used) + body_used + estimate_text_tokens(&footer)
        ),
        format!("Schema:      {}", scope_core::SCHEMA_VERSION),
        String::new(),
    ]
    .join("\n");

    let mut pack = header;
    if !body.is_empty() {
        pack.push_str("\n\n");
        pack.push_str(&body);
    }
    pack.push_str("\n\n");
    pack.push_str(&footer);

    Ok(pack)
}

fn format_public_surface(
    store: &scope_core::Store,
    target: &str,
) -> Result<String, scope_core::ScopeError> {
    let path = store.target_file_for_target(target)?;
    let Some(path) = path else {
        return Ok(String::new());
    };
    let surface = store.query_public_surface(&path)?;
    if surface.symbols.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec![format!("--- PUBLIC SURFACE ({}) ---", path.0)];
    for symbol in surface.symbols {
        lines.push(format!(
            "{} | {} | {} | line {}",
            symbol.qualname,
            symbol_kind_label(&symbol.kind),
            visibility_label(&symbol.visibility),
            symbol.line
        ));
    }
    Ok(lines.join("\n"))
}

fn format_direct_callers(
    store: &scope_core::Store,
    target: &str,
) -> Result<String, scope_core::ScopeError> {
    if !looks_like_symbol(target) {
        return Ok(String::new());
    }
    let records = store.query_callers(target, false)?;
    let records: Vec<_> = records
        .into_iter()
        .filter(|record| {
            matches!(
                record.certainty,
                scope_core::Certainty::Exact | scope_core::Certainty::Resolved
            )
        })
        .collect();
    if records.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec!["--- DIRECT CALLERS ---".to_string()];
    for record in records {
        lines.push(format_traversal_line(&record));
    }
    Ok(lines.join("\n"))
}

fn format_direct_callees(
    store: &scope_core::Store,
    target: &str,
) -> Result<String, scope_core::ScopeError> {
    if !looks_like_symbol(target) {
        return Ok(String::new());
    }
    let records = store.query_callees(target, false)?;
    if records.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec!["--- DIRECT CALLEES ---".to_string()];
    for record in records {
        lines.push(format_traversal_line(&record));
    }
    Ok(lines.join("\n"))
}

fn format_transitive_callers(should_read: &[scope_core::ContextFileRecord]) -> String {
    let nearby: Vec<_> = should_read
        .iter()
        .filter(|record| {
            record.distance == 2
                || record
                    .roles
                    .contains(&scope_core::ContextFileRole::NearbyContext)
                || record
                    .roles
                    .contains(&scope_core::ContextFileRole::Importer)
        })
        .collect();
    if nearby.is_empty() {
        return String::new();
    }
    let mut lines = vec!["--- TRANSITIVE CALLERS / NEARBY CONTEXT ---".to_string()];
    for record in nearby {
        lines.push(format_context_record_line(record));
    }
    lines.join("\n")
}

fn format_change_specific_section(
    store: &scope_core::Store,
    target: &str,
    change_type: &str,
) -> Result<String, scope_core::ScopeError> {
    let impacted = store.query_impact(target, change_type, None)?;
    if impacted.is_empty() {
        return Ok(String::new());
    }
    let title = match change_type {
        "rename" => "--- RENAME IMPACT ---",
        "delete" => "--- DELETE IMPACT ---",
        "signature" => "--- SIGNATURE IMPACT ---",
        "body" => "--- BODY IMPACT ---",
        "visibility" => "--- VISIBILITY IMPACT ---",
        "side-effect" => "--- SIDE-EFFECT IMPACT ---",
        _ => "--- IMPACT ---",
    };
    let mut lines = vec![title.to_string()];
    for record in impacted {
        lines.push(format_traversal_line(&record));
    }
    Ok(lines.join("\n"))
}

fn format_traversal_line(record: &scope_core::TraversalRecord) -> String {
    let path = record
        .path
        .as_ref()
        .map(|path| path.0.as_str())
        .unwrap_or("<unknown>");
    let label = record.qualname.as_deref().unwrap_or(path);
    format!(
        "{} | {} | certainty: {} | distance: {} | {}",
        path,
        label,
        certainty_label(&record.certainty),
        record.distance,
        record.reason
    )
}

fn format_context_record_line(record: &scope_core::ContextFileRecord) -> String {
    format!(
        "{} | tokens: {} | distance: {} | certainty: {} | roles: {}",
        record.path.0,
        record.estimated_tokens,
        record.distance,
        certainty_label(&record.certainty),
        record
            .roles
            .iter()
            .map(context_role_label)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn validate_new_name(new_name: &str) -> Result<(), scope_core::ScopeError> {
    if new_name.is_empty()
        || !new_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
    {
        return Err(scope_core::ScopeError::InvalidInput(
            "rename-plan requires a simple identifier for --to".to_string(),
        ));
    }
    Ok(())
}

fn looks_like_symbol(target: &str) -> bool {
    target.contains("::")
        && !target.ends_with(".rs")
        && !target.ends_with(".ts")
        && !target.ends_with(".js")
}

fn estimate_text_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

fn certainty_label(certainty: &scope_core::Certainty) -> &'static str {
    match certainty {
        scope_core::Certainty::Exact => "exact",
        scope_core::Certainty::Resolved => "resolved",
        scope_core::Certainty::Heuristic => "heuristic",
        scope_core::Certainty::Dynamic => "dynamic",
    }
}

fn visibility_label(visibility: &scope_core::Visibility) -> &'static str {
    match visibility {
        scope_core::Visibility::Local => "local",
        scope_core::Visibility::Module => "module",
        scope_core::Visibility::Package => "package",
        scope_core::Visibility::Public => "public",
        scope_core::Visibility::Unknown => "unknown",
    }
}

fn symbol_kind_label(kind: &scope_core::SymbolKind) -> &'static str {
    match kind {
        scope_core::SymbolKind::Function => "function",
        scope_core::SymbolKind::Method => "method",
        scope_core::SymbolKind::Struct => "struct",
        scope_core::SymbolKind::Class => "class",
        scope_core::SymbolKind::Enum => "enum",
        scope_core::SymbolKind::TypeAlias => "type_alias",
        scope_core::SymbolKind::Module => "module",
        scope_core::SymbolKind::Namespace => "namespace",
        scope_core::SymbolKind::Constant => "constant",
        scope_core::SymbolKind::Static => "static",
        scope_core::SymbolKind::Interface => "interface",
        scope_core::SymbolKind::Trait => "trait",
        scope_core::SymbolKind::Variable => "variable",
    }
}

fn context_role_label(role: &scope_core::ContextFileRole) -> &'static str {
    match role {
        scope_core::ContextFileRole::Target => "target",
        scope_core::ContextFileRole::DefinesTargetSymbol => "defines_target_symbol",
        scope_core::ContextFileRole::DirectCaller => "direct_caller",
        scope_core::ContextFileRole::DirectCallee => "direct_callee",
        scope_core::ContextFileRole::Importer => "importer",
        scope_core::ContextFileRole::Dependency => "dependency",
        scope_core::ContextFileRole::NearbyContext => "nearby_context",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexRunStats {
    indexed_files: usize,
    changed_files: usize,
    deleted_files: usize,
    affected_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchmarkIterationResult {
    indexed_files: usize,
    mutation_target: RepoPath,
    full_ms: u128,
    incremental_ms: u128,
    full_stats: IndexRunStats,
    incremental_stats: IndexRunStats,
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

fn run_benchmark(
    repo_root: &Path,
    fixture: Option<&str>,
    iterations: u32,
) -> Result<scope_core::stub::BenchmarkSummary, scope_core::ScopeError> {
    let iterations = iterations.max(1);
    let source_root = fixture
        .map(fixture_root)
        .transpose()?
        .unwrap_or_else(|| repo_root.to_path_buf());

    let mut runs = Vec::with_capacity(iterations as usize);

    for iteration in 0..iterations {
        let benchmark_root =
            prepare_benchmark_copy(&source_root, &format!("benchmark-{iteration}"))?;
        let summary = benchmark_iteration(&benchmark_root, fixture)?;
        runs.push(summary);
        fs::remove_dir_all(&benchmark_root)
            .map_err(|error| scope_core::ScopeError::io(&benchmark_root, error))?;
    }

    let indexed_files = runs.first().map(|run| run.indexed_files).unwrap_or(0);
    let mutation = scope_core::stub::BenchmarkMutationSummary {
        target_file: runs
            .first()
            .map(|run| run.mutation_target.clone())
            .unwrap_or_else(|| RepoPath::from("")),
        change_kind: "append_comment",
    };
    let full = summarize_phase(&runs, |run| run.full_ms, |run| &run.full_stats);
    let incremental = summarize_phase(
        &runs,
        |run| run.incremental_ms,
        |run| &run.incremental_stats,
    );
    let comparison = scope_core::stub::BenchmarkComparisonSummary {
        saved_ms: full.avg_ms as i128 - incremental.avg_ms as i128,
        incremental_pct_of_full: if full.avg_ms == 0 {
            0
        } else {
            ((incremental.avg_ms * 100) / full.avg_ms) as u32
        },
    };

    Ok(scope_core::stub::BenchmarkSummary {
        indexed_files,
        mutation,
        full,
        incremental,
        comparison,
    })
}

fn benchmark_iteration(
    benchmark_root: &Path,
    fixture: Option<&str>,
) -> Result<BenchmarkIterationResult, scope_core::ScopeError> {
    let db_path = benchmark_root.join(".scope/index.db");
    let store = scope_core::Store::open(&db_path)?;

    let started = Instant::now();
    let full_stats = index_repo(benchmark_root, &store)?;
    let full_ms = started.elapsed().as_millis();

    let target = select_benchmark_mutation_target(benchmark_root, fixture)?;
    apply_benchmark_edit(&target)?;

    let started = Instant::now();
    let incremental_stats = index_repo(benchmark_root, &store)?;
    let incremental_ms = started.elapsed().as_millis();

    Ok(BenchmarkIterationResult {
        indexed_files: full_stats.indexed_files,
        mutation_target: repo_relative_path(benchmark_root, &target),
        full_ms,
        incremental_ms,
        full_stats,
        incremental_stats,
    })
}

fn summarize_phase(
    runs: &[BenchmarkIterationResult],
    duration: impl Fn(&BenchmarkIterationResult) -> u128,
    stats: impl Fn(&BenchmarkIterationResult) -> &IndexRunStats,
) -> scope_core::stub::BenchmarkPhaseSummary {
    let min_ms = runs.iter().map(&duration).min().unwrap_or(0);
    let max_ms = runs.iter().map(&duration).max().unwrap_or(0);
    let avg_ms = if runs.is_empty() {
        0
    } else {
        runs.iter().map(&duration).sum::<u128>() / runs.len() as u128
    };

    let avg_indexed = if runs.is_empty() {
        0
    } else {
        runs.iter()
            .map(|run| stats(run).indexed_files)
            .sum::<usize>()
            / runs.len()
    };
    let avg_changed = if runs.is_empty() {
        0
    } else {
        runs.iter()
            .map(|run| stats(run).changed_files)
            .sum::<usize>()
            / runs.len()
    };
    let avg_deleted = if runs.is_empty() {
        0
    } else {
        runs.iter()
            .map(|run| stats(run).deleted_files)
            .sum::<usize>()
            / runs.len()
    };
    let avg_affected = if runs.is_empty() {
        0
    } else {
        runs.iter()
            .map(|run| stats(run).affected_files)
            .sum::<usize>()
            / runs.len()
    };

    scope_core::stub::BenchmarkPhaseSummary {
        avg_ms,
        min_ms,
        max_ms,
        files_processed_avg: avg_indexed,
        changed_files_avg: avg_changed,
        deleted_files_avg: avg_deleted,
        affected_files_avg: avg_affected,
    }
}

fn select_benchmark_mutation_target(
    repo_root: &Path,
    fixture: Option<&str>,
) -> Result<PathBuf, scope_core::ScopeError> {
    let preferred = fixture
        .map(|name| match name {
            "rust_small" => "src/parser.rs",
            "ts_small" | "test_map_ts" => "src/auth/verify.ts",
            _ => "",
        })
        .filter(|path| !path.is_empty())
        .map(|relative| repo_root.join(relative));

    if let Some(path) = preferred.filter(|path| path.exists()) {
        return Ok(path);
    }

    let entries = scan_repo(repo_root, &ScanConfig::default())?;
    entries
        .into_iter()
        .map(|entry| entry.absolute_path)
        .next()
        .ok_or_else(|| {
            scope_core::ScopeError::InvalidInput("benchmark found no source files".to_string())
        })
}

fn apply_benchmark_edit(path: &Path) -> Result<(), scope_core::ScopeError> {
    let mut content =
        fs::read_to_string(path).map_err(|error| scope_core::ScopeError::io(path, error))?;
    content.push_str("\n// scope benchmark mutation\n");
    fs::write(path, content).map_err(|error| scope_core::ScopeError::io(path, error))
}

fn repo_relative_path(repo_root: &Path, path: &Path) -> RepoPath {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    RepoPath::from(relative.to_string_lossy().replace('\\', "/"))
}

fn fixture_root(name: &str) -> Result<PathBuf, scope_core::ScopeError> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|error| scope_core::ScopeError::io("workspace root", error))?;
    Ok(repo.join("fixtures").join(name))
}

fn prepare_benchmark_copy(
    source_root: &Path,
    prefix: &str,
) -> Result<PathBuf, scope_core::ScopeError> {
    let nanos = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| scope_core::ScopeError::InvalidInput(error.to_string()))?
        .as_nanos();
    let dst = std::env::temp_dir().join(format!("scope-mcp-{prefix}-{nanos}"));
    copy_dir_recursive_skip_index(source_root, source_root, &dst)?;
    Ok(dst)
}

fn copy_dir_recursive_skip_index(
    root: &Path,
    src: &Path,
    dst: &Path,
) -> Result<(), scope_core::ScopeError> {
    fs::create_dir_all(dst).map_err(|error| scope_core::ScopeError::io(dst, error))?;
    for entry in fs::read_dir(src).map_err(|error| scope_core::ScopeError::io(src, error))? {
        let entry = entry.map_err(|error| scope_core::ScopeError::io(src, error))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| scope_core::ScopeError::io(&src_path, error))?
            .is_dir()
        {
            copy_dir_recursive_skip_index(root, &src_path, &dst_path)?;
        } else {
            if src_path
                .strip_prefix(root)
                .ok()
                .and_then(|relative| relative.to_str())
                == Some(".scope/index.db")
            {
                continue;
            }
            fs::copy(&src_path, &dst_path)
                .map_err(|error| scope_core::ScopeError::io(&src_path, error))?;
        }
    }
    Ok(())
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
        assert!(tools.iter().any(|tool| tool["name"] == "pack"));
        assert!(tools.iter().any(|tool| tool["name"] == "arch_check"));
        assert!(tools.iter().any(|tool| tool["name"] == "audit"));
        assert!(tools.iter().any(|tool| tool["name"] == "cochange"));
        assert!(tools.iter().any(|tool| tool["name"] == "report"));
        assert!(tools.iter().any(|tool| tool["name"] == "gate"));
        assert!(tools.iter().any(|tool| tool["name"] == "query"));
        assert!(tools.iter().any(|tool| tool["name"] == "surface"));
        assert!(tools.iter().any(|tool| tool["name"] == "surface_diff"));
        assert!(tools.iter().any(|tool| tool["name"] == "test_map_covers"));
        assert!(tools.iter().any(|tool| tool["name"] == "rename_plan"));
        assert!(tools.iter().any(|tool| tool["name"] == "doctor"));
        assert!(tools.iter().any(|tool| tool["name"] == "benchmark"));
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
    fn dispatch_audit_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("capability_audit");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "audit",
            &json!({
                "repo_root": repo.display().to_string(),
                "capability": "network"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "audit");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["capability"], "network");
        assert_eq!(
            value["data"]["result"]["summary"]["unexpected_entry_points"],
            1
        );
        fs::remove_dir_all(repo).unwrap();
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
    fn dispatch_entry_queries_return_scope_json_envelopes() {
        let repo = prepare_fixture_copy("capability_audit");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let list_output = dispatch_tool(
            "entry_list",
            &json!({ "repo_root": repo.display().to_string() }),
        )
        .unwrap();
        let list_value: Value = serde_json::from_str(&list_output).unwrap();
        assert_eq!(list_value["command"], "entry-list");
        assert_eq!(list_value["status"], "ok");
        assert_eq!(list_value["data"]["result"]["summary"]["entry_points"], 2);

        let cone_output = dispatch_tool(
            "entry_cone",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/workers/job.ts"
            }),
        )
        .unwrap();
        let cone_value: Value = serde_json::from_str(&cone_output).unwrap();
        assert_eq!(cone_value["command"], "entry-cone");
        assert_eq!(cone_value["status"], "ok");
        assert_eq!(cone_value["data"]["result"]["summary"]["reachable_files"], 3);
        assert_eq!(cone_value["data"]["result"]["summary"]["max_distance"], 2);

        let reaches_output = dispatch_tool(
            "entry_reaches",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/http/client.ts"
            }),
        )
        .unwrap();
        let reaches_value: Value = serde_json::from_str(&reaches_output).unwrap();
        assert_eq!(reaches_value["command"], "entry-reaches");
        assert_eq!(reaches_value["status"], "ok");
        assert_eq!(
            reaches_value["data"]["result"]["summary"]["reaching_entry_points"],
            2
        );
        assert_eq!(
            reaches_value["data"]["result"]["summary"]["nearest_distance"],
            2
        );

        let unreachable_output = dispatch_tool(
            "entry_unreachable",
            &json!({ "repo_root": repo.display().to_string() }),
        )
        .unwrap();
        let unreachable_value: Value = serde_json::from_str(&unreachable_output).unwrap();
        assert_eq!(unreachable_value["command"], "entry-unreachable");
        assert_eq!(unreachable_value["status"], "ok");
        assert_eq!(
            unreachable_value["data"]["result"]["unreachable_files"],
            0
        );
        assert_eq!(unreachable_value["data"]["result"]["reachable_files"], 4);

        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_split_and_mirror_return_scope_json_envelopes() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let split_output = dispatch_tool(
            "split",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/lib.rs",
                "clusters": 2
            }),
        )
        .unwrap();
        let split_value: Value = serde_json::from_str(&split_output).unwrap();
        assert_eq!(split_value["command"], "split");
        assert_eq!(split_value["status"], "ok");
        assert_eq!(split_value["data"]["result"]["target"], "src/lib.rs");
        assert_eq!(split_value["data"]["result"]["requested_clusters"], 2);

        let mirror_output = dispatch_tool(
            "mirror",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/lib.rs",
                "other": "src/parser.rs",
                "threshold": 0
            }),
        )
        .unwrap();
        let mirror_value: Value = serde_json::from_str(&mirror_output).unwrap();
        assert_eq!(mirror_value["command"], "mirror");
        assert_eq!(mirror_value["status"], "ok");
        assert_eq!(mirror_value["data"]["result"]["target"], "src/lib.rs");
        assert_eq!(mirror_value["data"]["result"]["other"], "src/parser.rs");
        assert!(mirror_value["data"]["result"]["similarity_score"].is_number());

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
        assert_eq!(
            value["data"]["result"]["source_file"],
            "src/auth/middleware.ts"
        );
        assert_eq!(value["data"]["result"]["summary"]["covering_tests"], 3);
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_utility_queries_return_scope_json_envelopes() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let _ = dispatch_tool(
            "snapshot_save",
            &json!({
                "repo_root": repo.display().to_string(),
                "name": "baseline"
            }),
        )
        .unwrap();

        let report_output = dispatch_tool(
            "report",
            &json!({
                "repo_root": repo.display().to_string(),
                "compare": "baseline"
            }),
        )
        .unwrap();
        let report_value: Value = serde_json::from_str(&report_output).unwrap();
        assert_eq!(report_value["command"], "report");
        assert_eq!(report_value["status"], "ok");
        assert_eq!(report_value["data"]["result"]["compare"]["target"], "baseline");
        assert!(report_value["data"]["result"]["metrics"]["total_files"].as_u64().unwrap() > 0);

        let gate_output = dispatch_tool(
            "gate",
            &json!({
                "repo_root": repo.display().to_string(),
                "compare": "baseline",
                "strict": true
            }),
        )
        .unwrap();
        let gate_value: Value = serde_json::from_str(&gate_output).unwrap();
        assert_eq!(gate_value["command"], "gate");
        assert_eq!(gate_value["status"], "ok");
        assert_eq!(gate_value["data"]["result"]["compare"], "baseline");
        assert_eq!(gate_value["data"]["result"]["summary"]["failed"], 1);
        assert!(gate_value["data"]["result"]["summary"]["passed"].as_u64().unwrap() > 0);

        let query_output = dispatch_tool(
            "query",
            &json!({
                "repo_root": repo.display().to_string(),
                "expr": "file \"src/lib.rs\" | .deps | count"
            }),
        )
        .unwrap();
        let query_value: Value = serde_json::from_str(&query_output).unwrap();
        assert_eq!(query_value["command"], "query");
        assert_eq!(query_value["status"], "ok");
        assert_eq!(query_value["data"]["input"], "file \"src/lib.rs\" | .deps | count");
        assert_eq!(query_value["data"]["result"]["number"], 3);

        let unused_output = dispatch_tool(
            "unused",
            &json!({ "repo_root": repo.display().to_string() }),
        )
        .unwrap();
        let unused_value: Value = serde_json::from_str(&unused_output).unwrap();
        assert_eq!(unused_value["command"], "unused");
        assert_eq!(unused_value["status"], "ok");
        assert_eq!(unused_value["data"]["result"]["summary"]["exported_symbols"], 8);
        assert_eq!(unused_value["data"]["result"]["summary"]["unused_symbols"], 6);

        let cycles_output = dispatch_tool(
            "cycles",
            &json!({
                "repo_root": repo.display().to_string(),
                "severity": "high"
            }),
        )
        .unwrap();
        let cycles_value: Value = serde_json::from_str(&cycles_output).unwrap();
        assert_eq!(cycles_value["command"], "cycles");
        assert_eq!(cycles_value["status"], "ok");
        assert_eq!(cycles_value["data"]["result"]["severity"], "high");
        assert_eq!(cycles_value["data"]["result"]["summary"]["cycle_count"], 0);

        let tree_output = dispatch_tool(
            "tree",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/parser.rs",
                "reverse": true,
                "depth": 2
            }),
        )
        .unwrap();
        let tree_value: Value = serde_json::from_str(&tree_output).unwrap();
        assert_eq!(tree_value["command"], "tree");
        assert_eq!(tree_value["status"], "ok");
        assert_eq!(tree_value["data"]["result"]["target"], "src/parser.rs");
        assert_eq!(tree_value["data"]["result"]["reverse"], true);
        assert_eq!(tree_value["data"]["result"]["summary"]["nodes"], 4);

        let diff_output = dispatch_tool(
            "diff",
            &json!({
                "repo_root": repo.display().to_string(),
                "branch": "HEAD"
            }),
        )
        .unwrap();
        let diff_value: Value = serde_json::from_str(&diff_output).unwrap();
        assert_eq!(diff_value["command"], "diff");
        assert_eq!(diff_value["status"], "ok");
        assert_eq!(diff_value["data"]["result"]["branch"], "HEAD");
        assert_eq!(diff_value["data"]["result"]["summary"]["changed_files"], 0);
        assert_eq!(diff_value["data"]["result"]["summary"]["affected_files"], 0);

        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_report_gate_and_query_reject_invalid_arguments() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let report_output = dispatch_tool(
            "report",
            &json!({
                "repo_root": repo.display().to_string(),
                "compare": 7
            }),
        )
        .unwrap();
        let report_value: Value = serde_json::from_str(&report_output).unwrap();
        assert_eq!(report_value["command"], "report");
        assert_eq!(report_value["status"], "error");
        assert_eq!(report_value["data"]["kind"], "invalid_input");
        assert!(report_value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("mcp tool argument `compare` must be a string when provided"));

        let gate_output = dispatch_tool(
            "gate",
            &json!({
                "repo_root": repo.display().to_string(),
                "strict": "yes"
            }),
        )
        .unwrap();
        let gate_value: Value = serde_json::from_str(&gate_output).unwrap();
        assert_eq!(gate_value["command"], "gate");
        assert_eq!(gate_value["status"], "error");
        assert_eq!(gate_value["data"]["kind"], "invalid_input");
        assert!(gate_value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("mcp tool argument `strict` must be a boolean when provided"));

        let query_missing_expr_output = dispatch_tool(
            "query",
            &json!({
                "repo_root": repo.display().to_string()
            }),
        )
        .unwrap();
        let query_missing_expr_value: Value =
            serde_json::from_str(&query_missing_expr_output).unwrap();
        assert_eq!(query_missing_expr_value["command"], "query");
        assert_eq!(query_missing_expr_value["status"], "error");
        assert_eq!(query_missing_expr_value["data"]["kind"], "invalid_input");
        assert!(query_missing_expr_value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("mcp tool arguments require `expr`"));

        let query_invalid_expr_output = dispatch_tool(
            "query",
            &json!({
                "repo_root": repo.display().to_string(),
                "expr": false
            }),
        )
        .unwrap();
        let query_invalid_expr_value: Value =
            serde_json::from_str(&query_invalid_expr_output).unwrap();
        assert_eq!(query_invalid_expr_value["command"], "query");
        assert_eq!(query_invalid_expr_value["status"], "error");
        assert_eq!(query_invalid_expr_value["data"]["kind"], "invalid_input");
        assert!(query_invalid_expr_value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("mcp tool argument `expr` must be a string when provided"));

        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_cochange_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();
        let context =
            bootstrap_from_arguments(&json!({ "repo_root": repo.display().to_string() })).unwrap();
        context
            .store
            .persist_file_churn(
                &RepoPath::from("src/parser.rs"),
                "c1",
                Some("agent@example.com"),
                None,
            )
            .unwrap();
        context
            .store
            .persist_file_churn(
                &RepoPath::from("src/utils.rs"),
                "c1",
                Some("agent@example.com"),
                None,
            )
            .unwrap();

        let output = dispatch_tool(
            "cochange",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/parser.rs"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["target"], "src/parser.rs");
        assert_eq!(value["data"]["result"]["files"][0]["path"], "src/utils.rs");
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn dispatch_cochange_rejects_invalid_min_shared_commits() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "cochange",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/parser.rs",
                "min_shared_commits": 0
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("min_shared_commits must be greater than 0"));
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn dispatch_cochange_rejects_invalid_top() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "cochange",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/parser.rs",
                "top": 0
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("top must be greater than 0"));
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn dispatch_cochange_rejects_missing_target() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "cochange",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/missing.rs"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("file not indexed: src/missing.rs"));
        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn dispatch_cochange_rejects_unsupported_sort() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "cochange",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/parser.rs",
                "sort": "bogus"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert!(value["data"]["message"]
            .as_str()
            .expect("error message should be a string")
            .contains("unsupported cochange sort: bogus"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_pack_returns_plain_text_context_pack() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "pack",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "parser::parse",
                "change_type": "body",
                "budget": 400
            }),
        )
        .unwrap();
        assert!(output.contains("=== SCOPE CONTEXT PACK ==="));
        assert!(output.contains("Target:      parser::parse"));
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_rename_plan_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "rename_plan",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "parser::parse",
                "to": "parse2"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "rename-plan");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["target"], "parser::parse");
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_doctor_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "doctor",
            &json!({
                "repo_root": repo.display().to_string(),
                "fix": false
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "doctor");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["checks"].is_array());
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn dispatch_benchmark_returns_scope_json_envelope() {
        let repo = prepare_fixture_copy("rust_small");
        let output = dispatch_tool(
            "benchmark",
            &json!({
                "repo_root": repo.display().to_string(),
                "iterations": 1
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "benchmark");
        assert_eq!(value["status"], "ok");
        assert!(value["data"]["summary"]["comparison"].is_object());
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
        assert_eq!(diff_value["data"]["result"]["before"]["name"], "baseline");
        assert_eq!(diff_value["data"]["result"]["after"]["name"], "baseline");
        assert!(diff_value["data"]["result"]["summary"]["files"]
            .as_u64()
            .is_some());

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

    #[test]
    fn dispatch_utility_queries_invalid_target_returns_domain_error_envelope() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = dispatch_tool(
            "index",
            &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
        )
        .unwrap();

        let output = dispatch_tool(
            "tree",
            &json!({
                "repo_root": repo.display().to_string(),
                "target": "src/missing.rs"
            }),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["command"], "tree");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");

        fs::remove_dir_all(repo).unwrap();
    }
}
