use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    process::Command,
};

pub fn create_cochange_fixture_repo(target_dir: &Path) {
    if target_dir.exists() {
        fs::remove_dir_all(target_dir).expect("existing cochange fixture repo should be removable");
    }
    fs::create_dir_all(target_dir.join("src"))
        .expect("cochange fixture src directory should exist");

    run_git(target_dir, ["init", "-q"]);
    run_git(target_dir, ["config", "user.name", "Scope Fixture"]);
    run_git(target_dir, ["config", "user.email", "fixture@example.com"]);

    fs::write(
        target_dir.join("src/parser.rs"),
        "pub fn parse(input: &str) -> Vec<&str> {\n    input.split(',').collect()\n}\n",
    )
    .expect("parser fixture file should be written");
    fs::write(
        target_dir.join("src/utils.rs"),
        "pub fn trim(input: &str) -> &str {\n    input.trim()\n}\n",
    )
    .expect("utils fixture file should be written");
    fs::write(
        target_dir.join("src/resolver.rs"),
        "pub fn resolve(input: &str) -> String {\n    input.to_string()\n}\n",
    )
    .expect("resolver fixture file should be written");
    fs::write(
        target_dir.join("Cargo.toml"),
        "[package]\nname = \"cochange_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("cochange fixture manifest should be written");

    run_git(target_dir, ["add", "."]);
    run_git_commit(target_dir, "2024-01-01T00:00:00Z", "initial fixture");

    append_fixture_comment(target_dir, "src/parser.rs", "c1");
    append_fixture_comment(target_dir, "src/utils.rs", "c1");
    run_git(target_dir, ["add", "src/parser.rs", "src/utils.rs"]);
    run_git_commit(
        target_dir,
        "2024-01-02T00:00:00Z",
        "parser and utils evolve together",
    );

    append_fixture_comment(target_dir, "src/parser.rs", "c2");
    append_fixture_comment(target_dir, "src/utils.rs", "c2");
    append_fixture_comment(target_dir, "src/resolver.rs", "c2");
    run_git(
        target_dir,
        ["add", "src/parser.rs", "src/utils.rs", "src/resolver.rs"],
    );
    run_git_commit(
        target_dir,
        "2024-01-03T00:00:00Z",
        "parser utils and resolver evolve together",
    );

    append_fixture_comment(target_dir, "src/parser.rs", "c3");
    run_git(target_dir, ["add", "src/parser.rs"]);
    run_git_commit(target_dir, "2024-01-04T00:00:00Z", "parser evolves alone");

    append_fixture_comment(target_dir, "src/resolver.rs", "c4");
    run_git(target_dir, ["add", "src/resolver.rs"]);
    run_git_commit(target_dir, "2024-01-05T00:00:00Z", "resolver evolves alone");
}

fn append_fixture_comment(target_dir: &Path, relative_path: &str, label: &str) {
    let mut file = OpenOptions::new()
        .append(true)
        .open(target_dir.join(relative_path))
        .expect("fixture file should be open for append");
    write!(file, "\n// commit {label}\n").expect("fixture comment should append");
}

fn run_git<const N: usize>(target_dir: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .current_dir(target_dir)
        .args(args)
        .output()
        .expect("git command should run for cochange fixture");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        output
            .status
            .code()
            .map_or_else(|| "signal".to_string(), |code| code.to_string()),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git_commit(target_dir: &Path, timestamp: &str, message: &str) {
    let output = Command::new("git")
        .current_dir(target_dir)
        .env("GIT_AUTHOR_DATE", timestamp)
        .env("GIT_COMMITTER_DATE", timestamp)
        .args(["commit", "-q", "-m", message])
        .output()
        .expect("git commit should run for cochange fixture");
    assert!(
        output.status.success(),
        "git commit {:?} failed: {}",
        message,
        String::from_utf8_lossy(&output.stderr)
    );
}
