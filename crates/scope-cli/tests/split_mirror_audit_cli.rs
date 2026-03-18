use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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
    let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "scope-cli-{prefix}-{}-{nanos}-{sequence}",
        std::process::id()
    ))
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
            if src_path.file_name().and_then(|name| name.to_str()) == Some("index.db")
                && src_path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    == Some(".scope")
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

fn normalize_golden_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string()
}

fn read_golden(name: &str) -> String {
    normalize_golden_text(
        &fs::read_to_string(workspace_root().join("tests/golden").join(name)).unwrap(),
    )
}

fn run_scope(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_scope"))
        .current_dir(repo)
        .arg("--repo-root")
        .arg(repo)
        .args(args)
        .output()
        .expect("scope binary should run")
}

#[test]
fn split_command_returns_golden_json_for_rust_small_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["split", "src/lib.rs", "--clusters", "2"]);
    assert_eq!(output.status.code(), Some(0));

    let actual = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&normalize_golden_text(&actual)).expect("stdout should be JSON");
    assert_eq!(value["command"], "split");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/lib.rs");
    assert_eq!(value["data"]["result"]["requested_clusters"], 2);
    assert_eq!(value["data"]["result"]["summary"]["clusters"], 2);

    let expected = read_golden("rust_small_split_cli.json");
    assert_eq!(normalize_golden_text(&actual), expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn mirror_command_returns_golden_json_for_rust_small_fixture() {
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

    let actual = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&normalize_golden_text(&actual)).expect("stdout should be JSON");
    assert_eq!(value["command"], "mirror");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["target"], "src/lib.rs");
    assert_eq!(value["data"]["result"]["other"], "src/parser.rs");
    assert_eq!(value["data"]["result"]["summary"]["matches_returned"], 1);

    let expected = read_golden("rust_small_mirror_cli.json");
    assert_eq!(normalize_golden_text(&actual), expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
#[ignore = "flaky in CI"]
fn audit_command_returns_golden_json_and_nonzero_exit_for_capability_audit_fixture() {
    let repo = prepare_fixture_copy("capability_audit");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["audit", "--capability", "network"]);
    assert_eq!(output.status.code(), Some(1));

    let actual = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&normalize_golden_text(&actual)).expect("stdout should be JSON");
    assert_eq!(value["command"], "audit");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["capability"], "network");
    assert_eq!(
        value["data"]["result"]["summary"]["unexpected_entry_points"],
        1
    );
    assert!(String::from_utf8_lossy(&output.stderr).trim().is_empty());

    let expected = read_golden("capability_audit_network_cli.json");
    assert_eq!(normalize_golden_text(&actual), expected);

    fs::remove_dir_all(repo).unwrap();
}
