use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use scope_core::{
    adapter_for_language, scan_repo, stub, ArchConfig, GateConfig, GateMetric, GateSeverity,
    ScanConfig, Store,
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

static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let salt = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("scope-gate-{prefix}-{nanos}-{salt}"))
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

fn read_golden_json(name: &str) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(golden_root().join(name))
            .unwrap()
            .replace("\r\n", "\n"),
    )
    .unwrap()
}

fn normalize_report_like_json(value: &mut serde_json::Value) {
    if let Some(generated_at) = value.pointer_mut("/data/result/generated_at") {
        *generated_at = serde_json::Value::from(0);
    }
    if let Some(generated_at) = value.pointer_mut("/data/result/report/generated_at") {
        *generated_at = serde_json::Value::from(0);
    }
    if let Some(items) = value
        .pointer_mut("/data/result/report/unreachable_detail")
        .and_then(|node| node.as_array_mut())
    {
        for item in items {
            if let Some(days) = item.get_mut("last_modified_days_ago") {
                *days = serde_json::Value::from(0);
            }
        }
    }
}

#[test]
fn gate_query_warning_matches_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let config = ArchConfig {
        gates: vec![GateConfig {
            metric: GateMetric::HealthScoreDelta,
            min: None,
            max: None,
            min_delta: Some(-1.0),
            max_delta: None,
            severity: GateSeverity::Warning,
            message: Some("health score should not regress much".to_string()),
            skip: false,
        }],
        ..ArchConfig::default()
    };

    let gate = store.query_gate(&config, None, false).unwrap();
    let mut actual = serde_json::to_value(stub::gate(gate)).unwrap();
    let mut expected = read_golden_json("rust_small_gate_warning.json");
    normalize_report_like_json(&mut actual);
    normalize_report_like_json(&mut expected);

    assert_eq!(actual, expected);

    drop(store);
    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn gate_query_skipped_matches_golden_json() {
    let repo = prepare_fixture_copy("rust_small");
    let store = index_fixture(&repo);
    let config = ArchConfig {
        gates: vec![GateConfig {
            metric: GateMetric::Cycles,
            min: None,
            max: None,
            min_delta: None,
            max_delta: None,
            severity: GateSeverity::Warning,
            message: Some("cycles temporarily ignored".to_string()),
            skip: true,
        }],
        ..ArchConfig::default()
    };

    let gate = store.query_gate(&config, None, false).unwrap();
    let mut actual = serde_json::to_value(stub::gate(gate)).unwrap();
    let mut expected = read_golden_json("rust_small_gate_skipped.json");
    normalize_report_like_json(&mut actual);
    normalize_report_like_json(&mut expected);

    assert_eq!(actual, expected);

    drop(store);
    fs::remove_dir_all(repo).unwrap();
}

#[test]
fn gate_query_compare_warning_matches_golden_json() {
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

    let config = ArchConfig {
        gates: vec![GateConfig {
            metric: GateMetric::HealthScoreDelta,
            min: None,
            max: None,
            min_delta: Some(-1.0),
            max_delta: None,
            severity: GateSeverity::Warning,
            message: Some("health score should not regress much".to_string()),
            skip: false,
        }],
        ..ArchConfig::default()
    };

    let gate = store.query_gate(&config, Some("baseline"), false).unwrap();
    let mut actual = serde_json::to_value(stub::gate(gate)).unwrap();
    let mut expected = read_golden_json("rust_small_gate_compare_baseline_warning.json");
    normalize_report_like_json(&mut actual);
    normalize_report_like_json(&mut expected);

    assert_eq!(actual, expected);

    drop(store);
    fs::remove_dir_all(repo).unwrap();
}
