mod cli;

use std::{env, fs, time::UNIX_EPOCH};

use clap::Parser;
use cli::{ArchCommand, ChangeType, Cli, Commands};
use scope_core::{Certainty, ContextFileRecord, ContextFileRole, RepoPath};
use scope_core::{
    adapter_for_language, arch_check, load_arch_config, scan_repo, BootstrapOptions, DatabaseInfo,
    ScanConfig, SymbolKind, Verbosity,
};

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{}", render_cli_error(&error));
            std::process::exit(2);
        }
    }
}

fn render_cli_error(error: &scope_core::ScopeError) -> String {
    let envelope = scope_core::JsonEnvelope::error("cli", error);
    serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| {
        "{\n  \"schema_version\": 1,\n  \"command\": \"cli\",\n  \"status\": \"error\",\n  \"data\": {\n    \"kind\": \"serialization\",\n    \"message\": \"failed to serialize CLI error envelope\"\n  },\n  \"warnings\": []\n}".to_string()
    })
}

fn run() -> Result<i32, scope_core::ScopeError> {
    let cli = Cli::parse();
    let cwd = env::current_dir().map_err(|error| scope_core::ScopeError::io(".", error))?;
    let verbosity = verbosity(&cli);
    let mut exit_code = 0;

    let output = match cli.command {
        Commands::Index(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone().or(args.repo_root.clone()),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let indexed_files = index_repo(&context.paths.repo_root, &context.store)?;
            let database = DatabaseInfo {
                path: context.paths.db_path.display().to_string(),
                schema_version: context.store.schema_version()?,
            };

            serde_json::to_string_pretty(&scope_core::stub::index(
                context.paths.repo_root.display().to_string(),
                args.no_git,
                args.watch,
                database,
                indexed_files,
            ))
        }
        Commands::Deps(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let dependencies = if args.reverse {
                context
                    .store
                    .query_reverse_deps(&scope_core::RepoPath::from(args.file.clone()))?
            } else {
                context
                    .store
                    .query_deps(&scope_core::RepoPath::from(args.file.clone()))?
            };

            serde_json::to_string_pretty(&scope_core::stub::deps(
                args.file,
                args.reverse,
                args.transitive,
                args.depth,
                dependencies,
            ))
        }
        Commands::Symbols(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let kind = args.kind.map(symbol_kind_name);
            let symbols = context.store.query_symbols(
                &scope_core::RepoPath::from(args.file.clone()),
                args.public_only,
                kind.clone(),
            )?;

            serde_json::to_string_pretty(&scope_core::stub::symbols(
                args.file,
                args.public_only,
                kind,
                symbols,
            ))
        }
        Commands::Calls(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let traversals = context.store.query_callees(&args.symbol, args.transitive)?;
            serde_json::to_string_pretty(&scope_core::stub::calls(
                args.symbol,
                args.transitive,
                traversals,
            ))
        }
        Commands::Callers(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let traversals = context.store.query_callers(&args.symbol, args.transitive)?;
            serde_json::to_string_pretty(&scope_core::stub::callers(
                args.symbol,
                args.transitive,
                traversals,
            ))
        }
        Commands::Impact(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let change_type = change_type_name(args.change_type);
            let impacted = context
                .store
                .query_impact(&args.target, &change_type, args.depth)?;
            serde_json::to_string_pretty(&scope_core::stub::impact(
                args.target,
                change_type,
                args.depth,
                impacted,
            ))
        }
        Commands::Explain(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let traversals = context
                .store
                .query_explain(&args.target, args.to.as_deref(), args.depth)?;
            serde_json::to_string_pretty(&scope_core::stub::explain(
                args.target,
                args.to,
                args.depth,
                traversals,
            ))
        }
        Commands::Why(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let path = context.store.query_why(&args.from, &args.to, args.depth)?;
            serde_json::to_string_pretty(&scope_core::stub::why(
                args.from,
                args.to,
                args.depth,
                path,
            ))
        }
        Commands::Context(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let change_type = change_type_name(args.change_type);
            let result = context
                .store
                .query_context(&args.targets, &change_type, args.budget)?;
            serde_json::to_string_pretty(&scope_core::stub::context(result))
        }
        Commands::Pack(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let change_type = change_type_name(args.change_type);
            let pack = build_context_pack(&context.store, &args.target, &change_type, args.budget)?;
            if let Some(output_path) = args.output {
                fs::write(&output_path, &pack)
                    .map_err(|error| scope_core::ScopeError::io(output_path.display().to_string(), error))?;
            }
            Ok(pack)
        }
        Commands::Arch(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            match args.command {
                ArchCommand::Check(_) => {
                    let config = load_arch_config(&context.paths.repo_root)?;
                    let result = arch_check(&context.store, &config)?;
                    if !result.violations.is_empty() {
                        exit_code = 1;
                    }
                    serde_json::to_string_pretty(&scope_core::stub::arch_check(result))
                }
            }
        }
        Commands::Doctor(args) => serde_json::to_string_pretty(&scope_core::stub::doctor(args.fix)),
        Commands::Benchmark(args) => serde_json::to_string_pretty(&scope_core::stub::benchmark(
            args.fixture,
            args.iterations,
        )),
    }
    .map_err(|error| scope_core::ScopeError::Serialization(error.to_string()))?;

    println!("{output}");
    Ok(exit_code)
}

