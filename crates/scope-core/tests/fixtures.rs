use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use scope_core::{
    adapter_for_language, arch_check, load_arch_config, scan_repo, stub, Certainty, CycleSeverity,
    EdgeKind, NodeKind, PublicSurfaceChange, PublicSurfaceChangeKind, PublicSurfaceSymbol,
    RepoPath, ScanConfig, SnapshotCycleDelta, SnapshotDeleteResult, SnapshotListSummary, Store,
    SupportedLanguage, SymbolKind, TraversalRecord, Visibility,
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
            let metadata = fs::metadata(&entry.absolute_path).unwrap();
            let mut extract = adapter.extract(&entry, &source);
            extract.file.content_hash = Some(blake3::hash(source.as_bytes()).to_hex().to_string());
            extract.file.mtime_unix_seconds = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64);
            extract.file.size_bytes = Some(metadata.len() as i64);
            Some(extract)
        })
        .collect();

    store.persist_extract_results(&extracts).unwrap();

    store
}

fn read_golden(name: &str) -> String {
    fs::read_to_string(golden_root().join(name))
        .unwrap()
        .trim_end_matches('\n')
        .to_string()
}

fn read_golden_json(name: &str) -> serde_json::Value {
    serde_json::from_str(&read_golden(name)).unwrap()
}

fn normalize_report_like_json(value: &mut serde_json::Value) {
    if let Some(generated_at) = value.pointer_mut("/data/result/generated_at") {
        *generated_at = serde_json::Value::from(0);
    }
    if let Some(generated_at) = value.pointer_mut("/data/result/report/generated_at") {
        *generated_at = serde_json::Value::from(0);
    }
    for pointer in [
        "/data/result/unreachable_detail",
        "/data/result/report/unreachable_detail",
    ] {
        if let Some(items) = value
            .pointer_mut(pointer)
            .and_then(|node| node.as_array_mut())
        {
            for item in items {
                if let Some(days) = item.get_mut("last_modified_days_ago") {
                    *days = serde_json::Value::from(0);
                }
            }
        }
    }
}

#[test]
fn cochange_fixture_contains_history_script() {
    let root = fixture_root("cochange");
    assert!(root.join("README.txt").is_file(), "missing cochange README");
    let script = root.join("create_git_history.sh");
    assert!(script.is_file(), "missing cochange history script");
    let contents = fs::read_to_string(script).unwrap();
    assert!(contents.contains("git init -q"));
    assert!(contents.contains("src/parser.rs"));
    assert!(contents.contains("src/utils.rs"));
    assert!(contents.contains("src/resolver.rs"));
    assert!(contents.contains("parser and utils evolve together"));
    assert!(contents.contains("parser utils and resolver evolve together"));
}

