use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use scope_core::{adapter_for_language, scan_repo, stub, ScanConfig, Store};

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
    std::env::temp_dir().join(format!("scope-rename-plan-{prefix}-{nanos}"))
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
fn symbol_rename_plan_dry_run_matches_golden_json() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let plan = store
        .build_rename_plan(
            &repo,
            "auth::middleware::verifyToken",
            "verifySession",
            false,
            false,
        )
        .unwrap();

    let actual = serde_json::to_string_pretty(&stub::rename_plan(plan.clone())).unwrap();
    let expected = read_golden("ts_small_verify_token_rename_plan.json");

    assert_eq!(actual, expected);
    assert_eq!(plan.steps.len(), 2);
    assert!(plan.skipped.is_empty());
    assert!(!plan.applied);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn symbol_rename_plan_apply_matches_golden_json_and_updates_fixture() {
    let repo = prepare_fixture_copy("ts_small");
    let store = index_fixture(&repo);

    let plan = store
        .build_rename_plan(
            &repo,
            "auth::middleware::verifyToken",
            "verifySession",
            true,
            true,
        )
        .unwrap();

    let actual = serde_json::to_string_pretty(&stub::rename_plan(plan.clone())).unwrap();
    let expected = read_golden("ts_small_verify_token_rename_plan_apply.json");

    assert_eq!(actual, expected);
    assert!(plan.applied);
    assert!(plan.skipped.is_empty());
    assert!(plan.warnings.is_empty());

    let root_index = fs::read_to_string(repo.join("src/index.ts")).unwrap();
    assert!(root_index.contains("export { verifyToken } from \"./auth/index\";"));

    let auth_index = fs::read_to_string(repo.join("src/auth/index.ts")).unwrap();
    assert!(auth_index.contains("export { verifySession } from \"./middleware\";"));
    assert!(!auth_index.contains("export { verifyToken } from \"./middleware\";"));

    let middleware = fs::read_to_string(repo.join("src/auth/middleware.ts")).unwrap();
    assert!(middleware.contains("export function verifySession(token: string): boolean"));
    assert!(!middleware.contains("export function verifyToken(token: string): boolean"));

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn file_rename_plan_dry_run_matches_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let plan = store
        .build_rename_plan(&repo, "src/parser.rs", "parser2", false, false)
        .unwrap();

    let actual = serde_json::to_string_pretty(&stub::rename_plan(plan.clone())).unwrap();
    let expected = read_golden("rust_small_parser_rename_plan.json");

    assert_eq!(actual, expected);
    assert_eq!(plan.steps.len(), 1);
    assert!(plan.skipped.is_empty());
    assert!(!plan.applied);
    assert_eq!(plan.summary.files_considered, 2);

    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn file_rename_plan_apply_matches_golden_json_and_rewrites_import_paths_only() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);

    let plan = store
        .build_rename_plan(&repo, "src/parser.rs", "parser2", true, false)
        .unwrap();

    let actual = serde_json::to_string_pretty(&stub::rename_plan(plan.clone())).unwrap();
    let expected = read_golden("rust_small_parser_rename_plan_apply.json");

    assert_eq!(actual, expected);
    assert!(plan.applied);
    assert!(plan
        .warnings
        .iter()
        .any(|warning| warning.contains("does not move files yet")));

    let resolver = fs::read_to_string(repo.join("src/resolver.rs")).unwrap();
    assert!(resolver.contains("use crate::parser2;"));
    assert!(!resolver.contains("use crate::parser;"));
    assert!(repo.join("src/parser.rs").is_file());
    assert!(!repo.join("src/parser2.rs").exists());

    fs::remove_dir_all(repo).unwrap();
}
