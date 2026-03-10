use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::{
    Certainty, DependencyRecord, EdgeKind, ExtractResult, FileRecord, ImportPath, NodeKind,
    RepoPath, ScopeError, ScopeResult, SymbolKind, SymbolRecord, TraversalRecord, Visibility,
};

pub const INDEX_SCHEMA_VERSION: u32 = 3;

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
                let mut target_file_ids = self.imported_file_ids_for_module(file_id, imports, module_name)?;
                if target_file_ids.is_empty() {
                    target_file_ids = self.file_ids_for_module_name(module_name)?;
                }

                if let Some(symbol_id) = self.unique_symbol_id_in_files(&target_file_ids, symbol_name)? {
                    return Ok(Some((symbol_id, Certainty::Resolved)));
                }
            }

            return Ok(None);
        }

        if let Some(symbol_id) = self.unique_symbol_id_in_file(file_id, &call_site.callee_name)? {
            return Ok(Some((symbol_id, Certainty::Exact)));
        }

        if let Some(symbol_id) = self.unique_imported_symbol_id(file_id, imports, &call_site.callee_name)? {
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
            params![format!("%/{module_name}.rs"), format!("%/{module_name}/mod.rs")],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )?;

        let mut file_ids = Vec::new();
        for row in rows {
            let (id, _path) = row?;
            file_ids.push(id);
        }
        Ok(file_ids)
    }

    fn unique_symbol_id_in_file(&self, file_id: i64, symbol_name: &str) -> ScopeResult<Option<i64>> {
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
                    target_file_ids.extend(self.reexport_target_file_ids(target_file_id, symbol_name)?);
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

    fn unique_symbol_id_in_files(&self, file_ids: &[i64], symbol_name: &str) -> ScopeResult<Option<i64>> {
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
        connection.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;
    }

    reconcile_schema(connection)?;
    connection.pragma_update(None, "user_version", INDEX_SCHEMA_VERSION)?;

    Ok(())
}

fn reconcile_schema(connection: &Connection) -> ScopeResult<()> {
    if !has_required_tables(connection, &["index_meta", "files", "imports", "file_edges"])? {
        connection.execute_batch(INITIAL_MIGRATION)?;
    }

    if !has_required_tables(connection, &["symbols"])? {
        connection.execute_batch(SYMBOLS_MIGRATION)?;
    }

    if !has_required_tables(connection, &["symbol_edges"])? {
        connection.execute_batch(SYMBOL_EDGES_MIGRATION)?;
    }

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

fn import_mentions_symbol(raw_text: &str, symbol_name: &str) -> bool {
    raw_text.contains(symbol_name)
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
    use crate::{CallSiteRecord, ImportRecord, ParseStatus, Span, SymbolRecord, SymbolKind, Visibility};
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

    fn sample_symbol(file: &str, name: &str, kind: SymbolKind, visibility: Visibility) -> SymbolRecord {
        SymbolRecord {
            file: RepoPath::from(file),
            name: name.to_string(),
            qualname: format!("{}::{name}", file.trim_start_matches("src/").trim_end_matches(".rs")),
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
        connection.execute_batch(
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
        ).unwrap();
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
                sample_symbol("src/lib.rs", "greet", SymbolKind::Function, Visibility::Public),
                sample_symbol("src/lib.rs", "helper", SymbolKind::Function, Visibility::Local),
                sample_symbol("src/lib.rs", "Parser", SymbolKind::Struct, Visibility::Package),
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
        assert!(functions.iter().all(|symbol| symbol.kind == SymbolKind::Function));

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
                sample_symbol("src/parser.rs", "parse", SymbolKind::Function, Visibility::Public),
                sample_symbol("src/parser.rs", "tokenize", SymbolKind::Function, Visibility::Local),
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
        store.persist_extract_result(&ExtractResult {
            file: parser.clone(),
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![sample_symbol("src/parser.rs", "parse", SymbolKind::Function, Visibility::Public)],
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        }).unwrap();

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
            symbols: vec![sample_symbol("src/resolver.rs", "resolve", SymbolKind::Function, Visibility::Public)],
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
            symbols: vec![sample_symbol("src/lib.rs", "greet", SymbolKind::Function, Visibility::Public)],
            call_sites: vec![sample_call("src/lib.rs", "lib::greet", "join", None, true, 7)],
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
                sample_symbol("src/parser.rs", "parse", SymbolKind::Function, Visibility::Public),
                sample_symbol("src/parser.rs", "tokenize", SymbolKind::Function, Visibility::Local),
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
        assert_eq!(store.query_callees("parser::parse", false).unwrap().len(), 1);

        let updated = ExtractResult {
            file: source,
            imports: Vec::new(),
            modules: Vec::new(),
            exports: Vec::new(),
            symbols: vec![
                sample_symbol("src/parser.rs", "parse", SymbolKind::Function, Visibility::Public),
                sample_symbol("src/parser.rs", "tokenize", SymbolKind::Function, Visibility::Local),
            ],
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        };
        store.persist_extract_result(&updated).unwrap();
        assert!(store.query_callees("parser::parse", false).unwrap().is_empty());

        std::fs::remove_dir_all(dir).unwrap();
    }
}
