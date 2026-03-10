use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use scope_core::{
    adapter_for_language, scan_repo, stub, EdgeKind, RepoPath, ScanConfig, Store,
    SupportedLanguage, SymbolKind,
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
    let entries = scan_repo(repo_root, &ScanConfig::default()).unwrap();
    let extracts: Vec<_> = entries
        .into_iter()
        .filter_map(|entry| {
            let adapter = adapter_for_language(entry.language)?;
            if !scope_core::adapters::supports_path(adapter, &entry.absolute_path) {
                return None;
            }
            let source = fs::read_to_string(&entry.absolute_path).unwrap();
            Some(adapter.extract(&entry, &source))
        })
        .collect();

    for extract in &extracts {
        store.persist_extract_result(extract).unwrap();
    }
    for extract in &extracts {
        store.persist_extract_result(extract).unwrap();
    }
    for extract in &extracts {
        store.refresh_call_edges(extract).unwrap();
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
fn ts_small_is_scanned_and_indexed_by_fixture_indexer() {
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

    assert_eq!(deps.len(), 2, "ts fixture should now be indexed");
    assert_eq!(
        deps.iter().map(|dep| dep.path.0.clone()).collect::<Vec<_>>(),
        vec!["src/auth/index.ts", "src/utils/formatter.ts"]
    );

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

#[test]
fn ts_small_forward_deps_match_expected_paths() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);
    let deps = store.query_deps(&RepoPath::from("src/index.ts")).unwrap();
    let dep_paths: Vec<_> = deps.iter().map(|dep| dep.path.0.clone()).collect();

    assert_eq!(dep_paths, vec!["src/auth/index.ts", "src/utils/formatter.ts"]);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn ts_small_reverse_deps_and_symbols_and_calls_work_conservatively() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let reverse = store.query_reverse_deps(&RepoPath::from("src/auth/jwt.ts")).unwrap();
    let reverse_paths: Vec<_> = reverse.iter().map(|dep| dep.path.0.clone()).collect();
    assert_eq!(reverse_paths, vec!["src/auth/middleware.ts"]);

    let jwt_symbols = store
        .query_symbols(&RepoPath::from("src/auth/jwt.ts"), false, None)
        .unwrap();
    let jwt_symbol_names: Vec<_> = jwt_symbols.iter().map(|symbol| symbol.name.clone()).collect();
    assert_eq!(jwt_symbol_names, vec!["sign", "verify"]);

    let middleware_symbols = store
        .query_symbols(&RepoPath::from("src/auth/middleware.ts"), false, None)
        .unwrap();
    let middleware_symbol_names: Vec<_> = middleware_symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(middleware_symbol_names, vec!["verifyToken"]);

    let verify_token_calls = store.query_callees("auth::middleware::verifyToken", false).unwrap();
    let verify_token_callees: Vec<_> = verify_token_calls
        .iter()
        .map(|traversal| traversal.qualname.clone().unwrap())
        .collect();
    assert_eq!(verify_token_callees, vec!["auth::jwt::verify"]);

    let format_calls = store.query_callees("utils::formatter::format", false).unwrap();
    let format_callees: Vec<_> = format_calls
        .iter()
        .map(|traversal| traversal.qualname.clone().unwrap())
        .collect();
    assert_eq!(format_callees, vec!["utils::logger::log"]);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_direct_calls_are_resolved_conservatively() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let greet_calls = store.query_callees("lib::greet", false).unwrap();
    let greet_callees: Vec<_> = greet_calls
        .iter()
        .map(|traversal| {
            (
                traversal.qualname.clone(),
                traversal.edge_kind.clone(),
                traversal.certainty.clone(),
            )
        })
        .collect();
    assert_eq!(
        greet_callees,
        vec![
            (
                Some("parser::parse".to_string()),
                EdgeKind::Call,
                scope_core::Certainty::Resolved,
            ),
            (
                Some("utils::format_output".to_string()),
                EdgeKind::Call,
                scope_core::Certainty::Resolved,
            ),
        ]
    );

    let parser_calls = store.query_callees("parser::parse", false).unwrap();
    assert_eq!(parser_calls.len(), 1);
    assert_eq!(parser_calls[0].qualname.as_deref(), Some("parser::tokenize"));
    assert_eq!(parser_calls[0].certainty, scope_core::Certainty::Exact);

    let parser_callers = store.query_callers("parser::parse", false).unwrap();
    let parser_caller_names: Vec<_> = parser_callers
        .iter()
        .filter_map(|traversal| traversal.qualname.clone())
        .collect();
    assert_eq!(parser_caller_names, vec!["lib::greet".to_string(), "resolver::resolve".to_string()]);

    fs::remove_dir_all(repo).unwrap();
}
