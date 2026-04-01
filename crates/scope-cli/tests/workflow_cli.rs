use std::{path::Path, path::PathBuf, process::Command};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve")
}

fn run_scope(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_scope"))
        .current_dir(repo)
        .args(args)
        .output()
        .expect("scope binary should run")
}

#[test]
fn workflow_list_discovers_repo_local_markdown_workflows() {
    let repo = workspace_root();
    let output = run_scope(&repo, &["workflow", "list"]);
    assert_eq!(output.status.code(), Some(0));

    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["command"], "workflow_list");
    assert_eq!(value["status"], "ok");
    let ids = value["data"]["workflows"]
        .as_array()
        .expect("workflows should be an array")
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"dependency-trace"));
    assert!(ids.contains(&"impact-review"));
}

#[test]
fn workflow_show_renders_arguments_and_defaults_into_steps() {
    let repo = workspace_root();
    let output = run_scope(
        &repo,
        &[
            "workflow",
            "show",
            "dependency-trace",
            "--arg",
            "target=parser::parse",
        ],
    );
    assert_eq!(output.status.code(), Some(0));

    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout should be JSON");
    assert_eq!(value["command"], "workflow_show");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["data"]["workflow"]["id"], "dependency-trace");
    assert_eq!(value["data"]["arguments"]["target"], "parser::parse");
    assert_eq!(value["data"]["arguments"]["change_type"], "body");
    assert_eq!(
        value["data"]["rendered_steps"][0]["command"],
        "scope deps parser::parse --transitive --depth 2"
    );
    assert!(value["data"]["rendered_markdown"]
        .as_str()
        .unwrap()
        .contains("parser::parse"));
}
