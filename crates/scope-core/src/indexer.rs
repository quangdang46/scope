use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    time::UNIX_EPOCH,
};

use crate::{
    adapter_for_language, adapters, scan_repo, ExtractResult, RepoPath, ScanConfig, ScanEntry,
    ScopeError, ScopeResult, Store,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRunStats {
    pub indexed_files: usize,
    pub changed_files: usize,
    pub deleted_files: usize,
    pub affected_files: usize,
}

#[derive(Debug, Clone)]
struct ScannedSourceFile {
    entry: ScanEntry,
    mtime_unix_seconds: Option<i64>,
    size_bytes: Option<i64>,
}

pub fn index_repo(repo_root: &Path, store: &Store) -> ScopeResult<IndexRunStats> {
    let scanned_files = scan_indexable_source_files(repo_root)?;
    let scanned_paths: HashSet<_> = scanned_files.keys().cloned().collect();
    let stored_states = store.list_file_states()?;

    if stored_states.is_empty() {
        let mut extracts = build_extracts(scanned_files.values().collect())?;
        extracts.sort_by(|left, right| left.file.path.cmp(&right.file.path));
        let indexed_files = extracts.len();
        store.persist_extract_results(&extracts)?;
        return Ok(IndexRunStats {
            indexed_files,
            changed_files: indexed_files,
            deleted_files: 0,
            affected_files: indexed_files,
        });
    }

    let mut candidates = Vec::new();
    for scanned in scanned_files.values() {
        let previous = stored_states.get(&scanned.entry.path);
        if metadata_matches(previous, scanned) {
            continue;
        }
        candidates.push(scanned);
    }

    let mut changed_or_new = Vec::new();
    let mut extract_map: HashMap<RepoPath, ExtractResult> = HashMap::new();
    for extract in build_extracts(candidates)? {
        if content_hash_matches(stored_states.get(&extract.file.path), &extract.file) {
            // Refresh stored metadata without reprocessing graph edges when contents match.
            store.upsert_file(&extract.file)?;
            continue;
        }

        changed_or_new.push(extract.file.path.clone());
        extract_map.insert(extract.file.path.clone(), extract);
    }

    let deleted_paths: Vec<_> = stored_states
        .keys()
        .filter(|path| !scanned_paths.contains(path))
        .cloned()
        .collect();

    let mut affected_paths: HashSet<_> = changed_or_new.iter().cloned().collect();
    let mut closure_seeds = changed_or_new;
    closure_seeds.extend(deleted_paths.iter().cloned());

    for dependent in store.reverse_dependency_closure(&closure_seeds)? {
        affected_paths.insert(dependent);
    }

    for path in &deleted_paths {
        let _ = store.delete_file(path)?;
    }

    let mut pending_extracts = Vec::new();
    for path in &affected_paths {
        if extract_map.contains_key(path) {
            continue;
        }
        if let Some(scanned) = scanned_files.get(path) {
            pending_extracts.push(scanned);
        }
    }
    for extract in build_extracts(pending_extracts)? {
        extract_map.insert(extract.file.path.clone(), extract);
    }

    let mut affected_extracts: Vec<_> = affected_paths
        .into_iter()
        .filter_map(|path| extract_map.remove(&path))
        .collect();
    affected_extracts.sort_by(|left, right| left.file.path.cmp(&right.file.path));
    store.persist_extract_results(&affected_extracts)?;

    Ok(IndexRunStats {
        indexed_files: scanned_files.len(),
        changed_files: closure_seeds.len().saturating_sub(deleted_paths.len()),
        deleted_files: deleted_paths.len(),
        affected_files: affected_extracts.len(),
    })
}

fn build_extracts(scanned: Vec<&ScannedSourceFile>) -> ScopeResult<Vec<ExtractResult>> {
    scanned.into_iter().map(build_extract).collect()
}

fn build_extract(scanned: &ScannedSourceFile) -> ScopeResult<ExtractResult> {
    let adapter = adapter_for_language(scanned.entry.language).ok_or_else(|| {
        ScopeError::UnsupportedLanguage {
            path: scanned.entry.absolute_path.display().to_string(),
        }
    })?;
    let source = fs::read_to_string(&scanned.entry.absolute_path)
        .map_err(|error| ScopeError::io(&scanned.entry.absolute_path, error))?;
    let mut extract = adapter.extract(&scanned.entry, &source);
    extract.file.content_hash = Some(blake3::hash(source.as_bytes()).to_hex().to_string());
    extract.file.mtime_unix_seconds = scanned.mtime_unix_seconds;
    extract.file.size_bytes = scanned.size_bytes;
    Ok(extract)
}

fn scan_indexable_source_files(root: &Path) -> ScopeResult<HashMap<RepoPath, ScannedSourceFile>> {
    let entries = scan_repo(root, &ScanConfig::default())?;
    let mut scanned = HashMap::new();

    for entry in entries {
        let Some(adapter) = adapter_for_language(entry.language) else {
            continue;
        };
        if !adapters::supports_path(adapter, &entry.absolute_path) {
            continue;
        }

        let metadata = fs::metadata(&entry.absolute_path).ok();
        let mtime_unix_seconds = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as i64);
        let size_bytes = metadata.as_ref().map(|metadata| metadata.len() as i64);

        scanned.insert(
            entry.path.clone(),
            ScannedSourceFile {
                entry,
                mtime_unix_seconds,
                size_bytes,
            },
        );
    }

    Ok(scanned)
}
fn metadata_matches(
    previous: Option<&crate::store::StoredFileState>,
    scanned: &ScannedSourceFile,
) -> bool {
    let Some(previous) = previous else {
        return false;
    };

    previous.mtime_unix_seconds.is_some()
        && previous.size_bytes.is_some()
        && previous.mtime_unix_seconds == scanned.mtime_unix_seconds
        && previous.size_bytes == scanned.size_bytes
}

fn content_hash_matches(
    previous: Option<&crate::store::StoredFileState>,
    current: &crate::FileRecord,
) -> bool {
    previous.and_then(|state| state.content_hash.as_ref()) == current.content_hash.as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root should resolve")
    }

    fn fixture_root(name: &str) -> PathBuf {
        repo_root().join("fixtures").join(name)
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("scope-indexer-{prefix}-{nanos}-{sequence}"))
    }

    fn copy_dir_recursive(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir_recursive(&src_path, &dst_path);
            } else if src_path
                .strip_prefix(src.parent().unwrap_or(src))
                .ok()
                .and_then(|relative| relative.to_str())
                == Some(".scope/index.db")
            {
                continue;
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

    #[test]
    fn incremental_index_only_rebuilds_changed_files_and_dependents() {
        let repo = prepare_fixture_copy("rust_small");
        let store = Store::open(&repo.join(".scope/index.db")).unwrap();

        let initial = index_repo(&repo, &store).unwrap();
        assert_eq!(initial.indexed_files, 5);
        assert_eq!(initial.affected_files, 5);

        let parser = repo.join("src/parser.rs");
        let mut source = fs::read_to_string(&parser).unwrap();
        source.push_str("\n// incremental index mutation\n");
        fs::write(&parser, source).unwrap();

        let incremental = index_repo(&repo, &store).unwrap();
        assert_eq!(incremental.changed_files, 1);
        assert_eq!(incremental.deleted_files, 0);
        assert_eq!(incremental.affected_files, 3);

        let _ = fs::remove_dir_all(repo);
    }
}