fn symbol_kind_name(kind: cli::SymbolKind) -> SymbolKind {
    match kind {
        cli::SymbolKind::Function => SymbolKind::Function,
        cli::SymbolKind::Struct => SymbolKind::Struct,
        cli::SymbolKind::Enum => SymbolKind::Enum,
        cli::SymbolKind::Trait => SymbolKind::Trait,
        cli::SymbolKind::Method => SymbolKind::Method,
        cli::SymbolKind::Module => SymbolKind::Module,
        cli::SymbolKind::Constant => SymbolKind::Constant,
        cli::SymbolKind::Variable => SymbolKind::Variable,
    }
}

fn verbosity(cli: &Cli) -> Verbosity {
    if cli.quiet {
        Verbosity::Quiet
    } else if cli.verbose {
        Verbosity::Verbose
    } else {
        Verbosity::Normal
    }
}

fn change_type_name(change_type: ChangeType) -> String {
    match change_type {
        ChangeType::Body => "body",
        ChangeType::Signature => "signature",
        ChangeType::Rename => "rename",
        ChangeType::Delete => "delete",
        ChangeType::Visibility => "visibility",
        ChangeType::SideEffect => "side-effect",
    }
    .to_string()
}

fn build_context_pack(
    store: &scope_core::Store,
    target: &str,
    change_type: &str,
    budget: usize,
) -> Result<String, scope_core::ScopeError> {
    let context = store.query_context(&[target.to_string()], change_type, Some(budget))?;
    let mut sections = Vec::new();

    let public_surface = format_public_surface(store, target)?;
    if !public_surface.is_empty() {
        sections.push(public_surface);
    }

    let direct_callers = format_direct_callers(store, target)?;
    if !direct_callers.is_empty() {
        sections.push(direct_callers);
    }

    let direct_callees = format_direct_callees(store, target)?;
    if !direct_callees.is_empty() {
        sections.push(direct_callees);
    }

    let transitive_callers = format_transitive_callers(&context.should_read);
    if !transitive_callers.is_empty() {
        sections.push(transitive_callers);
    }

    let change_section = format_change_specific_section(store, target, change_type)?;
    if !change_section.is_empty() {
        sections.push(change_section);
    }

    let header_without_used = vec![
        "=== SCOPE CONTEXT PACK ===".to_string(),
        format!("Target:      {target}"),
        format!("Change type: {change_type}"),
        format!("Budget:      {budget} tokens (approx)"),
        "Used:        0 tokens (approx)".to_string(),
        format!("Schema:      {}", scope_core::SCHEMA_VERSION),
        String::new(),
    ]
    .join("\n");
    let base_overhead = estimate_text_tokens(&header_without_used)
        + estimate_text_tokens(&format!(
            "END SCOPE PACK | schema: {} | truncated: yes",
            scope_core::SCHEMA_VERSION
        ));

    let mut body = String::new();
    let mut body_used = 0usize;
    let mut truncated = context.summary.truncated || base_overhead > budget;
    for section in sections {
        let section_tokens = estimate_text_tokens(&section);
        if base_overhead + body_used + section_tokens > budget {
            truncated = true;
            break;
        }
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&section);
        body_used += section_tokens;
    }

    let footer = format!(
        "END SCOPE PACK | schema: {} | truncated: {}",
        scope_core::SCHEMA_VERSION,
        if truncated { "yes" } else { "no" }
    );

    let header = vec![
        "=== SCOPE CONTEXT PACK ===".to_string(),
        format!("Target:      {target}"),
        format!("Change type: {change_type}"),
        format!("Budget:      {budget} tokens (approx)"),
        format!(
            "Used:        {} tokens (approx)",
            estimate_text_tokens(&header_without_used) + body_used + estimate_text_tokens(&footer)
        ),
        format!("Schema:      {}", scope_core::SCHEMA_VERSION),
        String::new(),
    ]
    .join("\n");

    let mut pack = header;
    if !body.is_empty() {
        pack.push_str("\n\n");
        pack.push_str(&body);
    }
    pack.push_str("\n\n");
    pack.push_str(&footer);

    Ok(pack)
}

