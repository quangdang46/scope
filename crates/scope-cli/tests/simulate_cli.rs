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
fn simulate_extract_command_returns_json_envelope_for_fixture_repo() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(
        &repo,
        &[
            "simulate",
            "extract",
            "lib::parser",
            "--into",
            "src/parser_extracted.rs",
        ],
    );
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout should be JSON");
    assert_eq!(value["command"], "simulate-extract");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["result"]["extraction"]["symbols"][0], "lib::parser");
    assert_eq!(
        value["data"]["result"]["extraction"]["from_file"],
        "src/lib.rs"
    );
    assert_eq!(
        value["data"]["result"]["extraction"]["into_file"],
        "src/parser_extracted.rs"
    );
    assert_eq!(value["data"]["result"]["graph_delta"]["edges_added"], 1);
    assert_eq!(value["data"]["result"]["graph_delta"]["edges_removed"], 0);
    assert_eq!(
        value["data"]["result"]["graph_delta"]["new_edges"][0]["to"],
        "src/parser_extracted.rs"
    );
    assert_eq!(value["data"]["result"]["recommendation"], "neutral");

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn simulate_extract_command_empty_symbol_list_emits_json_error_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_scope"))
        .args(["simulate", "extract", "", "--into", "src/parser_extracted.rs"])
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
        .contains("simulate extract requires at least one symbol"));
}
