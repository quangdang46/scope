use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::{
    arch_check_edges, snapshot, ArchConfig, ArchFileEdge, Certainty, ContextFileRecord,
    ContextFileRole, ContextResult, ContextSummary, DependencyRecord, EdgeKind, ExtractResult,
    FileRecord, ImportPath, NodeKind, PublicSurface, PublicSurfaceChange,
    PublicSurfaceChangeKind, PublicSurfaceDiff, PublicSurfaceDiffSummary, PublicSurfaceSymbol,
    RenameEdit, RenameEditKind, RenamePlan, RenamePlanStep, RenamePlanSummary, RepoPath,
    RiskRecord, RiskResult, RiskSort, RiskSummary, ScopeError, ScopeResult,
    SnapshotCentralityDelta, SnapshotDeleteResult, SnapshotDiffResult, SnapshotEdgeDelta,
    SnapshotEdgeRecord, SnapshotFileRecord, SnapshotGraph, SnapshotListResult,
    SnapshotListSummary, SnapshotMetadata, SnapshotSaveResult, SnapshotStabilityDelta, SnapshotStoredRecord,
    SnapshotSymbolRecord, StabilityCategory, StabilityRecord, StabilityResult, StabilitySort,
    StabilitySummary, SymbolKind, SymbolRecord, TestConfig, TestMapBuildResult,
    TestMapBuildSummary, TestMapCoveredByResult, TestMapCoveredBySummary,
    TestMapCoversResult, TestMapCoversSummary, TestMapRecord, TestMapUncoveredResult,
    TestMapUncoveredSummary, TraversalRecord, Visibility,
};

const DEFAULT_TRANSITIVE_DEPTH: u32 = 8;

pub const INDEX_SCHEMA_VERSION: u32 = 6;

const INITIAL_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS index_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    language TEXT NOT NULL,
    parse_status TEXT NOT NULL,
    is_barrel INTEGER NOT NULL DEFAULT 0,
    indexed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS imports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    raw_text TEXT NOT NULL,
    resolved_file_id INTEGER,
    import_path_kind TEXT NOT NULL,
    external_pkg TEXT,
    span_start INTEGER,
    span_end INTEGER,
    start_line INTEGER,
    certainty TEXT NOT NULL,
    FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE,
    FOREIGN KEY(resolved_file_id) REFERENCES files(id) ON DELETE SET NULL
);

CREATE TABLE IF NOT EXISTS file_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_file_id INTEGER NOT NULL,
    to_file_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    certainty TEXT NOT NULL,
    FOREIGN KEY(from_file_id) REFERENCES files(id) ON DELETE CASCADE,
    FOREIGN KEY(to_file_id) REFERENCES files(id) ON DELETE CASCADE,
    UNIQUE(from_file_id, to_file_id, kind)
);

"#;

const SYMBOLS_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    qualname TEXT NOT NULL,
    kind TEXT NOT NULL,
    visibility TEXT NOT NULL,
    exported INTEGER NOT NULL,
    span_start INTEGER NOT NULL,
    span_end INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE,
    UNIQUE(file_id, qualname)
);
"#;