fn format_public_surface(store: &scope_core::Store, target: &str) -> Result<String, scope_core::ScopeError> {
    let path = target_file_for_target(store, target)?;
    let Some(path) = path else {
        return Ok(String::new());
    };
    let symbols = store.query_symbols(&path, true, None)?;
    if symbols.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec![format!("--- PUBLIC SURFACE ({}) ---", path.0)];
    for symbol in symbols {
        lines.push(format!(
            "{} | {} | {} | line {}",
            symbol.qualname,
            symbol_kind_label(&symbol.kind),
            visibility_label(&symbol.visibility),
            symbol.span.start_line
        ));
    }
    Ok(lines.join("\n"))
}

fn format_direct_callers(store: &scope_core::Store, target: &str) -> Result<String, scope_core::ScopeError> {
    if !looks_like_symbol(target) {
        return Ok(String::new());
    }
    let records = store.query_callers(target, false)?;
    let records: Vec<_> = records
        .into_iter()
        .filter(|record| matches!(record.certainty, Certainty::Exact | Certainty::Resolved))
        .collect();
    if records.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec!["--- DIRECT CALLERS ---".to_string()];
    for record in records {
        lines.push(format_traversal_line(&record));
    }
    Ok(lines.join("\n"))
}

fn format_direct_callees(store: &scope_core::Store, target: &str) -> Result<String, scope_core::ScopeError> {
    if !looks_like_symbol(target) {
        return Ok(String::new());
    }
    let records = store.query_callees(target, false)?;
    if records.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec!["--- DIRECT CALLEES ---".to_string()];
    for record in records {
        lines.push(format_traversal_line(&record));
    }
    Ok(lines.join("\n"))
}

fn format_transitive_callers(should_read: &[ContextFileRecord]) -> String {
    let nearby: Vec<_> = should_read
        .iter()
        .filter(|record| {
            record.distance == 2
                || record.roles.contains(&ContextFileRole::NearbyContext)
                || record.roles.contains(&ContextFileRole::Importer)
        })
        .collect();
    if nearby.is_empty() {
        return String::new();
    }
    let mut lines = vec!["--- TRANSITIVE CALLERS / NEARBY CONTEXT ---".to_string()];
    for record in nearby {
        lines.push(format_context_record_line(record));
    }
    lines.join("\n")
}

