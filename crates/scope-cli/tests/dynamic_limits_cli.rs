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

fn run_scope(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_scope"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("scope binary should run")
}

#[test]
fn doctor_command_reports_partial_parse_health_for_dynamic_limits_fixture() {
    let repo = prepare_fixture_copy("dynamic_limits");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let output = run_scope(&repo, &["doctor"]);
    assert_eq!(output.status.code(), Some(0));

    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["command"], "doctor");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["stats"]["files"], 3);
    assert_eq!(value["data"]["stats"]["imports"], 2);
    assert_eq!(value["data"]["stats"]["symbols"], 3);
    assert_eq!(value["data"]["stats"]["call_edges"], 2);
    assert_eq!(value["data"]["stats"]["parse_status"]["ok"], 1);
    assert_eq!(value["data"]["stats"]["parse_status"]["partial"], 2);
    assert_eq!(value["data"]["stats"]["parse_status"]["error"], 0);

    let parse_check = value["data"]["checks"]
        .as_array()
        .expect("doctor checks should be an array")
        .iter()
        .find(|entry| entry["name"].as_str() == Some("parse_status"))
        .expect("parse_status check should be present");
    assert_eq!(parse_check["status"], "warn");
    assert_eq!(parse_check["detail"], "ok=1, partial=2, error=0");

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn deps_and_calls_commands_preserve_static_edges_without_inventing_dynamic_calls() {
    let repo = prepare_fixture_copy("dynamic_limits");
    let index_output = run_scope(&repo, &["index"]);
    assert_eq!(index_output.status.code(), Some(0));

    let deps_output = run_scope(&repo, &["deps", "src/index.js"]);
    assert_eq!(deps_output.status.code(), Some(0));
    let deps_value: serde_json::Value =
        serde_json::from_slice(&deps_output.stdout).expect("stdout should be JSON");
    assert_eq!(deps_value["command"], "deps");
    assert_eq!(deps_value["status"], "ok");
    assert_eq!(deps_value["data"]["target"], "src/index.js");
    assert_eq!(deps_value["data"]["reverse"], false);
    assert_eq!(deps_value["data"]["transitive"], false);

    let deps = deps_value["data"]["dependencies"]
        .as_array()
        .expect("dependencies should be an array");
    assert_eq!(
        deps.iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["src/computed_import.ts", "src/dynamic_require.js"]
    );
    assert!(deps
        .iter()
        .all(|entry| entry["certainty"].as_str() == Some("exact")));

    let boot_calls_output = run_scope(&repo, &["calls", "index::boot"]);
    assert_eq!(boot_calls_output.status.code(), Some(0));
    let boot_calls_value: serde_json::Value =
        serde_json::from_slice(&boot_calls_output.stdout).expect("stdout should be JSON");
    assert_eq!(boot_calls_value["command"], "calls");
    assert_eq!(boot_calls_value["status"], "ok");
    assert_eq!(boot_calls_value["data"]["symbol"], "index::boot");
    assert_eq!(boot_calls_value["data"]["transitive"], false);
    let boot_traversals = boot_calls_value["data"]["traversals"]
        .as_array()
        .expect("traversals should be an array");
    assert_eq!(boot_traversals.len(), 2);
    assert!(boot_traversals.iter().any(|entry| {
        entry["qualname"].as_str() == Some("computed_import::loadFeature")
            && entry["distance"].as_u64() == Some(1)
            && entry["certainty"].as_str() == Some("resolved")
    }));
    assert!(boot_traversals.iter().any(|entry| {
        entry["qualname"].as_str() == Some("dynamic_require::loadPlugin")
            && entry["distance"].as_u64() == Some(1)
            && entry["certainty"].as_str() == Some("resolved")
    }));

    let computed_calls_output = run_scope(&repo, &["calls", "computed_import::loadFeature"]);
    assert_eq!(computed_calls_output.status.code(), Some(0));
    let computed_calls_value: serde_json::Value =
        serde_json::from_slice(&computed_calls_output.stdout).expect("stdout should be JSON");
    assert_eq!(computed_calls_value["command"], "calls");
    assert_eq!(computed_calls_value["status"], "ok");
    assert_eq!(
        computed_calls_value["data"]["symbol"],
        "computed_import::loadFeature"
    );
    assert_eq!(computed_calls_value["data"]["transitive"], false);
    assert_eq!(
        computed_calls_value["data"]["traversals"]
            .as_array()
            .expect("traversals should be an array")
            .len(),
        0
    );

    let plugin_calls_output = run_scope(&repo, &["calls", "dynamic_require::loadPlugin"]);
    assert_eq!(plugin_calls_output.status.code(), Some(0));
    let plugin_calls_value: serde_json::Value =
        serde_json::from_slice(&plugin_calls_output.stdout).expect("stdout should be JSON");
    assert_eq!(plugin_calls_value["command"], "calls");
    assert_eq!(plugin_calls_value["status"], "ok");
    assert_eq!(
        plugin_calls_value["data"]["symbol"],
        "dynamic_require::loadPlugin"
    );
    assert_eq!(plugin_calls_value["data"]["transitive"], false);
    assert_eq!(
        plugin_calls_value["data"]["traversals"]
            .as_array()
            .expect("traversals should be an array")
            .len(),
        0
    );

    fs::remove_dir_all(repo).unwrap();
}
