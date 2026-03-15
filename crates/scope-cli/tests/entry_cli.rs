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

fn read_golden(name: &str) -> String {
    fs::read_to_string(workspace_root().join("tests/golden").join(name))
        .unwrap()
        .trim_end_matches('\n')
        .to_string()
}

fn run_scope(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_scope"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("scope binary should run")
}

#[test]
fn entry_list_command_returns_golden_json_for_capability_audit_fixture() {
    let repo = prepare_fixture_copy("capability_audit");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["entry", "list"]);
    assert_eq!(output.status.code(), Some(0));

    let actual = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(actual.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "entry-list");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["summary"]["entry_points"], 2);

    let expected = read_golden("capability_audit_entry_list.json");
    assert_eq!(actual.trim(), expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn entry_cone_command_returns_golden_json_for_capability_audit_fixture() {
    let repo = prepare_fixture_copy("capability_audit");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["entry", "cone", "src/workers/job.ts"]);
    assert_eq!(output.status.code(), Some(0));

    let actual = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(actual.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "entry-cone");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["entry"], "src/workers/job.ts");
    assert_eq!(value["data"]["result"]["summary"]["reachable_files"], 3);

    let expected = read_golden("capability_audit_entry_cone_job.json");
    assert_eq!(actual.trim(), expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn entry_reaches_command_returns_golden_json_for_capability_audit_fixture() {
    let repo = prepare_fixture_copy("capability_audit");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["entry", "reaches", "src/shared/api.ts"]);
    assert_eq!(output.status.code(), Some(0));

    let actual = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(actual.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "entry-reaches");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/shared/api.ts");
    assert_eq!(
        value["data"]["result"]["summary"]["reaching_entry_points"],
        2
    );

    let expected = read_golden("capability_audit_entry_reaches_shared_api.json");
    assert_eq!(actual.trim(), expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn entry_unreachable_command_returns_golden_json_for_capability_audit_fixture() {
    let repo = prepare_fixture_copy("capability_audit");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["entry", "unreachable"]);
    assert_eq!(output.status.code(), Some(0));

    let actual = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(actual.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "entry-unreachable");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["reachable_files"], 4);
    assert_eq!(value["data"]["result"]["unreachable_files"], 0);

    let expected = read_golden("capability_audit_entry_unreachable.json");
    assert_eq!(actual.trim(), expected);

    fs::remove_dir_all(repo).unwrap();
}
