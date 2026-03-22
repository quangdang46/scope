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

fn is_text_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(
            "rs" | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
                | "rb"
                | "go"
                | "toml"
                | "json"
                | "md"
                | "txt"
                | "yml"
                | "yaml"
                | "cfg"
                | "ini"
                | "html"
                | "css"
                | "sh"
        )
    )
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
            if is_text_file(&src_path) {
                let content = fs::read_to_string(&src_path).unwrap();
                fs::write(&dst_path, content.replace("\r\n", "\n")).unwrap();
            } else {
                fs::copy(&src_path, &dst_path).unwrap();
            }
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
fn report_command_defaults_to_markdown_output() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["report"]);
    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# scope Health Report"));
    assert!(stdout.contains("## Summary"));
    assert!(stdout.contains("## Recommendations"));
    assert!(!stdout.trim_start().starts_with('{'));

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn report_command_supports_explicit_json_flag() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["report", "--json"]);
    assert_eq!(output.status.code(), Some(0));

    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["command"], "report");
    assert_eq!(value["status"], "ok");
    assert!(
        value["data"]["result"]["metrics"]["total_files"]
            .as_u64()
            .unwrap()
            > 0
    );

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn report_command_writes_markdown_output_file() {
    let repo = prepare_fixture_copy("rust_small");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output_path = repo.join("health.md");
    let output = run_scope(
        &repo,
        &["report", "--output", output_path.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(0));

    let written = fs::read_to_string(&output_path).expect("report file should exist");
    assert!(written.contains("# scope Health Report"));
    assert!(written.contains("## Summary"));

    let _ = fs::remove_dir_all(repo);
}
