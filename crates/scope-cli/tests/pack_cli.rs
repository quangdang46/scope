use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve")
}

fn fixture_root(name: &str) -> PathBuf {
    workspace_root().join("fixtures").join(name)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("scope-cli-{prefix}-{nanos}"))
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();

    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type().unwrap();

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            if src_path
                .strip_prefix(src)
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
    copy_dir_recursive(&src, &dst);
    dst
}

fn run_scope(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_scope"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("scope binary should run")
}

fn run_scope_with_stdin(repo: &Path, args: &[&str], stdin_input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scope"))
        .current_dir(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("scope binary should run");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(stdin_input.as_bytes())
        .expect("stdin should accept test input");

    child.wait_with_output().expect("scope binary should exit")
}

#[test]
fn pack_invalid_target_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args([
            "pack",
            "does::not::exist",
            "--change-type",
            "rename",
            "--budget",
            "120",
        ])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn risk_invalid_days_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["risk", "--days", "0"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn risk_invalid_threshold_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["risk", "--threshold=-1"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn cochange_invalid_days_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["cochange", "src/lib.rs", "--days", "0"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn cochange_invalid_min_shared_commits_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["cochange", "src/lib.rs", "--min-shared-commits", "0"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
    assert!(value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("min_shared_commits must be greater than 0"));
}

#[test]
fn cochange_invalid_top_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["cochange", "src/lib.rs", "--top", "0"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
    assert!(value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("top must be greater than 0"));
}

#[test]
fn cochange_missing_target_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["cochange", "src/missing.rs"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
    assert!(value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("file not indexed: src/missing.rs"));
}

#[test]
fn cochange_command_returns_json_envelope_for_generated_git_fixture() {
    let fixture_root = fixture_root("cochange");
    let script = fixture_root.join("create_git_history.sh");
    let repo = unique_temp_dir("cochange-cli");

    let status = Command::new(&script).arg(&repo).status().unwrap();
    assert!(status.success());

    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["cochange", "src/parser.rs", "--days", "10000"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let actual = stdout.trim();
    let value: serde_json::Value = serde_json::from_str(actual).expect("stdout should be JSON");
    assert_eq!(value["command"], "cochange");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/parser.rs");
    assert_eq!(value["data"]["result"]["summary"]["target_commits"], 4);
    assert_eq!(value["data"]["result"]["files"][0]["path"], "src/utils.rs");

    let expected = fs::read_to_string(
        workspace_root().join("tests/golden/cochange_generated_cli.json"),
    )
    .unwrap();
    assert_eq!(actual, expected.trim_end_matches('\n'));

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn cochange_command_honors_filters_and_sort_for_generated_git_fixture() {
    let fixture_root = fixture_root("cochange");
    let script = fixture_root.join("create_git_history.sh");
    let repo = unique_temp_dir("cochange-cli-filtered");

    let status = Command::new(&script).arg(&repo).status().unwrap();
    assert!(status.success());

    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(
        &repo,
        &[
            "cochange",
            "src/parser.rs",
            "--days",
            "10000",
            "--min-shared-commits",
            "2",
            "--top",
            "1",
            "--sort",
            "shared-commits",
        ],
    );
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "cochange");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/parser.rs");
    assert_eq!(value["data"]["result"]["min_shared_commits"], 2);
    assert_eq!(value["data"]["result"]["top"], 1);
    assert_eq!(value["data"]["result"]["sort"], "shared_commits");
    assert_eq!(value["data"]["result"]["summary"]["related_files"], 1);
    let files = value["data"]["result"]["files"]
        .as_array()
        .expect("files should be an array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "src/utils.rs");
    assert_eq!(files[0]["shared_commits"], 3);

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn split_command_returns_json_envelope_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["split", "src/lib.rs", "--clusters", "2"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "split");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/lib.rs");
    assert_eq!(value["data"]["result"]["requested_clusters"], 2);
    assert!(value["data"]["result"]["summary"]["clusters"].as_u64().unwrap() >= 1);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn mirror_command_returns_json_envelope_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(
        &repo,
        &[
            "mirror",
            "src/lib.rs",
            "--other",
            "src/parser.rs",
            "--threshold",
            "0",
        ],
    );
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "mirror");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/lib.rs");
    assert_eq!(value["data"]["result"]["other"], "src/parser.rs");
    assert!(value["data"]["result"]["similarity_score"].is_number());

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn tree_invalid_target_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["tree", "does/not/exist.rs"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn test_map_covers_invalid_target_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["test-map", "covers", "does/not/exist.ts"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn test_map_covered_by_invalid_target_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["test-map", "covered-by", "does/not/exist.ts"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn unused_command_returns_json_envelope_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["unused"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "unused");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["summary"]["exported_symbols"], 8);
    assert_eq!(value["data"]["result"]["summary"]["unused_symbols"], 6);
    assert_eq!(
        value["data"]["result"]["symbols"][0]["qualname"],
        "lib::parser"
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn cycles_command_returns_filtered_json_envelope_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["cycles", "--severity", "high"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "cycles");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["summary"]["cycle_count"], 0);
    assert_eq!(value["data"]["result"]["severity"], "high");
    assert!(value["data"]["result"]["cycles"]
        .as_array()
        .expect("cycles should be an array")
        .is_empty());

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn tree_command_returns_recursive_json_envelope_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["tree", "src/lib.rs", "--depth", "2"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "tree");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/lib.rs");
    assert_eq!(value["data"]["result"]["depth"], 2);
    assert_eq!(value["data"]["result"]["summary"]["nodes"], 5);
    assert_eq!(value["data"]["result"]["tree"]["path"], "src/lib.rs");

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn tree_reverse_command_returns_reverse_dependency_json_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["tree", "src/parser.rs", "--reverse", "--depth", "2"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "tree");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/parser.rs");
    assert_eq!(value["data"]["result"]["reverse"], true);
    assert_eq!(value["data"]["result"]["depth"], 2);
    assert_eq!(value["data"]["result"]["summary"]["reverse"], true);
    assert_eq!(value["data"]["result"]["summary"]["nodes"], 4);
    assert_eq!(value["data"]["result"]["tree"]["path"], "src/parser.rs");
    assert_eq!(value["data"]["result"]["tree"]["children"][0]["path"], "src/lib.rs");
    assert_eq!(value["data"]["result"]["tree"]["children"][1]["path"], "src/resolver.rs");
    assert_eq!(
        value["data"]["result"]["tree"]["children"][1]["children"][0]["path"],
        "src/lib.rs"
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn diff_command_reports_no_changes_for_clean_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["diff", "HEAD"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "diff");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["branch"], "HEAD");
    assert_eq!(value["data"]["result"]["summary"]["changed_files"], 0);
    assert_eq!(value["data"]["result"]["summary"]["affected_files"], 0);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn diff_command_empty_branch_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["diff", ""])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
    assert!(value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("diff branch may not be empty"));
}

#[test]
fn query_expr_returns_json_envelope_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["query", "--expr", "file \"src/lib.rs\" | .deps | unique | count"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "query");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["input"], "file \"src/lib.rs\" | .deps | unique | count");
    assert_eq!(value["data"]["result"]["number"], 3);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn calls_and_callers_commands_return_live_json_for_direct_mode() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let calls_output = run_scope(&repo, &["calls", "lib::run"]);
    assert_eq!(calls_output.status.code(), Some(0));
    let calls_value: serde_json::Value =
        serde_json::from_slice(&calls_output.stdout).expect("stdout should be JSON");
    assert_eq!(calls_value["command"], "calls");
    assert_eq!(calls_value["status"], "ok");
    assert_eq!(calls_value["data"]["symbol"], "lib::run");
    assert!(calls_value["data"]["traversals"].is_array());

    let callers_output = run_scope(&repo, &["callers", "parser::parse"]);
    assert_eq!(callers_output.status.code(), Some(0));
    let callers_value: serde_json::Value =
        serde_json::from_slice(&callers_output.stdout).expect("stdout should be JSON");
    assert_eq!(callers_value["command"], "callers");
    assert_eq!(callers_value["status"], "ok");
    assert_eq!(callers_value["data"]["symbol"], "parser::parse");
    assert!(callers_value["data"]["traversals"].is_array());

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn calls_and_callers_commands_report_stub_status_for_transitive_mode() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let calls_output = run_scope(&repo, &["calls", "lib::run", "--transitive"]);
    assert_eq!(calls_output.status.code(), Some(0));
    let calls_value: serde_json::Value =
        serde_json::from_slice(&calls_output.stdout).expect("stdout should be JSON");
    assert_eq!(calls_value["command"], "calls");
    assert_eq!(calls_value["status"], "stub");
    assert_eq!(calls_value["data"]["symbol"], "lib::run");
    assert_eq!(calls_value["data"]["transitive"], true);
    assert!(calls_value["warnings"].as_array().is_some_and(|items| !items.is_empty()));

    let callers_output = run_scope(&repo, &["callers", "parser::parse", "--transitive"]);
    assert_eq!(callers_output.status.code(), Some(0));
    let callers_value: serde_json::Value =
        serde_json::from_slice(&callers_output.stdout).expect("stdout should be JSON");
    assert_eq!(callers_value["command"], "callers");
    assert_eq!(callers_value["status"], "stub");
    assert_eq!(callers_value["data"]["symbol"], "parser::parse");
    assert_eq!(callers_value["data"]["transitive"], true);
    assert!(callers_value["warnings"].as_array().is_some_and(|items| !items.is_empty()));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn query_expr_invalid_step_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["query", "--expr", "file \"src/lib.rs\" | .impact"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
}

#[test]
fn query_expr_supports_quoted_pipes_escapes_and_let_bindings() {
    let repo = prepare_fixture_copy("rust_small");
    let weird_file = repo.join("src/a|b.rs");
    fs::write(&weird_file, "pub fn odd_name() {}\n").unwrap();
    let quoted_escape_file = repo.join("src/a\"|b.rs");
    fs::write(&quoted_escape_file, "pub fn escaped_quote_name() {}\n").unwrap();
    let backslash_file = repo.join("src/path\\file.rs");
    fs::write(&backslash_file, "pub fn backslash_name() {}\n").unwrap();

    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let quoted_output = run_scope(&repo, &["query", "--expr", "file \"src/a|b.rs\" | .symbols | count"]);
    assert_eq!(quoted_output.status.code(), Some(0));
    let quoted_value: serde_json::Value =
        serde_json::from_slice(&quoted_output.stdout).expect("stdout should be JSON");
    assert_eq!(quoted_value["command"], "query");
    assert_eq!(quoted_value["status"], "ok");
    assert_eq!(quoted_value["data"]["input"], "file \"src/a|b.rs\" | .symbols | count");
    assert_eq!(quoted_value["data"]["result"]["number"], 1);

    let escaped_quote_output = run_scope(
        &repo,
        &[
            "query",
            "--expr",
            "file \"src/a\\\"|b.rs\" | .symbols | count",
        ],
    );
    assert_eq!(escaped_quote_output.status.code(), Some(0));
    let escaped_quote_value: serde_json::Value =
        serde_json::from_slice(&escaped_quote_output.stdout).expect("stdout should be JSON");
    assert_eq!(escaped_quote_value["command"], "query");
    assert_eq!(escaped_quote_value["status"], "ok");
    assert_eq!(escaped_quote_value["data"]["result"]["number"], 1);

    let backslash_output = run_scope(
        &repo,
        &[
            "query",
            "--expr",
            "file \"src/path\\\\file.rs\" | .symbols | count",
        ],
    );
    assert_eq!(backslash_output.status.code(), Some(0));
    let backslash_value: serde_json::Value =
        serde_json::from_slice(&backslash_output.stdout).expect("stdout should be JSON");
    assert_eq!(backslash_value["command"], "query");
    assert_eq!(backslash_value["status"], "ok");
    assert_eq!(backslash_value["data"]["result"]["number"], 1);

    let let_output = run_scope(
        &repo,
        &["query", "--expr", "let roots = file \"src/lib.rs\" | .deps | unique"],
    );
    assert_eq!(let_output.status.code(), Some(0));
    let let_value: serde_json::Value =
        serde_json::from_slice(&let_output.stdout).expect("stdout should be JSON");
    assert_eq!(let_value["command"], "query");
    assert_eq!(let_value["status"], "ok");
    assert_eq!(let_value["data"]["input"], "let roots = file \"src/lib.rs\" | .deps | unique");
    let files = let_value["data"]["result"]["files"]
        .as_array()
        .expect("files result should be an array");
    assert_eq!(files.len(), 3);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn query_expr_unterminated_quote_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["query", "--expr", "file \"src/lib.rs | .deps"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("stderr should be JSON");
    assert_eq!(value["command"], "cli");
    assert_eq!(value["status"], "error");
    assert_eq!(value["data"]["kind"], "invalid_input");
    assert!(value["data"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("unterminated quoted string"));
}

#[test]
fn query_repl_supports_help_bindings_and_exit() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope_with_stdin(
        &repo,
        &["query"],
        ":help\nlet roots = file \"src/lib.rs\" | .deps | unique\n:vars\n:exit\n",
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).trim().is_empty());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scope query REPL"));
    assert!(stdout.contains("Type :help for commands, :exit to quit."));
    assert!(stdout.contains("Sources: file \"...\", symbol \"...\", all-files, all-symbols, $name"));
    assert!(stdout.contains("Steps: .deps, .reverse, .symbols, .callers, .callees, unique, count"));
    assert!(stdout.contains("Bindings: let name = <expr>"));
    assert!(stdout.contains("\"command\": \"query\""));
    assert!(stdout.contains("roots"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn query_repl_prints_json_errors_and_honors_quit_alias() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope_with_stdin(&repo, &["query"], "file \"src/lib.rs\" | .impact\n:quit\n");
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).trim().is_empty());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scope query REPL"));
    assert!(stdout.contains("\"command\": \"cli\""));
    assert!(stdout.contains("\"status\": \"error\""));
    assert!(stdout.contains("unsupported query step `.impact`"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn serve_help_includes_core_flags() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["serve", "--help"])
        .output()
        .expect("scope binary should run");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--port"));
    assert!(stdout.contains("--open"));
    assert!(stdout.contains("--no-ui"));
}

#[test]
fn report_and_gate_help_include_expected_flags() {
    let report_output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["report", "--help"])
        .output()
        .expect("scope binary should run");
    assert_eq!(report_output.status.code(), Some(0));
    let report_stdout = String::from_utf8_lossy(&report_output.stdout);
    assert!(report_stdout.contains("--compare"));

    let gate_output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["gate", "--help"])
        .output()
        .expect("scope binary should run");
    assert_eq!(gate_output.status.code(), Some(0));
    let gate_stdout = String::from_utf8_lossy(&gate_output.stdout);
    assert!(gate_stdout.contains("--compare"));
    assert!(gate_stdout.contains("--strict"));
}

#[test]
fn report_and_gate_commands_return_live_json_envelopes() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let snapshot_output = run_scope(&repo, &["snapshot", "save", "--name", "baseline"]);
    assert_eq!(snapshot_output.status.code(), Some(0));

    let report_output = run_scope(&repo, &["report", "--compare", "baseline"]);
    assert_eq!(report_output.status.code(), Some(0));
    let report_value: serde_json::Value =
        serde_json::from_slice(&report_output.stdout).expect("stdout should be JSON");
    assert_eq!(report_value["command"], "report");
    assert_eq!(report_value["status"], "ok");
    assert_eq!(report_value["data"]["result"]["compare"]["target"], "baseline");
    assert!(report_value["data"]["result"]["metrics"]["total_files"].as_u64().unwrap() > 0);

    let gate_output = run_scope(&repo, &["gate", "--compare", "baseline", "--strict"]);
    assert_eq!(gate_output.status.code(), Some(1));
    let gate_value: serde_json::Value =
        serde_json::from_slice(&gate_output.stdout).expect("stdout should be JSON");
    assert_eq!(gate_value["command"], "gate");
    assert_eq!(gate_value["status"], "ok");
    assert_eq!(gate_value["data"]["result"]["compare"], "baseline");
    assert_eq!(gate_value["data"]["result"]["summary"]["failed"], 1);
    assert!(gate_value["data"]["result"]["summary"]["passed"].as_u64().unwrap() > 0);

    fs::remove_dir_all(repo).unwrap();
}
