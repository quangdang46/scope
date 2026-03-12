use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
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
