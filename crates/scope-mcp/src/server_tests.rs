use super::*;
use std::{
    collections::HashMap,
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

fn write_arch_config(repo: &Path, source: &str) {
    let scope_dir = repo.join(".scope");
    fs::create_dir_all(&scope_dir).unwrap();
    fs::write(scope_dir.join("arch.toml"), source).unwrap();
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
fn tool_registry_exposes_expected_report_gate_and_query_schemas() {
    let tools = tool_registry();
    let by_name: HashMap<&str, &Value> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(|name| (name, tool)))
        .collect();

    let report = by_name.get("report").expect("report tool should exist");
    assert_eq!(report["inputSchema"]["type"], "object");
    assert_eq!(
        report["inputSchema"]["properties"]["repo_root"]["type"],
        "string"
    );
    assert_eq!(
        report["inputSchema"]["properties"]["db_path"]["type"],
        "string"
    );
    assert_eq!(
        report["inputSchema"]["properties"]["compare"]["type"],
        "string"
    );
    assert_eq!(report["inputSchema"]["additionalProperties"], false);

    let gate = by_name.get("gate").expect("gate tool should exist");
    assert_eq!(gate["inputSchema"]["type"], "object");
    assert_eq!(
        gate["inputSchema"]["properties"]["repo_root"]["type"],
        "string"
    );
    assert_eq!(
        gate["inputSchema"]["properties"]["db_path"]["type"],
        "string"
    );
    assert_eq!(
        gate["inputSchema"]["properties"]["compare"]["type"],
        "string"
    );
    assert_eq!(
        gate["inputSchema"]["properties"]["strict"]["type"],
        "boolean"
    );
    assert_eq!(gate["inputSchema"]["additionalProperties"], false);

    let query = by_name.get("query").expect("query tool should exist");
    assert_eq!(query["inputSchema"]["type"], "object");
    assert_eq!(
        query["inputSchema"]["properties"]["repo_root"]["type"],
        "string"
    );
    assert_eq!(
        query["inputSchema"]["properties"]["db_path"]["type"],
        "string"
    );
    assert_eq!(query["inputSchema"]["properties"]["expr"]["type"], "string");
    assert_eq!(query["inputSchema"]["properties"]["exprs"]["type"], "array");
    assert_eq!(
        query["inputSchema"]["properties"]["exprs"]["items"]["type"],
        "string"
    );
    assert_eq!(query["inputSchema"]["properties"]["exprs"]["minItems"], 1);
    assert!(query["inputSchema"].get("anyOf").is_none());
    assert_eq!(query["inputSchema"]["additionalProperties"], false);
}

#[test]
fn initialize_advertises_server_instructions() {
    let response = initialize_response(json!(1));
    assert_eq!(response["result"]["serverInfo"]["name"], "scope-mcp");
    assert!(response["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("Use `index` before dependency-sensitive analysis"));
}

#[test]
fn read_message_accepts_lf_only_headers() {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }))
    .unwrap();
    let mut raw = Vec::new();
    raw.extend_from_slice(format!("Content-Length: {}\n\n", body.len()).as_bytes());
    raw.extend_from_slice(&body);

    let mut cursor = io::Cursor::new(raw);
    let (message, framing) = read_message(&mut cursor, MessageFraming::Unknown)
        .unwrap()
        .unwrap();
    assert_eq!(framing, MessageFraming::ContentLength);
    assert_eq!(message["method"], "initialize");
    assert_eq!(message["id"], 1);
}

#[test]
fn read_message_accepts_line_delimited_json() {
    let raw = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n";
    let mut cursor = io::Cursor::new(&raw[..]);
    let (message, framing) = read_message(&mut cursor, MessageFraming::Unknown)
        .unwrap()
        .unwrap();
    assert_eq!(framing, MessageFraming::LineDelimited);
    assert_eq!(message["method"], "initialize");
    assert_eq!(message["id"], 1);
}

#[test]
fn handle_message_answers_shutdown_requests() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "shutdown"
    });

    let response = handle_message(&request).expect("shutdown should return a response");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 3);
    assert_eq!(response["result"], json!({}));
}