const SYMBOL_EDGES_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS symbol_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_symbol_id INTEGER NOT NULL,
    to_symbol_id INTEGER NOT NULL,
    kind TEXT NOT NULL,
    certainty TEXT NOT NULL,
    call_line INTEGER NOT NULL,
    FOREIGN KEY(from_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
    FOREIGN KEY(to_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
    UNIQUE(from_symbol_id, to_symbol_id, kind, call_line)
);
"#;

const FILE_METADATA_MIGRATION_COLUMNS: &[(&str, &str)] = &[
    (
        "content_hash",
        "ALTER TABLE files ADD COLUMN content_hash TEXT",
    ),
    (
        "mtime_unix_seconds",
        "ALTER TABLE files ADD COLUMN mtime_unix_seconds INTEGER",
    ),
    (
        "size_bytes",
        "ALTER TABLE files ADD COLUMN size_bytes INTEGER",
    ),
];

const FILE_CHURN_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS file_churn (
    file_id INTEGER NOT NULL,
    commit_sha TEXT NOT NULL,
    author_email TEXT,
    committed_at INTEGER,
    PRIMARY KEY (file_id, commit_sha),
    FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_file_churn_file_id ON file_churn(file_id);
CREATE INDEX IF NOT EXISTS idx_file_churn_committed_at ON file_churn(committed_at);
"#;

const SNAPSHOTS_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS snapshots (
    name TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    commit_sha TEXT,
    schema_version INTEGER NOT NULL,
    snapshot_version INTEGER NOT NULL,
    payload BLOB NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_snapshots_created_at ON snapshots(created_at);
"#;

#[derive(Debug)]
pub struct Store {
    connection: Connection,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseInfo {
    pub path: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFileState {
    pub path: RepoPath,
    pub content_hash: Option<String>,
    pub mtime_unix_seconds: Option<i64>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ParseStatusCounts {
    pub ok: usize,
    pub partial: usize,
    pub error: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IndexHealthStats {
    pub files: usize,
    pub imports: usize,
    pub unresolved_imports: usize,
    pub symbols: usize,
    pub call_edges: usize,
    pub parse_status: ParseStatusCounts,
}

#[derive(Debug, Clone)]
struct ContextCandidate {
    path: RepoPath,
    score: u32,
    estimated_tokens: usize,
    distance: u32,
    certainty: Certainty,
    reasons: Vec<String>,
    roles: Vec<ContextFileRole>,
    pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameTarget {
    Symbol { qualname: String, symbol: SymbolRecord },
    File { path: RepoPath },
}

impl Store {
    pub fn open(db_path: &Path) -> ScopeResult<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ScopeError::database(db_path, error))?;
        }

        let connection =
            Connection::open(db_path).map_err(|error| ScopeError::database(db_path, error))?;
        configure_connection(&connection, db_path)?;
        run_migrations(&connection)?;
        bootstrap_meta(&connection)?;

        Ok(Self { connection })
    }

    pub fn schema_version(&self) -> ScopeResult<u32> {
        current_user_version(&self.connection)
    }

    pub fn upsert_file(&self, file: &FileRecord) -> ScopeResult<i64> {
        let indexed_at = unix_timestamp();
        self.connection.execute(
            "INSERT INTO files (
                path,
                language,
                parse_status,
                is_barrel,
                indexed_at,
                content_hash,
                mtime_unix_seconds,
                size_bytes
            )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET
                 language = excluded.language,
                 parse_status = excluded.parse_status,
                 is_barrel = excluded.is_barrel,
                 indexed_at = excluded.indexed_at,
                 content_hash = excluded.content_hash,
                 mtime_unix_seconds = excluded.mtime_unix_seconds,
                 size_bytes = excluded.size_bytes",
            params![
                file.path.0,
                file.language,
                parse_status_name(&file.parse_status),
                file.is_barrel as i64,
                indexed_at,
                file.content_hash,
                file.mtime_unix_seconds,
                file.size_bytes,
            ],
        )?;

        self.file_id(&file.path)?.ok_or_else(|| {
            ScopeError::Internal(format!("missing file row after upsert: {}", file.path.0))
        })
    }

    pub fn persist_extract_results(&self, results: &[ExtractResult]) -> ScopeResult<()> {
        for result in results {
            self.upsert_file(&result.file)?;
        }
        for result in results {
            self.persist_extract_result(result)?;
        }
        for result in results {
            self.refresh_call_edges(result)?;
        }
        Ok(())
    }

    pub fn file_state(&self, path: &RepoPath) -> ScopeResult<Option<StoredFileState>> {
        self.connection
            .query_row(
                "SELECT path, content_hash, mtime_unix_seconds, size_bytes FROM files WHERE path = ?1",
                [path.0.as_str()],
                |row| {
                    Ok(StoredFileState {
                        path: RepoPath(row.get::<_, String>(0)?),
                        content_hash: row.get(1)?,
                        mtime_unix_seconds: row.get(2)?,
                        size_bytes: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn classify_file_change(&self, current: &FileRecord) -> ScopeResult<Option<bool>> {
        let Some(previous) = self.file_state(&current.path)? else {
            return Ok(None);
        };

        Ok(Some(previous.content_hash != current.content_hash))
    }

    pub fn index_health_stats(&self) -> ScopeResult<IndexHealthStats> {
        let files = self.count_rows("files")?;
        let imports = self.count_rows("imports")?;
        let unresolved_imports = self.count_query(
            "SELECT COUNT(*) FROM imports WHERE resolved_file_id IS NULL AND import_path_kind != 'external'",
        )?;
        let symbols = self.count_rows("symbols")?;
        let call_edges = self.count_rows("symbol_edges")?;
        let parse_status = ParseStatusCounts {
            ok: self.count_query("SELECT COUNT(*) FROM files WHERE parse_status = 'ok'")?,
            partial: self
                .count_query("SELECT COUNT(*) FROM files WHERE parse_status = 'partial'")?,
            error: self.count_query("SELECT COUNT(*) FROM files WHERE parse_status = 'error'")?,
        };

        Ok(IndexHealthStats {
            files,
            imports,
            unresolved_imports,
            symbols,
            call_edges,
            parse_status,
        })
    }

    pub fn persist_extract_result(&self, result: &ExtractResult) -> ScopeResult<()> {
        let file_id = self.upsert_file(&result.file)?;
        self.delete_symbol_edges_for_file(file_id)?;
        self.connection
            .execute("DELETE FROM imports WHERE file_id = ?1", [file_id])?;
        self.connection
            .execute("DELETE FROM file_edges WHERE from_file_id = ?1", [file_id])?;
        self.connection
            .execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;

        for import in &result.imports {
            self.insert_import(file_id, import)?;
        }

        for symbol in &result.symbols {
            self.insert_symbol(file_id, symbol)?;
        }

        for module in &result.modules {
            if let Some(path) = &module.declared_path {
                if let Some(target_file_id) = self.file_id(path)? {
                    self.insert_file_edge(
                        file_id,
                        target_file_id,
                        "module",
                        certainty_name(&module.certainty),
                    )?;
                }
            }
        }

        self.insert_resolved_call_edges(file_id, result)?;

        Ok(())
    }

    pub fn refresh_call_edges(&self, result: &ExtractResult) -> ScopeResult<()> {
        let Some(file_id) = self.file_id(&result.file.path)? else {
            return Ok(());
        };
        self.delete_symbol_edges_for_file(file_id)?;
        self.insert_resolved_call_edges(file_id, result)
    }

    pub fn query_deps(&self, path: &RepoPath) -> ScopeResult<Vec<DependencyRecord>> {
        let Some(file_id) = self.file_id(path)? else {
            return Ok(Vec::new());
        };

        let mut statement = self.connection.prepare(
            "SELECT files.path, file_edges.kind, file_edges.certainty, imports.raw_text, imports.start_line
             FROM file_edges
             JOIN files ON files.id = file_edges.to_file_id
             LEFT JOIN imports ON imports.file_id = file_edges.from_file_id
                 AND imports.resolved_file_id = file_edges.to_file_id
             WHERE file_edges.from_file_id = ?1
             ORDER BY files.path ASC",
        )?;

        let rows = statement.query_map([file_id], |row| {
            Ok(DependencyRecord {
                kind: NodeKind::File,
                path: RepoPath(row.get::<_, String>(0)?),
                edge_kind: edge_kind_from_db(&row.get::<_, String>(1)?),
                certainty: certainty_from_db(&row.get::<_, String>(2)?),
                import_text: row.get(3)?,
                line: row.get(4)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn query_reverse_deps(&self, path: &RepoPath) -> ScopeResult<Vec<DependencyRecord>> {
        let Some(file_id) = self.file_id(path)? else {
            return Ok(Vec::new());
        };

        let mut statement = self.connection.prepare(
            "SELECT files.path, file_edges.kind, file_edges.certainty, imports.raw_text, imports.start_line
             FROM file_edges
             JOIN files ON files.id = file_edges.from_file_id
             LEFT JOIN imports ON imports.file_id = file_edges.from_file_id
                 AND imports.resolved_file_id = file_edges.to_file_id
             WHERE file_edges.to_file_id = ?1
             ORDER BY files.path ASC",
        )?;

        let rows = statement.query_map([file_id], |row| {
            Ok(DependencyRecord {
                kind: NodeKind::File,
                path: RepoPath(row.get::<_, String>(0)?),
                edge_kind: edge_kind_from_db(&row.get::<_, String>(1)?),
                certainty: certainty_from_db(&row.get::<_, String>(2)?),
                import_text: row.get(3)?,
                line: row.get(4)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn query_file_edges(&self) -> ScopeResult<Vec<ArchFileEdge>> {
        let mut statement = self.connection.prepare(
            "SELECT from_files.path, to_files.path, file_edges.kind, file_edges.certainty
             FROM file_edges
             JOIN files AS from_files ON from_files.id = file_edges.from_file_id
             JOIN files AS to_files ON to_files.id = file_edges.to_file_id
             ORDER BY from_files.path ASC, to_files.path ASC, file_edges.kind ASC",
        )?;

        let rows = statement.query_map([], |row| {
            Ok(ArchFileEdge {
                from_file: RepoPath(row.get::<_, String>(0)?),
                to_file: RepoPath(row.get::<_, String>(1)?),
                edge_kind: edge_kind_from_db(&row.get::<_, String>(2)?),
                certainty: certainty_from_db(&row.get::<_, String>(3)?),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn save_snapshot(
        &self,
        name: &str,
        commit: Option<String>,
    ) -> ScopeResult<SnapshotSaveResult> {
        if name.trim().is_empty() {
            return Err(ScopeError::InvalidInput(
                "snapshot name may not be empty".to_string(),
            ));
        }

        let replaced_existing = self.snapshot_exists(name)?;
        let metadata = SnapshotMetadata {
            name: name.to_string(),
            created_at: unix_timestamp(),
            commit,
            schema_version: INDEX_SCHEMA_VERSION,
            snapshot_version: snapshot::SNAPSHOT_VERSION,
        };
        let graph = self.build_snapshot_graph(metadata.created_at)?;
        let summary = snapshot::snapshot_summary(&graph);
        let record = SnapshotStoredRecord {
            metadata: metadata.clone(),
            graph,
        };
        let payload = snapshot::encode_snapshot(&record)?;
        self.connection.execute(
            "INSERT INTO snapshots (name, created_at, commit_sha, schema_version, snapshot_version, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET
                 created_at = excluded.created_at,
                 commit_sha = excluded.commit_sha,
                 schema_version = excluded.schema_version,
                 snapshot_version = excluded.snapshot_version,
                 payload = excluded.payload",
            params![
                metadata.name,
                metadata.created_at,
                metadata.commit,
                metadata.schema_version as i64,
                metadata.snapshot_version as i64,
                payload,
            ],
        )?;
        Ok(SnapshotSaveResult {
            snapshot: metadata,
            replaced_existing,
            summary,
        })
    }

    pub fn list_snapshots(&self) -> ScopeResult<SnapshotListResult> {
        let mut statement = self.connection.prepare(
            "SELECT name, created_at, commit_sha, schema_version, snapshot_version
             FROM snapshots
             ORDER BY created_at DESC, name ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SnapshotMetadata {
                name: row.get(0)?,
                created_at: row.get(1)?,
                commit: row.get(2)?,
                schema_version: row.get::<_, i64>(3)? as u32,
                snapshot_version: row.get::<_, i64>(4)? as u32,
            })
        })?;
        let snapshots = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(SnapshotListResult {
            summary: SnapshotListSummary {
                snapshot_count: snapshots.len(),
            },
            snapshots,
        })
    }

    pub fn delete_snapshot(&self, name: &str) -> ScopeResult<SnapshotDeleteResult> {
        let deleted = self
            .connection
            .execute("DELETE FROM snapshots WHERE name = ?1", [name])?
            > 0;
        Ok(SnapshotDeleteResult {
            name: name.to_string(),
            deleted,
        })
    }

    pub fn diff_snapshot(
        &self,
        before_name: &str,
        after_name: &str,
        config: &ArchConfig,
    ) -> ScopeResult<SnapshotDiffResult> {
        let before = self.load_snapshot(before_name)?;
        let after = self.load_snapshot(after_name)?;

        let added_file_edges = diff_edge_records(&before.graph.file_edges, &after.graph.file_edges);
        let removed_file_edges = diff_edge_records(&after.graph.file_edges, &before.graph.file_edges);
        let added_symbol_edges = diff_edge_records(&before.graph.symbol_edges, &after.graph.symbol_edges);
        let removed_symbol_edges = diff_edge_records(&after.graph.symbol_edges, &before.graph.symbol_edges);

        let before_file_edges = snapshot_file_edges(&before.graph);
        let after_file_edges = snapshot_file_edges(&after.graph);
        let (_, before_violations) = arch_check_edges(config, &before_file_edges)?;
        let (_, after_violations) = arch_check_edges(config, &after_file_edges)?;
        let introduced_violations = diff_violations(&before_violations, &after_violations);
        let resolved_violations = diff_violations(&after_violations, &before_violations);

        let before_surface = snapshot_public_surface(&before.graph);
        let after_surface = snapshot_public_surface(&after.graph);
        let surface_diff = diff_public_surfaces(&before_surface, &after_surface);
        let omitted = vec!["cycle detection delta not implemented yet".to_string()];

        Ok(SnapshotDiffResult {
            before: before.metadata,
            after: after.metadata,
            edge_delta: SnapshotEdgeDelta {
                file_edges_added: added_file_edges.len(),
                file_edges_removed: removed_file_edges.len(),
                symbol_edges_added: added_symbol_edges.len(),
                symbol_edges_removed: removed_symbol_edges.len(),
            },
            added_file_edges,
            removed_file_edges,
            added_symbol_edges,
            removed_symbol_edges,
            newly_central_files: diff_snapshot_centrality(&before.graph, &after.graph),
            introduced_violations,
            resolved_violations,
            stability: diff_snapshot_stability(&before.graph, &after.graph),
            surface_diff,
            summary: snapshot::snapshot_summary(&after.graph),
            omitted,
        })
    }

    pub fn build_test_map(&self, tests: &TestConfig) -> ScopeResult<TestMapBuildResult> {
        let test_files = self.detect_test_files(tests)?;
        let coverage_map = self.compute_test_coverage_map(&test_files)?;
        let all_source_files = self.list_source_files(&test_files);
        let covered_source_files = coverage_map.len();
        let uncovered_source_files = all_source_files
            .into_iter()
            .filter(|path| !coverage_map.contains_key(path))
            .count();

        Ok(TestMapBuildResult {
            tests: test_files.clone(),
            summary: TestMapBuildSummary {
                test_files: test_files.len(),
                covered_source_files,
                uncovered_source_files,
            },
        })
    }

    pub fn query_tests_covering(
        &self,
        source_file: &RepoPath,
        tests: &TestConfig,
    ) -> ScopeResult<TestMapCoversResult> {
        if self.file_id(source_file)?.is_none() {
            return Err(ScopeError::InvalidInput(format!(
                "scope test-map covers could not resolve target `{}`; use an indexed source file path",
                source_file.0
            )));
        }

        let test_files = self.detect_test_files(tests)?;
        if test_files.iter().any(|path| path == source_file) {
            return Err(ScopeError::InvalidInput(format!(
                "scope test-map covers requires a source file target, but `{}` matches configured test-file patterns",
                source_file.0
            )));
        }

        let coverage_map = self.compute_test_coverage_map(&test_files)?;
        let records = coverage_map.get(source_file).cloned().unwrap_or_default();
        Ok(TestMapCoversResult {
            source_file: source_file.clone(),
            summary: TestMapCoversSummary {
                covering_tests: records.len(),
                nearest_distance: records.first().map(|record| record.distance),
            },
            tests: records,
        })
    }

    pub fn query_test_coverage(
        &self,
        test_file: &RepoPath,
        tests: &TestConfig,
    ) -> ScopeResult<TestMapCoveredByResult> {
        if self.file_id(test_file)?.is_none() {
            return Err(ScopeError::InvalidInput(format!(
                "scope test-map covered-by could not resolve target `{}`; use an indexed test file path",
                test_file.0
            )));
        }

        let test_files = self.detect_test_files(tests)?;
        if !test_files.iter().any(|path| path == test_file) {
            return Err(ScopeError::InvalidInput(format!(
                "scope test-map covered-by requires a detected test file target, but `{}` does not match configured test-file patterns",
                test_file.0
            )));
        }
        let test_set: HashSet<_> = test_files.into_iter().collect();
        let records = self.forward_file_closure(test_file, Some(&test_set))?;
        Ok(TestMapCoveredByResult {
            test_file: test_file.clone(),
            summary: TestMapCoveredBySummary {
                covered_source_files: records.len(),
                nearest_distance: records.first().map(|record| record.distance),
            },
            covered_files: records,
        })
    }

    pub fn query_uncovered_files(&self, tests: &TestConfig) -> ScopeResult<TestMapUncoveredResult> {
        let test_files = self.detect_test_files(tests)?;
        let coverage_map = self.compute_test_coverage_map(&test_files)?;
        let source_files = self.list_source_files(&test_files);
        let source_files_considered = source_files.len();
        let mut uncovered = source_files
            .into_iter()
            .filter(|path| !coverage_map.contains_key(path))
            .collect::<Vec<_>>();
        uncovered.sort_by(|left, right| {
            let left_mtime = self
                .file_state(left)
                .ok()
                .and_then(|state| state.and_then(|file| file.mtime_unix_seconds))
                .unwrap_or(i64::MIN);
            let right_mtime = self
                .file_state(right)
                .ok()
                .and_then(|state| state.and_then(|file| file.mtime_unix_seconds))
                .unwrap_or(i64::MIN);
            right_mtime.cmp(&left_mtime).then_with(|| left.cmp(right))
        });
        Ok(TestMapUncoveredResult {
            summary: TestMapUncoveredSummary {
                source_files_considered,
                uncovered_source_files: uncovered.len(),
            },
            files: uncovered,
        })
    }

    fn detect_test_files(&self, tests: &TestConfig) -> ScopeResult<Vec<RepoPath>> {
        let all_files = self.list_indexed_files()?;
        let mut matches = all_files
            .into_iter()
            .filter(|path| matches_test_patterns(path, tests))
            .collect::<Vec<_>>();
        matches.sort();
        Ok(matches)
    }

    fn compute_test_coverage_map(
        &self,
        test_files: &[RepoPath],
    ) -> ScopeResult<HashMap<RepoPath, Vec<TestMapRecord>>> {
        let test_set: HashSet<_> = test_files.iter().cloned().collect();
        let mut coverage_map: HashMap<RepoPath, Vec<TestMapRecord>> = HashMap::new();
        for test_file in test_files {
            for record in self.forward_file_closure(test_file, Some(&test_set))? {
                coverage_map.entry(record.path.clone()).or_default().push(TestMapRecord {
                    path: test_file.clone(),
                    distance: record.distance,
                });
            }
        }
        for records in coverage_map.values_mut() {
            records.sort_by(|left, right| left.distance.cmp(&right.distance).then_with(|| left.path.cmp(&right.path)));
            records.dedup_by(|left, right| left.path == right.path && left.distance == right.distance);
        }
        Ok(coverage_map)
    }

    fn forward_file_closure(
        &self,
        start: &RepoPath,
        excluded: Option<&HashSet<RepoPath>>,
    ) -> ScopeResult<Vec<TestMapRecord>> {
        let Some(start_id) = self.file_id(start)? else {
            return Ok(Vec::new());
        };
        let mut visited = HashSet::from([start_id]);
        let mut queue = VecDeque::from([(start_id, 0u32)]);
        let mut covered = Vec::new();

        while let Some((file_id, distance)) = queue.pop_front() {
            let mut statement = self.connection.prepare(
                "SELECT files.id, files.path
                 FROM file_edges
                 JOIN files ON files.id = file_edges.to_file_id
                 WHERE file_edges.from_file_id = ?1
                 ORDER BY files.path ASC",
            )?;
            let rows = statement.query_map([file_id], |row| {
                Ok((row.get::<_, i64>(0)?, RepoPath(row.get::<_, String>(1)?)))
            })?;
            for row in rows {
                let (next_id, next_path) = row?;
                if !visited.insert(next_id) {
                    continue;
                }
                queue.push_back((next_id, distance + 1));
                if excluded.is_some_and(|set| set.contains(&next_path)) {
                    continue;
                }
                covered.push(TestMapRecord {
                    path: next_path,
                    distance: distance + 1,
                });
            }
        }

        covered.sort_by(|left, right| left.distance.cmp(&right.distance).then_with(|| left.path.cmp(&right.path)));
        Ok(covered)
    }

    fn list_source_files(&self, test_files: &[RepoPath]) -> Vec<RepoPath> {
        let test_set: HashSet<_> = test_files.iter().cloned().collect();
        self.list_indexed_files()
            .unwrap_or_default()
            .into_iter()
            .filter(|path| !test_set.contains(path))
            .collect()
    }

    pub fn persist_file_churn(
        &self,
        path: &RepoPath,
        commit_sha: &str,
        author_email: Option<&str>,
        committed_at: Option<i64>,
    ) -> ScopeResult<bool> {
        let Some(file_id) = self.file_id(path)? else {
            return Ok(false);
        };

        self.connection.execute(
            "INSERT INTO file_churn (file_id, commit_sha, author_email, committed_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(file_id, commit_sha) DO UPDATE SET
                 author_email = excluded.author_email,
                 committed_at = excluded.committed_at",
            params![file_id, commit_sha, author_email, committed_at],
        )?;
        Ok(true)
    }

    pub fn clear_file_churn(&self) -> ScopeResult<()> {
        self.connection.execute("DELETE FROM file_churn", [])?;
        Ok(())
    }

    pub fn query_stability(
        &self,
        file: Option<&RepoPath>,
        flag_threshold: Option<f64>,
        sort: StabilitySort,
    ) -> ScopeResult<StabilityResult> {
        if let Some(threshold) = flag_threshold {
            if !(0.0..=1.0).contains(&threshold) {
                return Err(ScopeError::InvalidInput(format!(
                    "flag threshold must be between 0.0 and 1.0: {threshold}"
                )));
            }
        }

        let all_files = self.list_indexed_files()?;
        let edges = self.query_file_edges()?;
        let mut records = stability_records_from_file_edges(&all_files, &edges, flag_threshold);
        let summary = stability_summary(&records);

        if let Some(path) = file {
            if !all_files_set(&records).contains(path) {
                return Err(ScopeError::InvalidInput(format!(
                    "file not indexed: {}",
                    path.0
                )));
            }
            records.retain(|record| &record.path == path);
        } else if flag_threshold.is_some() {
            records.retain(|record| record.flagged);
        }

        sort_stability_records(&mut records, sort.clone());

        let target = file.cloned();

        Ok(StabilityResult {
            file: target,
            flag_threshold,
            sort,
            files: records,
            summary,
        })
    }

    pub fn query_risk(
        &self,
        file: Option<&RepoPath>,
        days: u32,
        threshold: Option<f64>,
        top: Option<usize>,
        sort: RiskSort,
    ) -> ScopeResult<RiskResult> {
        if days == 0 {
            return Err(ScopeError::InvalidInput(
                "risk window days must be greater than 0".to_string(),
            ));
        }
        if let Some(threshold) = threshold {
            if threshold < 0.0 {
                return Err(ScopeError::InvalidInput(format!(
                    "risk threshold must be non-negative: {threshold}"
                )));
            }
        }
        if let Some(top) = top {
            if top == 0 {
                return Err(ScopeError::InvalidInput(
                    "top must be greater than 0".to_string(),
                ));
            }
        }

        let all_files = self.list_indexed_files()?;
        let edges = self.query_file_edges()?;
        let mut direct_dependents: HashMap<RepoPath, usize> = HashMap::new();
        let mut reverse_adjacency: HashMap<RepoPath, Vec<RepoPath>> = HashMap::new();

        for edge in edges
            .into_iter()
            .filter(|edge| edge.edge_kind == EdgeKind::Import)
        {
            *direct_dependents.entry(edge.to_file.clone()).or_default() += 1;
            reverse_adjacency
                .entry(edge.to_file)
                .or_default()
                .push(edge.from_file);
        }

        let since = unix_timestamp() - (days as i64 * 24 * 60 * 60);
        let mut churn_statement = self.connection.prepare(
            "SELECT files.path, COUNT(DISTINCT file_churn.commit_sha)
             FROM files
             LEFT JOIN file_churn ON file_churn.file_id = files.id AND (file_churn.committed_at IS NULL OR file_churn.committed_at >= ?1)
             GROUP BY files.path",
        )?;
        let churn_rows = churn_statement.query_map([since], |row| {
            Ok((
                RepoPath(row.get::<_, String>(0)?),
                row.get::<_, i64>(1)? as usize,
            ))
        })?;
        let churn_counts: HashMap<RepoPath, usize> = churn_rows
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .collect();
        let git_available = churn_counts.values().any(|count| *count > 0);

        let mut records = Vec::new();
        for path in all_files {
            let direct = direct_dependents.get(&path).copied().unwrap_or(0);
            let transitive = transitive_dependents_count(&path, &reverse_adjacency);
            let churn = churn_counts.get(&path).copied().unwrap_or(0);
            let raw_score = if git_available {
                ((transitive + 1) as f64).log2() * ((churn + 1) as f64).log2()
            } else {
                ((direct + 1) as f64).log2()
            };
            let reason = if git_available {
                format!(
                    "{} direct dependents, {} transitive dependents, {} churn commits in last {} days",
                    direct, transitive, churn, days
                )
            } else {
                format!(
                    "git churn unavailable; score falls back to dependency-only blast radius ({} direct dependents, {} transitive dependents)",
                    direct, transitive
                )
            };
            records.push(RiskRecord {
                path,
                direct_dependents: direct,
                transitive_dependents: transitive,
                churn_commits: churn,
                score: raw_score,
                normalized_score: 0,
                reason,
            });
        }

        if let Some(path) = file {
            if !records.iter().any(|record| &record.path == path) {
                return Err(ScopeError::InvalidInput(format!("file not indexed: {}", path.0)));
            }
            records.retain(|record| &record.path == path);
        }
        if let Some(threshold) = threshold {
            records.retain(|record| record.score >= threshold);
        }

        let max_score = records.iter().map(|record| record.score).fold(0.0, f64::max);
        for record in &mut records {
            record.normalized_score = if max_score > 0.0 {
                ((record.score / max_score) * 100.0).round() as u32
            } else {
                0
            };
        }

        sort_risk_records(&mut records, sort.clone());

        if let Some(limit) = top {
            records.truncate(limit);
        }

        let summary = RiskSummary {
            git_available,
            scored_files: records.len(),
            avg_score: if records.is_empty() {
                0.0
            } else {
                records.iter().map(|record| record.score).sum::<f64>() / records.len() as f64
            },
            max_score,
        };

        Ok(RiskResult {
            file: file.cloned(),
            top,
            days,
            sort,
            files: records,
            summary,
        })
    }

    pub fn query_symbols(
        &self,
        path: &RepoPath,
        public_only: bool,
        kind: Option<SymbolKind>,
    ) -> ScopeResult<Vec<SymbolRecord>> {
        let Some(file_id) = self.file_id(path)? else {
            return Ok(Vec::new());
        };

        let mut statement = self.connection.prepare(
            "SELECT name, qualname, kind, visibility, exported, span_start, span_end, start_line, end_line
             FROM symbols
             WHERE file_id = ?1
             ORDER BY start_line ASC, name ASC",
        )?;

        let rows = statement.query_map([file_id], |row| {
            Ok(SymbolRecord {
                file: path.clone(),
                name: row.get(0)?,
                qualname: row.get(1)?,
                kind: symbol_kind_from_db(&row.get::<_, String>(2)?),
                visibility: visibility_from_db(&row.get::<_, String>(3)?),
                exported: row.get::<_, i64>(4)? != 0,
                span: crate::Span {
                    start_byte: row.get::<_, i64>(5)? as u32,
                    end_byte: row.get::<_, i64>(6)? as u32,
                    start_line: row.get::<_, i64>(7)? as u32,
                    end_line: row.get::<_, i64>(8)? as u32,
                },
            })
        })?;

        let mut symbols = rows.collect::<Result<Vec<_>, _>>()?;
        if public_only {
            symbols.retain(|symbol| symbol.exported);
        }
        if let Some(kind) = kind {
            symbols.retain(|symbol| symbol.kind == kind);
        }
        Ok(symbols)
    }

    pub fn query_public_surface(&self, path: &RepoPath) -> ScopeResult<PublicSurface> {
        let mut symbols: Vec<_> = self
            .query_symbols(path, true, None)?
            .into_iter()
            .map(|symbol| PublicSurfaceSymbol {
                file: symbol.file,
                name: symbol.name,
                qualname: symbol.qualname,
                kind: symbol.kind,
                visibility: symbol.visibility,
                line: symbol.span.start_line,
            })
            .collect();
        symbols.sort_by(|left, right| {
            left.line
                .cmp(&right.line)
                .then_with(|| left.qualname.cmp(&right.qualname))
        });
        Ok(PublicSurface {
            file: path.clone(),
            symbols,
        })
    }

    pub fn resolve_rename_target(&self, target: &str) -> ScopeResult<RenameTarget> {
        if let Some(symbol_id) = self.symbol_id(target)? {
            let symbol = self.symbol_record_by_id(symbol_id)?.ok_or_else(|| {
                ScopeError::InvalidInput(format!("missing indexed symbol for target: {target}"))
            })?;
            return Ok(RenameTarget::Symbol {
                qualname: target.to_string(),
                symbol,
            });
        }

        let path = RepoPath::from(target.to_string());
        if self.file_id(&path)?.is_some() {
            return Ok(RenameTarget::File { path });
        }

        Err(ScopeError::InvalidInput(format!(
            "rename-plan could not resolve target `{target}`; use an indexed file path or symbol qualname"
        )))
    }

    pub fn build_rename_plan(
        &self,
        repo_root: &Path,
        target: &str,
        new_name: &str,
        apply_requested: bool,
        force_requested: bool,
    ) -> ScopeResult<RenamePlan> {
        let resolved = self.resolve_rename_target(target)?;
        let mut steps = Vec::new();
        let mut skipped = Vec::new();
        let mut warnings = Vec::new();
        let mut applied_files = 0usize;
        let mut applied_edits = 0usize;
        let target_file = match &resolved {
            RenameTarget::Symbol { symbol, .. } => symbol.file.clone(),
            RenameTarget::File { path } => path.clone(),
        };
        let old_name = match &resolved {
            RenameTarget::Symbol { symbol, .. } => symbol.name.clone(),
            RenameTarget::File { path } => file_stem_name(path)?,
        };

        if old_name == new_name {
            return Err(ScopeError::InvalidInput(
                "rename-plan target already uses the requested name".to_string(),
            ));
        }

        match &resolved {
            RenameTarget::Symbol { qualname, symbol } => {
                let definition_source = std::fs::read_to_string(repo_root.join(&symbol.file.0))
                    .map_err(|error| ScopeError::io(repo_root.join(&symbol.file.0), error))?;
                if let Some(definition_edit) = definition_edit_from_symbol(symbol, &definition_source, new_name) {
                    let definition = RenamePlanStep {
                        path: symbol.file.clone(),
                        distance: 0,
                        certainty: Certainty::Exact,
                        roles: vec!["target".to_string(), "defines_target_symbol".to_string()],
                        reasons: vec![format!("defines symbol {qualname}")],
                        edits: vec![definition_edit],
                        apply_safe: true,
                    };
                    steps.push(definition);
                } else {
                    skipped.push(RenamePlanStep {
                        path: symbol.file.clone(),
                        distance: 0,
                        certainty: Certainty::Heuristic,
                        roles: vec!["target".to_string(), "defines_target_symbol".to_string()],
                        reasons: vec![format!("defines symbol {qualname}")],
                        edits: vec![RenameEdit {
                            start_byte: 0,
                            end_byte: 0,
                            line: symbol.span.start_line,
                            before_text: old_name.clone(),
                            after_text: new_name.to_string(),
                            kind: RenameEditKind::DeferredUnknown,
                            verified: false,
                            deferred_reason: Some(
                                "could not derive exact identifier span from the current source line"
                                    .to_string(),
                            ),
                        }],
                        apply_safe: false,
                    });
                }

                for step in self.collect_symbol_import_rename_steps(symbol, &old_name, new_name)? {
                    steps.push(step);
                }

                for step in self.collect_symbol_deferred_steps(qualname)? {
                    skipped.push(step);
                }
            }
            RenameTarget::File { path } => {
                for step in self.collect_file_import_path_steps(path, &old_name, new_name)? {
                    steps.push(step);
                }
                warnings.push(
                    "file-target rename planning does not move files yet; only import-path rewrites are planned"
                        .to_string(),
                );
            }
        }

        steps.sort_by(|left, right| left.path.cmp(&right.path));
        skipped.sort_by(|left, right| left.path.cmp(&right.path));

        let safe_edits_planned = steps.iter().map(|step| step.edits.len()).sum::<usize>();
        let deferred_edits_planned = skipped.iter().map(|step| step.edits.len()).sum::<usize>();
        let edits_planned = safe_edits_planned + deferred_edits_planned;
        let blocked = !force_requested && !skipped.is_empty();

        if apply_requested {
            if blocked {
                warnings.push(
                    "apply blocked because deferred or unsupported sites remain; rerun with --force to apply only the safe subset"
                        .to_string(),
                );
            } else {
                let applied = self.apply_rename_plan_steps(repo_root, &steps)?;
                applied_files = applied.0;
                applied_edits = applied.1;
                if !skipped.is_empty() {
                    warnings.push(
                        "apply completed only for safe sites; inspect deferred sites manually"
                            .to_string(),
                    );
                }
            }
        }

        let files_considered = edits_planned_file_count(&resolved, &steps, &skipped);
        let files_planned = steps.len();
        let files_skipped = skipped.len();

        Ok(RenamePlan {
            target: target.to_string(),
            target_file,
            old_name,
            new_name: new_name.to_string(),
            apply_requested,
            force_requested,
            applied: apply_requested && !blocked && applied_files > 0,
            steps,
            skipped,
            warnings,
            summary: RenamePlanSummary {
                files_considered,
                files_planned,
                files_skipped,
                edits_planned,
                safe_edits_planned,
                deferred_edits_planned,
                applied_files,
                applied_edits,
                blocked,
            },
        })
    }

    fn build_snapshot_graph(&self, created_at: i64) -> ScopeResult<SnapshotGraph> {
        let mut file_statement = self.connection.prepare(
            "SELECT path, language, content_hash FROM files ORDER BY path ASC",
        )?;
        let files = file_statement
            .query_map([], |row| {
                Ok(SnapshotFileRecord {
                    path: RepoPath(row.get::<_, String>(0)?),
                    language: row.get(1)?,
                    content_hash: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut symbol_statement = self.connection.prepare(
            "SELECT files.path, symbols.name, symbols.qualname, symbols.kind, symbols.visibility, symbols.exported
             FROM symbols
             JOIN files ON files.id = symbols.file_id
             ORDER BY files.path ASC, symbols.qualname ASC",
        )?;
        let symbols = symbol_statement
            .query_map([], |row| {
                Ok(SnapshotSymbolRecord {
                    file: RepoPath(row.get::<_, String>(0)?),
                    name: row.get(1)?,
                    qualname: row.get(2)?,
                    kind: symbol_kind_from_db(&row.get::<_, String>(3)?),
                    visibility: visibility_from_db(&row.get::<_, String>(4)?),
                    exported: row.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut file_edge_statement = self.connection.prepare(
            "SELECT from_files.path, to_files.path, file_edges.kind, file_edges.certainty
             FROM file_edges
             JOIN files AS from_files ON from_files.id = file_edges.from_file_id
             JOIN files AS to_files ON to_files.id = file_edges.to_file_id
             ORDER BY from_files.path ASC, to_files.path ASC, file_edges.kind ASC",
        )?;
        let file_edges = file_edge_statement
            .query_map([], |row| {
                Ok(SnapshotEdgeRecord {
                    from: row.get(0)?,
                    to: row.get(1)?,
                    kind: edge_kind_from_db(&row.get::<_, String>(2)?),
                    certainty: certainty_from_db(&row.get::<_, String>(3)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut symbol_edge_statement = self.connection.prepare(
            "SELECT from_symbols.qualname, to_symbols.qualname, symbol_edges.kind, symbol_edges.certainty
             FROM symbol_edges
             JOIN symbols AS from_symbols ON from_symbols.id = symbol_edges.from_symbol_id
             JOIN symbols AS to_symbols ON to_symbols.id = symbol_edges.to_symbol_id
             ORDER BY from_symbols.qualname ASC, to_symbols.qualname ASC, symbol_edges.kind ASC, symbol_edges.call_line ASC",
        )?;
        let symbol_edges = symbol_edge_statement
            .query_map([], |row| {
                Ok(SnapshotEdgeRecord {
                    from: row.get(0)?,
                    to: row.get(1)?,
                    kind: match row.get::<_, String>(2)?.as_str() {
                        "call" => EdgeKind::Call,
                        "export" => EdgeKind::Export,
                        "define" => EdgeKind::Define,
                        "contain" => EdgeKind::Contain,
                        "dynamic" => EdgeKind::Dynamic,
                        _ => EdgeKind::Import,
                    },
                    certainty: certainty_from_db(&row.get::<_, String>(3)?),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(SnapshotGraph {
            schema_version: INDEX_SCHEMA_VERSION,
            snapshot_version: snapshot::SNAPSHOT_VERSION,
            created_at,
            files,
            symbols,
            file_edges,
            symbol_edges,
        })
    }

    fn snapshot_exists(&self, name: &str) -> ScopeResult<bool> {
        let exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshots WHERE name = ?1)",
            [name],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(exists != 0)
    }

    fn load_snapshot(&self, name: &str) -> ScopeResult<SnapshotStoredRecord> {
        let payload = self
            .connection
            .query_row(
                "SELECT payload FROM snapshots WHERE name = ?1",
                [name],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        let payload = payload.ok_or_else(|| ScopeError::NotFound {
            kind: "snapshot",
            value: name.to_string(),
        })?;
        snapshot::decode_snapshot(&payload)
    }

    pub fn target_file_for_target(&self, target: &str) -> ScopeResult<Option<RepoPath>> {
        if looks_like_symbol(target) {
            let result = self.query_context(&[target.to_string()], "body", None)?;
            Ok(result.must_read.first().map(|record| record.path.clone()))
        } else {
            Ok(Some(RepoPath::from(target.to_string())))
        }
    }

    pub fn resolve_surface_target(&self, target: &str) -> ScopeResult<RepoPath> {
        self.target_file_for_target(target)?.ok_or_else(|| {
            ScopeError::InvalidInput(format!("missing indexed file for target: {target}"))
        })
    }

    pub fn diff_public_surface(
        &self,
        before: &RepoPath,
        after: &RepoPath,
    ) -> ScopeResult<PublicSurfaceDiff> {
        let before_surface = self.query_public_surface(before)?;
        let after_surface = self.query_public_surface(after)?;

        let before_by_identity: HashMap<_, _> = before_surface
            .symbols
            .iter()
            .cloned()
            .map(|symbol| (public_surface_identity(&symbol), symbol))
            .collect();
        let after_by_identity: HashMap<_, _> = after_surface
            .symbols
            .iter()
            .cloned()
            .map(|symbol| (public_surface_identity(&symbol), symbol))
            .collect();

        let mut identities: Vec<_> = before_by_identity
            .keys()
            .chain(after_by_identity.keys())
            .cloned()
            .collect();
        identities.sort();
        identities.dedup();

        let mut changes = Vec::new();
        for identity in identities {
            match (
                before_by_identity.get(&identity),
                after_by_identity.get(&identity),
            ) {
                (Some(before_symbol), None) => changes.push(PublicSurfaceChange {
                    kind: PublicSurfaceChangeKind::Removed,
                    before: Some(before_symbol.clone()),
                    after: None,
                }),
                (None, Some(after_symbol)) => changes.push(PublicSurfaceChange {
                    kind: PublicSurfaceChangeKind::Added,
                    before: None,
                    after: Some(after_symbol.clone()),
                }),
                (Some(before_symbol), Some(after_symbol)) if before_symbol != after_symbol => {
                    changes.push(PublicSurfaceChange {
                        kind: PublicSurfaceChangeKind::Modified,
                        before: Some(before_symbol.clone()),
                        after: Some(after_symbol.clone()),
                    });
                }
                _ => {}
            }
        }

        let summary = PublicSurfaceDiffSummary {
            added_count: changes
                .iter()
                .filter(|change| change.kind == PublicSurfaceChangeKind::Added)
                .count(),
            removed_count: changes
                .iter()
                .filter(|change| change.kind == PublicSurfaceChangeKind::Removed)
                .count(),
            modified_count: changes
                .iter()
                .filter(|change| change.kind == PublicSurfaceChangeKind::Modified)
                .count(),
        };

        Ok(PublicSurfaceDiff {
            before_file: before.clone(),
            after_file: after.clone(),
            changes,
            summary,
        })
    }

    pub fn query_callees(
        &self,
        symbol_qualname: &str,
        transitive: bool,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if transitive {
            return Ok(Vec::new());
        }

        self.query_symbol_edges(symbol_qualname, false)
    }

    pub fn query_callers(
        &self,
        symbol_qualname: &str,
        transitive: bool,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if transitive {
            return Ok(Vec::new());
        }

        self.query_symbol_edges(symbol_qualname, true)
    }

    pub fn query_impact(
        &self,
        target: &str,
        change_type: &str,
        depth: Option<usize>,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        let max_depth = depth
            .map(|value| value as u32)
            .unwrap_or_else(|| default_depth_for_change_type(change_type));
        let mut impacted = Vec::new();

        if let Some(symbol_id) = self.symbol_id(target)? {
            let symbol_depth = if change_type == "body" { 1 } else { max_depth };
            if includes_callers(change_type) {
                impacted.extend(self.traverse_reverse_callers(symbol_id, target, symbol_depth)?);
            }

            if includes_importers(change_type) {
                if let Some(file_id) = self.file_id_for_symbol(symbol_id)? {
                    impacted.extend(self.traverse_reverse_importers(
                        file_id,
                        target,
                        max_depth,
                        change_type == "side-effect",
                    )?);
                }
            }

            return Ok(dedup_traversals(impacted));
        }

        let path = RepoPath::from(target.to_string());
        let Some(file_id) = self.file_id(&path)? else {
            return Ok(Vec::new());
        };

        let file_depth = if change_type == "body" { 1 } else { max_depth };
        Ok(self.traverse_reverse_importers(
            file_id,
            target,
            file_depth,
            change_type == "side-effect",
        )?)
    }

    pub fn query_why(
        &self,
        from: &str,
        to: &str,
        depth: Option<usize>,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if from == to {
            return Ok(Vec::new());
        }

        let max_depth = depth
            .map(|value| value as u32)
            .unwrap_or(DEFAULT_TRANSITIVE_DEPTH);

        if let (Some(from_symbol_id), Some(to_symbol_id)) =
            (self.symbol_id(from)?, self.symbol_id(to)?)
        {
            return self.shortest_symbol_path(from_symbol_id, from, to_symbol_id, to, max_depth);
        }

        if let (Some(from_file_id), Some(to_file_id)) = (
            self.file_id(&RepoPath::from(from.to_string()))?,
            self.file_id(&RepoPath::from(to.to_string()))?,
        ) {
            return self.shortest_file_path(from_file_id, from, to_file_id, to, max_depth);
        }

        let from_is_symbol = self.symbol_id(from)?.is_some();
        let to_is_symbol = self.symbol_id(to)?.is_some();
        let from_is_file = self.file_id(&RepoPath::from(from.to_string()))?.is_some();
        let to_is_file = self.file_id(&RepoPath::from(to.to_string()))?.is_some();

        if (from_is_symbol && to_is_file) || (from_is_file && to_is_symbol) {
            return Err(ScopeError::InvalidInput(
                "scope why requires both endpoints to be files or both to be symbols".to_string(),
            ));
        }

        Ok(Vec::new())
    }

    pub fn query_explain(
        &self,
        target: &str,
        to: Option<&str>,
        depth: Option<usize>,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        let max_depth = depth
            .map(|value| value as u32)
            .unwrap_or(DEFAULT_TRANSITIVE_DEPTH);
        let mut traversals = Vec::new();

        if let Some(symbol_id) = self.symbol_id(target)? {
            traversals.extend(self.traverse_reverse_callers(symbol_id, target, max_depth)?);
            traversals.extend(self.traverse_forward_callees(symbol_id, target, max_depth)?);
            if let Some(file_id) = self.file_id_for_symbol(symbol_id)? {
                traversals
                    .extend(self.traverse_reverse_importers(file_id, target, max_depth, false)?);
            }
        } else if let Some(file_id) = self.file_id(&RepoPath::from(target.to_string()))? {
            traversals.extend(
                self.query_deps(&RepoPath::from(target.to_string()))?
                    .into_iter()
                    .map(|dependency| TraversalRecord {
                        kind: dependency.kind,
                        path: Some(dependency.path),
                        qualname: None,
                        edge_kind: dependency.edge_kind,
                        certainty: dependency.certainty,
                        reason: format!("depends on {target}"),
                        distance: 1,
                    }),
            );
            traversals.extend(self.traverse_reverse_importers(file_id, target, max_depth, false)?);
        }

        let traversals = dedup_traversals(traversals);
        if let Some(to) = to {
            Ok(traversals
                .into_iter()
                .filter(|record| {
                    record.qualname.as_deref() == Some(to)
                        || record.path.as_ref().is_some_and(|path| path.0 == to)
                })
                .collect())
        } else {
            Ok(traversals)
        }
    }

    pub fn query_context(
        &self,
        targets: &[String],
        change_type: &str,
        budget: Option<usize>,
    ) -> ScopeResult<ContextResult> {
        if targets.is_empty() {
            return Err(ScopeError::InvalidInput(
                "scope context requires at least one --target".to_string(),
            ));
        }

        let mut candidates = HashMap::<RepoPath, ContextCandidate>::new();

        for target in targets {
            if let Some(symbol_id) = self.symbol_id(target)? {
                self.add_symbol_target_context(&mut candidates, symbol_id, target, change_type)?;
                continue;
            }

            let path = RepoPath::from(target.clone());
            let Some(file_id) = self.file_id(&path)? else {
                return Err(ScopeError::InvalidInput(format!(
                    "scope context could not resolve target `{target}`; use an indexed file path or symbol qualname"
                )));
            };
            self.add_file_target_context(&mut candidates, file_id, &path, change_type)?;
        }

        let mut ranked: Vec<_> = candidates.into_values().collect();
        ranked.sort_by(|left, right| compare_context_candidates(left, right));

        let mut must_read = Vec::new();
        let mut should_read = Vec::new();
        let mut must_tokens = 0usize;
        let mut should_tokens = 0usize;
        let mut skipped_count = 0usize;
        let mut truncated = false;

        for candidate in ranked {
            let pinned = candidate.pinned;
            let record = ContextFileRecord {
                path: candidate.path,
                score: candidate.score,
                estimated_tokens: candidate.estimated_tokens,
                distance: candidate.distance,
                certainty: candidate.certainty,
                reasons: candidate.reasons,
                roles: candidate.roles,
            };

            let goes_to_must = record.distance <= 1;
            if goes_to_must {
                let would_exceed = budget
                    .is_some_and(|limit| !pinned && must_tokens + record.estimated_tokens > limit);
                if would_exceed {
                    truncated = true;
                    should_tokens += record.estimated_tokens;
                    should_read.push(record);
                } else {
                    must_tokens += record.estimated_tokens;
                    must_read.push(record);
                }
            } else if budget
                .is_some_and(|limit| must_tokens + should_tokens + record.estimated_tokens > limit)
            {
                truncated = true;
                skipped_count += 1;
            } else {
                should_tokens += record.estimated_tokens;
                should_read.push(record);
            }
        }

        let must_read_count = must_read.len();
        let should_read_count = should_read.len();
        Ok(ContextResult {
            targets: targets.to_vec(),
            change_type: change_type.to_string(),
            budget,
            must_read,
            should_read,
            summary: ContextSummary {
                targets_count: targets.len(),
                must_read_count,
                should_read_count,
                skipped_count,
                estimated_tokens: must_tokens + should_tokens,
                budget,
                truncated,
            },
        })
    }

    fn add_symbol_target_context(
        &self,
        candidates: &mut HashMap<RepoPath, ContextCandidate>,
        symbol_id: i64,
        target: &str,
        change_type: &str,
    ) -> ScopeResult<()> {
        let Some(file_id) = self.file_id_for_symbol(symbol_id)? else {
            return Ok(());
        };
        let Some(path) = self.path_for_file_id(file_id)? else {
            return Ok(());
        };

        self.upsert_context_candidate(
            candidates,
            path.clone(),
            ContextFileRole::Target,
            200,
            0,
            Certainty::Exact,
            format!("defines or contains target {target}"),
            true,
        )?;
        self.upsert_context_candidate(
            candidates,
            path.clone(),
            ContextFileRole::DefinesTargetSymbol,
            180,
            0,
            Certainty::Exact,
            format!("defines symbol {target}"),
            true,
        )?;

        if includes_callers(change_type) {
            let symbol_depth = if change_type == "body" { 1 } else { 2 };
            for traversal in self.traverse_reverse_callers(symbol_id, target, symbol_depth)? {
                self.upsert_traversal_candidate(
                    candidates,
                    traversal,
                    ContextFileRole::DirectCaller,
                    120,
                )?;
            }
        }

        for traversal in self.traverse_forward_callees(symbol_id, target, 1)? {
            self.upsert_traversal_candidate(
                candidates,
                traversal,
                ContextFileRole::DirectCallee,
                100,
            )?;
        }

        if includes_importers(change_type) {
            let importer_depth = if change_type == "body" { 1 } else { 2 };
            for traversal in self.traverse_reverse_importers(
                file_id,
                target,
                importer_depth,
                change_type == "side-effect",
            )? {
                self.upsert_traversal_candidate(
                    candidates,
                    traversal,
                    ContextFileRole::Importer,
                    120,
                )?;
            }
        }

        Ok(())
    }

    fn add_file_target_context(
        &self,
        candidates: &mut HashMap<RepoPath, ContextCandidate>,
        file_id: i64,
        path: &RepoPath,
        change_type: &str,
    ) -> ScopeResult<()> {
        self.upsert_context_candidate(
            candidates,
            path.clone(),
            ContextFileRole::Target,
            200,
            0,
            Certainty::Exact,
            format!("target file {}", path.0),
            true,
        )?;

        for dependency in self.query_deps(path)? {
            self.upsert_dependency_candidate(
                candidates,
                dependency,
                ContextFileRole::Dependency,
                100,
            )?;
        }

        let importer_depth = if change_type == "body" { 1 } else { 2 };
        for traversal in self.traverse_reverse_importers(
            file_id,
            &path.0,
            importer_depth,
            change_type == "side-effect",
        )? {
            self.upsert_traversal_candidate(candidates, traversal, ContextFileRole::Importer, 120)?;
        }

        Ok(())
    }

    fn upsert_traversal_candidate(
        &self,
        candidates: &mut HashMap<RepoPath, ContextCandidate>,
        traversal: TraversalRecord,
        role: ContextFileRole,
        base_score: u32,
    ) -> ScopeResult<()> {
        let Some(path) = traversal.path else {
            return Ok(());
        };
        let role = if traversal.distance <= 1 {
            role
        } else {
            ContextFileRole::NearbyContext
        };
        self.upsert_context_candidate(
            candidates,
            path,
            role,
            score_for_candidate(base_score, traversal.distance, &traversal.certainty),
            traversal.distance,
            traversal.certainty,
            traversal.reason,
            false,
        )
    }

    fn upsert_dependency_candidate(
        &self,
        candidates: &mut HashMap<RepoPath, ContextCandidate>,
        dependency: DependencyRecord,
        role: ContextFileRole,
        base_score: u32,
    ) -> ScopeResult<()> {
        self.upsert_context_candidate(
            candidates,
            dependency.path,
            role,
            score_for_candidate(base_score, 1, &dependency.certainty),
            1,
            dependency.certainty,
            dependency
                .import_text
                .map(|text| format!("imported via {text}"))
                .unwrap_or_else(|| "direct dependency of target file".to_string()),
            false,
        )
    }

    fn upsert_context_candidate(
        &self,
        candidates: &mut HashMap<RepoPath, ContextCandidate>,
        path: RepoPath,
        role: ContextFileRole,
        score: u32,
        distance: u32,
        certainty: Certainty,
        reason: String,
        pinned: bool,
    ) -> ScopeResult<()> {
        let estimated_tokens = self.estimate_file_tokens(&path)?;
        let candidate = candidates
            .entry(path.clone())
            .or_insert_with(|| ContextCandidate {
                path,
                score,
                estimated_tokens,
                distance,
                certainty: certainty.clone(),
                reasons: Vec::new(),
                roles: Vec::new(),
                pinned,
            });

        candidate.score = candidate.score.max(score);
        candidate.distance = candidate.distance.min(distance);
        candidate.certainty = better_certainty(&candidate.certainty, &certainty);
        candidate.pinned |= pinned;
        if !candidate.reasons.iter().any(|existing| existing == &reason) {
            candidate.reasons.push(reason);
        }
        if !candidate.roles.contains(&role) {
            candidate.roles.push(role);
            candidate.roles.sort();
        }
        candidate.reasons.sort();
        Ok(())
    }

    fn estimate_file_tokens(&self, path: &RepoPath) -> ScopeResult<usize> {
        Ok(self
            .file_state(path)?
            .and_then(|state| state.size_bytes)
            .map(|bytes| ((bytes.max(1) as usize) / 4).max(1))
            .unwrap_or(64))
    }

    pub fn list_indexed_files(&self) -> ScopeResult<Vec<RepoPath>> {
        let mut statement = self
            .connection
            .prepare("SELECT path FROM files ORDER BY path ASC")?;
        let rows = statement.query_map([], |row| Ok(RepoPath(row.get::<_, String>(0)?)))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn delete_file(&self, path: &RepoPath) -> ScopeResult<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM files WHERE path = ?1", [path.0.as_str()])?;
        Ok(deleted > 0)
    }

    pub fn reverse_dependency_closure(&self, paths: &[RepoPath]) -> ScopeResult<Vec<RepoPath>> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        for path in paths {
            let Some(file_id) = self.file_id(path)? else {
                continue;
            };
            visited.insert(file_id);
            queue.push_back(file_id);
        }

        let mut dependents = Vec::new();

        while let Some(file_id) = queue.pop_front() {
            let mut statement = self.connection.prepare(
                "SELECT DISTINCT importer_files.id, importer_files.path
                 FROM file_edges
                 JOIN files AS importer_files ON importer_files.id = file_edges.from_file_id
                 WHERE file_edges.to_file_id = ?1
                   AND file_edges.kind IN ('import', 'module')
                 ORDER BY importer_files.path ASC",
            )?;
            let rows = statement.query_map([file_id], |row| {
                Ok((row.get::<_, i64>(0)?, RepoPath(row.get::<_, String>(1)?)))
            })?;

            for row in rows {
                let (dependent_id, dependent_path) = row?;
                if visited.insert(dependent_id) {
                    dependents.push(dependent_path.clone());
                    queue.push_back(dependent_id);
                }
            }
        }

        dependents.sort();
        dependents.dedup();
        Ok(dependents)
    }

    fn path_for_file_id(&self, file_id: i64) -> ScopeResult<Option<RepoPath>> {
        self.connection
            .query_row("SELECT path FROM files WHERE id = ?1", [file_id], |row| {
                Ok(RepoPath(row.get::<_, String>(0)?))
            })
            .optional()
            .map_err(Into::into)
    }

    fn file_id(&self, path: &RepoPath) -> ScopeResult<Option<i64>> {
        self.connection
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                [path.0.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }


    fn symbol_id(&self, qualname: &str) -> ScopeResult<Option<i64>> {
        self.connection
            .query_row(
                "SELECT id FROM symbols WHERE qualname = ?1",
                [qualname],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    fn insert_import(&self, file_id: i64, import: &crate::ImportRecord) -> ScopeResult<()> {
        let (kind, resolved_file_id, external_pkg) = match &import.import_path {
            ImportPath::Relative(path) => ("relative", self.file_id(path)?, None),
            ImportPath::External(package) => ("external", None, Some(package.clone())),
            ImportPath::Unresolved => ("unresolved", None, None),
        };

        self.connection.execute(
            "INSERT INTO imports (
                file_id, raw_text, resolved_file_id, import_path_kind, external_pkg, span_start, span_end, start_line, certainty
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                file_id,
                import.raw_text,
                resolved_file_id,
                kind,
                external_pkg,
                import.span.start_byte,
                import.span.end_byte,
                import.span.start_line,
                certainty_name(&import.certainty),
            ],
        )?;

        if let Some(target_file_id) = resolved_file_id {
            self.insert_file_edge(
                file_id,
                target_file_id,
                "import",
                certainty_name(&import.certainty),
            )?;
        }

        Ok(())
    }

    fn delete_symbol_edges_for_file(&self, file_id: i64) -> ScopeResult<()> {
        self.connection.execute(
            "DELETE FROM symbol_edges
             WHERE from_symbol_id IN (SELECT id FROM symbols WHERE file_id = ?1)",
            [file_id],
        )?;
        Ok(())
    }

    fn insert_resolved_call_edges(&self, file_id: i64, result: &ExtractResult) -> ScopeResult<()> {
        for call_site in &result.call_sites {
            if call_site.is_method {
                continue;
            }

            let Some(caller_qualname) = &call_site.caller_qualname else {
                continue;
            };
            let Some(caller_symbol_id) = self.symbol_id(caller_qualname)? else {
                continue;
            };

            let Some((callee_symbol_id, certainty)) =
                self.resolve_call_callee(file_id, &result.imports, call_site)?
            else {
                continue;
            };

            self.insert_symbol_edge(
                caller_symbol_id,
                callee_symbol_id,
                "call",
                certainty_name(&certainty),
                call_site.span.start_line as i64,
            )?;
        }

        Ok(())
    }

    fn resolve_call_callee(
        &self,
        file_id: i64,
        imports: &[crate::ImportRecord],
        call_site: &crate::CallSiteRecord,
    ) -> ScopeResult<Option<(i64, Certainty)>> {
        if let Some(callee_qualname) = &call_site.callee_qualname {
            if let Some(symbol_id) = self.symbol_id(callee_qualname)? {
                return Ok(Some((symbol_id, Certainty::Resolved)));
            }

            if let Some((module_name, symbol_name)) = callee_qualname.rsplit_once("::") {
                let mut target_file_ids =
                    self.imported_file_ids_for_module(file_id, imports, module_name)?;
                if target_file_ids.is_empty() {
                    target_file_ids = self.file_ids_for_module_name(module_name)?;
                }

                if let Some(symbol_id) =
                    self.unique_symbol_id_in_files(&target_file_ids, symbol_name)?
                {
                    return Ok(Some((symbol_id, Certainty::Resolved)));
                }
            }

            return Ok(None);
        }

        if let Some(symbol_id) = self.unique_symbol_id_in_file(file_id, &call_site.callee_name)? {
            return Ok(Some((symbol_id, Certainty::Exact)));
        }

        if let Some(symbol_id) =
            self.unique_imported_symbol_id(file_id, imports, &call_site.callee_name)?
        {
            return Ok(Some((symbol_id, Certainty::Resolved)));
        }

        Ok(None)
    }

    fn imported_file_ids_for_module(
        &self,
        file_id: i64,
        imports: &[crate::ImportRecord],
        module_name: &str,
    ) -> ScopeResult<Vec<i64>> {
        let mut file_ids = Vec::new();
        for import in imports {
            let Some(imported_name) = import.raw_text.rsplit("::").next() else {
                continue;
            };
            let imported_name = imported_name
                .trim_end_matches(';')
                .trim()
                .trim_start_matches('{')
                .trim_end_matches('}')
                .split_whitespace()
                .last()
                .unwrap_or_default();

            if imported_name != module_name {
                continue;
            }

            if let ImportPath::Relative(path) = &import.import_path {
                if let Some(target_file_id) = self.file_id(path)? {
                    file_ids.push(target_file_id);
                }
            }
        }

        if file_ids.is_empty() {
            let mut statement = self.connection.prepare(
                "SELECT to_file_id FROM file_edges
                 WHERE from_file_id = ?1 AND kind = 'module'",
            )?;
            let rows = statement.query_map([file_id], |row| row.get(0))?;
            let mut filtered_file_ids = Vec::new();
            for row in rows {
                let candidate_file_id: i64 = row?;
                let path = self.file_path_for_id(candidate_file_id)?;
                let file_name_matches = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == format!("{module_name}.rs") || name == "mod.rs");
                let parent_matches = path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == module_name);
                if file_name_matches || parent_matches {
                    filtered_file_ids.push(candidate_file_id);
                }
            }
            file_ids = filtered_file_ids;
        }

        file_ids.sort_unstable();
        file_ids.dedup();
        Ok(file_ids)
    }

    fn file_ids_for_module_name(&self, module_name: &str) -> ScopeResult<Vec<i64>> {
        let mut statement = self.connection.prepare(
            "SELECT id, path FROM files WHERE path LIKE ?1 OR path LIKE ?2 ORDER BY path ASC",
        )?;
        let rows = statement.query_map(
            params![
                format!("%/{module_name}.rs"),
                format!("%/{module_name}/mod.rs")
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )?;

        let mut file_ids = Vec::new();
        for row in rows {
            let (id, _path) = row?;
            file_ids.push(id);
        }
        Ok(file_ids)
    }

    fn unique_symbol_id_in_file(
        &self,
        file_id: i64,
        symbol_name: &str,
    ) -> ScopeResult<Option<i64>> {
        self.unique_symbol_id_in_files(&[file_id], symbol_name)
    }

    fn unique_imported_symbol_id(
        &self,
        file_id: i64,
        imports: &[crate::ImportRecord],
        symbol_name: &str,
    ) -> ScopeResult<Option<i64>> {
        let mut target_file_ids = Vec::new();
        for import in imports {
            if !import_mentions_symbol(&import.raw_text, symbol_name) {
                continue;
            }

            if let ImportPath::Relative(path) = &import.import_path {
                if let Some(target_file_id) = self.file_id(path)? {
                    target_file_ids.push(target_file_id);
                    target_file_ids
                        .extend(self.reexport_target_file_ids(target_file_id, symbol_name)?);
                }
            }
        }

        if target_file_ids.is_empty() {
            let mut statement = self.connection.prepare(
                "SELECT to_file_id FROM file_edges WHERE from_file_id = ?1 AND kind = 'import'",
            )?;
            let rows = statement.query_map([file_id], |row| row.get(0))?;
            for row in rows {
                target_file_ids.push(row?);
            }
        }

        target_file_ids.sort_unstable();
        target_file_ids.dedup();
        self.unique_symbol_id_in_files(&target_file_ids, symbol_name)
    }

    fn reexport_target_file_ids(&self, file_id: i64, symbol_name: &str) -> ScopeResult<Vec<i64>> {
        let mut statement = self.connection.prepare(
            "SELECT imports.resolved_file_id, imports.raw_text
             FROM imports
             WHERE imports.file_id = ?1 AND imports.resolved_file_id IS NOT NULL",
        )?;
        let rows = statement.query_map([file_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut targets = Vec::new();
        for row in rows {
            let (target_file_id, raw_text) = row?;
            if raw_text.starts_with("export ") && import_mentions_symbol(&raw_text, symbol_name) {
                targets.push(target_file_id);
            }
        }
        Ok(targets)
    }

    fn unique_symbol_id_in_files(
        &self,
        file_ids: &[i64],
        symbol_name: &str,
    ) -> ScopeResult<Option<i64>> {
        let mut matches = Vec::new();
        for file_id in file_ids {
            let mut statement = self.connection.prepare(
                "SELECT id FROM symbols
                 WHERE file_id = ?1 AND name = ?2 AND kind = 'function'",
            )?;
            let rows = statement.query_map(params![file_id, symbol_name], |row| row.get(0))?;
            for row in rows {
                matches.push(row?);
            }
        }

        matches.sort_unstable();
        matches.dedup();
        if matches.len() == 1 {
            Ok(matches.into_iter().next())
        } else {
            Ok(None)
        }
    }

    fn file_path_for_id(&self, file_id: i64) -> ScopeResult<std::path::PathBuf> {
        let path: String = self.connection.query_row(
            "SELECT path FROM files WHERE id = ?1",
            [file_id],
            |row| row.get(0),
        )?;
        Ok(std::path::PathBuf::from(path))
    }

    fn count_rows(&self, table: &str) -> ScopeResult<usize> {
        self.count_query(&format!("SELECT COUNT(*) FROM {table}"))
    }

    fn count_query(&self, query: &str) -> ScopeResult<usize> {
        self.connection
            .query_row(query, [], |row| row.get::<_, i64>(0))
            .map(|count| count as usize)
            .map_err(Into::into)
    }

    fn insert_symbol_edge(
        &self,
        from_symbol_id: i64,
        to_symbol_id: i64,
        kind: &str,
        certainty: &str,
        call_line: i64,
    ) -> ScopeResult<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO symbol_edges (from_symbol_id, to_symbol_id, kind, certainty, call_line)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![from_symbol_id, to_symbol_id, kind, certainty, call_line],
        )?;
        Ok(())
    }

    fn query_symbol_edges(
        &self,
        symbol_qualname: &str,
        reverse: bool,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        let Some(symbol_id) = self.symbol_id(symbol_qualname)? else {
            return Ok(Vec::new());
        };

        let (join_column, filter_column, reason_prefix) = if reverse {
            ("from_symbol_id", "to_symbol_id", "called directly by")
        } else {
            ("to_symbol_id", "from_symbol_id", "calls")
        };

        let query = format!(
            "SELECT files.path, symbols.qualname, symbol_edges.certainty
             FROM symbol_edges
             JOIN symbols ON symbols.id = symbol_edges.{join_column}
             JOIN files ON files.id = symbols.file_id
             WHERE symbol_edges.{filter_column} = ?1 AND symbol_edges.kind = 'call'
             ORDER BY symbols.qualname ASC"
        );
        let mut statement = self.connection.prepare(&query)?;
        let rows = statement.query_map([symbol_id], |row| {
            Ok(TraversalRecord {
                kind: NodeKind::Symbol,
                path: Some(RepoPath(row.get::<_, String>(0)?)),
                qualname: Some(row.get(1)?),
                edge_kind: EdgeKind::Call,
                certainty: certainty_from_db(&row.get::<_, String>(2)?),
                reason: format!("{reason_prefix} {symbol_qualname}"),
                distance: 1,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn file_id_for_symbol(&self, symbol_id: i64) -> ScopeResult<Option<i64>> {
        self.connection
            .query_row(
                "SELECT file_id FROM symbols WHERE id = ?1",
                [symbol_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    fn symbol_record_by_id(&self, symbol_id: i64) -> ScopeResult<Option<SymbolRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT files.path, symbols.name, symbols.qualname, symbols.kind, symbols.visibility, symbols.exported, symbols.span_start, symbols.span_end, symbols.start_line, symbols.end_line
             FROM symbols
             JOIN files ON files.id = symbols.file_id
             WHERE symbols.id = ?1",
        )?;
        statement
            .query_row([symbol_id], |row| {
                Ok(SymbolRecord {
                    file: RepoPath(row.get::<_, String>(0)?),
                    name: row.get(1)?,
                    qualname: row.get(2)?,
                    kind: symbol_kind_from_db(&row.get::<_, String>(3)?),
                    visibility: visibility_from_db(&row.get::<_, String>(4)?),
                    exported: row.get::<_, i64>(5)? != 0,
                    span: crate::Span {
                        start_byte: row.get::<_, i64>(6)? as u32,
                        end_byte: row.get::<_, i64>(7)? as u32,
                        start_line: row.get::<_, i64>(8)? as u32,
                        end_line: row.get::<_, i64>(9)? as u32,
                    },
                })
            })
            .optional()
            .map_err(Into::into)
    }

    fn collect_symbol_import_rename_steps(
        &self,
        symbol: &SymbolRecord,
        old_name: &str,
        new_name: &str,
    ) -> ScopeResult<Vec<RenamePlanStep>> {
        let Some(file_id) = self.file_id(&symbol.file)? else {
            return Ok(Vec::new());
        };
        let mut steps = Vec::new();
        let mut statement = self.connection.prepare(
            "SELECT files.path, imports.raw_text, imports.span_start, imports.span_end, imports.start_line, imports.certainty
             FROM imports
             JOIN files ON files.id = imports.file_id
             WHERE imports.resolved_file_id = ?1
             ORDER BY files.path ASC, imports.start_line ASC",
        )?;
        let rows = statement.query_map([file_id], |row| {
            Ok((
                RepoPath(row.get::<_, String>(0)?),
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as u32,
                row.get::<_, i64>(3)? as u32,
                row.get::<_, i64>(4)? as u32,
                certainty_from_db(&row.get::<_, String>(5)?),
            ))
        })?;

        for row in rows {
            let (path, raw_text, span_start, _span_end, start_line, certainty) = row?;
            if let Some(edit) = rename_edit_from_import(&raw_text, span_start, start_line, old_name, new_name) {
                let mut reasons = vec![format!(
                    "import statement references defining file {}",
                    symbol.file.0
                )];
                if raw_text.starts_with("export ") {
                    reasons.push(format!("re-export mentions symbol {}", symbol.qualname));
                }
                steps.push(RenamePlanStep {
                    path,
                    distance: 1,
                    certainty,
                    roles: vec!["importer".to_string()],
                    reasons,
                    edits: vec![edit],
                    apply_safe: true,
                });
            }
        }

        Ok(steps)
    }

    fn collect_file_import_path_steps(
        &self,
        target: &RepoPath,
        old_name: &str,
        new_name: &str,
    ) -> ScopeResult<Vec<RenamePlanStep>> {
        let Some(file_id) = self.file_id(target)? else {
            return Ok(Vec::new());
        };
        let mut steps = Vec::new();
        let mut statement = self.connection.prepare(
            "SELECT files.path, imports.raw_text, imports.span_start, imports.start_line, imports.certainty
             FROM imports
             JOIN files ON files.id = imports.file_id
             WHERE imports.resolved_file_id = ?1
             ORDER BY files.path ASC, imports.start_line ASC",
        )?;
        let rows = statement.query_map([file_id], |row| {
            Ok((
                RepoPath(row.get::<_, String>(0)?),
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)? as u32,
                row.get::<_, i64>(3)? as u32,
                certainty_from_db(&row.get::<_, String>(4)?),
            ))
        })?;

        for row in rows {
            let (path, raw_text, span_start, start_line, certainty) = row?;
            if let Some(edit) = rename_edit_from_import_path(&raw_text, span_start, start_line, old_name, new_name) {
                steps.push(RenamePlanStep {
                    path,
                    distance: 1,
                    certainty,
                    roles: vec!["importer".to_string()],
                    reasons: vec![format!("import path resolves to target file {}", target.0)],
                    edits: vec![edit],
                    apply_safe: true,
                });
            }
        }

        Ok(steps)
    }

    fn collect_symbol_deferred_steps(&self, qualname: &str) -> ScopeResult<Vec<RenamePlanStep>> {
        let mut steps = Vec::new();
        for traversal in self.query_callers(qualname, false)? {
            let Some(path) = traversal.path else {
                continue;
            };
            steps.push(RenamePlanStep {
                path,
                distance: traversal.distance,
                certainty: traversal.certainty,
                roles: vec!["direct_caller".to_string()],
                reasons: vec![traversal.reason],
                edits: vec![RenameEdit {
                    start_byte: 0,
                    end_byte: 0,
                    line: 0,
                    before_text: qualname.rsplit("::").next().unwrap_or_default().to_string(),
                    after_text: String::new(),
                    kind: RenameEditKind::DeferredCallSite,
                    verified: false,
                    deferred_reason: Some(
                        "call-site token spans are not persisted in the index yet".to_string(),
                    ),
                }],
                apply_safe: false,
            });
        }
        Ok(steps)
    }

    fn apply_rename_plan_steps(
        &self,
        repo_root: &Path,
        steps: &[RenamePlanStep],
    ) -> ScopeResult<(usize, usize)> {
        let mut per_file: HashMap<RepoPath, Vec<RenameEdit>> = HashMap::new();
        for step in steps {
            for edit in &step.edits {
                if edit.verified {
                    per_file.entry(step.path.clone()).or_default().push(edit.clone());
                }
            }
        }

        let mut applied_files = 0usize;
        let mut applied_edits = 0usize;
        let mut paths: Vec<_> = per_file.into_iter().collect();
        paths.sort_by(|left, right| left.0.cmp(&right.0));

        for (path, edits) in paths {
            let absolute_path = repo_root.join(&path.0);
            let source = std::fs::read_to_string(&absolute_path)
                .map_err(|error| ScopeError::io(&absolute_path, error))?;
            let updated = apply_rename_edits_to_source(&source, &edits)?;
            if updated != source {
                write_updated_source_atomically(&absolute_path, &updated)?;
                applied_files += 1;
                applied_edits += edits.len();
            }
        }

        Ok((applied_files, applied_edits))
    }

    fn traverse_reverse_callers(
        &self,
        start_symbol_id: i64,
        target_label: &str,
        max_depth: u32,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if max_depth == 0 {
            return Ok(Vec::new());
        }

        let mut visited = HashSet::from([start_symbol_id]);
        let mut queue = VecDeque::from([(start_symbol_id, 0u32)]);
        let mut traversals = Vec::new();

        while let Some((symbol_id, distance)) = queue.pop_front() {
            if distance >= max_depth {
                continue;
            }

            let mut statement = self.connection.prepare(
                "SELECT caller_symbols.id, caller_files.path, caller_symbols.qualname, symbol_edges.certainty
                 FROM symbol_edges
                 JOIN symbols AS caller_symbols ON caller_symbols.id = symbol_edges.from_symbol_id
                 JOIN files AS caller_files ON caller_files.id = caller_symbols.file_id
                 WHERE symbol_edges.to_symbol_id = ?1 AND symbol_edges.kind = 'call'
                 ORDER BY caller_symbols.qualname ASC",
            )?;
            let rows = statement.query_map([symbol_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;

            for row in rows {
                let (caller_id, caller_path, caller_qualname, certainty) = row?;
                let hop_distance = distance + 1;
                traversals.push(TraversalRecord {
                    kind: NodeKind::Symbol,
                    path: Some(RepoPath(caller_path)),
                    qualname: Some(caller_qualname.clone()),
                    edge_kind: EdgeKind::Call,
                    certainty: certainty_from_db(&certainty),
                    reason: if hop_distance == 1 {
                        format!("calls {target_label} directly")
                    } else {
                        format!("calls a symbol that reaches {target_label}")
                    },
                    distance: hop_distance,
                });
                if visited.insert(caller_id) {
                    queue.push_back((caller_id, hop_distance));
                }
            }
        }

        Ok(traversals)
    }

    fn traverse_forward_callees(
        &self,
        start_symbol_id: i64,
        target_label: &str,
        max_depth: u32,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if max_depth == 0 {
            return Ok(Vec::new());
        }

        let mut visited = HashSet::from([start_symbol_id]);
        let mut queue = VecDeque::from([(start_symbol_id, 0u32)]);
        let mut traversals = Vec::new();

        while let Some((symbol_id, distance)) = queue.pop_front() {
            if distance >= max_depth {
                continue;
            }

            let mut statement = self.connection.prepare(
                "SELECT callee_symbols.id, callee_files.path, callee_symbols.qualname, symbol_edges.certainty
                 FROM symbol_edges
                 JOIN symbols AS callee_symbols ON callee_symbols.id = symbol_edges.to_symbol_id
                 JOIN files AS callee_files ON callee_files.id = callee_symbols.file_id
                 WHERE symbol_edges.from_symbol_id = ?1 AND symbol_edges.kind = 'call'
                 ORDER BY callee_symbols.qualname ASC",
            )?;
            let rows = statement.query_map([symbol_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;

            for row in rows {
                let (callee_id, callee_path, callee_qualname, certainty) = row?;
                let hop_distance = distance + 1;
                traversals.push(TraversalRecord {
                    kind: NodeKind::Symbol,
                    path: Some(RepoPath(callee_path)),
                    qualname: Some(callee_qualname.clone()),
                    edge_kind: EdgeKind::Call,
                    certainty: certainty_from_db(&certainty),
                    reason: if hop_distance == 1 {
                        format!("is called by {target_label}")
                    } else {
                        format!("is downstream of {target_label} in the call graph")
                    },
                    distance: hop_distance,
                });
                if visited.insert(callee_id) {
                    queue.push_back((callee_id, hop_distance));
                }
            }
        }

        Ok(traversals)
    }

    fn traverse_reverse_importers(
        &self,
        start_file_id: i64,
        target_label: &str,
        max_depth: u32,
        import_only_reason: bool,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if max_depth == 0 {
            return Ok(Vec::new());
        }

        let mut visited = HashSet::from([start_file_id]);
        let mut queue = VecDeque::from([(start_file_id, 0u32)]);
        let mut traversals = Vec::new();

        while let Some((file_id, distance)) = queue.pop_front() {
            if distance >= max_depth {
                continue;
            }

            let mut statement = self.connection.prepare(
                "SELECT importer_files.id, importer_files.path, file_edges.certainty
                 FROM file_edges
                 JOIN files AS importer_files ON importer_files.id = file_edges.from_file_id
                 WHERE file_edges.to_file_id = ?1 AND file_edges.kind = 'import'
                 ORDER BY importer_files.path ASC",
            )?;
            let rows = statement.query_map([file_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;

            for row in rows {
                let (importer_id, importer_path, certainty) = row?;
                let hop_distance = distance + 1;
                traversals.push(TraversalRecord {
                    kind: NodeKind::File,
                    path: Some(RepoPath(importer_path.clone())),
                    qualname: None,
                    edge_kind: EdgeKind::Import,
                    certainty: certainty_from_db(&certainty),
                    reason: if hop_distance == 1 || import_only_reason {
                        format!("imports {target_label}")
                    } else {
                        format!("imports a file that reaches {target_label}")
                    },
                    distance: hop_distance,
                });
                if visited.insert(importer_id) {
                    queue.push_back((importer_id, hop_distance));
                }
            }
        }

        Ok(traversals)
    }

    fn shortest_file_path(
        &self,
        start_file_id: i64,
        start_label: &str,
        goal_file_id: i64,
        _goal_label: &str,
        max_depth: u32,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if max_depth == 0 {
            return Ok(Vec::new());
        }

        let mut visited = HashSet::from([start_file_id]);
        let mut queue = VecDeque::from([(start_file_id, 0u32)]);
        let mut predecessors = std::collections::HashMap::<i64, (i64, RepoPath, Certainty)>::new();

        while let Some((file_id, distance)) = queue.pop_front() {
            if distance >= max_depth {
                continue;
            }

            let mut statement = self.connection.prepare(
                "SELECT dependency_files.id, dependency_files.path, file_edges.certainty
                 FROM file_edges
                 JOIN files AS dependency_files ON dependency_files.id = file_edges.to_file_id
                 WHERE file_edges.from_file_id = ?1 AND file_edges.kind = 'import'
                 ORDER BY dependency_files.path ASC",
            )?;
            let rows = statement.query_map([file_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    RepoPath(row.get::<_, String>(1)?),
                    certainty_from_db(&row.get::<_, String>(2)?),
                ))
            })?;

            for row in rows {
                let (next_id, next_path, certainty) = row?;
                if visited.insert(next_id) {
                    predecessors.insert(next_id, (file_id, next_path.clone(), certainty));
                    if next_id == goal_file_id {
                        return self.reconstruct_file_path(
                            predecessors,
                            start_file_id,
                            start_label,
                            goal_file_id,
                        );
                    }
                    queue.push_back((next_id, distance + 1));
                }
            }
        }

        Ok(Vec::new())
    }

    fn shortest_symbol_path(
        &self,
        start_symbol_id: i64,
        start_label: &str,
        goal_symbol_id: i64,
        _goal_label: &str,
        max_depth: u32,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        if max_depth == 0 {
            return Ok(Vec::new());
        }

        let mut visited = HashSet::from([start_symbol_id]);
        let mut queue = VecDeque::from([(start_symbol_id, 0u32)]);
        let mut predecessors =
            std::collections::HashMap::<i64, (i64, RepoPath, String, Certainty)>::new();

        while let Some((symbol_id, distance)) = queue.pop_front() {
            if distance >= max_depth {
                continue;
            }

            let mut statement = self.connection.prepare(
                "SELECT callee_symbols.id, callee_files.path, callee_symbols.qualname, symbol_edges.certainty
                 FROM symbol_edges
                 JOIN symbols AS callee_symbols ON callee_symbols.id = symbol_edges.to_symbol_id
                 JOIN files AS callee_files ON callee_files.id = callee_symbols.file_id
                 WHERE symbol_edges.from_symbol_id = ?1 AND symbol_edges.kind = 'call'
                 ORDER BY callee_symbols.qualname ASC",
            )?;
            let rows = statement.query_map([symbol_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    RepoPath(row.get::<_, String>(1)?),
                    row.get::<_, String>(2)?,
                    certainty_from_db(&row.get::<_, String>(3)?),
                ))
            })?;

            for row in rows {
                let (next_id, next_path, next_qualname, certainty) = row?;
                if visited.insert(next_id) {
                    predecessors.insert(
                        next_id,
                        (
                            symbol_id,
                            next_path.clone(),
                            next_qualname.clone(),
                            certainty,
                        ),
                    );
                    if next_id == goal_symbol_id {
                        return self.reconstruct_symbol_path(
                            predecessors,
                            start_symbol_id,
                            start_label,
                            goal_symbol_id,
                        );
                    }
                    queue.push_back((next_id, distance + 1));
                }
            }
        }

        Ok(Vec::new())
    }

    fn reconstruct_file_path(
        &self,
        predecessors: std::collections::HashMap<i64, (i64, RepoPath, Certainty)>,
        start_file_id: i64,
        start_label: &str,
        goal_file_id: i64,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        let mut current_id = goal_file_id;
        let mut reversed = Vec::new();

        while current_id != start_file_id {
            let Some((previous_id, path, certainty)) = predecessors.get(&current_id) else {
                return Ok(Vec::new());
            };
            reversed.push((current_id, *previous_id, path.clone(), certainty.clone()));
            current_id = *previous_id;
        }

        reversed.reverse();
        let mut previous_label = start_label.to_string();
        let mut path = Vec::with_capacity(reversed.len());

        for (index, (_current_id, _previous_id, next_path, certainty)) in
            reversed.into_iter().enumerate()
        {
            path.push(TraversalRecord {
                kind: NodeKind::File,
                path: Some(next_path.clone()),
                qualname: None,
                edge_kind: EdgeKind::Import,
                certainty,
                reason: format!("imported by {previous_label}"),
                distance: (index + 1) as u32,
            });
            previous_label = next_path.0;
        }

        Ok(path)
    }

    fn reconstruct_symbol_path(
        &self,
        predecessors: std::collections::HashMap<i64, (i64, RepoPath, String, Certainty)>,
        start_symbol_id: i64,
        start_label: &str,
        goal_symbol_id: i64,
    ) -> ScopeResult<Vec<TraversalRecord>> {
        let mut current_id = goal_symbol_id;
        let mut reversed = Vec::new();

        while current_id != start_symbol_id {
            let Some((previous_id, path, qualname, certainty)) = predecessors.get(&current_id)
            else {
                return Ok(Vec::new());
            };
            reversed.push((
                current_id,
                *previous_id,
                path.clone(),
                qualname.clone(),
                certainty.clone(),
            ));
            current_id = *previous_id;
        }

        reversed.reverse();
        let mut previous_label = start_label.to_string();
        let mut path = Vec::with_capacity(reversed.len());

        for (index, (_current_id, _previous_id, next_path, next_qualname, certainty)) in
            reversed.into_iter().enumerate()
        {
            path.push(TraversalRecord {
                kind: NodeKind::Symbol,
                path: Some(next_path),
                qualname: Some(next_qualname.clone()),
                edge_kind: EdgeKind::Call,
                certainty,
                reason: format!("called by {previous_label}"),
                distance: (index + 1) as u32,
            });
            previous_label = next_qualname;
        }

        Ok(path)
    }

    fn insert_symbol(&self, file_id: i64, symbol: &SymbolRecord) -> ScopeResult<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO symbols (
                file_id, name, qualname, kind, visibility, exported, span_start, span_end, start_line, end_line
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                file_id,
                symbol.name,
                symbol.qualname,
                symbol_kind_name(&symbol.kind),
                visibility_name(&symbol.visibility),
                symbol.exported as i64,
                symbol.span.start_byte,
                symbol.span.end_byte,
                symbol.span.start_line,
                symbol.span.end_line,
            ],
        )?;
        Ok(())
    }

    fn insert_file_edge(
        &self,
        from_file_id: i64,
        to_file_id: i64,
        kind: &str,
        certainty: &str,
    ) -> ScopeResult<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO file_edges (from_file_id, to_file_id, kind, certainty)
             VALUES (?1, ?2, ?3, ?4)",
            params![from_file_id, to_file_id, kind, certainty],
        )?;
        Ok(())
    }
}

fn configure_connection(connection: &Connection, db_path: &Path) -> ScopeResult<()> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| ScopeError::database(db_path, error))?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| ScopeError::database(db_path, error))?;
    Ok(())
}

fn current_user_version(connection: &Connection) -> ScopeResult<u32> {
    let version = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

fn run_migrations(connection: &Connection) -> ScopeResult<()> {
    let current_version = current_user_version(connection)?;
    if current_version > INDEX_SCHEMA_VERSION {
        return Err(ScopeError::Migration(format!(
            "database schema version {current_version} is newer than supported version {INDEX_SCHEMA_VERSION}"
        )));
    }

    if current_version < 1 {
        connection.execute_batch(INITIAL_MIGRATION)?;
        connection.pragma_update(None, "user_version", 1)?;
    }

    if current_version < 2 {
        connection.execute_batch(SYMBOLS_MIGRATION)?;
        connection.pragma_update(None, "user_version", 2)?;
    }

    if current_version < 3 {
        connection.execute_batch(SYMBOL_EDGES_MIGRATION)?;
        connection.pragma_update(None, "user_version", 3)?;
    }

    if current_version < 4 {
        connection.pragma_update(None, "user_version", 4)?;
    }

    if current_version < 5 {
        connection.execute_batch(FILE_CHURN_MIGRATION)?;
        connection.pragma_update(None, "user_version", 5)?;
    }

    if current_version < 6 {
        connection.execute_batch(SNAPSHOTS_MIGRATION)?;
        connection.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;
    }

    reconcile_schema(connection)?;
    connection.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;

    Ok(())
}

fn reconcile_schema(connection: &Connection) -> ScopeResult<()> {
    if !has_required_tables(
        connection,
        &["index_meta", "files", "imports", "file_edges"],
    )? {
        connection.execute_batch(INITIAL_MIGRATION)?;
    }

    if !has_required_tables(connection, &["symbols"])? {
        connection.execute_batch(SYMBOLS_MIGRATION)?;
    }

    if !has_required_tables(connection, &["symbol_edges"])? {
        connection.execute_batch(SYMBOL_EDGES_MIGRATION)?;
    }

    if !has_required_tables(connection, &["file_churn"])? {
        connection.execute_batch(FILE_CHURN_MIGRATION)?;
    }

    if !has_required_tables(connection, &["snapshots"])? {
        connection.execute_batch(SNAPSHOTS_MIGRATION)?;
    }

    add_file_metadata_columns(connection)?;

    Ok(())
}

fn edits_planned_file_count(
    resolved: &RenameTarget,
    steps: &[RenamePlanStep],
    skipped: &[RenamePlanStep],
) -> usize {
    let mut files = HashSet::new();
    match resolved {
        RenameTarget::Symbol { symbol, .. } => {
            files.insert(symbol.file.clone());
        }
        RenameTarget::File { path } => {
            files.insert(path.clone());
        }
    }
    for step in steps {
        files.insert(step.path.clone());
    }
    for step in skipped {
        files.insert(step.path.clone());
    }
    files.len()
}

fn file_stem_name(path: &RepoPath) -> ScopeResult<String> {
    std::path::Path::new(&path.0)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            ScopeError::InvalidInput(format!(
                "rename-plan could not derive file name from target: {}",
                path.0
            ))
        })
}

fn definition_edit_from_symbol(
    symbol: &SymbolRecord,
    source: &str,
    new_name: &str,
) -> Option<RenameEdit> {
    let start = symbol.span.start_byte as usize;
    let end = symbol.span.end_byte as usize;
    let line = source.get(start..end)?;
    let offset = identifier_match_offset(line, &symbol.name)?;
    Some(RenameEdit {
        start_byte: symbol.span.start_byte + offset as u32,
        end_byte: symbol.span.start_byte + (offset + symbol.name.len()) as u32,
        line: symbol.span.start_line,
        before_text: symbol.name.clone(),
        after_text: new_name.to_string(),
        kind: RenameEditKind::Definition,
        verified: true,
        deferred_reason: None,
    })
}

fn rename_edit_from_import(
    raw_text: &str,
    span_start: u32,
    start_line: u32,
    old_name: &str,
    new_name: &str,
) -> Option<RenameEdit> {
    identifier_match_offset(raw_text, old_name).map(|offset| RenameEdit {
        start_byte: span_start + offset as u32,
        end_byte: span_start + (offset + old_name.len()) as u32,
        line: start_line,
        before_text: old_name.to_string(),
        after_text: new_name.to_string(),
        kind: if raw_text.starts_with("export ") {
            RenameEditKind::ImportSpecifier
        } else {
            RenameEditKind::ImportSpecifier
        },
        verified: true,
        deferred_reason: None,
    })
}

fn rename_edit_from_import_path(
    raw_text: &str,
    span_start: u32,
    start_line: u32,
    old_name: &str,
    new_name: &str,
) -> Option<RenameEdit> {
    raw_text.find(old_name).map(|offset| RenameEdit {
        start_byte: span_start + offset as u32,
        end_byte: span_start + (offset + old_name.len()) as u32,
        line: start_line,
        before_text: old_name.to_string(),
        after_text: new_name.to_string(),
        kind: RenameEditKind::ImportPath,
        verified: true,
        deferred_reason: None,
    })
}

fn identifier_match_offset(source: &str, needle: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut index = 0usize;
    while index + needle_bytes.len() <= bytes.len() {
        if &bytes[index..index + needle_bytes.len()] == needle_bytes
            && is_identifier_boundary(bytes, index.checked_sub(1).unwrap_or(usize::MAX), true)
            && is_identifier_boundary(bytes, index + needle_bytes.len(), false)
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn is_identifier_boundary(bytes: &[u8], index: usize, is_left: bool) -> bool {
    if is_left {
        if index == usize::MAX || index >= bytes.len() {
            return true;
        }
        !is_identifier_byte(bytes[index])
    } else {
        if index >= bytes.len() {
            return true;
        }
        !is_identifier_byte(bytes[index])
    }
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$'
}

fn apply_rename_edits_to_source(source: &str, edits: &[RenameEdit]) -> ScopeResult<String> {
    let mut ordered = edits.to_vec();
    ordered.sort_by(|left, right| right.start_byte.cmp(&left.start_byte));
    let mut updated = source.to_string();
    for edit in ordered {
        let range = edit.start_byte as usize..edit.end_byte as usize;
        if updated.get(range.clone()) != Some(edit.before_text.as_str()) {
            return Err(ScopeError::InvalidInput(format!(
                "rename-plan apply mismatch at {}..{}",
                edit.start_byte, edit.end_byte
            )));
        }
        updated.replace_range(range, &edit.after_text);
    }
    Ok(updated)
}

fn write_updated_source_atomically(path: &Path, updated: &str) -> ScopeResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ScopeError::InvalidInput(format!("path has no parent: {}", path.display())))?;
    let temp_path = parent.join(format!(
        ".scope-rename-{}.tmp",
        std::process::id()
    ));
    std::fs::write(&temp_path, updated).map_err(|error| ScopeError::io(&temp_path, error))?;
    std::fs::rename(&temp_path, path).map_err(|error| ScopeError::io(path, error))?;
    Ok(())
}

fn has_required_tables(connection: &Connection, tables: &[&str]) -> ScopeResult<bool> {
    for table in tables {
        if !table_exists(connection, table)? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn table_exists(connection: &Connection, table: &str) -> ScopeResult<bool> {
    let exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(exists != 0)
}

fn add_file_metadata_columns(connection: &Connection) -> ScopeResult<()> {
    for (column, statement) in FILE_METADATA_MIGRATION_COLUMNS {
        if !table_has_column(connection, "files", column)? {
            connection.execute(statement, [])?;
        }
    }

    Ok(())
}

fn table_has_column(connection: &Connection, table: &str, column: &str) -> ScopeResult<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;

    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }

    Ok(false)
}

fn bootstrap_meta(connection: &Connection) -> ScopeResult<()> {
    connection.execute(
        "INSERT OR REPLACE INTO index_meta (key, value) VALUES ('schema_version', ?1)",
        [INDEX_SCHEMA_VERSION.to_string()],
    )?;
    Ok(())
}

fn parse_status_name(status: &crate::ParseStatus) -> &'static str {
    match status {
        crate::ParseStatus::Ok => "ok",
        crate::ParseStatus::Partial => "partial",
        crate::ParseStatus::Error => "error",
    }
}

fn certainty_name(certainty: &Certainty) -> &'static str {
    match certainty {
        Certainty::Exact => "exact",
        Certainty::Resolved => "resolved",
        Certainty::Heuristic => "heuristic",
        Certainty::Dynamic => "dynamic",
    }
}

fn certainty_from_db(value: &str) -> Certainty {
    match value {
        "exact" => Certainty::Exact,
        "resolved" => Certainty::Resolved,
        "dynamic" => Certainty::Dynamic,
        _ => Certainty::Heuristic,
    }
}

fn symbol_kind_name(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Struct => "struct",
        SymbolKind::Class => "class",
        SymbolKind::Enum => "enum",
        SymbolKind::TypeAlias => "type_alias",
        SymbolKind::Module => "module",
        SymbolKind::Namespace => "namespace",
        SymbolKind::Constant => "constant",
        SymbolKind::Static => "static",
        SymbolKind::Interface => "interface",
        SymbolKind::Trait => "trait",
        SymbolKind::Variable => "variable",
    }
}

fn symbol_kind_from_db(value: &str) -> SymbolKind {
    match value {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "struct" => SymbolKind::Struct,
        "class" => SymbolKind::Class,
        "enum" => SymbolKind::Enum,
        "type_alias" => SymbolKind::TypeAlias,
        "module" => SymbolKind::Module,
        "namespace" => SymbolKind::Namespace,
        "constant" => SymbolKind::Constant,
        "static" => SymbolKind::Static,
        "interface" => SymbolKind::Interface,
        "trait" => SymbolKind::Trait,
        _ => SymbolKind::Variable,
    }
}

fn visibility_name(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Local => "local",
        Visibility::Module => "module",
        Visibility::Package => "package",
        Visibility::Public => "public",
        Visibility::Unknown => "unknown",
    }
}

fn all_files_set(records: &[StabilityRecord]) -> HashSet<RepoPath> {
    records.iter().map(|record| record.path.clone()).collect()
}

fn stability_records_from_file_edges(
    all_files: &[RepoPath],
    edges: &[ArchFileEdge],
    flag_threshold: Option<f64>,
) -> Vec<StabilityRecord> {
    let mut fan_in: HashMap<RepoPath, usize> = HashMap::new();
    let mut fan_out: HashMap<RepoPath, usize> = HashMap::new();

    for edge in edges.iter().filter(|edge| edge.edge_kind == EdgeKind::Import) {
        *fan_out.entry(edge.from_file.clone()).or_default() += 1;
        *fan_in.entry(edge.to_file.clone()).or_default() += 1;
    }

    all_files
        .iter()
        .cloned()
        .map(|path| {
            let incoming = fan_in.get(&path).copied().unwrap_or(0);
            let outgoing = fan_out.get(&path).copied().unwrap_or(0);
            let total = incoming + outgoing;
            let instability = if total == 0 {
                0.0
            } else {
                outgoing as f64 / total as f64
            };
            let category = stability_category(incoming, outgoing, instability);
            let flagged = incoming > 10 && instability > flag_threshold.unwrap_or(0.5);
            let reason = stability_reason(incoming, outgoing, instability, &category, flagged);

            StabilityRecord {
                path,
                fan_in: incoming,
                fan_out: outgoing,
                instability,
                category,
                flagged,
                reason,
            }
        })
        .collect()
}

fn stability_category(fan_in: usize, fan_out: usize, instability: f64) -> StabilityCategory {
    if fan_in == 0 && fan_out == 0 {
        StabilityCategory::Isolated
    } else if instability > 0.5 && fan_in > 10 {
        StabilityCategory::UnstableAndCentral
    } else if instability < 0.2 && fan_in > 5 {
        StabilityCategory::StableAbstraction
    } else if fan_in == 0 && instability >= 0.95 {
        StabilityCategory::HealthyLeaf
    } else if (0.35..=0.65).contains(&instability) {
        StabilityCategory::Balanced
    } else {
        StabilityCategory::Stable
    }
}

fn stability_reason(
    fan_in: usize,
    fan_out: usize,
    instability: f64,
    category: &StabilityCategory,
    flagged: bool,
) -> Option<String> {
    if flagged {
        Some(format!(
            "high fan-in ({fan_in}) but also high instability ({instability:.2}) — structural liability"
        ))
    } else {
        match category {
            StabilityCategory::StableAbstraction => Some(format!(
                "low instability ({instability:.2}) with high fan-in ({fan_in}) — shared stable abstraction"
            )),
            StabilityCategory::HealthyLeaf => Some(format!(
                "no downstream dependents and fan-out {fan_out} — healthy leaf node"
            )),
            StabilityCategory::Isolated => Some("no direct imports and no dependents — isolated file".to_string()),
            _ => None,
        }
    }
}

fn stability_summary(records: &[StabilityRecord]) -> StabilitySummary {
    let avg_instability = if records.is_empty() {
        0.0
    } else {
        records.iter().map(|record| record.instability).sum::<f64>() / records.len() as f64
    };

    let mut summary = StabilitySummary {
        avg_instability,
        flagged_count: 0,
        stable_count: 0,
        stable_abstraction_count: 0,
        balanced_count: 0,
        healthy_leaf_count: 0,
        isolated_count: 0,
    };

    for record in records {
        if record.flagged {
            summary.flagged_count += 1;
        }
        match record.category {
            StabilityCategory::Stable => summary.stable_count += 1,
            StabilityCategory::StableAbstraction => summary.stable_abstraction_count += 1,
            StabilityCategory::Balanced => summary.balanced_count += 1,
            StabilityCategory::UnstableAndCentral => {}
            StabilityCategory::HealthyLeaf => summary.healthy_leaf_count += 1,
            StabilityCategory::Isolated => summary.isolated_count += 1,
        }
    }

    summary
}

fn sort_stability_records(records: &mut [StabilityRecord], sort: StabilitySort) {
    match sort {
        StabilitySort::Instability => records.sort_by(|left, right| {
            right
                .instability
                .total_cmp(&left.instability)
                .then(right.fan_in.cmp(&left.fan_in))
                .then(left.path.0.cmp(&right.path.0))
        }),
        StabilitySort::FanIn => records.sort_by(|left, right| {
            right
                .fan_in
                .cmp(&left.fan_in)
                .then(right.instability.total_cmp(&left.instability))
                .then(left.path.0.cmp(&right.path.0))
        }),
        StabilitySort::FanOut => records.sort_by(|left, right| {
            right
                .fan_out
                .cmp(&left.fan_out)
                .then(right.instability.total_cmp(&left.instability))
                .then(left.path.0.cmp(&right.path.0))
        }),
        StabilitySort::Path => records.sort_by(|left, right| left.path.0.cmp(&right.path.0)),
    }
}

fn sort_risk_records(records: &mut [RiskRecord], sort: RiskSort) {
    match sort {
        RiskSort::Score => records.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then(right.transitive_dependents.cmp(&left.transitive_dependents))
                .then(right.churn_commits.cmp(&left.churn_commits))
                .then(left.path.0.cmp(&right.path.0))
        }),
        RiskSort::Churn => records.sort_by(|left, right| {
            right
                .churn_commits
                .cmp(&left.churn_commits)
                .then(right.score.total_cmp(&left.score))
                .then(left.path.0.cmp(&right.path.0))
        }),
        RiskSort::Dependents => records.sort_by(|left, right| {
            right
                .transitive_dependents
                .cmp(&left.transitive_dependents)
                .then(right.score.total_cmp(&left.score))
                .then(left.path.0.cmp(&right.path.0))
        }),
        RiskSort::Path => records.sort_by(|left, right| left.path.0.cmp(&right.path.0)),
    }
}

fn visibility_from_db(value: &str) -> Visibility {
    match value {
        "module" => Visibility::Module,
        "package" => Visibility::Package,
        "public" => Visibility::Public,
        "unknown" => Visibility::Unknown,
        _ => Visibility::Local,
    }
}

fn edge_kind_from_db(value: &str) -> EdgeKind {
    match value {
        "module" => EdgeKind::Contain,
        _ => EdgeKind::Import,
    }
}

fn transitive_dependents_count(
    path: &RepoPath,
    reverse_adjacency: &HashMap<RepoPath, Vec<RepoPath>>,
) -> usize {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    if let Some(initial) = reverse_adjacency.get(path) {
        for dependent in initial {
            if visited.insert(dependent.clone()) {
                queue.push_back(dependent.clone());
            }
        }
    }

    while let Some(current) = queue.pop_front() {
        if let Some(next) = reverse_adjacency.get(&current) {
            for dependent in next {
                if visited.insert(dependent.clone()) {
                    queue.push_back(dependent.clone());
                }
            }
        }
    }

    visited.len()
}

fn snapshot_file_edges(graph: &SnapshotGraph) -> Vec<ArchFileEdge> {
    let mut edges = graph
        .file_edges
        .iter()
        .map(|edge| ArchFileEdge {
            from_file: RepoPath::from(edge.from.clone()),
            to_file: RepoPath::from(edge.to.clone()),
            edge_kind: edge.kind.clone(),
            certainty: edge.certainty.clone(),
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        left.from_file
            .cmp(&right.from_file)
            .then_with(|| left.to_file.cmp(&right.to_file))
            .then_with(|| format!("{:?}", left.edge_kind).cmp(&format!("{:?}", right.edge_kind)))
    });
    edges
}

fn snapshot_public_surface(graph: &SnapshotGraph) -> PublicSurface {
    let file = graph
        .symbols
        .first()
        .map(|symbol| symbol.file.clone())
        .or_else(|| graph.files.first().map(|file| file.path.clone()))
        .unwrap_or_else(|| RepoPath::from("snapshot"));
    let mut symbols = graph
        .symbols
        .iter()
        .filter(|symbol| symbol.exported)
        .enumerate()
        .map(|(index, symbol)| PublicSurfaceSymbol {
            file: symbol.file.clone(),
            name: symbol.name.clone(),
            qualname: symbol.qualname.clone(),
            kind: symbol.kind.clone(),
            visibility: symbol.visibility.clone(),
            line: index as u32 + 1,
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| left.qualname.cmp(&right.qualname));
    PublicSurface { file, symbols }
}

fn diff_public_surfaces(before: &PublicSurface, after: &PublicSurface) -> PublicSurfaceDiff {
    let before_by_identity: HashMap<_, _> = before
        .symbols
        .iter()
        .cloned()
        .map(|symbol| (public_surface_identity(&symbol), symbol))
        .collect();
    let after_by_identity: HashMap<_, _> = after
        .symbols
        .iter()
        .cloned()
        .map(|symbol| (public_surface_identity(&symbol), symbol))
        .collect();

    let mut identities: Vec<_> = before_by_identity
        .keys()
        .chain(after_by_identity.keys())
        .cloned()
        .collect();
    identities.sort();
    identities.dedup();

    let mut changes = Vec::new();
    for identity in identities {
        match (
            before_by_identity.get(&identity),
            after_by_identity.get(&identity),
        ) {
            (Some(before_symbol), None) => changes.push(PublicSurfaceChange {
                kind: PublicSurfaceChangeKind::Removed,
                before: Some(before_symbol.clone()),
                after: None,
            }),
            (None, Some(after_symbol)) => changes.push(PublicSurfaceChange {
                kind: PublicSurfaceChangeKind::Added,
                before: None,
                after: Some(after_symbol.clone()),
            }),
            (Some(before_symbol), Some(after_symbol)) if before_symbol != after_symbol => {
                changes.push(PublicSurfaceChange {
                    kind: PublicSurfaceChangeKind::Modified,
                    before: Some(before_symbol.clone()),
                    after: Some(after_symbol.clone()),
                });
            }
            _ => {}
        }
    }

    let summary = PublicSurfaceDiffSummary {
        added_count: changes
            .iter()
            .filter(|change| change.kind == PublicSurfaceChangeKind::Added)
            .count(),
        removed_count: changes
            .iter()
            .filter(|change| change.kind == PublicSurfaceChangeKind::Removed)
            .count(),
        modified_count: changes
            .iter()
            .filter(|change| change.kind == PublicSurfaceChangeKind::Modified)
            .count(),
    };

    PublicSurfaceDiff {
        before_file: before.file.clone(),
        after_file: after.file.clone(),
        changes,
        summary,
    }
}

fn diff_snapshot_stability(before: &SnapshotGraph, after: &SnapshotGraph) -> SnapshotStabilityDelta {
    let before_files = before.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>();
    let after_files = after.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>();
    let before_edges = snapshot_file_edges(before);
    let after_edges = snapshot_file_edges(after);
    let before_records = stability_records_from_file_edges(&before_files, &before_edges, None);
    let after_records = stability_records_from_file_edges(&after_files, &after_edges, None);
    let before_avg = stability_summary(&before_records).avg_instability;
    let after_avg = stability_summary(&after_records).avg_instability;
    SnapshotStabilityDelta {
        before_avg_instability: before_avg,
        after_avg_instability: after_avg,
        delta: after_avg - before_avg,
    }
}

fn diff_snapshot_centrality(
    before: &SnapshotGraph,
    after: &SnapshotGraph,
) -> Vec<SnapshotCentralityDelta> {
    let fan_in_map = |graph: &SnapshotGraph| -> HashMap<RepoPath, usize> {
        let mut fan_in = HashMap::new();
        for edge in graph.file_edges.iter().filter(|edge| edge.kind == EdgeKind::Import) {
            *fan_in.entry(RepoPath::from(edge.to.clone())).or_default() += 1;
        }
        fan_in
    };

    let before_fan_in = fan_in_map(before);
    let after_fan_in = fan_in_map(after);
    let mut paths = before
        .files
        .iter()
        .map(|file| file.path.clone())
        .chain(after.files.iter().map(|file| file.path.clone()))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    let mut deltas = paths
        .into_iter()
        .filter_map(|path| {
            let before_value = before_fan_in.get(&path).copied().unwrap_or(0);
            let after_value = after_fan_in.get(&path).copied().unwrap_or(0);
            let delta = after_value as isize - before_value as isize;
            (delta > 0).then_some(SnapshotCentralityDelta {
                path,
                before_fan_in: before_value,
                after_fan_in: after_value,
                delta,
            })
        })
        .collect::<Vec<_>>();
    deltas.sort_by(|left, right| right.delta.cmp(&left.delta).then_with(|| left.path.cmp(&right.path)));
    deltas
}

fn diff_edge_records(before: &[SnapshotEdgeRecord], after: &[SnapshotEdgeRecord]) -> Vec<SnapshotEdgeRecord> {
    let before_set = before.iter().cloned().collect::<HashSet<_>>();
    let mut diff = after
        .iter()
        .filter(|edge| !before_set.contains(*edge))
        .cloned()
        .collect::<Vec<_>>();
    diff.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| format!("{:?}", left.kind).cmp(&format!("{:?}", right.kind)))
    });
    diff
}

fn diff_violations(before: &[crate::ArchViolation], after: &[crate::ArchViolation]) -> Vec<crate::ArchViolation> {
    let before_set = before.iter().cloned().collect::<HashSet<_>>();
    after
        .iter()
        .filter(|violation| !before_set.contains(*violation))
        .cloned()
        .collect()
}

fn import_mentions_symbol(raw_text: &str, symbol_name: &str) -> bool {
    raw_text.contains(symbol_name)
}

fn includes_callers(change_type: &str) -> bool {
    !matches!(change_type, "side-effect")
}

fn includes_importers(change_type: &str) -> bool {
    !matches!(change_type, "body")
}

fn default_depth_for_change_type(change_type: &str) -> u32 {
    if change_type == "body" {
        1
    } else {
        DEFAULT_TRANSITIVE_DEPTH
    }
}

fn looks_like_symbol(target: &str) -> bool {
    target.contains("::")
        && !target.ends_with(".rs")
        && !target.ends_with(".ts")
        && !target.ends_with(".js")
}

fn matches_test_patterns(path: &RepoPath, tests: &TestConfig) -> bool {
    let included = tests.patterns.iter().any(|pattern| {
        glob::Pattern::new(pattern)
            .map(|pattern| pattern.matches(&path.0))
            .unwrap_or(false)
    });
    if !included {
        return false;
    }
    !tests.exclude_patterns.iter().any(|pattern| {
        glob::Pattern::new(pattern)
            .map(|pattern| pattern.matches(&path.0))
            .unwrap_or(false)
    })
}

fn score_for_candidate(base_score: u32, distance: u32, certainty: &Certainty) -> u32 {
    let certainty_weight = match certainty {
        Certainty::Exact => 100u32,
        Certainty::Resolved => 85,
        Certainty::Heuristic => 50,
        Certainty::Dynamic => 25,
    };
    let distance_penalty = distance.max(1);
    ((base_score * certainty_weight) / 100) / distance_penalty
}

fn better_certainty(current: &Certainty, candidate: &Certainty) -> Certainty {
    if certainty_rank(candidate) < certainty_rank(current) {
        candidate.clone()
    } else {
        current.clone()
    }
}

fn public_surface_identity(symbol: &PublicSurfaceSymbol) -> String {
    format!("{}:{}", symbol_kind_name(&symbol.kind), symbol.name)
}

fn certainty_rank(certainty: &Certainty) -> u8 {
    match certainty {
        Certainty::Exact => 0,
        Certainty::Resolved => 1,
        Certainty::Heuristic => 2,
        Certainty::Dynamic => 3,
    }
}

fn compare_context_candidates(
    left: &ContextCandidate,
    right: &ContextCandidate,
) -> std::cmp::Ordering {
    right
        .pinned
        .cmp(&left.pinned)
        .then_with(|| right.score.cmp(&left.score))
        .then_with(|| left.distance.cmp(&right.distance))
        .then_with(|| left.path.cmp(&right.path))
}

fn dedup_traversals(traversals: Vec<TraversalRecord>) -> Vec<TraversalRecord> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();

    for traversal in traversals {
        let key = (
            traversal.kind.clone(),
            traversal.path.clone().map(|path| path.0),
            traversal.qualname.clone(),
            traversal.edge_kind.clone(),
            traversal.distance,
        );
        if seen.insert(key) {
            deduped.push(traversal);
        }
    }

    deduped.sort_by(|left, right| {
        left.distance
            .cmp(&right.distance)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.qualname.cmp(&right.qualname))
    });
    deduped
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CallSiteRecord, ImportRecord, ParseStatus, PublicSurfaceChangeKind, Span, SymbolKind,
        SymbolRecord, Visibility,
    };
    use rusqlite::Connection;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("scope-{prefix}-{nanos}"))
    }

    fn sample_file(path: &str) -> FileRecord {
        FileRecord {
            path: RepoPath::from(path),
            language: "rust".to_string(),
            parse_status: ParseStatus::Ok,
            is_barrel: false,
            content_hash: Some(format!("hash:{path}")),
            mtime_unix_seconds: Some(1),
            size_bytes: Some(10),
        }
    }

    fn sample_span(line: u32) -> Span {
        Span {
            start_byte: 0,
            end_byte: 10,
            start_line: line,
            end_line: line,
        }
    }

    fn sample_symbol(
        file: &str,
        name: &str,
        kind: SymbolKind,
        visibility: Visibility,
    ) -> SymbolRecord {
        SymbolRecord {
            file: RepoPath::from(file),
            name: name.to_string(),
            qualname: format!(
                "{}::{name}",
                file.trim_start_matches("src/").trim_end_matches(".rs")
            ),
            kind,
            visibility: visibility.clone(),
            exported: matches!(visibility, Visibility::Public | Visibility::Package),
            span: sample_span(1),
        }
    }

    fn sample_call(
        file: &str,
        caller_qualname: &str,
        callee_name: &str,
        callee_qualname: Option<&str>,
        is_method: bool,
        line: u32,
    ) -> CallSiteRecord {
        CallSiteRecord {
            file: RepoPath::from(file),
            caller_qualname: Some(caller_qualname.to_string()),
            callee_name: callee_name.to_string(),
            callee_qualname: callee_qualname.map(ToOwned::to_owned),
            is_method,
            span: sample_span(line),
            certainty: Certainty::Exact,
        }
    }

    #[test]
    fn opens_and_bootstraps_new_database() {
        let dir = unique_temp_dir("db-open");
        let db_path = dir.join("index.db");

        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        assert!(db_path.exists());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reopening_database_is_idempotent() {
        let dir = unique_temp_dir("db-reopen");
        let db_path = dir.join("index.db");

        let first = Store::open(&db_path).unwrap();
        assert_eq!(first.schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        drop(first);

        let second = Store::open(&db_path).unwrap();
        assert_eq!(second.schema_version().unwrap(), INDEX_SCHEMA_VERSION);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repairs_version_three_database_missing_base_tables() {
        let dir = unique_temp_dir("db-repair-base");
        let db_path = dir.join("index.db");
        std::fs::create_dir_all(&dir).unwrap();

        let connection = Connection::open(&db_path).unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        connection.execute_batch(SYMBOLS_MIGRATION).unwrap();
        connection.execute_batch(SYMBOL_EDGES_MIGRATION).unwrap();
        drop(connection);

        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        assert!(store.upsert_file(&sample_file("src/lib.rs")).is_ok());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repairs_partial_schema_without_requiring_version_downgrade() {
        let dir = unique_temp_dir("db-repair-partial");
        let db_path = dir.join("index.db");
        std::fs::create_dir_all(&dir).unwrap();

        let connection = Connection::open(&db_path).unwrap();
        connection.pragma_update(None, "user_version", 3).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS index_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                language TEXT NOT NULL,
                parse_status TEXT NOT NULL,
                is_barrel INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER NOT NULL
            );",
            )
            .unwrap();
        drop(connection);

        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.schema_version().unwrap(), INDEX_SCHEMA_VERSION);

        let target = sample_file("src/parser.rs");
        store.upsert_file(&target).unwrap();

        let source = sample_file("src/resolver.rs");
        let extract = ExtractResult {
            file: source.clone(),
            imports: vec![ImportRecord {
                file: source.path.clone(),
                raw_text: "use crate::parser;".to_string(),
                import_path: ImportPath::Relative(target.path.clone()),
                span: sample_span(1),
                certainty: Certainty::Exact,
            }],
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: Vec::new(),
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();
        let deps = store.query_deps(&source.path).unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].path, target.path);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persists_file_metadata_and_classifies_content_changes() {
        let dir = unique_temp_dir("db-file-metadata");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let file = sample_file("src/lib.rs");
        store.upsert_file(&file).unwrap();

        let persisted = store.file_state(&file.path).unwrap().unwrap();
        assert_eq!(persisted.path, file.path);
        assert_eq!(persisted.content_hash, file.content_hash);
        assert_eq!(persisted.mtime_unix_seconds, file.mtime_unix_seconds);
        assert_eq!(persisted.size_bytes, file.size_bytes);

        let unchanged = store.classify_file_change(&file).unwrap();
        assert_eq!(unchanged, Some(false));

        let mut changed = file.clone();
        changed.content_hash = Some("different-hash".to_string());
        let changed_state = store.classify_file_change(&changed).unwrap();
        assert_eq!(changed_state, Some(true));

        let new_file = sample_file("src/new.rs");
        let new_state = store.classify_file_change(&new_file).unwrap();
        assert_eq!(new_state, None);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn repairs_missing_file_metadata_columns() {
        let dir = unique_temp_dir("db-repair-file-metadata");
        let db_path = dir.join("index.db");
        std::fs::create_dir_all(&dir).unwrap();

        let connection = Connection::open(&db_path).unwrap();
        connection.pragma_update(None, "user_version", 4).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS index_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                language TEXT NOT NULL,
                parse_status TEXT NOT NULL,
                is_barrel INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                raw_text TEXT NOT NULL,
                resolved_file_id INTEGER,
                import_path_kind TEXT NOT NULL,
                external_pkg TEXT,
                span_start INTEGER,
                span_end INTEGER,
                start_line INTEGER,
                certainty TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS file_edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_file_id INTEGER NOT NULL,
                to_file_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                certainty TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                qualname TEXT NOT NULL,
                kind TEXT NOT NULL,
                visibility TEXT NOT NULL,
                exported INTEGER NOT NULL,
                span_start INTEGER NOT NULL,
                span_end INTEGER NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS symbol_edges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_symbol_id INTEGER NOT NULL,
                to_symbol_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                certainty TEXT NOT NULL,
                call_line INTEGER NOT NULL
            );",
            )
            .unwrap();
        drop(connection);

        let store = Store::open(&db_path).unwrap();
        assert_eq!(store.schema_version().unwrap(), INDEX_SCHEMA_VERSION);
        assert!(store.upsert_file(&sample_file("src/lib.rs")).is_ok());
        let persisted = store
            .file_state(&RepoPath::from("src/lib.rs"))
            .unwrap()
            .unwrap();
        assert!(persisted.content_hash.is_some());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_newer_unsupported_schema_versions() {
        let dir = unique_temp_dir("db-newer-version");
        let db_path = dir.join("index.db");
        std::fs::create_dir_all(&dir).unwrap();

        let connection = Connection::open(&db_path).unwrap();
        connection
            .pragma_update(None, "user_version", INDEX_SCHEMA_VERSION + 1)
            .unwrap();
        drop(connection);

        let error = Store::open(&db_path).unwrap_err();
        assert!(matches!(error, ScopeError::Migration(_)));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persists_file_records_and_forward_dependencies() {
        let dir = unique_temp_dir("db-deps");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let target = sample_file("src/parser.rs");
        store.upsert_file(&target).unwrap();

        let source = sample_file("src/resolver.rs");
        let extract = ExtractResult {
            file: source.clone(),
            imports: vec![ImportRecord {
                file: source.path.clone(),
                raw_text: "use crate::parser;".to_string(),
                import_path: ImportPath::Relative(target.path.clone()),
                span: sample_span(1),
                certainty: Certainty::Exact,
            }],
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: Vec::new(),
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();
        let deps = store.query_deps(&source.path).unwrap();

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].path, target.path);
        assert_eq!(deps[0].edge_kind, EdgeKind::Import);
        assert_eq!(deps[0].certainty, Certainty::Exact);
        assert_eq!(deps[0].import_text.as_deref(), Some("use crate::parser;"));
        assert_eq!(deps[0].line, Some(1));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn batch_persistence_resolves_cross_file_dependencies_without_manual_upserts() {
        let dir = unique_temp_dir("db-batch-deps");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let target = sample_file("src/parser.rs");
        let source = sample_file("src/resolver.rs");
        let extracts = vec![
            ExtractResult {
                file: target.clone(),
                imports: Vec::new(),
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: source.clone(),
                imports: vec![ImportRecord {
                    file: source.path.clone(),
                    raw_text: "use crate::parser;".to_string(),
                    import_path: ImportPath::Relative(target.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
        ];

        store.persist_extract_results(&extracts).unwrap();
        let deps = store.query_deps(&source.path).unwrap();

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].path, target.path);
        assert_eq!(deps[0].edge_kind, EdgeKind::Import);
        assert_eq!(deps[0].certainty, Certainty::Exact);
        assert_eq!(deps[0].import_text.as_deref(), Some("use crate::parser;"));
        assert_eq!(deps[0].line, Some(1));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn supports_reverse_dependency_queries() {
        let dir = unique_temp_dir("db-reverse-deps");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let parser = sample_file("src/parser.rs");
        let resolver = sample_file("src/resolver.rs");
        store.upsert_file(&parser).unwrap();

        let extract = ExtractResult {
            file: resolver.clone(),
            imports: vec![ImportRecord {
                file: resolver.path.clone(),
                raw_text: "use crate::parser;".to_string(),
                import_path: ImportPath::Relative(parser.path.clone()),
                span: sample_span(1),
                certainty: Certainty::Exact,
            }],
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: Vec::new(),
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();
        let reverse = store.query_reverse_deps(&parser.path).unwrap();

        assert_eq!(reverse.len(), 1);
        assert_eq!(reverse[0].path, resolver.path);
        assert_eq!(reverse[0].edge_kind, EdgeKind::Import);
        assert_eq!(
            reverse[0].import_text.as_deref(),
            Some("use crate::parser;")
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lists_indexed_files_deletes_files_and_computes_reverse_dependency_closure() {
        let dir = unique_temp_dir("db-incremental-helpers");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let parser = sample_file("src/parser.rs");
        let resolver = sample_file("src/resolver.rs");
        let app = sample_file("src/app.rs");

        let extracts = vec![
            ExtractResult {
                file: parser.clone(),
                imports: Vec::new(),
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: resolver.clone(),
                imports: vec![ImportRecord {
                    file: resolver.path.clone(),
                    raw_text: "use crate::parser;".to_string(),
                    import_path: ImportPath::Relative(parser.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: app.clone(),
                imports: vec![ImportRecord {
                    file: app.path.clone(),
                    raw_text: "use crate::resolver;".to_string(),
                    import_path: ImportPath::Relative(resolver.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
        ];

        store.persist_extract_results(&extracts).unwrap();

        let indexed = store.list_indexed_files().unwrap();
        assert_eq!(
            indexed,
            vec![app.path.clone(), parser.path.clone(), resolver.path.clone()]
        );

        let closure = store
            .reverse_dependency_closure(std::slice::from_ref(&parser.path))
            .unwrap();
        assert_eq!(closure, vec![app.path.clone(), resolver.path.clone()]);

        assert!(store.delete_file(&parser.path).unwrap());
        assert!(!store.delete_file(&parser.path).unwrap());
        assert!(store.query_reverse_deps(&parser.path).unwrap().is_empty());
        assert_eq!(
            store.list_indexed_files().unwrap(),
            vec![app.path.clone(), resolver.path.clone()]
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn persists_and_queries_symbols() {
        let dir = unique_temp_dir("db-symbols");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let source = sample_file("src/lib.rs");
        let extract = ExtractResult {
            file: source.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                sample_symbol(
                    "src/lib.rs",
                    "greet",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
                sample_symbol(
                    "src/lib.rs",
                    "helper",
                    SymbolKind::Function,
                    Visibility::Local,
                ),
                sample_symbol(
                    "src/lib.rs",
                    "Parser",
                    SymbolKind::Struct,
                    Visibility::Package,
                ),
            ],
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();

        let all = store.query_symbols(&source.path, false, None).unwrap();
        assert_eq!(all.len(), 3);
        let all_names: Vec<_> = all.iter().map(|symbol| symbol.name.as_str()).collect();
        assert_eq!(all_names, vec!["Parser", "greet", "helper"]);
        assert_eq!(all[0].kind, SymbolKind::Struct);

        let public_only = store.query_symbols(&source.path, true, None).unwrap();
        assert_eq!(public_only.len(), 2);
        assert!(public_only.iter().all(|symbol| symbol.exported));

        let functions = store
            .query_symbols(&source.path, false, Some(SymbolKind::Function))
            .unwrap();
        assert_eq!(functions.len(), 2);
        assert!(functions
            .iter()
            .all(|symbol| symbol.kind == SymbolKind::Function));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn queries_public_surface_from_exported_symbols() {
        let dir = unique_temp_dir("db-public-surface");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let source = sample_file("src/lib.rs");
        let extract = ExtractResult {
            file: source.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                sample_symbol(
                    "src/lib.rs",
                    "greet",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
                sample_symbol(
                    "src/lib.rs",
                    "Parser",
                    SymbolKind::Struct,
                    Visibility::Package,
                ),
                sample_symbol(
                    "src/lib.rs",
                    "helper",
                    SymbolKind::Function,
                    Visibility::Local,
                ),
            ],
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();

        let surface = store.query_public_surface(&source.path).unwrap();
        assert_eq!(surface.file, source.path);
        assert_eq!(surface.symbols.len(), 2);
        let names: Vec<_> = surface
            .symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect();
        assert_eq!(names, vec!["Parser", "greet"]);
        assert!(surface
            .symbols
            .iter()
            .all(|symbol| matches!(symbol.visibility, Visibility::Public | Visibility::Package)));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn diffs_public_surface_between_two_files() {
        let dir = unique_temp_dir("db-public-surface-diff");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let before = sample_file("snapshots/before.rs");
        let after = sample_file("snapshots/after.rs");
        let before_extract = ExtractResult {
            file: before.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                sample_symbol(
                    "snapshots/before.rs",
                    "greet",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
                sample_symbol(
                    "snapshots/before.rs",
                    "Parser",
                    SymbolKind::Struct,
                    Visibility::Public,
                ),
            ],
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };
        let mut renamed_parser = sample_symbol(
            "snapshots/after.rs",
            "Parser",
            SymbolKind::Struct,
            Visibility::Public,
        );
        renamed_parser.span = sample_span(8);
        let after_extract = ExtractResult {
            file: after.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                renamed_parser,
                sample_symbol(
                    "snapshots/after.rs",
                    "format",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
            ],
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&before_extract).unwrap();
        store.persist_extract_result(&after_extract).unwrap();

        let diff = store
            .diff_public_surface(&before.path, &after.path)
            .unwrap();
        assert_eq!(diff.before_file, before.path);
        assert_eq!(diff.after_file, after.path);
        assert_eq!(diff.summary.added_count, 1);
        assert_eq!(diff.summary.removed_count, 1);
        assert_eq!(diff.summary.modified_count, 1);
        assert_eq!(diff.changes.len(), 3);
        assert!(diff
            .changes
            .iter()
            .any(|change| change.kind == PublicSurfaceChangeKind::Added
                && change
                    .after
                    .as_ref()
                    .is_some_and(|symbol| symbol.name == "format")));
        assert!(diff
            .changes
            .iter()
            .any(|change| change.kind == PublicSurfaceChangeKind::Removed
                && change
                    .before
                    .as_ref()
                    .is_some_and(|symbol| symbol.name == "greet")));
        assert!(diff
            .changes
            .iter()
            .any(|change| change.kind == PublicSurfaceChangeKind::Modified
                && change
                    .before
                    .as_ref()
                    .is_some_and(|symbol| symbol.name == "Parser")
                && change.after.as_ref().is_some_and(|symbol| symbol.line == 8)));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolves_same_file_direct_calls() {
        let dir = unique_temp_dir("db-calls-same-file");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let source = sample_file("src/parser.rs");
        let extract = ExtractResult {
            file: source.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                sample_symbol(
                    "src/parser.rs",
                    "parse",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
                sample_symbol(
                    "src/parser.rs",
                    "tokenize",
                    SymbolKind::Function,
                    Visibility::Local,
                ),
            ],
            call_sites: vec![sample_call(
                "src/parser.rs",
                "parser::parse",
                "tokenize",
                None,
                false,
                2,
            )],
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();

        let callees = store.query_callees("parser::parse", false).unwrap();
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].qualname.as_deref(), Some("parser::tokenize"));
        assert_eq!(callees[0].certainty, Certainty::Exact);

        let callers = store.query_callers("parser::tokenize", false).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].qualname.as_deref(), Some("parser::parse"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolves_imported_module_direct_calls() {
        let dir = unique_temp_dir("db-calls-imported");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let parser = sample_file("src/parser.rs");
        store.upsert_file(&parser).unwrap();
        store
            .persist_extract_result(&ExtractResult {
                file: parser.clone(),
                imports: Vec::new(),
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/parser.rs",
                    "parse",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            })
            .unwrap();

        let resolver = sample_file("src/resolver.rs");
        let extract = ExtractResult {
            file: resolver.clone(),
            imports: vec![ImportRecord {
                file: resolver.path.clone(),
                raw_text: "use crate::parser;".to_string(),
                import_path: ImportPath::Relative(parser.path.clone()),
                span: sample_span(1),
                certainty: Certainty::Exact,
            }],
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![sample_symbol(
                "src/resolver.rs",
                "resolve",
                SymbolKind::Function,
                Visibility::Public,
            )],
            call_sites: vec![sample_call(
                "src/resolver.rs",
                "resolver::resolve",
                "parse",
                Some("parser::parse"),
                false,
                4,
            )],
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();

        let callees = store.query_callees("resolver::resolve", false).unwrap();
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].qualname.as_deref(), Some("parser::parse"));
        assert_eq!(callees[0].certainty, Certainty::Resolved);

        let callers = store.query_callers("parser::parse", false).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].qualname.as_deref(), Some("resolver::resolve"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn skips_unresolved_method_calls() {
        let dir = unique_temp_dir("db-calls-method");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let source = sample_file("src/lib.rs");
        let extract = ExtractResult {
            file: source.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![sample_symbol(
                "src/lib.rs",
                "greet",
                SymbolKind::Function,
                Visibility::Public,
            )],
            call_sites: vec![sample_call(
                "src/lib.rs",
                "lib::greet",
                "join",
                None,
                true,
                7,
            )],
            parse_diagnostics: Vec::new(),
        };

        store.persist_extract_result(&extract).unwrap();

        let callees = store.query_callees("lib::greet", false).unwrap();
        assert!(callees.is_empty());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reindexing_replaces_stale_call_edges() {
        let dir = unique_temp_dir("db-calls-reindex");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let source = sample_file("src/parser.rs");
        let initial = ExtractResult {
            file: source.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                sample_symbol(
                    "src/parser.rs",
                    "parse",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
                sample_symbol(
                    "src/parser.rs",
                    "tokenize",
                    SymbolKind::Function,
                    Visibility::Local,
                ),
            ],
            call_sites: vec![sample_call(
                "src/parser.rs",
                "parser::parse",
                "tokenize",
                None,
                false,
                2,
            )],
            parse_diagnostics: Vec::new(),
        };
        store.persist_extract_result(&initial).unwrap();
        assert_eq!(
            store.query_callees("parser::parse", false).unwrap().len(),
            1
        );

        let updated = ExtractResult {
            file: source,
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                sample_symbol(
                    "src/parser.rs",
                    "parse",
                    SymbolKind::Function,
                    Visibility::Public,
                ),
                sample_symbol(
                    "src/parser.rs",
                    "tokenize",
                    SymbolKind::Function,
                    Visibility::Local,
                ),
            ],
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };
        store.persist_extract_result(&updated).unwrap();
        assert!(store
            .query_callees("parser::parse", false)
            .unwrap()
            .is_empty());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn impact_body_returns_only_direct_callers() {
        let dir = unique_temp_dir("db-impact-body");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let parser = sample_file("src/parser.rs");
        let resolver = sample_file("src/resolver.rs");
        let main = sample_file("src/main.rs");

        let extracts = vec![
            ExtractResult {
                file: parser.clone(),
                imports: Vec::new(),
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/parser.rs",
                    "parse",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: resolver.clone(),
                imports: vec![ImportRecord {
                    file: resolver.path.clone(),
                    raw_text: "use crate::parser;".to_string(),
                    import_path: ImportPath::Relative(parser.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/resolver.rs",
                    "resolve",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: vec![sample_call(
                    "src/resolver.rs",
                    "resolver::resolve",
                    "parse",
                    Some("parser::parse"),
                    false,
                    2,
                )],
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: main.clone(),
                imports: vec![ImportRecord {
                    file: main.path.clone(),
                    raw_text: "use crate::resolver;".to_string(),
                    import_path: ImportPath::Relative(resolver.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/main.rs",
                    "run",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: vec![sample_call(
                    "src/main.rs",
                    "main::run",
                    "resolve",
                    Some("resolver::resolve"),
                    false,
                    2,
                )],
                parse_diagnostics: Vec::new(),
            },
        ];

        store.persist_extract_results(&extracts).unwrap();
        let impacted = store.query_impact("parser::parse", "body", None).unwrap();

        assert_eq!(impacted.len(), 1);
        assert_eq!(impacted[0].qualname.as_deref(), Some("resolver::resolve"));
        assert_eq!(impacted[0].distance, 1);
        assert_eq!(impacted[0].edge_kind, EdgeKind::Call);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn impact_signature_includes_transitive_callers_and_importers() {
        let dir = unique_temp_dir("db-impact-signature");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let parser = sample_file("src/parser.rs");
        let resolver = sample_file("src/resolver.rs");
        let main = sample_file("src/main.rs");

        let extracts = vec![
            ExtractResult {
                file: parser.clone(),
                imports: Vec::new(),
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/parser.rs",
                    "parse",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: resolver.clone(),
                imports: vec![ImportRecord {
                    file: resolver.path.clone(),
                    raw_text: "use crate::parser;".to_string(),
                    import_path: ImportPath::Relative(parser.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/resolver.rs",
                    "resolve",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: vec![sample_call(
                    "src/resolver.rs",
                    "resolver::resolve",
                    "parse",
                    Some("parser::parse"),
                    false,
                    2,
                )],
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: main.clone(),
                imports: vec![ImportRecord {
                    file: main.path.clone(),
                    raw_text: "use crate::resolver;".to_string(),
                    import_path: ImportPath::Relative(resolver.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/main.rs",
                    "run",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: vec![sample_call(
                    "src/main.rs",
                    "main::run",
                    "resolve",
                    Some("resolver::resolve"),
                    false,
                    2,
                )],
                parse_diagnostics: Vec::new(),
            },
        ];

        store.persist_extract_results(&extracts).unwrap();
        let impacted = store
            .query_impact("parser::parse", "signature", None)
            .unwrap();

        assert!(impacted.iter().any(|record| record.qualname.as_deref()
            == Some("resolver::resolve")
            && record.distance == 1));
        assert!(impacted
            .iter()
            .any(|record| record.qualname.as_deref() == Some("main::run") && record.distance == 2));
        assert!(impacted.iter().any(|record| record
            .path
            .as_ref()
            .is_some_and(|path| path.0 == "src/resolver.rs")
            && record.edge_kind == EdgeKind::Import));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn impact_delete_and_side_effect_follow_reverse_imports() {
        let dir = unique_temp_dir("db-impact-importers");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let parser = sample_file("src/parser.rs");
        let resolver = sample_file("src/resolver.rs");
        let main = sample_file("src/main.rs");
        let extracts = vec![
            ExtractResult {
                file: parser.clone(),
                imports: Vec::new(),
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: resolver.clone(),
                imports: vec![ImportRecord {
                    file: resolver.path.clone(),
                    raw_text: "use crate::parser;".to_string(),
                    import_path: ImportPath::Relative(parser.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: main.clone(),
                imports: vec![ImportRecord {
                    file: main.path.clone(),
                    raw_text: "use crate::resolver;".to_string(),
                    import_path: ImportPath::Relative(resolver.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: Vec::new(),
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
        ];

        store.persist_extract_results(&extracts).unwrap();

        let deleted = store.query_impact("src/parser.rs", "delete", None).unwrap();
        assert!(deleted.iter().any(|record| record
            .path
            .as_ref()
            .is_some_and(|path| path.0 == "src/resolver.rs")
            && record.distance == 1));
        assert!(deleted.iter().any(|record| record
            .path
            .as_ref()
            .is_some_and(|path| path.0 == "src/main.rs")
            && record.distance == 2));

        let side_effect = store
            .query_impact("src/parser.rs", "side-effect", None)
            .unwrap();
        assert!(side_effect
            .iter()
            .all(|record| record.edge_kind == EdgeKind::Import));
        assert!(side_effect.iter().any(|record| record
            .path
            .as_ref()
            .is_some_and(|path| path.0 == "src/main.rs")
            && record.distance == 2));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn explain_returns_incident_call_and_import_evidence() {
        let dir = unique_temp_dir("db-explain");
        let db_path = dir.join("index.db");
        let store = Store::open(&db_path).unwrap();

        let parser = sample_file("src/parser.rs");
        let resolver = sample_file("src/resolver.rs");
        let extracts = vec![
            ExtractResult {
                file: parser.clone(),
                imports: Vec::new(),
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/parser.rs",
                    "parse",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: Vec::new(),
                parse_diagnostics: Vec::new(),
            },
            ExtractResult {
                file: resolver.clone(),
                imports: vec![ImportRecord {
                    file: resolver.path.clone(),
                    raw_text: "use crate::parser;".to_string(),
                    import_path: ImportPath::Relative(parser.path.clone()),
                    span: sample_span(1),
                    certainty: Certainty::Exact,
                }],
                modules: Vec::new(),
                exports: Vec::new(),
                symbols: vec![sample_symbol(
                    "src/resolver.rs",
                    "resolve",
                    SymbolKind::Function,
                    Visibility::Public,
                )],
                call_sites: vec![sample_call(
                    "src/resolver.rs",
                    "resolver::resolve",
                    "parse",
                    Some("parser::parse"),
                    false,
                    2,
                )],
                parse_diagnostics: Vec::new(),
            },
        ];

        store.persist_extract_results(&extracts).unwrap();
        let explain = store.query_explain("parser::parse", None, None).unwrap();

        assert!(explain.iter().any(|record| record.qualname.as_deref()
            == Some("resolver::resolve")
            && record.edge_kind == EdgeKind::Call));
        assert!(explain.iter().any(|record| record
            .path
            .as_ref()
            .is_some_and(|path| path.0 == "src/resolver.rs")
            && record.edge_kind == EdgeKind::Import));

        std::fs::remove_dir_all(dir).unwrap();
    }
}
