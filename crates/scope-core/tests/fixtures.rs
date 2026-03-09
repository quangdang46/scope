use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve")
}

fn fixture_root(name: &str) -> PathBuf {
    repo_root().join("fixtures").join(name)
}

#[test]
fn planned_fixture_directories_exist() {
    for fixture in [
        "rust_small",
        "ts_small",
        "dynamic_limits",
        "arch_violations",
    ] {
        assert!(
            fixture_root(fixture).is_dir(),
            "missing fixture directory: {fixture}"
        );
    }
}

#[test]
fn rust_small_fixture_has_expected_files() {
    let root = fixture_root("rust_small");
    for relative in [
        "Cargo.toml",
        "src/main.rs",
        "src/lib.rs",
        "src/parser.rs",
        "src/resolver.rs",
        "src/utils.rs",
    ] {
        assert!(
            root.join(relative).is_file(),
            "missing rust_small file: {relative}"
        );
    }
}

#[test]
fn ts_small_fixture_has_expected_files() {
    let root = fixture_root("ts_small");
    for relative in [
        "package.json",
        "src/index.ts",
        "src/auth/index.ts",
        "src/auth/middleware.ts",
        "src/auth/jwt.ts",
        "src/utils/logger.ts",
        "src/utils/formatter.ts",
    ] {
        assert!(
            root.join(relative).is_file(),
            "missing ts_small file: {relative}"
        );
    }
}

#[test]
fn golden_directory_exists_for_future_snapshots() {
    assert!(repo_root().join("tests/golden").is_dir());
}
