use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use scope_core::{
    scan_repo, stub, Adapter, RepoPath, RustAdapter, ScanConfig, Store, SupportedLanguage,
    SymbolKind,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root should resolve")
}

fn fixture_root(name: &str) -> PathBuf {
    repo_root().join("fixtures").join(name)
}

fn golden_root() -> PathBuf {
    repo_root().join("tests/golden")
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("scope-{prefix}-{nanos}"))
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();

    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry.file_type().unwrap();

        if file_type.is_dir() {
            if entry.file_name() == ".scope" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path);
        } else {
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

fn index_fixture(repo_root: &Path) -> Store {
    let store = Store::open(&repo_root.join(".scope/index.db")).unwrap();
    let adapter = RustAdapter;
    let entries = scan_repo(repo_root, &ScanConfig::default()).unwrap();

    for entry in entries {
        if entry.language != SupportedLanguage::Rust {
            continue;
        }
        if !scope_core::adapters::supports_path(&adapter, &entry.absolute_path) {
            continue;
        }

        let source = fs::read_to_string(&entry.absolute_path).unwrap();
        let extract = adapter.extract(&entry, &source);
        store.persist_extract_result(&extract).unwrap();
    }

    store
}

fn read_golden(name: &str) -> String {
    fs::read_to_string(golden_root().join(name)).unwrap()
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

#[test]
fn rust_small_forward_deps_match_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let deps = store.query_deps(&RepoPath::from("src/lib.rs")).unwrap();
    let envelope = stub::deps("src/lib.rs".to_string(), false, false, None, deps);
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("rust_small_lib_deps.json");

    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_reverse_deps_match_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let deps = store.query_reverse_deps(&RepoPath::from("src/parser.rs")).unwrap();
    let envelope = stub::deps("src/parser.rs".to_string(), true, false, None, deps);
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("rust_small_parser_reverse_deps.json");

    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn ts_small_is_scanned_but_not_indexed_by_rust_fixture_indexer() {
    let entries = scan_repo(&fixture_root("ts_small"), &ScanConfig::default()).unwrap();

    assert!(!entries.is_empty());
    assert!(
        entries
            .iter()
            .all(|entry| entry.language == SupportedLanguage::TypeScript),
        "expected TypeScript fixture entries to be discovered"
    );

    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);
    let deps = store.query_deps(&RepoPath::from("src/index.ts")).unwrap();

    assert!(deps.is_empty(), "non-Rust fixture should not be indexed yet");

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_symbols_query_matches_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let symbols = store
        .query_symbols(&RepoPath::from("src/lib.rs"), false, None)
        .unwrap();
    let envelope = stub::symbols("src/lib.rs".to_string(), false, None, symbols);
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("rust_small_lib_symbols.json");

    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_public_symbols_query_matches_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let symbols = store
        .query_symbols(&RepoPath::from("src/parser.rs"), true, None)
        .unwrap();
    let envelope = stub::symbols("src/parser.rs".to_string(), true, None, symbols);
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("rust_small_parser_public_symbols.json");

    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_function_symbols_query_matches_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let symbols = store
        .query_symbols(&RepoPath::from("src/lib.rs"), false, Some(SymbolKind::Function))
        .unwrap();
    let envelope = stub::symbols(
        "src/lib.rs".to_string(),
        false,
        Some(SymbolKind::Function),
        symbols,
    );
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("rust_small_lib_function_symbols.json");

    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}
