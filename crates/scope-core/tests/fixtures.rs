use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use scope_core::{
    adapter_for_language, arch_check, load_arch_config, scan_repo, stub, Certainty, EdgeKind,
    NodeKind, PublicSurfaceChangeKind, RepoPath, ScanConfig, Store, SupportedLanguage, SymbolKind,
    TraversalRecord,
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

    let auth_deps = store.query_deps(&RepoPath::from("src/auth/index.ts")).unwrap();
    let auth_dep_paths: Vec<_> = auth_deps.iter().map(|dep| dep.path.0.clone()).collect();
    assert_eq!(auth_dep_paths, vec!["src/auth/aliases.ts", "src/auth/middleware.ts"]);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn ts_small_reverse_deps_and_symbols_and_calls_work_conservatively() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let reverse = store.query_reverse_deps(&RepoPath::from("src/auth/jwt.ts")).unwrap();
    let reverse_paths: Vec<_> = reverse.iter().map(|dep| dep.path.0.clone()).collect();
    assert_eq!(reverse_paths, vec!["src/auth/aliases.ts", "src/auth/middleware.ts"]);

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

    let alias_symbols = store
        .query_symbols(&RepoPath::from("src/auth/aliases.ts"), false, None)
        .unwrap();
    let alias_symbol_names: Vec<_> = alias_symbols.iter().map(|symbol| symbol.name.clone()).collect();
    assert_eq!(alias_symbol_names, vec!["verifyJwt"]);

    let auth_index_symbols = store
        .query_symbols(&RepoPath::from("src/auth/index.ts"), true, None)
        .unwrap();
    let auth_index_symbol_names: Vec<_> = auth_index_symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert_eq!(auth_index_symbol_names, vec!["verifyToken", "verifyJwt"]);

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
    assert_eq!(parser_calls[0].qualname.as_deref(), Some("parser::tokenize"));
    assert_eq!(parser_calls[0].certainty, scope_core::Certainty::Exact);

    let parser_calls_transitive = store.query_callees("parser::parse", true).unwrap();
    let parser_calls_transitive_envelope =
        stub::calls("parser::parse".to_string(), true, parser_calls_transitive.clone());
    let parser_calls_transitive_actual =
        serde_json::to_string_pretty(&parser_calls_transitive_envelope).unwrap();
    let parser_calls_transitive_expected = read_golden("rust_small_parse_calls_transitive_stub.json");
    assert_eq!(parser_calls_transitive_actual, parser_calls_transitive_expected);
    assert!(parser_calls_transitive.is_empty());

    let farewell_calls = store.query_callees("lib::farewell", false).unwrap();
    let farewell_envelope = stub::calls("lib::farewell".to_string(), false, farewell_calls.clone());
    let farewell_actual = serde_json::to_string_pretty(&farewell_envelope).unwrap();
    let farewell_expected = read_golden("rust_small_farewell_calls.json");
    assert_eq!(farewell_actual, farewell_expected);
    assert!(farewell_calls.is_empty(), "farewell should conservatively omit dynamic formatting internals");

    let parser_callers = store.query_callers("parser::parse", false).unwrap();
    let callers_envelope = stub::callers("parser::parse".to_string(), false, parser_callers.clone());
    let callers_actual = serde_json::to_string_pretty(&callers_envelope).unwrap();
    let callers_expected = read_golden("rust_small_parse_callers.json");
    assert_eq!(callers_actual, callers_expected);
    let parser_caller_names: Vec<_> = parser_callers
        .iter()
        .filter_map(|traversal| traversal.qualname.clone())
        .collect();
    assert_eq!(parser_caller_names, vec!["lib::greet".to_string(), "resolver::resolve".to_string()]);

    let parser_callers_transitive = store.query_callers("parser::parse", true).unwrap();
    let parser_callers_transitive_envelope =
        stub::callers("parser::parse".to_string(), true, parser_callers_transitive.clone());
    let parser_callers_transitive_actual =
        serde_json::to_string_pretty(&parser_callers_transitive_envelope).unwrap();
    let parser_callers_transitive_expected =
        read_golden("rust_small_parse_callers_transitive_stub.json");
    assert_eq!(parser_callers_transitive_actual, parser_callers_transitive_expected);
    assert!(parser_callers_transitive.is_empty());

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
    assert!(traversals.iter().any(|record| record.qualname.as_deref() == Some("lib::greet")));
    assert!(traversals.iter().any(|record| record.qualname.as_deref() == Some("parser::tokenize")));
    assert!(traversals.iter().any(|record| record.path.as_ref().map(|path| path.0.as_str()) == Some("src/resolver.rs")));

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

    let surface = store.query_public_surface(&RepoPath::from("src/parser.rs")).unwrap();
    let names: Vec<_> = surface.symbols.iter().map(|symbol| symbol.name.as_str()).collect();

    assert_eq!(surface.file, RepoPath::from("src/parser.rs"));
    assert_eq!(names, vec!["parse"]);
    assert!(surface.symbols.iter().all(|symbol| symbol.qualname.starts_with("parser::")));

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
            && change.after.as_ref().is_some_and(|symbol| symbol.name == "verifyJwt")));
    assert!(diff
        .changes
        .iter()
        .filter(|change| change.kind == PublicSurfaceChangeKind::Removed)
        .any(|change| change.before.as_ref().is_some_and(|symbol| symbol.name == "sign")));
    assert!(diff
        .changes
        .iter()
        .filter(|change| change.kind == PublicSurfaceChangeKind::Removed)
        .any(|change| change.before.as_ref().is_some_and(|symbol| symbol.name == "verify")));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn fixture_indexing_persists_file_fingerprint_metadata() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let state = store
        .file_state(&RepoPath::from("src/lib.rs"))
        .unwrap()
        .unwrap();
    assert!(state.content_hash.as_ref().is_some_and(|hash| !hash.is_empty()));
    assert!(state.mtime_unix_seconds.is_some());
    assert!(state.size_bytes.is_some_and(|size| size > 0));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn why_queries_match_golden_json_and_handle_limits() {
    let rust_repo = prepare_fixture_copy("rust_small");
    let rust_store = index_fixture(&rust_repo);

    let symbol_path = rust_store.query_why("lib::greet", "parser::tokenize", None).unwrap();
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

    let same_target = rust_store.query_why("lib::greet", "lib::greet", None).unwrap();
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
    assert_eq!(file_path[0].path.as_ref().map(|path| path.0.as_str()), Some("src/auth/index.ts"));
    assert_eq!(file_path[1].path.as_ref().map(|path| path.0.as_str()), Some("src/auth/middleware.ts"));

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
        .query_context(&["auth::middleware::verifyToken".to_string()], "rename", None)
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
        .query_context(&["auth::middleware::verifyToken".to_string()], "rename", Some(90))
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
