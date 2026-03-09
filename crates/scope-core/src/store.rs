use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::{
    Certainty, DependencyRecord, EdgeKind, ExtractResult, FileRecord, ImportPath, NodeKind,
    RepoPath, ScopeError, ScopeResult,
};

pub const INDEX_SCHEMA_VERSION: u32 = 1;

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

#[derive(Debug)]
pub struct Store {
    connection: Connection,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseInfo {
    pub path: String,
    pub schema_version: u32,
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
            "INSERT INTO files (path, language, parse_status, is_barrel, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
                 language = excluded.language,
                 parse_status = excluded.parse_status,
                 is_barrel = excluded.is_barrel,
                 indexed_at = excluded.indexed_at",
            params![
                file.path.0,
                file.language,
                parse_status_name(&file.parse_status),
                file.is_barrel as i64,
                indexed_at,
            ],
        )?;

        self.file_id(&file.path)?.ok_or_else(|| {
            ScopeError::Internal(format!("missing file row after upsert: {}", file.path.0))
        })
    }

    pub fn persist_extract_result(&self, result: &ExtractResult) -> ScopeResult<()> {
        let file_id = self.upsert_file(&result.file)?;
        self.connection
            .execute("DELETE FROM imports WHERE file_id = ?1", [file_id])?;
        self.connection
            .execute("DELETE FROM file_edges WHERE from_file_id = ?1", [file_id])?;

        for import in &result.imports {
            self.insert_import(file_id, import)?;
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

        Ok(())
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
        connection.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;
    }

    Ok(())
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

fn edge_kind_from_db(value: &str) -> EdgeKind {
    match value {
        "module" => EdgeKind::Contain,
        _ => EdgeKind::Import,
    }
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
    use crate::{ImportRecord, ParseStatus, Span};
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
}