#[test]
fn exit_notifications_mark_the_server_for_shutdown() {
    let request = json!({
        "jsonrpc": "2.0",
        "method": "exit"
    });

    assert!(is_exit_notification(&request));
    assert!(handle_message(&request).is_none());
}

#[test]
fn write_message_uses_line_delimited_json_when_requested() {
    let mut output = Vec::new();
    write_message(
        &mut output,
        &json!({"jsonrpc": "2.0", "id": 1, "result": {}}),
        MessageFraming::LineDelimited,
    )
    .unwrap();
    let rendered = String::from_utf8(output).unwrap();
    assert!(rendered.starts_with('{'));
    assert!(rendered.ends_with("\n"));
    assert!(!rendered.contains("Content-Length:"));
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
fn dispatch_deps_supports_transitive_closure_with_depth_limit() {
    let repo = prepare_fixture_copy("ts_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let output = dispatch_tool(
        "deps",
        &json!({
            "repo_root": repo.display().to_string(),
            "file": "src/index.ts",
            "transitive": true,
            "depth": 2
        }),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["command"], "deps");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["target"], "src/index.ts");
    assert_eq!(value["data"]["transitive"], true);
    assert_eq!(value["data"]["depth"], 2);
    assert_eq!(
        value["data"]["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "src/auth/index.ts",
            "src/utils/formatter.ts",
            "src/auth/aliases.ts",
            "src/auth/middleware.ts",
            "src/utils/logger.ts"
        ]
    );
    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn handle_message_wraps_query_tool_call_in_jsonrpc_result() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "query",
            "arguments": {
                "repo_root": repo.display().to_string(),
                "expr": "file \"src/lib.rs\" | .deps | count"
            }
        }
    });
    let response = handle_message(&request).expect("tools/call should return a response");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 7);
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("content text should be a string");
    let payload: Value = serde_json::from_str(text).expect("wrapped content should be JSON");
    assert_eq!(response["result"]["structuredContent"], payload);
    assert_eq!(payload["command"], "query");
    assert_eq!(payload["status"], "ok");
    assert_eq!(
        payload["data"]["input"],
        "file \"src/lib.rs\" | .deps | count"
    );
    assert_eq!(payload["data"]["result"]["number"], 3);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn handle_message_wraps_multi_expr_query_tool_call_in_jsonrpc_result() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let request = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "name": "query",
            "arguments": {
                "repo_root": repo.display().to_string(),
                "exprs": [
                    "let roots = file \"src/lib.rs\" | .deps | unique",
                    "$roots | count"
                ]
            }
        }
    });
    let response = handle_message(&request).expect("tools/call should return a response");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 8);
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("content text should be a string");
    let payload: Value = serde_json::from_str(text).expect("wrapped content should be JSON");
    assert_eq!(response["result"]["structuredContent"], payload);
    assert_eq!(payload["command"], "query");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["data"]["input"], "$roots | count");
    assert_eq!(payload["data"]["result"]["number"], 3);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn handle_message_returns_jsonrpc_error_for_unknown_tool_calls() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": {
            "name": "does_not_exist",
            "arguments": {}
        }
    });
    let response = handle_message(&request).expect("unknown tools should return an error response");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 9);
    assert_eq!(response["error"]["code"], -32602);
    assert!(response["error"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("unknown tool"));
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
    write_arch_config(
        &repo,
        r#"
[[capability]]
name = "network"
pattern = "src/http/**"
symbols = ["fetch"]
expected_callers = ["src/workers/**"]

[[entry_point]]
pattern = "src/workers/**"

[[entry_point]]
pattern = "src/cli/**"
"#,
    );
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
    assert_eq!(
        cone_value["data"]["result"]["summary"]["reachable_files"],
        3
    );
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
    assert_eq!(unreachable_value["data"]["result"]["unreachable_files"], 0);
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
    assert_eq!(
        report_value["data"]["result"]["compare"]["target"],
        "baseline"
    );
    assert_eq!(
        report_value["data"]["result"]["compare"]["baseline_health_score"],
        94.0
    );
    assert_eq!(
        report_value["data"]["result"]["compare"]["health_score_delta"],
        -6.0
    );
    assert_eq!(
        report_value["data"]["result"]["compare"]["unreachable_files_delta"],
        3
    );
    assert_eq!(
        report_value["data"]["result"]["compare"]["public_surface_removed_delta"],
        0
    );
    assert!(
        report_value["data"]["result"]["metrics"]["total_files"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        report_value["data"]["result"]["recommendations"],
        json!([
            "review 3 unreachable files for dead code or missing entry-point declarations",
            "health score regressed by 6.0 points versus baseline"
        ])
    );

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
    assert!(
        gate_value["data"]["result"]["summary"]["passed"]
            .as_u64()
            .unwrap()
            > 0
    );

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
    assert_eq!(
        query_value["data"]["input"],
        "file \"src/lib.rs\" | .deps | count"
    );
    assert_eq!(query_value["data"]["result"]["number"], 3);

    let unused_output = dispatch_tool(
        "unused",
        &json!({ "repo_root": repo.display().to_string() }),
    )
    .unwrap();
    let unused_value: Value = serde_json::from_str(&unused_output).unwrap();
    assert_eq!(unused_value["command"], "unused");
    assert_eq!(unused_value["status"], "ok");
    assert_eq!(
        unused_value["data"]["result"]["summary"]["exported_symbols"],
        8
    );
    assert_eq!(
        unused_value["data"]["result"]["summary"]["unused_symbols"],
        6
    );

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
    let query_missing_expr_value: Value = serde_json::from_str(&query_missing_expr_output).unwrap();
    assert_eq!(query_missing_expr_value["command"], "query");
    assert_eq!(query_missing_expr_value["status"], "error");
    assert_eq!(query_missing_expr_value["data"]["kind"], "invalid_input");
    assert!(query_missing_expr_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("mcp tool arguments require `expr` or `exprs`"));

    let query_invalid_expr_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": false
        }),
    )
    .unwrap();
    let query_invalid_expr_value: Value = serde_json::from_str(&query_invalid_expr_output).unwrap();
    assert_eq!(query_invalid_expr_value["command"], "query");
    assert_eq!(query_invalid_expr_value["status"], "error");
    assert_eq!(query_invalid_expr_value["data"]["kind"], "invalid_input");
    assert!(query_invalid_expr_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("mcp tool argument `expr` must be a string when provided"));

    let query_invalid_exprs_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "exprs": false
        }),
    )
    .unwrap();
    let query_invalid_exprs_value: Value =
        serde_json::from_str(&query_invalid_exprs_output).unwrap();
    assert_eq!(query_invalid_exprs_value["command"], "query");
    assert_eq!(query_invalid_exprs_value["status"], "error");
    assert_eq!(query_invalid_exprs_value["data"]["kind"], "invalid_input");
    assert!(query_invalid_exprs_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("mcp tool argument `exprs` must be an array of strings when provided"));

    let query_empty_exprs_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "exprs": []
        }),
    )
    .unwrap();
    let query_empty_exprs_value: Value = serde_json::from_str(&query_empty_exprs_output).unwrap();
    assert_eq!(query_empty_exprs_value["command"], "query");
    assert_eq!(query_empty_exprs_value["status"], "error");
    assert_eq!(query_empty_exprs_value["data"]["kind"], "invalid_input");
    assert!(query_empty_exprs_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("mcp tool argument `exprs` must contain at least one expression"));

    let query_both_expr_and_exprs_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "all-files | count",
            "exprs": ["all-symbols | count"]
        }),
    )
    .unwrap();
    let query_both_expr_and_exprs_value: Value =
        serde_json::from_str(&query_both_expr_and_exprs_output).unwrap();
    assert_eq!(query_both_expr_and_exprs_value["command"], "query");
    assert_eq!(query_both_expr_and_exprs_value["status"], "error");
    assert_eq!(
        query_both_expr_and_exprs_value["data"]["kind"],
        "invalid_input"
    );
    assert!(query_both_expr_and_exprs_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("mcp tool arguments accept either `expr` or `exprs`, but not both"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_report_and_gate_missing_compare_snapshot_return_not_found_errors() {
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
            "compare": "missing"
        }),
    )
    .unwrap();
    let report_value: Value = serde_json::from_str(&report_output).unwrap();
    assert_eq!(report_value["command"], "report");
    assert_eq!(report_value["status"], "error");
    assert_eq!(report_value["data"]["kind"], "not_found");
    assert_eq!(
        report_value["data"]["message"],
        "snapshot not found: missing"
    );

    let gate_output = dispatch_tool(
        "gate",
        &json!({
            "repo_root": repo.display().to_string(),
            "compare": "missing"
        }),
    )
    .unwrap();
    let gate_value: Value = serde_json::from_str(&gate_output).unwrap();
    assert_eq!(gate_value["command"], "gate");
    assert_eq!(gate_value["status"], "error");
    assert_eq!(gate_value["data"]["kind"], "not_found");
    assert_eq!(gate_value["data"]["message"], "snapshot not found: missing");

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_gate_warns_for_delta_only_config_without_compare() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();
    write_arch_config(
        &repo,
        r#"[[gate]]
metric = "health_score_delta"
min_delta = -1.0
severity = "warning"
message = "health score should not regress much"
skip = false
"#,
    );

    let gate_output = dispatch_tool(
        "gate",
        &json!({
            "repo_root": repo.display().to_string()
        }),
    )
    .unwrap();
    let gate_value: Value = serde_json::from_str(&gate_output).unwrap();
    assert_eq!(gate_value["command"], "gate");
    assert_eq!(gate_value["status"], "ok");
    assert!(
        gate_value["data"]["result"]["summary"]["warnings"]
            .as_u64()
            .expect("warnings summary should be numeric")
            >= 1
    );
    let evaluations = gate_value["data"]["result"]["evaluations"]
        .as_array()
        .expect("evaluations should be an array");
    let evaluation = evaluations
        .iter()
        .find(|evaluation| evaluation["metric"] == "health_score_delta")
        .expect("health_score_delta evaluation should be present");
    assert_eq!(evaluation["status"], "warning");
    assert_eq!(evaluation["severity"], "warning");
    assert!(evaluation["detail"]
        .as_str()
        .expect("detail should be a string")
        .contains("comparison snapshot required for min_delta"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_gate_respects_skipped_custom_gate_config() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();
    write_arch_config(
        &repo,
        r#"[[gate]]
metric = "cycles"
severity = "warning"
message = "cycles temporarily ignored"
skip = true
"#,
    );

    let gate_output = dispatch_tool(
        "gate",
        &json!({
            "repo_root": repo.display().to_string()
        }),
    )
    .unwrap();
    let gate_value: Value = serde_json::from_str(&gate_output).unwrap();
    assert_eq!(gate_value["command"], "gate");
    assert_eq!(gate_value["status"], "ok");
    assert!(
        gate_value["data"]["result"]["summary"]["skipped"]
            .as_u64()
            .expect("skipped summary should be numeric")
            >= 1
    );
    let evaluations = gate_value["data"]["result"]["evaluations"]
        .as_array()
        .expect("evaluations should be an array");
    let evaluation = evaluations
        .iter()
        .find(|evaluation| evaluation["metric"] == "cycles")
        .expect("cycles evaluation should be present");
    assert_eq!(evaluation["status"], "skipped");
    assert_eq!(evaluation["severity"], "warning");
    assert_eq!(
        evaluation["detail"],
        "gate explicitly skipped by configuration"
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_gate_reports_compare_baseline_warning_details() {
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

    let parser_path = repo.join("src/parser.rs");
    let updated = fs::read_to_string(&parser_path)
        .unwrap()
        .replace("pub fn parse", "pub fn parse_token");
    fs::write(&parser_path, updated).unwrap();
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();
    write_arch_config(
        &repo,
        r#"[[gate]]
metric = "health_score_delta"
min_delta = -1.0
severity = "warning"
message = "health score should not regress much"
skip = false
"#,
    );

    let gate_output = dispatch_tool(
        "gate",
        &json!({
            "repo_root": repo.display().to_string(),
            "compare": "baseline"
        }),
    )
    .unwrap();
    let gate_value: Value = serde_json::from_str(&gate_output).unwrap();
    assert_eq!(gate_value["command"], "gate");
    assert_eq!(gate_value["status"], "ok");
    assert_eq!(gate_value["data"]["result"]["compare"], "baseline");
    assert_eq!(gate_value["data"]["result"]["summary"]["warnings"], 1);
    assert_eq!(gate_value["data"]["result"]["summary"]["failed"], 0);
    let evaluations = gate_value["data"]["result"]["evaluations"]
        .as_array()
        .expect("evaluations should be an array");
    let evaluation = evaluations
        .iter()
        .find(|evaluation| evaluation["metric"] == "health_score_delta")
        .expect("health_score_delta evaluation should be present");
    assert_eq!(evaluation["status"], "warning");
    assert_eq!(evaluation["severity"], "warning");
    assert_eq!(evaluation["current_value"], -12.0);
    assert_eq!(evaluation["baseline_value"], 0.0);
    assert_eq!(evaluation["delta"], -12.0);
    assert_eq!(
        evaluation["detail"],
        "delta -12.00 is below min_delta -1.00"
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_query_supports_multiple_exprs_with_shared_bindings() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "exprs": [
                "let roots = file \"src/lib.rs\" | .deps | unique",
                "$roots | count"
            ]
        }),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["command"], "query");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["input"], "$roots | count");
    assert_eq!(value["data"]["result"]["number"], 3);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_query_supports_all_sources_and_reverse_step() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let all_files_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "all-files | count"
        }),
    )
    .unwrap();
    let all_files_value: Value = serde_json::from_str(&all_files_output).unwrap();
    assert_eq!(all_files_value["command"], "query");
    assert_eq!(all_files_value["status"], "ok");
    assert_eq!(all_files_value["data"]["result"]["number"], 5);

    let all_symbols_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "all-symbols | count"
        }),
    )
    .unwrap();
    let all_symbols_value: Value = serde_json::from_str(&all_symbols_output).unwrap();
    assert_eq!(all_symbols_value["command"], "query");
    assert_eq!(all_symbols_value["status"], "ok");
    assert!(
        all_symbols_value["data"]["result"]["number"]
            .as_u64()
            .expect("symbol count should be numeric")
            >= 4
    );

    let reverse_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/parser.rs\" | .reverse | unique | count"
        }),
    )
    .unwrap();
    let reverse_value: Value = serde_json::from_str(&reverse_output).unwrap();
    assert_eq!(reverse_value["command"], "query");
    assert_eq!(reverse_value["status"], "ok");
    assert_eq!(reverse_value["data"]["result"]["number"], 2);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_query_supports_transitive_call_steps() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let callers_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "symbol \"parser::parse\" | .callers_transitive | count"
        }),
    )
    .unwrap();
    let callers_value: Value = serde_json::from_str(&callers_output).unwrap();
    assert_eq!(callers_value["command"], "query");
    assert_eq!(callers_value["status"], "ok");
    assert_eq!(callers_value["data"]["result"]["number"], 2);

    let callees_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "symbol \"parser::parse\" | .callees_transitive | count"
        }),
    )
    .unwrap();
    let callees_value: Value = serde_json::from_str(&callees_output).unwrap();
    assert_eq!(callees_value["command"], "query");
    assert_eq!(callees_value["status"], "ok");
    assert_eq!(callees_value["data"]["result"]["number"], 1);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_query_supports_transitive_dependency_and_symbol_filter_steps() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let deps_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/lib.rs\" | .deps_transitive(1) | count"
        }),
    )
    .unwrap();
    let deps_value: Value = serde_json::from_str(&deps_output).unwrap();
    assert_eq!(deps_value["command"], "query");
    assert_eq!(deps_value["status"], "ok");
    assert_eq!(deps_value["data"]["result"]["number"], 3);

    let reverse_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/parser.rs\" | .reverse_transitive(2) | count"
        }),
    )
    .unwrap();
    let reverse_value: Value = serde_json::from_str(&reverse_output).unwrap();
    assert_eq!(reverse_value["command"], "query");
    assert_eq!(reverse_value["status"], "ok");
    assert_eq!(reverse_value["data"]["result"]["number"], 2);

    let symbols_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/lib.rs\" | .symbols(public_only=true, kind=\"function\") | count"
        }),
    )
    .unwrap();
    let symbols_value: Value = serde_json::from_str(&symbols_output).unwrap();
    assert_eq!(symbols_value["command"], "query");
    assert_eq!(symbols_value["status"], "ok");
    assert_eq!(symbols_value["data"]["result"]["number"], 2);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_query_surfaces_invalid_step_argument_errors() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let malformed_depth_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/main.rs\" | .deps_transitive(foo)"
        }),
    )
    .unwrap();
    let malformed_depth_value: Value = serde_json::from_str(&malformed_depth_output).unwrap();
    assert_eq!(malformed_depth_value["command"], "query");
    assert_eq!(malformed_depth_value["status"], "error");
    assert_eq!(malformed_depth_value["data"]["kind"], "invalid_input");
    assert!(malformed_depth_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("optional non-negative integer depth"));

    let invalid_symbol_kind_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/lib.rs\" | .symbols(kind=\"class\")"
        }),
    )
    .unwrap();
    let invalid_symbol_kind_value: Value =
        serde_json::from_str(&invalid_symbol_kind_output).unwrap();
    assert_eq!(invalid_symbol_kind_value["command"], "query");
    assert_eq!(invalid_symbol_kind_value["status"], "error");
    assert_eq!(invalid_symbol_kind_value["data"]["kind"], "invalid_input");
    assert!(invalid_symbol_kind_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("unsupported query symbol kind `class`"));

    let duplicate_symbols_arg_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/lib.rs\" | .symbols(public_only=true, public_only=false)"
        }),
    )
    .unwrap();
    let duplicate_symbols_arg_value: Value =
        serde_json::from_str(&duplicate_symbols_arg_output).unwrap();
    assert_eq!(duplicate_symbols_arg_value["command"], "query");
    assert_eq!(duplicate_symbols_arg_value["status"], "error");
    assert_eq!(duplicate_symbols_arg_value["data"]["kind"], "invalid_input");
    assert!(duplicate_symbols_arg_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("duplicate `public_only`"));

    let unsupported_step_output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "file \"src/lib.rs\" | .impact"
        }),
    )
    .unwrap();
    let unsupported_step_value: Value = serde_json::from_str(&unsupported_step_output).unwrap();
    assert_eq!(unsupported_step_value["command"], "query");
    assert_eq!(unsupported_step_value["status"], "error");
    assert_eq!(unsupported_step_value["data"]["kind"], "invalid_input");
    assert!(unsupported_step_value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("unsupported query step `.impact`; supported steps are .deps, .reverse, .deps_transitive, .reverse_transitive, .symbols, .callers, .callees, unique, and count; plus .callers_transitive and .callees_transitive"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dispatch_query_surfaces_unknown_bindings() {
    let repo = prepare_fixture_copy("rust_small");
    let _ = dispatch_tool(
        "index",
        &json!({ "repo_root": repo.display().to_string(), "no_git": true }),
    )
    .unwrap();

    let output = dispatch_tool(
        "query",
        &json!({
            "repo_root": repo.display().to_string(),
            "expr": "$missing | count"
        }),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["command"], "query");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
    assert!(value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("unknown query binding `$missing`"));

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
fn dispatch_rename_plan_rejects_path_like_file_destination() {
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
            "target": "src/parser.rs",
            "to": "src/parser2.rs"
        }),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["command"], "rename-plan");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
    assert_eq!(
        value["data"]["message"],
        "invalid command input: rename-plan file targets currently accept only a bare file stem for --to; file moves are not implemented"
    );
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
    assert_eq!(diff_value["data"]["result"]["cycles"]["before"], 0);
    assert_eq!(diff_value["data"]["result"]["cycles"]["after"], 0);
    assert_eq!(diff_value["data"]["result"]["cycles"]["introduced"], 0);
    assert_eq!(diff_value["data"]["result"]["cycles"]["resolved"], 0);
    assert_eq!(diff_value["data"]["result"]["omitted"], json!([]));
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
