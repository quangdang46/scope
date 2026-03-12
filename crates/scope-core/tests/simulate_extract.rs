use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use scope_core::{adapter_for_language, scan_repo, stub, ArchConfig, RepoPath, ScanConfig, Store};

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
    std::env::temp_dir().join(format!("scope-simulate-{prefix}-{nanos}"))
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
fn simulate_extract_reports_expected_graph_delta_for_rust_fixture() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let result = store
        .simulate_extract(
            &["lib::parser".to_string()],
            &RepoPath::from("src/parser_extracted.rs"),
            &ArchConfig::default(),
        )
        .unwrap();

    assert_eq!(result.extraction.symbols, vec!["lib::parser"]);
    assert_eq!(result.extraction.from_file, RepoPath::from("src/lib.rs"));
    assert_eq!(
        result.extraction.into_file,
        RepoPath::from("src/parser_extracted.rs")
    );
    assert_eq!(result.graph_delta.edges_added, 1);
    assert_eq!(result.graph_delta.edges_removed, 0);
    assert_eq!(result.graph_delta.cycles_introduced, 0);
    assert_eq!(result.graph_delta.new_layer_violations, 0);
    assert_eq!(result.graph_delta.new_edges.len(), 1);
    assert_eq!(result.graph_delta.new_edges[0].from, "src/lib.rs");
    assert_eq!(result.graph_delta.new_edges[0].to, "src/parser_extracted.rs");
    assert_eq!(result.recommendation_reasons.len(), 1);
    assert!(result.warnings.is_empty());

    let actual = serde_json::to_string_pretty(&stub::simulate_extract(result)).unwrap();
    let expected = read_golden("rust_small_simulate_extract.json");
    assert_eq!(actual, expected);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn simulate_extract_rejects_symbols_from_multiple_files() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let error = store
        .simulate_extract(
            &["lib::parser".to_string(), "resolver::resolve".to_string()],
            &RepoPath::from("src/extracted.rs"),
            &ArchConfig::default(),
        )
        .unwrap_err();

    assert!(matches!(error, scope_core::ScopeError::InvalidInput(_)));
    assert!(error
        .to_string()
        .contains("requires all symbols to come from the same indexed file"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn simulate_extract_rejects_existing_target_file() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let error = store
        .simulate_extract(
            &["lib::parser".to_string()],
            &RepoPath::from("src/parser.rs"),
            &ArchConfig::default(),
        )
        .unwrap_err();

    assert!(matches!(error, scope_core::ScopeError::InvalidInput(_)));
    assert!(error
        .to_string()
        .contains("target file `src/parser.rs` already exists in the index"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn simulate_extract_rejects_duplicate_symbols() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let error = store
        .simulate_extract(
            &["lib::parser".to_string(), "lib::parser".to_string()],
            &RepoPath::from("src/parser_extracted.rs"),
            &ArchConfig::default(),
        )
        .unwrap_err();

    assert!(matches!(error, scope_core::ScopeError::InvalidInput(_)));
    assert!(error
        .to_string()
        .contains("received duplicate symbol `lib::parser`"));

    let _ = fs::remove_dir_all(repo);
}

#[test]
fn simulate_extract_rejects_empty_symbol_names() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let error = store
        .simulate_extract(
            &["   ".to_string()],
            &RepoPath::from("src/parser_extracted.rs"),
            &ArchConfig::default(),
        )
        .unwrap_err();

    assert!(matches!(error, scope_core::ScopeError::InvalidInput(_)));
    assert!(error
        .to_string()
        .contains("does not allow empty symbol names"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn simulate_extract_rejects_unresolved_symbols() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let error = store
        .simulate_extract(
            &["lib::missing_symbol".to_string()],
            &RepoPath::from("src/parser_extracted.rs"),
            &ArchConfig::default(),
        )
        .unwrap_err();

    assert!(matches!(error, scope_core::ScopeError::InvalidInput(_)));
    assert!(error
        .to_string()
        .contains("could not resolve symbol `lib::missing_symbol`"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn simulate_extract_rejects_target_file_matching_source_file() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let error = store
        .simulate_extract(
            &["lib::parser".to_string()],
            &RepoPath::from("src/lib.rs"),
            &ArchConfig::default(),
        )
        .unwrap_err();

    assert!(matches!(error, scope_core::ScopeError::InvalidInput(_)));
    assert!(error
        .to_string()
        .contains("target file must differ from the source file"));

    fs::remove_dir_all(repo).unwrap();
}