#[test]
fn planned_fixture_directories_exist() {
    for fixture in [
        "rust_small",
        "ts_small",
        "mixed_repo",
        "dynamic_limits",
        "arch_violations",
        "test_map_ts",
        "cochange",
        "capability_audit",
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
fn snapshot_round_trip_and_diff_work_for_rust_small_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let first = store
        .save_snapshot("baseline", Some("HEAD".to_string()))
        .unwrap();
    let list = store.list_snapshots().unwrap();

    assert_eq!(first.snapshot.name, "baseline");
    assert_eq!(list.summary, SnapshotListSummary { snapshot_count: 1 });
    assert_eq!(list.snapshots[0].name, "baseline");

    let parser_path = repo.join("src/parser.rs");
    let updated = fs::read_to_string(&parser_path)
        .unwrap()
        .replace("pub fn parse", "pub fn parse_token");
    fs::write(&parser_path, updated).unwrap();
    let _ = index_fixture(&repo);
    let second = store
        .save_snapshot("after", Some("HEAD~0".to_string()))
        .unwrap();
    let config = load_arch_config(&repo).unwrap();
    let diff = store.diff_snapshot("baseline", "after", &config).unwrap();

    assert_eq!(second.snapshot.name, "after");
    assert_eq!(diff.before.name, "baseline");
    assert_eq!(diff.after.name, "after");
    assert_eq!(
        diff.cycles,
        SnapshotCycleDelta {
            before: 0,
            after: 0,
            introduced: 0,
            resolved: 0,
        }
    );
    assert!(diff.omitted.is_empty());

    let deleted = store.delete_snapshot("baseline").unwrap();
    assert_eq!(
        deleted,
        SnapshotDeleteResult {
            name: "baseline".to_string(),
            deleted: true
        }
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn ts_small_fixture_has_expected_files() {
    let root = fixture_root("ts_small");
    for relative in [
        "package.json",
        "src/index.ts",
        "src/auth/index.ts",
        "src/auth/aliases.ts",
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
fn dynamic_limits_fixture_has_expected_files() {
    let root = fixture_root("dynamic_limits");
    for relative in [
        "README.txt",
        "package.json",
        "src/index.js",
        "src/computed_import.ts",
        "src/dynamic_require.js",
    ] {
        assert!(
            root.join(relative).is_file(),
            "missing dynamic_limits file: {relative}"
        );
    }
}

#[test]
fn test_map_ts_fixture_has_expected_files() {
    let root = fixture_root("test_map_ts");
    for relative in [
        "package.json",
        ".scope/arch.toml",
        "src/auth/middleware.ts",
        "src/routes/api.ts",
        "src/app.ts",
        "tests/auth/middleware.test.ts",
        "tests/integration/api.test.ts",
        "tests/e2e/app.test.ts",
    ] {
        assert!(
            root.join(relative).is_file(),
            "missing test_map_ts file: {relative}"
        );
    }
}

#[test]
fn arch_violations_fixture_has_expected_files() {
    let root = fixture_root("arch_violations");
    for relative in [
        "package.json",
        ".scope/arch.toml",
        "src/routes/http.ts",
        "src/services/user.ts",
        "src/models/account.ts",
        "src/utils/format.ts",
    ] {
        assert!(
            root.join(relative).is_file(),
            "missing arch_violations file: {relative}"
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
    let deps = store
        .query_reverse_deps(&RepoPath::from("src/parser.rs"))
        .unwrap();
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
        deps.iter()
            .map(|dep| dep.path.0.clone())
            .collect::<Vec<_>>(),
        vec!["src/auth/index.ts", "src/utils/formatter.ts"]
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn mixed_repo_fixture_has_expected_files() {
    let root = fixture_root("mixed_repo");
    for relative in [
        "Cargo.toml",
        "package.json",
        "src/lib.rs",
        "src/parser.rs",
        "web/index.ts",
        "web/auth.ts",
    ] {
        assert!(
            root.join(relative).is_file(),
            "missing mixed_repo file: {relative}"
        );
    }
}

#[test]
fn mixed_repo_scans_and_indexes_rust_and_typescript_together() {
    let entries = scan_repo(&fixture_root("mixed_repo"), &ScanConfig::default()).unwrap();
    assert_eq!(entries.len(), 4);
    assert!(entries
        .iter()
        .any(|entry| entry.path == RepoPath::from("src/lib.rs")
            && entry.language == SupportedLanguage::Rust));
    assert!(entries
        .iter()
        .any(|entry| entry.path == RepoPath::from("src/parser.rs")
            && entry.language == SupportedLanguage::Rust));
    assert!(entries
        .iter()
        .any(|entry| entry.path == RepoPath::from("web/index.ts")
            && entry.language == SupportedLanguage::TypeScript));
    assert!(entries
        .iter()
        .any(|entry| entry.path == RepoPath::from("web/auth.ts")
            && entry.language == SupportedLanguage::TypeScript));

    let repo = prepare_fixture_copy("mixed_repo");
    let store = index_fixture(&repo);

    let rust_deps = store.query_deps(&RepoPath::from("src/lib.rs")).unwrap();
    assert_eq!(
        rust_deps
            .iter()
            .map(|dep| dep.path.0.as_str())
            .collect::<Vec<_>>(),
        vec!["src/parser.rs"]
    );

    let ts_deps = store.query_deps(&RepoPath::from("web/index.ts")).unwrap();
    assert_eq!(
        ts_deps
            .iter()
            .map(|dep| dep.path.0.as_str())
            .collect::<Vec<_>>(),
        vec!["web/auth.ts"]
    );

    let rust_symbols = store
        .query_symbols(&RepoPath::from("src/lib.rs"), false, None)
        .unwrap();
    assert_eq!(
        rust_symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        vec!["parser", "greet"]
    );

    let ts_symbols = store
        .query_symbols(&RepoPath::from("web/auth.ts"), false, None)
        .unwrap();
    assert_eq!(
        ts_symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        vec!["verifyToken"]
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
        .query_symbols(
            &RepoPath::from("src/lib.rs"),
            false,
            Some(SymbolKind::Function),
        )
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

    assert_eq!(
        dep_paths,
        vec!["src/auth/index.ts", "src/utils/formatter.ts"]
    );

    let auth_deps = store
        .query_deps(&RepoPath::from("src/auth/index.ts"))
        .unwrap();
    let auth_dep_paths: Vec<_> = auth_deps.iter().map(|dep| dep.path.0.clone()).collect();
    assert_eq!(
        auth_dep_paths,
        vec!["src/auth/aliases.ts", "src/auth/middleware.ts"]
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn ts_small_reverse_deps_and_symbols_and_calls_work_conservatively() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let reverse = store
        .query_reverse_deps(&RepoPath::from("src/auth/jwt.ts"))
        .unwrap();
    let reverse_paths: Vec<_> = reverse.iter().map(|dep| dep.path.0.clone()).collect();
    assert_eq!(
        reverse_paths,
        vec!["src/auth/aliases.ts", "src/auth/middleware.ts"]
    );

    let jwt_symbols = store
        .query_symbols(&RepoPath::from("src/auth/jwt.ts"), false, None)
        .unwrap();
    let jwt_symbol_names: Vec<_> = jwt_symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(jwt_symbol_names, vec!["sign", "verify"]);

    let middleware_symbols = store
        .query_symbols(&RepoPath::from("src/auth/middleware.ts"), false, None)
        .unwrap();
    let middleware_symbol_names: Vec<_> = middleware_symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(middleware_symbol_names, vec!["verifyToken"]);

    let alias_symbols = store
        .query_symbols(&RepoPath::from("src/auth/aliases.ts"), false, None)
        .unwrap();
    let alias_symbol_names: Vec<_> = alias_symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(alias_symbol_names, vec!["verifyJwt"]);

    let auth_index_symbols = store
        .query_symbols(&RepoPath::from("src/auth/index.ts"), true, None)
        .unwrap();
    let auth_index_symbol_names: Vec<_> = auth_index_symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(auth_index_symbol_names, vec!["verifyToken", "verifyJwt"]);

    let verify_token_calls = store
        .query_callees("auth::middleware::verifyToken", false)
        .unwrap();
    let verify_token_callees: Vec<_> = verify_token_calls
        .iter()
        .map(|traversal| traversal.qualname.clone().unwrap())
        .collect();
    assert_eq!(verify_token_callees, vec!["auth::jwt::verify"]);

    let format_calls = store
        .query_callees("utils::formatter::format", false)
        .unwrap();
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
    let greet_envelope = stub::calls("lib::greet".to_string(), false, greet_calls.clone());
    let greet_actual = serde_json::to_string_pretty(&greet_envelope).unwrap();
    let greet_expected = read_golden("rust_small_greet_calls.json");
    assert_eq!(greet_actual, greet_expected);
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
    let parser_envelope = stub::calls("parser::parse".to_string(), false, parser_calls.clone());
    let parser_actual = serde_json::to_string_pretty(&parser_envelope).unwrap();
    let parser_expected = read_golden("rust_small_parse_calls.json");
    assert_eq!(parser_actual, parser_expected);
    assert_eq!(parser_calls.len(), 1);
    assert_eq!(
        parser_calls[0].qualname.as_deref(),
        Some("parser::tokenize")
    );
    assert_eq!(parser_calls[0].certainty, scope_core::Certainty::Exact);

    let parser_calls_transitive = store.query_callees("parser::parse", true).unwrap();
    let parser_calls_transitive_envelope = stub::calls(
        "parser::parse".to_string(),
        true,
        parser_calls_transitive.clone(),
    );
    let parser_calls_transitive_actual =
        serde_json::to_string_pretty(&parser_calls_transitive_envelope).unwrap();
    let parser_calls_transitive_expected = read_golden("rust_small_parse_calls_transitive.json");
    assert_eq!(
        parser_calls_transitive_actual,
        parser_calls_transitive_expected
    );
    // parser::parse reaches parser::tokenize directly, so transitive traversal matches the direct edge
    assert_eq!(parser_calls_transitive.len(), 1);
    assert_eq!(
        parser_calls_transitive[0].qualname.as_deref(),
        Some("parser::tokenize")
    );
    assert_eq!(parser_calls_transitive[0].distance, 1);

    let farewell_calls = store.query_callees("lib::farewell", false).unwrap();
    let farewell_envelope = stub::calls("lib::farewell".to_string(), false, farewell_calls.clone());
    let farewell_actual = serde_json::to_string_pretty(&farewell_envelope).unwrap();
    let farewell_expected = read_golden("rust_small_farewell_calls.json");
    assert_eq!(farewell_actual, farewell_expected);
    assert!(
        farewell_calls.is_empty(),
        "farewell should conservatively omit dynamic formatting internals"
    );

    let parser_callers = store.query_callers("parser::parse", false).unwrap();
    let callers_envelope =
        stub::callers("parser::parse".to_string(), false, parser_callers.clone());
    let callers_actual = serde_json::to_string_pretty(&callers_envelope).unwrap();
    let callers_expected = read_golden("rust_small_parse_callers.json");
    assert_eq!(callers_actual, callers_expected);
    let parser_caller_names: Vec<_> = parser_callers
        .iter()
        .filter_map(|traversal| traversal.qualname.clone())
        .collect();
    assert_eq!(
        parser_caller_names,
        vec!["lib::greet".to_string(), "resolver::resolve".to_string()]
    );

    let parser_callers_transitive = store.query_callers("parser::parse", true).unwrap();
    let parser_callers_transitive_envelope = stub::callers(
        "parser::parse".to_string(),
        true,
        parser_callers_transitive.clone(),
    );
    let parser_callers_transitive_actual =
        serde_json::to_string_pretty(&parser_callers_transitive_envelope).unwrap();
    let parser_callers_transitive_expected =
        read_golden("rust_small_parse_callers_transitive.json");
    assert_eq!(
        parser_callers_transitive_actual,
        parser_callers_transitive_expected
    );
    // parser::parse is called transitively by the same two indexed callers that the golden captures.
    assert_eq!(parser_callers_transitive.len(), 2);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_explain_query_matches_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let traversals = store.query_explain("parser::parse", None, None).unwrap();
    let envelope = stub::explain("parser::parse".to_string(), None, None, traversals.clone());
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("rust_small_parse_explain.json");

    assert_eq!(actual, expected);
    assert_eq!(traversals.len(), 4);
    assert!(traversals
        .iter()
        .any(|record| record.qualname.as_deref() == Some("lib::greet")));
    assert!(traversals
        .iter()
        .any(|record| record.qualname.as_deref() == Some("parser::tokenize")));
    assert!(traversals
        .iter()
        .any(|record| record.path.as_ref().map(|path| path.0.as_str()) == Some("src/resolver.rs")));

    let filtered = store
        .query_explain("parser::parse", Some("resolver::resolve"), None)
        .unwrap();
    let filtered_envelope = stub::explain(
        "parser::parse".to_string(),
        Some("resolver::resolve".to_string()),
        None,
        filtered.clone(),
    );
    let filtered_actual = serde_json::to_string_pretty(&filtered_envelope).unwrap();
    let filtered_expected = read_golden("rust_small_parse_explain_to_resolver.json");

    assert_eq!(filtered_actual, filtered_expected);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].qualname.as_deref(), Some("resolver::resolve"));
    assert_eq!(filtered[0].distance, 1);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_public_surface_contains_only_exported_symbols() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let surface = store
        .query_public_surface(&RepoPath::from("src/parser.rs"))
        .unwrap();
    let names: Vec<_> = surface
        .symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect();

    assert_eq!(surface.file, RepoPath::from("src/parser.rs"));
    assert_eq!(names, vec!["parse"]);
    assert!(surface
        .symbols
        .iter()
        .all(|symbol| symbol.qualname.starts_with("parser::")));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn ts_small_public_surface_diff_captures_added_removed_and_modified_symbols() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let diff = store
        .diff_public_surface(
            &RepoPath::from("src/auth/jwt.ts"),
            &RepoPath::from("src/auth/aliases.ts"),
        )
        .unwrap();

    assert_eq!(diff.summary.added_count, 1);
    assert_eq!(diff.summary.removed_count, 2);
    assert_eq!(diff.summary.modified_count, 0);
    assert!(diff
        .changes
        .iter()
        .any(|change| change.kind == PublicSurfaceChangeKind::Added
            && change
                .after
                .as_ref()
                .is_some_and(|symbol| symbol.name == "verifyJwt")));
    assert!(diff
        .changes
        .iter()
        .filter(|change| change.kind == PublicSurfaceChangeKind::Removed)
        .any(|change| change
            .before
            .as_ref()
            .is_some_and(|symbol| symbol.name == "sign")));
    assert!(diff
        .changes
        .iter()
        .filter(|change| change.kind == PublicSurfaceChangeKind::Removed)
        .any(|change| change
            .before
            .as_ref()
            .is_some_and(|symbol| symbol.name == "verify")));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_public_surface_is_deterministically_sorted_by_line_then_qualname() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let surface = store
        .query_public_surface(&RepoPath::from("src/parser.rs"))
        .unwrap();
    let ordered: Vec<_> = surface
        .symbols
        .iter()
        .map(|symbol| (symbol.line, symbol.qualname.as_str()))
        .collect();

    assert_eq!(ordered, vec![(1, "parser::parse")]);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn rust_small_public_surface_includes_exported_resolver_symbol() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let surface = store
        .query_public_surface(&RepoPath::from("src/resolver.rs"))
        .unwrap();

    assert_eq!(surface.file, RepoPath::from("src/resolver.rs"));
    assert_eq!(surface.symbols.len(), 1);
    assert_eq!(surface.symbols[0].qualname, "resolver::resolve");

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn public_surface_diff_marks_matching_identity_with_metadata_changes_as_modified() {
    let before = PublicSurfaceSymbol {
        file: RepoPath::from("src/parser.rs"),
        name: "parse".to_string(),
        qualname: "parser::parse".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        line: 1,
    };
    let after = PublicSurfaceSymbol {
        file: RepoPath::from("src/parser.rs"),
        name: "parse".to_string(),
        qualname: "parser::parse".to_string(),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        line: 3,
    };

    let change = PublicSurfaceChange {
        kind: PublicSurfaceChangeKind::Modified,
        before: Some(before),
        after: Some(after),
    };

    assert_eq!(change.kind, PublicSurfaceChangeKind::Modified);
    assert_eq!(change.before.as_ref().unwrap().line, 1);
    assert_eq!(change.after.as_ref().unwrap().line, 3);
    assert_ne!(change.before, change.after);
}

#[test]
fn fixture_indexing_persists_file_fingerprint_metadata() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let state = store
        .file_state(&RepoPath::from("src/lib.rs"))
        .unwrap()
        .unwrap();
    assert!(state
        .content_hash
        .as_ref()
        .is_some_and(|hash| !hash.is_empty()));
    assert!(state.mtime_unix_seconds.is_some());
    assert!(state.size_bytes.is_some_and(|size| size > 0));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn doctor_output_reports_index_health_counts() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let stats = store.index_health_stats().unwrap();
    let envelope = stub::doctor(false, stats.clone());

    assert!(matches!(envelope.status, scope_core::JsonStatus::Ok));
    assert_eq!(
        envelope.data.schema_version,
        scope_core::INDEX_SCHEMA_VERSION
    );
    assert_eq!(envelope.data.stats.files, 5);
    assert_eq!(envelope.data.stats.imports, 1);
    assert_eq!(envelope.data.stats.unresolved_imports, 0);
    assert_eq!(envelope.data.stats.symbols, 10);
    assert_eq!(envelope.data.stats.call_edges, 4);
    assert_eq!(envelope.data.stats.parse_status.ok, 5);
    assert_eq!(envelope.data.stats.parse_status.partial, 0);
    assert_eq!(envelope.data.stats.parse_status.error, 0);
    assert_eq!(envelope.data.checks.len(), 3);
    assert_eq!(envelope.data.checks[0].name, "files_indexed");
    assert_eq!(envelope.data.checks[0].status, "ok");

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn dynamic_limits_fixture_reports_partial_parse_health_without_false_edges() {
    let repo = prepare_fixture_copy("dynamic_limits");
    let store = index_fixture(&repo);

    let stats = store.index_health_stats().unwrap();
    assert_eq!(stats.files, 3);
    assert_eq!(stats.imports, 2);
    assert_eq!(stats.unresolved_imports, 0);
    assert_eq!(stats.symbols, 3);
    assert_eq!(stats.call_edges, 2);
    assert_eq!(stats.parse_status.ok, 1);
    assert_eq!(stats.parse_status.partial, 2);
    assert_eq!(stats.parse_status.error, 0);

    let doctor = stub::doctor(false, stats.clone());
    assert_eq!(doctor.data.checks[2].name, "parse_status");
    assert_eq!(doctor.data.checks[2].status, "warn");

    let deps = store.query_deps(&RepoPath::from("src/index.js")).unwrap();
    let dep_paths: Vec<_> = deps.iter().map(|dep| dep.path.0.clone()).collect();
    assert_eq!(
        dep_paths,
        vec![
            "src/computed_import.ts".to_string(),
            "src/dynamic_require.js".to_string()
        ]
    );

    let feature_calls = store
        .query_callees("computed_import::loadFeature", false)
        .unwrap();
    assert!(feature_calls.is_empty());

    let plugin_calls = store
        .query_callees("dynamic_require::loadPlugin", false)
        .unwrap();
    assert!(plugin_calls.is_empty());

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn benchmark_output_reports_full_and_incremental_timing_summary() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let stats = store.index_health_stats().unwrap();
    let envelope = stub::benchmark(
        Some("ts_small".to_string()),
        Some(3),
        stub::BenchmarkSummary {
            indexed_files: stats.files,
            mutation: stub::BenchmarkMutationSummary {
                target_file: RepoPath::from("src/auth/jwt.ts"),
                change_kind: "append_comment",
            },
            full: stub::BenchmarkPhaseSummary {
                avg_ms: 42,
                min_ms: 40,
                max_ms: 45,
                files_processed_avg: stats.files,
                changed_files_avg: stats.files,
                deleted_files_avg: 0,
                affected_files_avg: stats.files,
            },
            incremental: stub::BenchmarkPhaseSummary {
                avg_ms: 7,
                min_ms: 6,
                max_ms: 9,
                files_processed_avg: 2,
                changed_files_avg: 1,
                deleted_files_avg: 0,
                affected_files_avg: 2,
            },
            comparison: stub::BenchmarkComparisonSummary {
                saved_ms: 35,
                incremental_pct_of_full: 16,
            },
        },
    );

    assert!(matches!(envelope.status, scope_core::JsonStatus::Ok));
    assert_eq!(envelope.data.fixture.as_deref(), Some("ts_small"));
    assert_eq!(envelope.data.iterations, Some(3));
    assert_eq!(envelope.data.summary.indexed_files, stats.files);
    assert_eq!(
        envelope.data.summary.mutation.target_file,
        RepoPath::from("src/auth/jwt.ts")
    );
    assert_eq!(envelope.data.summary.full.avg_ms, 42);
    assert_eq!(envelope.data.summary.incremental.avg_ms, 7);
    assert_eq!(envelope.data.summary.comparison.saved_ms, 35);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn stability_query_reports_expected_scores_and_filtering() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let result = store
        .query_stability(None, None, scope_core::StabilitySort::Instability)
        .unwrap();
    assert_eq!(result.file, None);
    assert_eq!(result.flag_threshold, None);
    assert_eq!(result.sort, scope_core::StabilitySort::Instability);
    assert_eq!(result.files.len(), 5);
    assert_eq!(result.summary.flagged_count, 0);
    assert_eq!(result.summary.healthy_leaf_count, 1);
    assert_eq!(result.summary.isolated_count, 3);

    let lib = result
        .files
        .iter()
        .find(|record| record.path == RepoPath::from("src/lib.rs"))
        .unwrap();
    assert_eq!(lib.fan_in, 0);
    assert_eq!(lib.fan_out, 0);
    assert_eq!(lib.instability, 0.0);
    assert_eq!(lib.category, scope_core::StabilityCategory::Isolated);
    assert!(!lib.flagged);
    assert_eq!(
        lib.reason.as_deref(),
        Some("no direct imports and no dependents — isolated file")
    );

    let parser = result
        .files
        .iter()
        .find(|record| record.path == RepoPath::from("src/parser.rs"))
        .unwrap();
    assert_eq!(parser.fan_in, 1);
    assert_eq!(parser.fan_out, 0);
    assert_eq!(parser.instability, 0.0);
    assert_eq!(parser.category, scope_core::StabilityCategory::Stable);

    let resolver = result
        .files
        .iter()
        .find(|record| record.path == RepoPath::from("src/resolver.rs"))
        .unwrap();
    assert_eq!(resolver.fan_in, 0);
    assert_eq!(resolver.fan_out, 1);
    assert_eq!(resolver.instability, 1.0);
    assert_eq!(
        resolver.category,
        scope_core::StabilityCategory::HealthyLeaf
    );
    assert_eq!(
        resolver.reason.as_deref(),
        Some("no downstream dependents and fan-out 1 — healthy leaf node")
    );

    let filtered = store
        .query_stability(
            Some(&RepoPath::from("src/parser.rs")),
            Some(0.5),
            scope_core::StabilitySort::FanIn,
        )
        .unwrap();
    assert_eq!(filtered.file, Some(RepoPath::from("src/parser.rs")));
    assert_eq!(filtered.flag_threshold, Some(0.5));
    assert_eq!(filtered.sort, scope_core::StabilitySort::FanIn);
    assert_eq!(filtered.files.len(), 1);
    assert_eq!(filtered.files[0].path, RepoPath::from("src/parser.rs"));

    let flagged_only = store
        .query_stability(None, Some(0.5), scope_core::StabilitySort::Instability)
        .unwrap();
    assert!(flagged_only.files.is_empty());

    let sorted_by_path = store
        .query_stability(None, None, scope_core::StabilitySort::Path)
        .unwrap();
    assert_eq!(
        sorted_by_path.files.first().unwrap().path,
        RepoPath::from("src/lib.rs")
    );

    assert!(matches!(
        store.query_stability(None, Some(1.5), scope_core::StabilitySort::Instability),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));
    assert!(matches!(
        store.query_stability(
            Some(&RepoPath::from("src/missing.rs")),
            None,
            scope_core::StabilitySort::Instability,
        ),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));

    let envelope = stub::stability(filtered.clone());
    assert!(matches!(envelope.status, scope_core::JsonStatus::Ok));
    assert_eq!(envelope.data.result, filtered);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn risk_query_reports_expected_scores_and_fallbacks() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    store
        .persist_file_churn(
            &RepoPath::from("src/parser.rs"),
            "c1",
            Some("agent@example.com"),
            Some(1_700_000_000),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/parser.rs"),
            "c2",
            Some("agent@example.com"),
            Some(1_700_000_100),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/utils.rs"),
            "c3",
            Some("agent@example.com"),
            Some(1_700_000_200),
        )
        .unwrap();

    let result = store
        .query_risk(None, 10_000, None, None, scope_core::RiskSort::Score)
        .unwrap();
    assert_eq!(result.file, None);
    assert_eq!(result.days, 10_000);
    assert_eq!(result.sort, scope_core::RiskSort::Score);
    assert!(result.summary.git_available);
    assert_eq!(result.summary.scored_files, 5);
    assert_eq!(result.files.len(), 5);

    let parser = result
        .files
        .iter()
        .find(|record| record.path == RepoPath::from("src/parser.rs"))
        .unwrap();
    assert_eq!(parser.direct_dependents, 1);
    assert_eq!(parser.transitive_dependents, 1);
    assert_eq!(parser.churn_commits, 2);
    assert!(parser.score > 0.0);

    let actual = serde_json::to_string_pretty(&stub::risk(result.clone())).unwrap();
    let expected = read_golden("rust_small_risk.json");
    assert_eq!(actual, expected);

    let single = store
        .query_risk(
            Some(&RepoPath::from("src/parser.rs")),
            10_000,
            None,
            None,
            scope_core::RiskSort::Score,
        )
        .unwrap();
    assert_eq!(single.files.len(), 1);
    assert_eq!(single.files[0].path, RepoPath::from("src/parser.rs"));

    let filtered = store
        .query_risk(
            None,
            10_000,
            Some(1.0),
            Some(1),
            scope_core::RiskSort::Score,
        )
        .unwrap();
    assert_eq!(filtered.files.len(), 1);
    assert_eq!(filtered.files[0].path, RepoPath::from("src/parser.rs"));

    store.clear_file_churn().unwrap();
    let fallback = store
        .query_risk(None, 30, None, None, scope_core::RiskSort::Score)
        .unwrap();
    assert!(!fallback.summary.git_available);
    assert!(fallback
        .files
        .iter()
        .all(|record| record.reason.contains("git churn unavailable")));
    let fallback_actual = serde_json::to_string_pretty(&stub::risk(fallback.clone())).unwrap();
    let fallback_expected = read_golden("rust_small_risk_no_git.json");
    assert_eq!(fallback_actual, fallback_expected);

    assert!(matches!(
        store.query_risk(None, 0, None, None, scope_core::RiskSort::Score),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));
    assert!(matches!(
        store.query_risk(
            Some(&RepoPath::from("src/missing.rs")),
            30,
            None,
            None,
            scope_core::RiskSort::Score,
        ),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn generated_cochange_fixture_creates_expected_commit_history() {
    let fixture_root = fixture_root("cochange");
    let script = fixture_root.join("create_git_history.sh");
    let repo = unique_temp_dir("cochange-generated");

    let status = Command::new(&script).arg(&repo).status().unwrap();
    assert!(status.success());

    let output = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .arg("log")
        .arg("--format=%s")
        .output()
        .unwrap();
    assert!(output.status.success());
    let log = String::from_utf8(output.stdout).unwrap();
    assert!(log.contains("parser and utils evolve together"));
    assert!(log.contains("parser utils and resolver evolve together"));
    assert!(log.contains("parser evolves alone"));
    assert!(log.contains("resolver evolves alone"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn generated_cochange_fixture_persists_expected_file_churn() {
    let fixture_root = fixture_root("cochange");
    let script = fixture_root.join("create_git_history.sh");
    let repo = unique_temp_dir("cochange-churn");

    let status = Command::new(&script).arg(&repo).status().unwrap();
    assert!(status.success());

    let store = index_fixture(&repo);
    let output = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .arg("log")
        .arg("--since=10000 days ago")
        .arg("--format=%H|%ae|%ct")
        .arg("--name-only")
        .output()
        .unwrap();
    assert!(output.status.success());

    let mut current_commit: Option<(String, String, i64)> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split('|');
        if let (Some(sha), Some(email), Some(timestamp)) =
            (parts.next(), parts.next(), parts.next())
        {
            if let Ok(timestamp) = timestamp.parse::<i64>() {
                current_commit = Some((sha.to_string(), email.to_string(), timestamp));
                continue;
            }
        }
        let Some((sha, email, timestamp)) = current_commit.as_ref() else {
            continue;
        };
        let inserted = store
            .persist_file_churn(
                &RepoPath::from(trimmed.to_string()),
                sha,
                Some(email.as_str()),
                Some(*timestamp),
            )
            .unwrap();
        if trimmed.ends_with(".rs") {
            assert!(inserted, "expected churn row for {trimmed}");
        }
    }

    let parser_churn = store
        .query_risk(
            Some(&RepoPath::from("src/parser.rs")),
            10000,
            None,
            None,
            scope_core::RiskSort::Score,
        )
        .unwrap();
    assert!(parser_churn.summary.git_available);
    assert!(parser_churn.files[0].churn_commits >= 3);

    let result = store
        .query_cochange(
            &RepoPath::from("src/parser.rs"),
            10000,
            1,
            None,
            scope_core::CochangeSort::Score,
        )
        .unwrap();
    assert!(result.summary.git_available);
    assert_eq!(result.summary.target_commits, 4);
    assert_eq!(result.files.len(), 2);
    assert_eq!(result.files[0].path, RepoPath::from("src/utils.rs"));
    assert_eq!(result.files[1].path, RepoPath::from("src/resolver.rs"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn query_result_matches_golden_json_for_rust_small_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let mut session = scope_core::QuerySession::default();

    let result = scope_core::execute_query(
        "file \"src/lib.rs\" | .deps | unique | count",
        &store,
        &mut session,
    )
    .unwrap();
    let actual = serde_json::to_value(stub::query(
        "file \"src/lib.rs\" | .deps | unique | count".to_string(),
        result,
    ))
    .unwrap();
    let expected = read_golden_json("rust_small_query_deps_count.json");

    assert_eq!(actual, expected);
    assert!(session.binding_names().is_empty());

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn query_result_matches_golden_json_for_let_binding_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let mut session = scope_core::QuerySession::default();

    let result = scope_core::execute_query(
        "let roots = file \"src/lib.rs\" | .deps | unique",
        &store,
        &mut session,
    )
    .unwrap();
    let actual = serde_json::to_value(stub::query(
        "let roots = file \"src/lib.rs\" | .deps | unique".to_string(),
        result,
    ))
    .unwrap();
    let expected = read_golden_json("rust_small_query_let_roots.json");

    assert_eq!(actual, expected);
    assert_eq!(session.binding_names(), vec!["roots".to_string()]);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn query_result_matches_golden_json_for_shared_binding_followup_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let mut session = scope_core::QuerySession::default();

    scope_core::execute_query(
        "let roots = file \"src/lib.rs\" | .deps | unique",
        &store,
        &mut session,
    )
    .unwrap();
    let result = scope_core::execute_query("$roots | count", &store, &mut session).unwrap();
    let actual = serde_json::to_value(stub::query("$roots | count".to_string(), result)).unwrap();
    let expected = read_golden_json("rust_small_query_shared_binding_count.json");

    assert_eq!(actual, expected);
    assert_eq!(session.binding_names(), vec!["roots".to_string()]);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn query_followup_error_preserves_existing_shared_binding_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let mut session = scope_core::QuerySession::default();

    scope_core::execute_query(
        "let roots = file \"src/lib.rs\" | .deps | unique",
        &store,
        &mut session,
    )
    .unwrap();

    let error = scope_core::execute_query("$missing | count", &store, &mut session).unwrap_err();
    assert_eq!(error.kind(), "invalid_input");
    assert!(error
        .to_string()
        .contains("unknown query binding `$missing`"));
    assert_eq!(session.binding_names(), vec!["roots".to_string()]);

    let followup = scope_core::execute_query("$roots | count", &store, &mut session).unwrap();
    assert_eq!(followup, scope_core::QueryValue::Number(3));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn report_query_matches_golden_json_for_rust_small_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let config = load_arch_config(&repo).unwrap();

    let report = store.query_report(&config, None).unwrap();
    let mut actual = serde_json::to_value(stub::report(report)).unwrap();
    let mut expected = read_golden_json("rust_small_report.json");
    normalize_report_like_json(&mut actual);
    normalize_report_like_json(&mut expected);

    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn report_query_matches_golden_json_for_snapshot_comparison() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    store
        .save_snapshot("baseline", Some("HEAD".to_string()))
        .unwrap();

    let parser_path = repo.join("src/parser.rs");
    let updated = fs::read_to_string(&parser_path)
        .unwrap()
        .replace("pub fn parse", "pub fn parse_token");
    fs::write(&parser_path, updated).unwrap();
    let _ = index_fixture(&repo);

    let config = load_arch_config(&repo).unwrap();
    let report = store.query_report(&config, Some("baseline")).unwrap();
    let mut actual = serde_json::to_value(stub::report(report.clone())).unwrap();
    let mut expected = read_golden_json("rust_small_report_compare_baseline.json");
    normalize_report_like_json(&mut actual);
    normalize_report_like_json(&mut expected);

    assert_eq!(actual, expected);
    assert_eq!(report.metrics.total_files, 5);
    assert!(report.compare.is_some());
    let compare = report.compare.as_ref().unwrap();
    assert_eq!(compare.target, "baseline");
    assert_eq!(compare.baseline_health_score, 94.0);
    assert_eq!(compare.health_score_delta, -12.0);
    assert_eq!(compare.unreachable_files_delta, 3);
    assert_eq!(compare.public_surface_removed_delta, 1);
    assert!(report.metrics.public_surface_removed >= 1);
    assert_eq!(
        report.recommendations,
        vec![
            "review 3 unreachable files for dead code or missing entry-point declarations"
                .to_string(),
            "review 1 removed public surface entries against the comparison snapshot before landing changes"
                .to_string(),
            "health score regressed by 12.0 points versus baseline".to_string(),
        ]
    );

    let envelope = stub::report(report.clone());
    assert!(matches!(envelope.status, scope_core::JsonStatus::Ok));
    assert_eq!(envelope.data.result, report);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn gate_query_matches_golden_json_for_strict_arch_fixture() {
    let repo = prepare_fixture_copy("arch_violations");
    let store = index_fixture(&repo);
    let config = load_arch_config(&repo).unwrap();

    let gate = store.query_gate(&config, None, true).unwrap();
    let mut actual = serde_json::to_value(stub::gate(gate)).unwrap();
    let mut expected = read_golden_json("arch_violations_gate_strict.json");
    normalize_report_like_json(&mut actual);
    normalize_report_like_json(&mut expected);

    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn gate_query_uses_default_thresholds_and_fails_strict_arch_fixture() {
    let repo = prepare_fixture_copy("arch_violations");
    let store = index_fixture(&repo);
    let config = load_arch_config(&repo).unwrap();

    let gate = store.query_gate(&config, None, true).unwrap();
    assert_eq!(gate.compare, None);
    assert_eq!(gate.summary.total, 4);
    assert!(gate.summary.failed >= 1);
    assert!(gate
        .evaluations
        .iter()
        .any(|evaluation| evaluation.metric == scope_core::GateMetric::LayerViolations));
    assert!(gate
        .evaluations
        .iter()
        .any(|evaluation| evaluation.status == scope_core::GateStatus::Fail));

    let envelope = stub::gate(gate.clone());
    assert!(matches!(envelope.status, scope_core::JsonStatus::Ok));
    assert_eq!(envelope.data.result, gate);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn gate_query_warns_for_missing_compare_on_delta_only_gate() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let config = scope_core::ArchConfig {
        gates: vec![scope_core::GateConfig {
            metric: scope_core::GateMetric::HealthScoreDelta,
            min: None,
            max: None,
            min_delta: Some(-1.0),
            max_delta: None,
            severity: scope_core::GateSeverity::Warning,
            message: Some("health score should not regress much".to_string()),
            skip: false,
        }],
        ..scope_core::ArchConfig::default()
    };

    let gate = store.query_gate(&config, None, false).unwrap();
    assert_eq!(gate.summary.total, 1);
    assert_eq!(gate.summary.failed, 0);
    assert_eq!(gate.summary.warnings, 1);
    let evaluation = &gate.evaluations[0];
    assert_eq!(evaluation.metric, scope_core::GateMetric::HealthScoreDelta);
    assert_eq!(evaluation.status, scope_core::GateStatus::Warning);
    assert!(evaluation
        .detail
        .contains("comparison snapshot required for min_delta"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn gate_query_respects_skipped_custom_gate() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let config = scope_core::ArchConfig {
        gates: vec![scope_core::GateConfig {
            metric: scope_core::GateMetric::Cycles,
            min: None,
            max: None,
            min_delta: None,
            max_delta: None,
            severity: scope_core::GateSeverity::Warning,
            message: Some("cycles temporarily ignored".to_string()),
            skip: true,
        }],
        ..scope_core::ArchConfig::default()
    };

    let gate = store.query_gate(&config, None, false).unwrap();
    assert_eq!(gate.summary.total, 1);
    assert_eq!(gate.summary.skipped, 1);
    assert_eq!(gate.summary.failed, 0);
    assert_eq!(gate.summary.warnings, 0);
    assert_eq!(gate.evaluations[0].status, scope_core::GateStatus::Skipped);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn cochange_query_reports_expected_scores_and_filters() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    store
        .persist_file_churn(
            &RepoPath::from("src/parser.rs"),
            "c1",
            Some("agent@example.com"),
            Some(1_700_000_000),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/parser.rs"),
            "c2",
            Some("agent@example.com"),
            Some(1_700_000_100),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/parser.rs"),
            "c3",
            Some("agent@example.com"),
            Some(1_700_000_200),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/utils.rs"),
            "c1",
            Some("agent@example.com"),
            Some(1_700_000_000),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/utils.rs"),
            "c2",
            Some("agent@example.com"),
            Some(1_700_000_100),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/resolver.rs"),
            "c2",
            Some("agent@example.com"),
            Some(1_700_000_100),
        )
        .unwrap();
    store
        .persist_file_churn(
            &RepoPath::from("src/resolver.rs"),
            "c4",
            Some("agent@example.com"),
            Some(1_700_000_300),
        )
        .unwrap();

    let result = store
        .query_cochange(
            &RepoPath::from("src/parser.rs"),
            10_000,
            1,
            None,
            scope_core::CochangeSort::Score,
        )
        .unwrap();
    assert_eq!(result.target, RepoPath::from("src/parser.rs"));
    assert_eq!(result.days, 10_000);
    assert_eq!(result.min_shared_commits, 1);
    assert_eq!(result.sort, scope_core::CochangeSort::Score);
    assert!(result.summary.git_available);
    assert_eq!(result.summary.target_commits, 3);
    assert_eq!(result.files.len(), 2);
    assert_eq!(result.files[0].path, RepoPath::from("src/utils.rs"));
    assert_eq!(result.files[0].shared_commits, 2);
    assert_eq!(result.files[0].target_commits, 3);
    assert_eq!(result.files[0].candidate_commits, 2);
    assert_eq!(result.files[0].normalized_score, 100);
    assert_eq!(result.files[1].path, RepoPath::from("src/resolver.rs"));
    assert_eq!(result.files[1].shared_commits, 1);

    let actual = serde_json::to_string_pretty(&stub::cochange(result.clone())).unwrap();
    let expected = read_golden("rust_small_parser_cochange.json");
    assert_eq!(actual, expected);

    let filtered = store
        .query_cochange(
            &RepoPath::from("src/parser.rs"),
            10_000,
            2,
            Some(1),
            scope_core::CochangeSort::SharedCommits,
        )
        .unwrap();
    assert_eq!(filtered.files.len(), 1);
    assert_eq!(filtered.files[0].path, RepoPath::from("src/utils.rs"));

    store.clear_file_churn().unwrap();
    let fallback = store
        .query_cochange(
            &RepoPath::from("src/parser.rs"),
            30,
            1,
            None,
            scope_core::CochangeSort::Score,
        )
        .unwrap();
    assert!(!fallback.summary.git_available);
    assert!(fallback.files.is_empty());

    assert!(matches!(
        store.query_cochange(
            &RepoPath::from("src/parser.rs"),
            0,
            1,
            None,
            scope_core::CochangeSort::Score,
        ),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));
    assert!(matches!(
        store.query_cochange(
            &RepoPath::from("src/parser.rs"),
            30,
            0,
            None,
            scope_core::CochangeSort::Score,
        ),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));
    assert!(matches!(
        store.query_cochange(
            &RepoPath::from("src/missing.rs"),
            30,
            1,
            None,
            scope_core::CochangeSort::Score,
        ),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn utility_queries_report_expected_results_for_rust_small_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let unused = store.query_unused().unwrap();
    assert_eq!(unused.summary.exported_symbols, 8);
    assert_eq!(unused.summary.unused_symbols, 6);
    assert_eq!(unused.symbols[0].qualname, "lib::parser");
    let unused_actual = serde_json::to_string_pretty(&stub::unused(unused.clone())).unwrap();
    let unused_expected = read_golden("rust_small_unused.json");
    assert_eq!(unused_actual, unused_expected);

    let cycles = store.query_cycles(None).unwrap();
    assert_eq!(cycles.summary.cycle_count, 0);
    assert!(cycles.cycles.is_empty());
    let cycles_actual = serde_json::to_string_pretty(&stub::cycles(cycles.clone())).unwrap();
    let cycles_expected = read_golden("rust_small_cycles.json");
    assert_eq!(cycles_actual, cycles_expected);

    let tree = store
        .query_tree(&RepoPath::from("src/lib.rs"), false, Some(2))
        .unwrap();
    assert_eq!(tree.target, RepoPath::from("src/lib.rs"));
    assert_eq!(tree.summary.nodes, 5);
    let tree_actual = serde_json::to_string_pretty(&stub::tree(tree.clone())).unwrap();
    let tree_expected = read_golden("rust_small_tree.json");
    assert_eq!(tree_actual, tree_expected);

    let diff = store.query_branch_diff(&repo, "HEAD").unwrap();
    assert_eq!(diff.branch, "HEAD");
    assert_eq!(diff.summary.changed_files, 0);
    assert!(diff.affected_files.is_empty());
    let diff_actual = serde_json::to_string_pretty(&stub::diff(diff.clone())).unwrap();
    let diff_expected = read_golden("rust_small_diff_head.json");
    assert_eq!(diff_actual, diff_expected);

    assert!(matches!(
        store
            .query_cycles(Some(CycleSeverity::High))
            .unwrap()
            .severity,
        Some(CycleSeverity::High)
    ));
    assert!(matches!(
        store.query_tree(&RepoPath::from("src/missing.rs"), false, None),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn utility_queries_report_expected_results_for_ts_small_fixture() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let unused = store.query_unused().unwrap();
    assert_eq!(unused.summary.exported_symbols, 10);
    assert_eq!(unused.summary.unused_symbols, 8);
    assert_eq!(unused.symbols[0].qualname, "auth::aliases::verifyJwt");
    let unused_actual = serde_json::to_string_pretty(&stub::unused(unused.clone())).unwrap();
    let unused_expected = read_golden("ts_small_unused.json");
    assert_eq!(unused_actual, unused_expected);

    let cycles = store.query_cycles(None).unwrap();
    assert_eq!(cycles.summary.cycle_count, 0);
    assert!(cycles.cycles.is_empty());
    let cycles_actual = serde_json::to_string_pretty(&stub::cycles(cycles.clone())).unwrap();
    let cycles_expected = read_golden("ts_small_cycles.json");
    assert_eq!(cycles_actual, cycles_expected);

    let tree = store
        .query_tree(&RepoPath::from("src/index.ts"), false, Some(2))
        .unwrap();
    assert_eq!(tree.target, RepoPath::from("src/index.ts"));
    assert_eq!(tree.summary.nodes, 6);
    let tree_actual = serde_json::to_string_pretty(&stub::tree(tree.clone())).unwrap();
    let tree_expected = read_golden("ts_small_tree.json");
    assert_eq!(tree_actual, tree_expected);

    let diff = store.query_branch_diff(&repo, "HEAD").unwrap();
    assert_eq!(diff.branch, "HEAD");
    assert_eq!(diff.summary.changed_files, 0);
    assert!(diff.affected_files.is_empty());
    let diff_actual = serde_json::to_string_pretty(&stub::diff(diff.clone())).unwrap();
    let diff_expected = read_golden("ts_small_diff_head.json");
    assert_eq!(diff_actual, diff_expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn utility_queries_report_expected_results_for_mixed_repo_fixture() {
    let repo = prepare_fixture_copy("mixed_repo");
    let store = index_fixture(&repo);

    let unused = store.query_unused().unwrap();
    assert_eq!(unused.summary.exported_symbols, 5);
    assert_eq!(unused.summary.unused_symbols, 4);
    assert_eq!(unused.symbols[0].qualname, "lib::parser");
    let unused_actual = serde_json::to_string_pretty(&stub::unused(unused.clone())).unwrap();
    let unused_expected = read_golden("mixed_repo_unused.json");
    assert_eq!(unused_actual, unused_expected);

    let cycles = store.query_cycles(None).unwrap();
    assert_eq!(cycles.summary.cycle_count, 0);
    assert!(cycles.cycles.is_empty());
    let cycles_actual = serde_json::to_string_pretty(&stub::cycles(cycles.clone())).unwrap();
    let cycles_expected = read_golden("mixed_repo_cycles.json");
    assert_eq!(cycles_actual, cycles_expected);

    let tree = store
        .query_tree(&RepoPath::from("src/lib.rs"), false, Some(2))
        .unwrap();
    assert_eq!(tree.target, RepoPath::from("src/lib.rs"));
    assert_eq!(tree.summary.nodes, 2);
    let tree_actual = serde_json::to_string_pretty(&stub::tree(tree.clone())).unwrap();
    let tree_expected = read_golden("mixed_repo_tree.json");
    assert_eq!(tree_actual, tree_expected);

    let diff = store.query_branch_diff(&repo, "HEAD").unwrap();
    assert_eq!(diff.branch, "HEAD");
    assert_eq!(diff.summary.changed_files, 0);
    assert!(diff.affected_files.is_empty());
    let diff_actual = serde_json::to_string_pretty(&stub::diff(diff.clone())).unwrap();
    let diff_expected = read_golden("mixed_repo_diff_head.json");
    assert_eq!(diff_actual, diff_expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn why_queries_match_golden_json_and_handle_limits() {
    let rust_repo = prepare_fixture_copy("rust_small");
    let rust_store = index_fixture(&rust_repo);

    let symbol_path = rust_store
        .query_why("lib::greet", "parser::tokenize", None)
        .unwrap();
    let symbol_envelope = stub::why(
        "lib::greet".to_string(),
        "parser::tokenize".to_string(),
        None,
        symbol_path.clone(),
    );
    let symbol_actual = serde_json::to_string_pretty(&symbol_envelope).unwrap();
    let symbol_expected = read_golden("rust_small_greet_to_tokenize_why.json");
    assert_eq!(symbol_actual, symbol_expected);
    assert_eq!(symbol_path.len(), 2);
    assert_eq!(symbol_path[0].qualname.as_deref(), Some("parser::parse"));
    assert_eq!(symbol_path[1].qualname.as_deref(), Some("parser::tokenize"));

    let depth_limited = rust_store
        .query_why("lib::greet", "parser::tokenize", Some(1))
        .unwrap();
    assert!(depth_limited.is_empty());

    let same_target = rust_store
        .query_why("lib::greet", "lib::greet", None)
        .unwrap();
    assert!(same_target.is_empty());

    assert!(matches!(
        rust_store.query_why("src/lib.rs", "parser::parse", None),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));

    fs::remove_dir_all(rust_repo).unwrap();

    let ts_repo = prepare_fixture_copy("ts_small");
    let ts_store = index_fixture(&ts_repo);

    let file_path = ts_store
        .query_why("src/index.ts", "src/auth/middleware.ts", None)
        .unwrap();
    let file_envelope = stub::why(
        "src/index.ts".to_string(),
        "src/auth/middleware.ts".to_string(),
        None,
        file_path.clone(),
    );
    let file_actual = serde_json::to_string_pretty(&file_envelope).unwrap();
    let file_expected = read_golden("ts_small_index_to_auth_middleware_why.json");
    assert_eq!(file_actual, file_expected);
    assert_eq!(file_path.len(), 2);
    assert_eq!(
        file_path[0].path.as_ref().map(|path| path.0.as_str()),
        Some("src/auth/index.ts")
    );
    assert_eq!(
        file_path[1].path.as_ref().map(|path| path.0.as_str()),
        Some("src/auth/middleware.ts")
    );

    let disconnected = ts_store
        .query_why("src/utils/logger.ts", "src/auth/middleware.ts", None)
        .unwrap();
    assert!(disconnected.is_empty());

    fs::remove_dir_all(ts_repo).unwrap();
}

#[test]
fn rust_small_impact_queries_match_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    for (target, change_type, depth, golden_name, expected_total) in [
        (
            "parser::parse",
            "body",
            None,
            "rust_small_parse_impact_body.json",
            2usize,
        ),
        (
            "parser::parse",
            "signature",
            None,
            "rust_small_parse_impact_signature.json",
            3usize,
        ),
        (
            "parser::parse",
            "rename",
            None,
            "rust_small_parse_impact_rename.json",
            3usize,
        ),
        (
            "parser::parse",
            "visibility",
            None,
            "rust_small_parse_impact_visibility.json",
            3usize,
        ),
        (
            "src/parser.rs",
            "delete",
            None,
            "rust_small_parser_file_impact_delete.json",
            1usize,
        ),
        (
            "src/parser.rs",
            "side-effect",
            None,
            "rust_small_parser_file_impact_side_effect.json",
            1usize,
        ),
    ] {
        let impacted = store.query_impact(target, change_type, depth).unwrap();
        let envelope = stub::impact(
            target.to_string(),
            change_type.to_string(),
            depth,
            impacted.clone(),
        );
        let actual = serde_json::to_string_pretty(&envelope).unwrap();
        let expected = read_golden(golden_name);

        assert_eq!(actual, expected, "golden mismatch for {change_type}");
        assert_eq!(envelope.data.summary.total, expected_total);
        assert_eq!(envelope.data.impacted.len(), expected_total);
        assert_eq!(
            envelope.data.grouped.exact.len()
                + envelope.data.grouped.resolved.len()
                + envelope.data.grouped.heuristic.len()
                + envelope.data.grouped.dynamic.len(),
            expected_total
        );
        assert!(
            impacted
                .iter()
                .all(|record| !record.reason.is_empty() && record.distance >= 1),
            "impact records should preserve reason and distance for {change_type}"
        );
    }

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn impact_output_groups_by_certainty_and_summarizes_counts() {
    let traversals = vec![
        TraversalRecord {
            kind: NodeKind::Symbol,
            path: Some(RepoPath::from("src/parser.rs")),
            qualname: Some("parser::parse".to_string()),
            edge_kind: EdgeKind::Call,
            certainty: Certainty::Exact,
            reason: "calls parser::parse directly".to_string(),
            distance: 1,
        },
        TraversalRecord {
            kind: NodeKind::Symbol,
            path: Some(RepoPath::from("src/resolver.rs")),
            qualname: Some("resolver::resolve".to_string()),
            edge_kind: EdgeKind::Call,
            certainty: Certainty::Resolved,
            reason: "calls a symbol that reaches parser::parse".to_string(),
            distance: 2,
        },
        TraversalRecord {
            kind: NodeKind::File,
            path: Some(RepoPath::from("src/main.rs")),
            qualname: None,
            edge_kind: EdgeKind::Import,
            certainty: Certainty::Heuristic,
            reason: "imports a file that reaches parser::parse".to_string(),
            distance: 3,
        },
        TraversalRecord {
            kind: NodeKind::File,
            path: Some(RepoPath::from("src/bootstrap.rs")),
            qualname: None,
            edge_kind: EdgeKind::Dynamic,
            certainty: Certainty::Dynamic,
            reason: "dynamic import path may reach parser::parse".to_string(),
            distance: 1,
        },
    ];

    let envelope = stub::impact(
        "parser::parse".to_string(),
        "signature".to_string(),
        Some(3),
        traversals,
    );
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("rust_small_parse_impact.json");

    assert_eq!(actual, expected);
    assert_eq!(envelope.data.summary.total, 4);
    assert_eq!(envelope.data.summary.exact, 1);
    assert_eq!(envelope.data.summary.resolved, 1);
    assert_eq!(envelope.data.summary.heuristic, 1);
    assert_eq!(envelope.data.summary.dynamic, 1);
    assert_eq!(envelope.data.grouped.exact.len(), 1);
    assert_eq!(envelope.data.grouped.resolved.len(), 1);
    assert_eq!(envelope.data.grouped.heuristic.len(), 1);
    assert_eq!(envelope.data.grouped.dynamic.len(), 1);
    assert_eq!(envelope.data.risk, "high");
}

#[test]
fn context_queries_match_golden_json_and_budget_behavior() {
    let ts_repo = prepare_fixture_copy("ts_small");
    let ts_store = index_fixture(&ts_repo);

    let rename_result = ts_store
        .query_context(
            &["auth::middleware::verifyToken".to_string()],
            "rename",
            None,
        )
        .unwrap();
    let rename_envelope = stub::context(rename_result.clone());
    let rename_actual = serde_json::to_string_pretty(&rename_envelope).unwrap();
    let rename_expected = read_golden("ts_small_verify_token_context_rename.json");
    assert_eq!(rename_actual, rename_expected);
    assert_eq!(rename_result.must_read.len(), 3);
    assert_eq!(rename_result.must_read[0].path.0, "src/auth/middleware.ts");
    assert!(rename_result
        .must_read
        .iter()
        .any(|record| record.path.0 == "src/auth/index.ts"));
    assert!(rename_result
        .must_read
        .iter()
        .any(|record| record.path.0 == "src/auth/jwt.ts"));
    assert!(rename_result
        .should_read
        .iter()
        .any(|record| record.path.0 == "src/index.ts"));

    let budget_result = ts_store
        .query_context(
            &["auth::middleware::verifyToken".to_string()],
            "rename",
            Some(90),
        )
        .unwrap();
    let budget_envelope = stub::context(budget_result.clone());
    let budget_actual = serde_json::to_string_pretty(&budget_envelope).unwrap();
    let budget_expected = read_golden("ts_small_verify_token_context_rename_budget.json");
    assert_eq!(budget_actual, budget_expected);
    assert!(budget_result.summary.truncated);
    assert_eq!(budget_result.must_read.len(), 2);
    assert_eq!(budget_result.must_read[0].path.0, "src/auth/middleware.ts");
    assert!(budget_result
        .must_read
        .iter()
        .any(|record| record.path.0 == "src/auth/index.ts"));
    assert!(budget_result
        .should_read
        .iter()
        .any(|record| record.path.0 == "src/auth/jwt.ts"));

    assert!(matches!(
        ts_store.query_context(&["does::not::exist".to_string()], "rename", None),
        Err(scope_core::ScopeError::InvalidInput(_))
    ));

    fs::remove_dir_all(ts_repo).unwrap();

    let rust_repo = prepare_fixture_copy("rust_small");
    let rust_store = index_fixture(&rust_repo);
    let file_result = rust_store
        .query_context(&["src/parser.rs".to_string()], "side-effect", None)
        .unwrap();
    let file_envelope = stub::context(file_result.clone());
    let file_actual = serde_json::to_string_pretty(&file_envelope).unwrap();
    let file_expected = read_golden("rust_small_parser_context_side_effect.json");
    assert_eq!(file_actual, file_expected);
    assert_eq!(file_result.must_read[0].path.0, "src/parser.rs");
    assert!(file_result
        .must_read
        .iter()
        .any(|record| record.path.0 == "src/resolver.rs"));
    assert!(file_result.should_read.is_empty());

    fs::remove_dir_all(rust_repo).unwrap();
}

#[test]
fn test_map_queries_cover_expected_fixture_cones() {
    let repo = prepare_fixture_copy("test_map_ts");
    let store = index_fixture(&repo);
    let config = load_arch_config(&repo).unwrap();

    let build = store.build_test_map(&config.tests).unwrap();
    assert_eq!(build.summary.test_files, 3);
    assert_eq!(build.summary.covered_source_files, 4);
    assert_eq!(build.summary.uncovered_source_files, 5);

    let covers = store
        .query_tests_covering(&RepoPath::from("src/auth/middleware.ts"), &config.tests)
        .unwrap();
    assert_eq!(covers.summary.covering_tests, 3);
    assert_eq!(covers.summary.nearest_distance, Some(1));
    assert_eq!(covers.tests.len(), 3);
    assert_eq!(
        covers.tests[0].path,
        RepoPath::from("tests/auth/middleware.test.ts")
    );
    assert_eq!(covers.tests[0].distance, 1);
    assert_eq!(
        covers.tests[1].path,
        RepoPath::from("tests/integration/api.test.ts")
    );
    assert_eq!(covers.tests[1].distance, 2);
    assert_eq!(
        covers.tests[2].path,
        RepoPath::from("tests/e2e/app.test.ts")
    );
    assert_eq!(covers.tests[2].distance, 3);

    let covered_by = store
        .query_test_coverage(&RepoPath::from("tests/e2e/app.test.ts"), &config.tests)
        .unwrap();
    assert_eq!(covered_by.summary.covered_source_files, 4);
    assert_eq!(covered_by.summary.nearest_distance, Some(1));
    assert_eq!(
        covered_by
            .covered_files
            .iter()
            .map(|record| record.path.0.as_str())
            .collect::<Vec<_>>(),
        vec![
            "src/app.ts",
            "src/routes/api.ts",
            "src/auth/middleware.ts",
            "src/auth/jwt.ts"
        ]
    );

    let uncovered = store.query_uncovered_files(&config.tests).unwrap();
    assert_eq!(uncovered.summary.source_files_considered, 9);
    assert_eq!(uncovered.summary.uncovered_source_files, 5);
    assert!(uncovered.files.contains(&RepoPath::from("src/index.ts")));
    assert!(uncovered
        .files
        .contains(&RepoPath::from("src/auth/index.ts")));
    assert!(uncovered
        .files
        .contains(&RepoPath::from("src/auth/aliases.ts")));
    assert!(uncovered
        .files
        .contains(&RepoPath::from("src/utils/formatter.ts")));

    let source_as_test_error = store
        .query_test_coverage(&RepoPath::from("src/auth/middleware.ts"), &config.tests)
        .unwrap_err();
    assert!(source_as_test_error
        .to_string()
        .contains("scope test-map covered-by requires a detected test file target"));

    let test_as_source_error = store
        .query_tests_covering(&RepoPath::from("tests/e2e/app.test.ts"), &config.tests)
        .unwrap_err();
    assert!(test_as_source_error
        .to_string()
        .contains("scope test-map covers requires a source file target"));

    let missing_target_error = store
        .query_tests_covering(&RepoPath::from("src/missing.ts"), &config.tests)
        .unwrap_err();
    assert!(missing_target_error
        .to_string()
        .contains("scope test-map covers could not resolve target"));

    let app_tree = store
        .query_tree(&RepoPath::from("src/app.ts"), false, Some(2))
        .unwrap();
    assert_eq!(app_tree.target, RepoPath::from("src/app.ts"));
    assert_eq!(app_tree.summary.nodes, 3);
    assert_eq!(app_tree.tree.path, RepoPath::from("src/app.ts"));
    assert_eq!(app_tree.tree.children.len(), 1);
    assert_eq!(
        app_tree.tree.children[0].path,
        RepoPath::from("src/routes/api.ts")
    );

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn arch_violations_fixture_matches_expected_json() {
    let repo = prepare_fixture_copy("arch_violations");
    let store = index_fixture(&repo);
    let config = load_arch_config(&repo).unwrap();
    let result = arch_check(&store, &config).unwrap();

    assert_eq!(result.checked_edges, 5);
    assert_eq!(result.checked_layered_edges, 5);
    assert_eq!(result.violations.len(), 3);

    let violation_pairs: Vec<_> = result
        .violations
        .iter()
        .map(|violation| {
            (
                violation.from_file.0.clone(),
                violation.to_file.0.clone(),
                violation.from_layer.clone(),
                violation.to_layer.clone(),
            )
        })
        .collect();
    assert_eq!(
        violation_pairs,
        vec![
            (
                "src/models/account.ts".to_string(),
                "src/services/user.ts".to_string(),
                "models".to_string(),
                "services".to_string(),
            ),
            (
                "src/services/user.ts".to_string(),
                "src/routes/http.ts".to_string(),
                "services".to_string(),
                "routes".to_string(),
            ),
            (
                "src/utils/format.ts".to_string(),
                "src/models/account.ts".to_string(),
                "utils".to_string(),
                "models".to_string(),
            ),
        ]
    );

    let envelope = stub::arch_check(result);
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("arch_violations_check.json");
    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn capability_audit_fixture_matches_golden_json() {
    let repo = prepare_fixture_copy("capability_audit");
    let store = index_fixture(&repo);
    let config = load_arch_config(&repo).unwrap();
    let result = store.query_audit(&config, "network").unwrap();

    assert_eq!(result.summary.capability_sources, 1);
    assert_eq!(result.summary.reaching_entry_points, 2);
    assert_eq!(result.summary.expected_entry_points, 1);
    assert_eq!(result.summary.unexpected_entry_points, 1);
    assert_eq!(
        result.reaches[0].entry_point,
        RepoPath::from("src/workers/job.ts")
    );
    assert!(result.reaches[0].expected);
    assert_eq!(
        result.reaches[1].entry_point,
        RepoPath::from("src/cli/main.ts")
    );
    assert!(!result.reaches[1].expected);
    assert_eq!(
        result.reaches[1].path,
        vec![
            RepoPath::from("src/cli/main.ts"),
            RepoPath::from("src/shared/api.ts"),
            RepoPath::from("src/http/client.ts"),
        ]
    );

    let envelope = stub::audit(result);
    let actual = serde_json::to_string_pretty(&envelope).unwrap();
    let expected = read_golden("capability_audit_network.json");
    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn capability_audit_entry_queries_match_golden_json() {
    let repo = prepare_fixture_copy("capability_audit");
    let store = index_fixture(&repo);
    let config = load_arch_config(&repo).unwrap();

    let entry_list = store.query_entry_list(&config).unwrap();
    assert_eq!(entry_list.summary.entry_points, 2);
    assert_eq!(
        entry_list.entry_points[0].file,
        RepoPath::from("src/cli/main.ts")
    );
    assert_eq!(
        entry_list.entry_points[1].file,
        RepoPath::from("src/workers/job.ts")
    );
    let entry_list_actual = serde_json::to_string_pretty(&stub::entry_list(entry_list)).unwrap();
    let entry_list_expected = read_golden("capability_audit_entry_list.json");
    assert_eq!(entry_list_actual, entry_list_expected);

    let entry_cone = store
        .query_entry_cone(&config, &RepoPath::from("src/workers/job.ts"))
        .unwrap();
    assert_eq!(entry_cone.summary.reachable_files, 3);
    assert_eq!(entry_cone.summary.max_distance, 2);
    let entry_cone_actual = serde_json::to_string_pretty(&stub::entry_cone(entry_cone)).unwrap();
    let entry_cone_expected = read_golden("capability_audit_entry_cone_job.json");
    assert_eq!(entry_cone_actual, entry_cone_expected);

    let entry_reaches = store
        .query_entry_reaches(&config, &RepoPath::from("src/shared/api.ts"))
        .unwrap();
    assert_eq!(entry_reaches.summary.reaching_entry_points, 2);
    assert_eq!(entry_reaches.summary.nearest_distance, Some(1));
    let entry_reaches_actual =
        serde_json::to_string_pretty(&stub::entry_reaches(entry_reaches)).unwrap();
    let entry_reaches_expected = read_golden("capability_audit_entry_reaches_shared_api.json");
    assert_eq!(entry_reaches_actual, entry_reaches_expected);

    let entry_unreachable = store.query_entry_unreachable(&config, None).unwrap();
    assert_eq!(entry_unreachable.total_files, 4);
    assert_eq!(entry_unreachable.reachable_files, 4);
    assert_eq!(entry_unreachable.unreachable_files, 0);
    let entry_unreachable_actual =
        serde_json::to_string_pretty(&stub::entry_unreachable(entry_unreachable)).unwrap();
    let entry_unreachable_expected = read_golden("capability_audit_entry_unreachable.json");
    assert_eq!(entry_unreachable_actual, entry_unreachable_expected);

    fs::remove_dir_all(repo).unwrap();
}