fn format_change_specific_section(
    store: &scope_core::Store,
    target: &str,
    change_type: &str,
) -> Result<String, scope_core::ScopeError> {
    let impacted = store.query_impact(target, change_type, None)?;
    if impacted.is_empty() {
        return Ok(String::new());
    }
    let title = match change_type {
        "rename" => "--- RENAME IMPACT ---",
        "delete" => "--- DELETE IMPACT ---",
        "signature" => "--- SIGNATURE IMPACT ---",
        "body" => "--- BODY IMPACT ---",
        "visibility" => "--- VISIBILITY IMPACT ---",
        "side-effect" => "--- SIDE-EFFECT IMPACT ---",
        _ => "--- IMPACT ---",
    };
    let mut lines = vec![title.to_string()];
    for record in impacted {
        lines.push(format_traversal_line(&record));
    }
    Ok(lines.join("\n"))
}

fn format_traversal_line(record: &scope_core::TraversalRecord) -> String {
    let path = record.path.as_ref().map(|path| path.0.as_str()).unwrap_or("<unknown>");
    let label = record.qualname.as_deref().unwrap_or(path);
    format!(
        "{} | {} | certainty: {} | distance: {} | {}",
        path,
        label,
        certainty_label(&record.certainty),
        record.distance,
        record.reason
    )
}

fn format_context_record_line(record: &ContextFileRecord) -> String {
    format!(
        "{} | tokens: {} | distance: {} | certainty: {} | roles: {}",
        record.path.0,
        record.estimated_tokens,
        record.distance,
        certainty_label(&record.certainty),
        record
            .roles
            .iter()
            .map(context_role_label)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn target_file_for_target(
    store: &scope_core::Store,
    target: &str,
) -> Result<Option<RepoPath>, scope_core::ScopeError> {
    if looks_like_symbol(target) {
        let result = store.query_context(&[target.to_string()], "body", None)?;
        Ok(result.must_read.first().map(|record| record.path.clone()))
    } else {
        Ok(Some(RepoPath::from(target.to_string())))
    }
}

fn looks_like_symbol(target: &str) -> bool {
    target.contains("::") && !target.ends_with(".rs") && !target.ends_with(".ts") && !target.ends_with(".js")
}

fn estimate_text_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

fn certainty_label(certainty: &Certainty) -> &'static str {
    match certainty {
        Certainty::Exact => "exact",
        Certainty::Resolved => "resolved",
        Certainty::Heuristic => "heuristic",
        Certainty::Dynamic => "dynamic",
    }
}

fn visibility_label(visibility: &scope_core::Visibility) -> &'static str {
    match visibility {
        scope_core::Visibility::Local => "local",
        scope_core::Visibility::Module => "module",
        scope_core::Visibility::Package => "package",
        scope_core::Visibility::Public => "public",
        scope_core::Visibility::Unknown => "unknown",
    }
}

fn symbol_kind_label(kind: &scope_core::SymbolKind) -> &'static str {
    match kind {
        scope_core::SymbolKind::Function => "function",
        scope_core::SymbolKind::Method => "method",
        scope_core::SymbolKind::Struct => "struct",
        scope_core::SymbolKind::Class => "class",
        scope_core::SymbolKind::Enum => "enum",
        scope_core::SymbolKind::TypeAlias => "type_alias",
        scope_core::SymbolKind::Module => "module",
        scope_core::SymbolKind::Namespace => "namespace",
        scope_core::SymbolKind::Constant => "constant",
        scope_core::SymbolKind::Static => "static",
        scope_core::SymbolKind::Interface => "interface",
        scope_core::SymbolKind::Trait => "trait",
        scope_core::SymbolKind::Variable => "variable",
    }
}

fn context_role_label(role: &ContextFileRole) -> &'static str {
    match role {
        ContextFileRole::Target => "target",
        ContextFileRole::DefinesTargetSymbol => "defines_target_symbol",
        ContextFileRole::DirectCaller => "direct_caller",
        ContextFileRole::DirectCallee => "direct_callee",
        ContextFileRole::Importer => "importer",
        ContextFileRole::Dependency => "dependency",
        ContextFileRole::NearbyContext => "nearby_context",
    }
}

#[cfg(test)]
mod tests {
    use super::{build_context_pack, index_repo, render_cli_error};
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
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
        std::env::temp_dir().join(format!("scope-cli-{prefix}-{nanos}"))
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

    fn read_golden(name: &str) -> String {
        fs::read_to_string(golden_root().join(name))
            .unwrap()
            .trim_end_matches('\n')
            .to_string()
    }

    #[test]
    fn render_cli_error_returns_machine_readable_json() {
        let output = render_cli_error(&scope_core::ScopeError::InvalidInput("missing target".to_string()));
        let value = match serde_json::from_str::<serde_json::Value>(&output) {
            Ok(value) => value,
            Err(error) => panic!("expected valid JSON output, got error: {error}"),
        };

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["command"], "cli");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert_eq!(value["data"]["message"], "invalid command input: missing target");
        assert_eq!(value["warnings"], serde_json::json!([]));
    }

    #[test]
    fn rust_small_parse_pack_body_matches_golden() {
        let repo = prepare_fixture_copy("rust_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let actual = build_context_pack(&store, "parser::parse", "body", 400).unwrap();
        let expected = read_golden("rust_small_parse_pack_body.txt");

        assert_eq!(actual, expected);
        assert!(actual.contains("--- PUBLIC SURFACE (src/parser.rs) ---"));
        assert!(actual.contains("--- DIRECT CALLERS ---"));
        assert!(actual.contains("--- BODY IMPACT ---"));
        assert!(actual.contains("truncated: no"));

        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn ts_small_verify_token_pack_rename_matches_golden() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let actual = build_context_pack(&store, "auth::middleware::verifyToken", "rename", 400).unwrap();
        let expected = read_golden("ts_small_verify_token_pack_rename.txt");

        assert_eq!(actual, expected);
        assert!(actual.contains("--- PUBLIC SURFACE (src/auth/middleware.ts) ---"));
        assert!(actual.contains("--- DIRECT CALLEES ---"));
        assert!(actual.contains("--- RENAME IMPACT ---"));
        assert!(actual.contains("truncated: no"));

        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn ts_small_verify_token_pack_rename_budget_matches_golden() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let actual = build_context_pack(&store, "auth::middleware::verifyToken", "rename", 120).unwrap();
        let expected = read_golden("ts_small_verify_token_pack_rename_budget.txt");

        assert_eq!(actual, expected);
        assert!(actual.contains("truncated: yes"));

        fs::remove_dir_all(repo).unwrap();
    }
}

fn index_repo(
    repo_root: &std::path::Path,
    store: &scope_core::Store,
) -> Result<usize, scope_core::ScopeError> {
    let entries = scan_repo(repo_root, &ScanConfig::default())?;
    let extracts: Vec<_> = entries
        .into_iter()
        .filter_map(|entry| {
            let adapter = adapter_for_language(entry.language)?;
            if !scope_core::adapters::supports_path(adapter, &entry.absolute_path) {
                return None;
            }
            let source = fs::read_to_string(&entry.absolute_path).ok()?;
            let mut extract = adapter.extract(&entry, &source);
            let metadata = fs::metadata(&entry.absolute_path).ok()?;
            let modified = metadata.modified().ok()?;
            let modified_seconds = modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs() as i64);
            extract.file.content_hash = Some(blake3::hash(source.as_bytes()).to_hex().to_string());
            extract.file.mtime_unix_seconds = modified_seconds;
            extract.file.size_bytes = Some(metadata.len() as i64);
            Some(extract)
        })
        .collect();

    store.persist_extract_results(&extracts)?;

    Ok(extracts.len())
}
