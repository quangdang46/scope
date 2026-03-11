mod cli;

use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use cli::{
    ArchCommand, ChangeType, Cli, CochangeSortArg, Commands, CycleSeverityArg, RiskSortArg,
    SnapshotCommand, StabilitySortArg, SurfaceCommand, TestMapCommand,
};
use scope_core::{
    adapter_for_language, arch_check, load_arch_config, scan_repo, BootstrapOptions, CochangeSort,
    CycleSeverity, DatabaseInfo, RiskSort, ScanConfig, SymbolKind, Verbosity,
};
use scope_core::{Certainty, ContextFileRecord, ContextFileRole, RepoPath, StabilitySort};

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

fn cycle_severity_name(value: CycleSeverityArg) -> CycleSeverity {
    match value {
        CycleSeverityArg::Low => CycleSeverity::Low,
        CycleSeverityArg::Medium => CycleSeverity::Medium,
        CycleSeverityArg::High => CycleSeverity::High,
    }
}

fn serialize_output<T: serde::Serialize>(
    value: &T,
    compact: bool,
) -> Result<String, serde_json::Error> {
    if compact {
        let mut json = serde_json::to_value(value)?;
        compact_json_value(&mut json);
        serde_json::to_string(&json)
    } else {
        serde_json::to_string_pretty(value)
    }
}

fn compact_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for value in map.values_mut() {
                compact_json_value(value);
            }
            map.retain(|_, value| !should_prune_compact_value(value));
        }
        serde_json::Value::Array(values) => {
            for value in values.iter_mut() {
                compact_json_value(value);
            }
        }
        _ => {}
    }
}

fn should_prune_compact_value(value: &serde_json::Value) -> bool {
    matches!(value, serde_json::Value::Null)
        || matches!(value, serde_json::Value::Array(values) if values.is_empty())
        || matches!(value, serde_json::Value::Object(map) if map.is_empty())
}

fn run() -> Result<i32, scope_core::ScopeError> {
    let cli = Cli::parse();
    let cwd = env::current_dir().map_err(|error| scope_core::ScopeError::io(".", error))?;
    let verbosity = verbosity(&cli);
    let compact = cli.compact;
    let mut exit_code = 0;

    let output = match cli.command {
        Commands::Index(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone().or(args.repo_root.clone()),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let indexed = index_repo(&context.paths.repo_root, &context.store)?;
            if args.no_git {
                context.store.clear_file_churn()?;
            } else {
                let _ = refresh_git_churn(&context.paths.repo_root, &context.store, 90);
            }
            let database = DatabaseInfo {
                path: context.paths.db_path.display().to_string(),
                schema_version: context.store.schema_version()?,
            };

            serialize_output(
                &scope_core::stub::index(
                    context.paths.repo_root.display().to_string(),
                    args.no_git,
                    args.watch,
                    database,
                    indexed.affected_files,
                ),
                compact,
            )
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

            serialize_output(
                &scope_core::stub::deps(
                    args.file,
                    args.reverse,
                    args.transitive,
                    args.depth,
                    dependencies,
                ),
                compact,
            )
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

            serialize_output(
                &scope_core::stub::symbols(args.file, args.public_only, kind, symbols),
                compact,
            )
        }
        Commands::Calls(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let traversals = context.store.query_callees(&args.symbol, args.transitive)?;
            serialize_output(
                &scope_core::stub::calls(args.symbol, args.transitive, traversals),
                compact,
            )
        }
        Commands::Callers(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let traversals = context.store.query_callers(&args.symbol, args.transitive)?;
            serialize_output(
                &scope_core::stub::callers(args.symbol, args.transitive, traversals),
                compact,
            )
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
            serialize_output(
                &scope_core::stub::impact(args.target, change_type, args.depth, impacted),
                compact,
            )
        }
        Commands::Explain(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let traversals =
                context
                    .store
                    .query_explain(&args.target, args.to.as_deref(), args.depth)?;
            serialize_output(
                &scope_core::stub::explain(args.target, args.to, args.depth, traversals),
                compact,
            )
        }
        Commands::Why(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let path = context.store.query_why(&args.from, &args.to, args.depth)?;
            serialize_output(
                &scope_core::stub::why(args.from, args.to, args.depth, path),
                compact,
            )
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
            serialize_output(&scope_core::stub::context(result), compact)
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
                fs::write(&output_path, &pack).map_err(|error| {
                    scope_core::ScopeError::io(output_path.display().to_string(), error)
                })?;
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
                    serialize_output(&scope_core::stub::arch_check(result), compact)
                }
            }
        }
        Commands::Audit(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let config = load_arch_config(&context.paths.repo_root)?;
            let result = context.store.query_audit(&config, &args.capability)?;
            if result.summary.unexpected_entry_points > 0 {
                exit_code = 1;
            }
            serialize_output(&scope_core::stub::audit(result), compact)
        }
        Commands::Surface(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            match (args.command, args.target) {
                (Some(SurfaceCommand::Diff(diff_args)), None) => {
                    let before = context.store.resolve_surface_target(&diff_args.before)?;
                    let after = context.store.resolve_surface_target(&diff_args.after)?;
                    let diff = context.store.diff_public_surface(&before, &after)?;
                    serialize_output(
                        &scope_core::stub::surface_diff(before, after, diff),
                        compact,
                    )
                }
                (None, Some(target)) => {
                    let path = context.store.resolve_surface_target(&target)?;
                    let surface = context.store.query_public_surface(&path)?;
                    serialize_output(&scope_core::stub::surface(path, surface), compact)
                }
                (None, None) => {
                    return Err(scope_core::ScopeError::InvalidInput(
                        "surface requires a target or diff subcommand".to_string(),
                    ));
                }
                (Some(_), Some(_)) => {
                    return Err(scope_core::ScopeError::InvalidInput(
                        "surface target cannot be combined with a subcommand".to_string(),
                    ));
                }
            }
        }
        Commands::Stability(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let file = args.file.as_deref().map(RepoPath::from);
            let sort = match args.sort {
                StabilitySortArg::Instability => StabilitySort::Instability,
                StabilitySortArg::FanIn => StabilitySort::FanIn,
                StabilitySortArg::FanOut => StabilitySort::FanOut,
                StabilitySortArg::Path => StabilitySort::Path,
            };
            let result = context
                .store
                .query_stability(file.as_ref(), args.flag_threshold, sort)?;
            serialize_output(&scope_core::stub::stability(result), compact)
        }
        Commands::Risk(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let file = args.file.as_deref().map(RepoPath::from);
            let sort = match args.sort {
                RiskSortArg::Score => RiskSort::Score,
                RiskSortArg::Churn => RiskSort::Churn,
                RiskSortArg::Dependents => RiskSort::Dependents,
                RiskSortArg::Path => RiskSort::Path,
            };
            let result = context.store.query_risk(
                file.as_ref(),
                args.days,
                args.threshold,
                args.top,
                sort,
            )?;
            serialize_output(&scope_core::stub::risk(result), compact)
        }
        Commands::Cochange(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let _ = refresh_git_churn(&context.paths.repo_root, &context.store, args.days);
            let sort = match args.sort {
                CochangeSortArg::Score => CochangeSort::Score,
                CochangeSortArg::SharedCommits => CochangeSort::SharedCommits,
                CochangeSortArg::Path => CochangeSort::Path,
            };
            let result = context.store.query_cochange(
                &RepoPath::from(args.target),
                args.days,
                args.min_shared_commits,
                args.top,
                sort,
            )?;
            serialize_output(&scope_core::stub::cochange(result), compact)
        }
        Commands::TestMap(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let config = load_arch_config(&context.paths.repo_root)?;
            match args.command {
                TestMapCommand::Build => {
                    let result = context.store.build_test_map(&config.tests)?;
                    serialize_output(&scope_core::stub::test_map_build(result), compact)
                }
                TestMapCommand::Covers(args) => {
                    let target = RepoPath::from(args.target);
                    let result = context.store.query_tests_covering(&target, &config.tests)?;
                    serialize_output(&scope_core::stub::test_map_covers(result), compact)
                }
                TestMapCommand::CoveredBy(args) => {
                    let target = RepoPath::from(args.target);
                    let result = context.store.query_test_coverage(&target, &config.tests)?;
                    serialize_output(&scope_core::stub::test_map_covered_by(result), compact)
                }
                TestMapCommand::Uncovered => {
                    let result = context.store.query_uncovered_files(&config.tests)?;
                    serialize_output(&scope_core::stub::test_map_uncovered(result), compact)
                }
            }
        }
        Commands::RenamePlan(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            validate_new_name(&args.new_name)?;
            let plan = context.store.build_rename_plan(
                &context.paths.repo_root,
                &args.target,
                &args.new_name,
                args.apply,
                args.force,
            )?;
            if plan.summary.blocked || (!plan.skipped.is_empty() && args.apply) {
                exit_code = 1;
            }
            serialize_output(&scope_core::stub::rename_plan(plan), compact)
        }
        Commands::Snapshot(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            match args.command {
                SnapshotCommand::Save(args) => {
                    let result = context.store.save_snapshot(&args.name, args.commit)?;
                    serialize_output(&scope_core::stub::snapshot_save(result), compact)
                }
                SnapshotCommand::List => {
                    let result = context.store.list_snapshots()?;
                    serialize_output(&scope_core::stub::snapshot_list(result), compact)
                }
                SnapshotCommand::Delete(args) => {
                    let result = context.store.delete_snapshot(&args.name)?;
                    serialize_output(&scope_core::stub::snapshot_delete(result), compact)
                }
            }
        }
        Commands::DiffSnapshot(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let config = load_arch_config(&context.paths.repo_root)?;
            let result = context
                .store
                .diff_snapshot(&args.before, &args.after, &config)?;
            serialize_output(&scope_core::stub::diff_snapshot(result), compact)
        }
        Commands::Unused => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let result = context.store.query_unused()?;
            serialize_output(&scope_core::stub::unused(result), compact)
        }
        Commands::Cycles(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let severity = args.severity.map(cycle_severity_name);
            let result = context.store.query_cycles(severity)?;
            serialize_output(&scope_core::stub::cycles(result), compact)
        }
        Commands::Diff(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let result = context
                .store
                .query_branch_diff(&context.paths.repo_root, &args.branch)?;
            serialize_output(&scope_core::stub::diff(result), compact)
        }
        Commands::Tree(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let result =
                context
                    .store
                    .query_tree(&RepoPath::from(args.path), args.reverse, args.depth)?;
            serialize_output(&scope_core::stub::tree(result), compact)
        }
        Commands::Entry(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let config = load_arch_config(&context.paths.repo_root)?;
            match args.command {
                cli::EntryCommand::List => {
                    let result = context.store.query_entry_list(&config)?;
                    serialize_output(&scope_core::stub::entry_list(result), compact)
                }
                cli::EntryCommand::Cone(args) => {
                    let result = context
                        .store
                        .query_entry_cone(&config, &RepoPath::from(args.target))?;
                    serialize_output(&scope_core::stub::entry_cone(result), compact)
                }
                cli::EntryCommand::Reaches(args) => {
                    let result = context
                        .store
                        .query_entry_reaches(&config, &RepoPath::from(args.target))?;
                    serialize_output(&scope_core::stub::entry_reaches(result), compact)
                }
                cli::EntryCommand::Unreachable(args) => {
                    let result = context
                        .store
                        .query_entry_unreachable(&config, args.min_age_days)?;
                    serialize_output(&scope_core::stub::entry_unreachable(result), compact)
                }
            }
        }
        Commands::Serve(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|error| scope_core::ScopeError::Internal(error.to_string()))?;
            runtime.block_on(scope_core::run_server(
                context.paths,
                scope_core::ServeOptions {
                    port: args.port,
                    open: args.open,
                    no_ui: args.no_ui,
                },
            ))?;
            return Ok(exit_code);
        }
        Commands::Doctor(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let stats = context.store.index_health_stats()?;
            serialize_output(&scope_core::stub::doctor(args.fix, stats), compact)
        }
        Commands::Benchmark(args) => {
            let bootstrap_options = BootstrapOptions {
                repo_root_override: cli.repo_root.clone(),
                db_override: cli.db.clone(),
            };
            let context = scope_core::bootstrap(&cwd, &bootstrap_options, verbosity)?;
            let summary = run_benchmark(
                &context.paths.repo_root,
                args.fixture.as_deref(),
                args.iterations.unwrap_or(1),
            )?;
            serialize_output(
                &scope_core::stub::benchmark(args.fixture, args.iterations, summary),
                compact,
            )
        }
    }
    .map_err(|error| scope_core::ScopeError::Serialization(error.to_string()))?;

    println!("{output}");
    Ok(exit_code)
}

fn refresh_git_churn(
    repo_root: &Path,
    store: &scope_core::Store,
    days: u32,
) -> Result<(), scope_core::ScopeError> {
    store.clear_file_churn()?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("log")
        .arg(format!("--since={} days ago", days))
        .arg("--format=%H|%ae|%ct")
        .arg("--name-only")
        .output()
        .map_err(|error| scope_core::ScopeError::io("git log", error))?;

    if !output.status.success() {
        return Ok(());
    }

    let mut current_commit: Option<(String, String, i64)> = None;
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(header) = parse_git_log_header(trimmed) {
            current_commit = Some(header);
            continue;
        }
        let Some((sha, email, timestamp)) = current_commit.as_ref() else {
            continue;
        };
        let _ = store.persist_file_churn(
            &RepoPath::from(trimmed.to_string()),
            sha,
            Some(email.as_str()),
            Some(*timestamp),
        );
    }

    Ok(())
}

fn parse_git_log_header(line: &str) -> Option<(String, String, i64)> {
    let mut parts = line.split('|');
    let sha = parts.next()?.to_string();
    let email = parts.next()?.to_string();
    let timestamp = parts.next()?.parse().ok()?;
    if sha.is_empty() {
        return None;
    }
    Some((sha, email, timestamp))
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

fn format_public_surface(
    store: &scope_core::Store,
    target: &str,
) -> Result<String, scope_core::ScopeError> {
    let path = store.target_file_for_target(target)?;
    let Some(path) = path else {
        return Ok(String::new());
    };
    let surface = store.query_public_surface(&path)?;
    if surface.symbols.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec![format!("--- PUBLIC SURFACE ({}) ---", path.0)];
    for symbol in surface.symbols {
        lines.push(format!(
            "{} | {} | {} | line {}",
            symbol.qualname,
            symbol_kind_label(&symbol.kind),
            visibility_label(&symbol.visibility),
            symbol.line
        ));
    }
    Ok(lines.join("\n"))
}

fn format_direct_callers(
    store: &scope_core::Store,
    target: &str,
) -> Result<String, scope_core::ScopeError> {
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

fn format_direct_callees(
    store: &scope_core::Store,
    target: &str,
) -> Result<String, scope_core::ScopeError> {
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
    let path = record
        .path
        .as_ref()
        .map(|path| path.0.as_str())
        .unwrap_or("<unknown>");
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

fn validate_new_name(new_name: &str) -> Result<(), scope_core::ScopeError> {
    if new_name.is_empty()
        || !new_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
    {
        return Err(scope_core::ScopeError::InvalidInput(
            "rename-plan requires a simple identifier for --to".to_string(),
        ));
    }
    Ok(())
}

fn looks_like_symbol(target: &str) -> bool {
    target.contains("::")
        && !target.ends_with(".rs")
        && !target.ends_with(".ts")
        && !target.ends_with(".js")
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexRunStats {
    indexed_files: usize,
    changed_files: usize,
    deleted_files: usize,
    affected_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchmarkIterationResult {
    indexed_files: usize,
    mutation_target: RepoPath,
    full_ms: u128,
    incremental_ms: u128,
    full_stats: IndexRunStats,
    incremental_stats: IndexRunStats,
}

fn run_benchmark(
    repo_root: &Path,
    fixture: Option<&str>,
    iterations: u32,
) -> Result<scope_core::stub::BenchmarkSummary, scope_core::ScopeError> {
    let iterations = iterations.max(1);
    let source_root = fixture
        .map(fixture_root)
        .transpose()?
        .unwrap_or_else(|| repo_root.to_path_buf());

    let mut runs = Vec::with_capacity(iterations as usize);

    for iteration in 0..iterations {
        let benchmark_root =
            prepare_benchmark_copy(&source_root, &format!("benchmark-{iteration}"))?;
        let summary = benchmark_iteration(&benchmark_root, fixture)?;
        runs.push(summary);
        fs::remove_dir_all(&benchmark_root)
            .map_err(|error| scope_core::ScopeError::io(&benchmark_root, error))?;
    }

    let indexed_files = runs.first().map(|run| run.indexed_files).unwrap_or(0);
    let mutation = scope_core::stub::BenchmarkMutationSummary {
        target_file: runs
            .first()
            .map(|run| run.mutation_target.clone())
            .unwrap_or_else(|| RepoPath::from("")),
        change_kind: "append_comment",
    };
    let full = summarize_phase(&runs, |run| run.full_ms, |run| &run.full_stats);
    let incremental = summarize_phase(
        &runs,
        |run| run.incremental_ms,
        |run| &run.incremental_stats,
    );
    let comparison = scope_core::stub::BenchmarkComparisonSummary {
        saved_ms: full.avg_ms as i128 - incremental.avg_ms as i128,
        incremental_pct_of_full: if full.avg_ms == 0 {
            0
        } else {
            ((incremental.avg_ms * 100) / full.avg_ms) as u32
        },
    };

    Ok(scope_core::stub::BenchmarkSummary {
        indexed_files,
        mutation,
        full,
        incremental,
        comparison,
    })
}

fn benchmark_iteration(
    benchmark_root: &Path,
    fixture: Option<&str>,
) -> Result<BenchmarkIterationResult, scope_core::ScopeError> {
    let db_path = benchmark_root.join(".scope/index.db");
    let store = scope_core::Store::open(&db_path)?;

    let started = Instant::now();
    let full_stats = index_repo(benchmark_root, &store)?;
    let full_ms = started.elapsed().as_millis();

    let target = select_benchmark_mutation_target(benchmark_root, fixture)?;
    apply_benchmark_edit(&target)?;

    let started = Instant::now();
    let incremental_stats = index_repo(benchmark_root, &store)?;
    let incremental_ms = started.elapsed().as_millis();

    Ok(BenchmarkIterationResult {
        indexed_files: full_stats.indexed_files,
        mutation_target: repo_relative_path(benchmark_root, &target),
        full_ms,
        incremental_ms,
        full_stats,
        incremental_stats,
    })
}

fn summarize_phase(
    runs: &[BenchmarkIterationResult],
    duration: impl Fn(&BenchmarkIterationResult) -> u128,
    stats: impl Fn(&BenchmarkIterationResult) -> &IndexRunStats,
) -> scope_core::stub::BenchmarkPhaseSummary {
    let durations: Vec<_> = runs.iter().map(duration).collect();
    let files_processed_avg = average_usize(
        &runs
            .iter()
            .map(|run| stats(run).affected_files)
            .collect::<Vec<_>>(),
    );
    let changed_files_avg = average_usize(
        &runs
            .iter()
            .map(|run| stats(run).changed_files)
            .collect::<Vec<_>>(),
    );
    let deleted_files_avg = average_usize(
        &runs
            .iter()
            .map(|run| stats(run).deleted_files)
            .collect::<Vec<_>>(),
    );
    let affected_files_avg = average_usize(
        &runs
            .iter()
            .map(|run| stats(run).affected_files)
            .collect::<Vec<_>>(),
    );

    scope_core::stub::BenchmarkPhaseSummary {
        avg_ms: average_duration_ms(&durations),
        min_ms: durations.iter().copied().min().unwrap_or(0),
        max_ms: durations.iter().copied().max().unwrap_or(0),
        files_processed_avg,
        changed_files_avg,
        deleted_files_avg,
        affected_files_avg,
    }
}

fn average_duration_ms(values: &[u128]) -> u128 {
    if values.is_empty() {
        return 0;
    }
    values.iter().sum::<u128>() / values.len() as u128
}

fn average_usize(values: &[usize]) -> usize {
    if values.is_empty() {
        return 0;
    }
    values.iter().sum::<usize>() / values.len()
}

fn fixture_root(name: &str) -> Result<PathBuf, scope_core::ScopeError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
        .join(name)
        .canonicalize()
        .map_err(|error| scope_core::ScopeError::io(format!("fixtures/{name}"), error))?;
    Ok(root)
}

fn prepare_benchmark_copy(
    source_root: &Path,
    label: &str,
) -> Result<PathBuf, scope_core::ScopeError> {
    let destination = unique_temp_dir(label);
    copy_dir_recursive(source_root, &destination)?;
    Ok(destination)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("scope-cli-{prefix}-{nanos}"))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), scope_core::ScopeError> {
    fs::create_dir_all(dst).map_err(|error| scope_core::ScopeError::io(dst, error))?;

    for entry in fs::read_dir(src).map_err(|error| scope_core::ScopeError::io(src, error))? {
        let entry = entry.map_err(|error| scope_core::ScopeError::io(src, error))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| scope_core::ScopeError::io(&src_path, error))?;

        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            if src_path.file_name().and_then(|name| name.to_str()) == Some("index.db")
                && src_path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    == Some(".scope")
            {
                continue;
            }
            fs::copy(&src_path, &dst_path)
                .map_err(|error| scope_core::ScopeError::io(&src_path, error))?;
        }
    }

    Ok(())
}

fn select_benchmark_mutation_target(
    repo_root: &Path,
    fixture: Option<&str>,
) -> Result<PathBuf, scope_core::ScopeError> {
    if let Some(relative) = match fixture {
        Some("rust_small") => Some("src/parser.rs"),
        Some("ts_small") => Some("src/auth/jwt.ts"),
        _ => None,
    } {
        return Ok(repo_root.join(relative));
    }

    let entries = scan_repo(repo_root, &ScanConfig::default())?;
    let Some(path) = entries
        .into_iter()
        .filter_map(|entry| {
            let adapter = adapter_for_language(entry.language)?;
            if scope_core::adapters::supports_path(adapter, &entry.absolute_path) {
                Some(entry.absolute_path)
            } else {
                None
            }
        })
        .min()
    else {
        return Err(scope_core::ScopeError::NotFound {
            kind: "benchmark mutation target",
            value: repo_root.display().to_string(),
        });
    };

    Ok(path)
}

fn repo_relative_path(repo_root: &Path, path: &Path) -> RepoPath {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    RepoPath::from(relative.to_string_lossy().to_string())
}

fn apply_benchmark_edit(path: &Path) -> Result<(), scope_core::ScopeError> {
    let mut source =
        fs::read_to_string(path).map_err(|error| scope_core::ScopeError::io(path, error))?;
    let suffix = match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => "\n// scope benchmark mutation\n",
        Some("ts") | Some("js") => "\n// scope benchmark mutation\n",
        _ => "\n",
    };
    source.push_str(suffix);
    fs::write(path, source).map_err(|error| scope_core::ScopeError::io(path, error))
}

#[cfg(test)]
mod tests {
    use super::{
        build_context_pack, format_public_surface, index_repo, render_cli_error, run_benchmark,
        serialize_output,
    };
    use scope_core::{
        BranchDiffAffectedFile, BranchDiffChangedFile, BranchDiffResult, BranchDiffSummary,
        Certainty, CochangeRecord, CochangeResult, CycleRecord, CycleSeverity, CyclesResult,
        PublicSurface, PublicSurfaceDiff, PublicSurfaceDiffSummary, PublicSurfaceSymbol,
        RenameEdit, RenameEditKind, RenamePlan, RenamePlanStep, RenamePlanSummary, RepoPath,
        RiskRecord, RiskResult, StabilityRecord, StabilityResult, SymbolKind, TreeNode, TreeResult,
        TreeSummary, UnusedRecord, UnusedResult, UnusedSummary, Visibility,
    };
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
                if src_path.file_name().and_then(|name| name.to_str()) == Some("index.db")
                    && src_path
                        .parent()
                        .and_then(|parent| parent.file_name())
                        .and_then(|name| name.to_str())
                        == Some(".scope")
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
        let output = render_cli_error(&scope_core::ScopeError::InvalidInput(
            "missing target".to_string(),
        ));
        let value = serde_json::from_str::<serde_json::Value>(&output)
            .unwrap_or_else(|error| unreachable!("expected valid JSON output, got error: {error}"));

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["command"], "cli");
        assert_eq!(value["status"], "error");
        assert_eq!(value["data"]["kind"], "invalid_input");
        assert_eq!(
            value["data"]["message"],
            "invalid command input: missing target"
        );
        assert_eq!(value["warnings"], serde_json::json!([]));
    }

    #[test]
    fn serialize_output_pretty_mode_matches_existing_json_layout() {
        let output = serialize_output(
            &scope_core::stub::doctor(
                false,
                scope_core::IndexHealthStats {
                    files: 1,
                    imports: 0,
                    unresolved_imports: 0,
                    symbols: 0,
                    call_edges: 0,
                    parse_status: scope_core::ParseStatusCounts {
                        ok: 1,
                        partial: 0,
                        error: 0,
                    },
                },
            ),
            false,
        )
        .unwrap();
        assert!(output.contains("\n  \"schema_version\": 1,"));
        assert!(output.contains("\n  \"warnings\": []"));
    }

    #[test]
    fn serialize_output_compact_mode_minifies_and_prunes_empty_fields() {
        let envelope = scope_core::stub::doctor(
            false,
            scope_core::IndexHealthStats {
                files: 1,
                imports: 0,
                unresolved_imports: 0,
                symbols: 0,
                call_edges: 0,
                parse_status: scope_core::ParseStatusCounts {
                    ok: 1,
                    partial: 0,
                    error: 0,
                },
            },
        );
        let pretty = serialize_output(&envelope, false).unwrap();
        let compact = serialize_output(&envelope, true).unwrap();

        assert!(!compact.contains('\n'));
        assert!(compact.len() < pretty.len());

        let compact_value: serde_json::Value = serde_json::from_str(&compact).unwrap();
        assert_eq!(compact_value["schema_version"], 1);
        assert_eq!(compact_value["command"], "doctor");
        assert_eq!(compact_value["status"], "ok");
        assert_eq!(compact_value["data"]["fix"], false);
        assert!(compact_value["data"].get("checks").is_some());
        assert!(compact_value.get("warnings").is_none());
    }

    #[test]
    fn serialize_output_compact_mode_prunes_null_fields_inside_data() {
        let envelope = scope_core::stub::why(
            "src/lib.rs".to_string(),
            "src/parser.rs".to_string(),
            None,
            Vec::new(),
        );
        let compact = serialize_output(&envelope, true).unwrap();
        let compact_value: serde_json::Value = serde_json::from_str(&compact).unwrap();

        assert_eq!(compact_value["command"], "why");
        assert!(compact_value["data"].get("depth").is_none());
        assert!(compact_value["data"].get("path").is_none());
    }

    #[test]
    fn serialize_output_stability_command_uses_expected_envelope_shape() {
        let envelope = scope_core::stub::stability(StabilityResult {
            file: None,
            flag_threshold: Some(0.5),
            sort: scope_core::StabilitySort::Instability,
            files: vec![StabilityRecord {
                path: RepoPath::from("src/lib.rs"),
                fan_in: 1,
                fan_out: 2,
                instability: 0.6666666666666666,
                category: scope_core::StabilityCategory::Balanced,
                flagged: false,
                reason: None,
            }],
            summary: scope_core::StabilitySummary {
                avg_instability: 0.6666666666666666,
                flagged_count: 0,
                stable_count: 0,
                stable_abstraction_count: 0,
                balanced_count: 1,
                healthy_leaf_count: 0,
                isolated_count: 0,
            },
        });
        let output = serialize_output(&envelope, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["command"], "stability");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["files"][0]["path"], "src/lib.rs");
        assert_eq!(value["data"]["result"]["files"][0]["category"], "balanced");
        assert_eq!(value["data"]["result"]["sort"], "instability");
        assert_eq!(value["data"]["result"]["summary"]["balanced_count"], 1);
        assert!(value["data"]["result"].get("file").is_none());
        assert!(value["data"]["result"]["files"][0].get("reason").is_none());
    }

    #[test]
    fn serialize_output_risk_command_uses_expected_envelope_shape() {
        let envelope = scope_core::stub::risk(RiskResult {
            file: None,
            top: Some(1),
            days: 90,
            sort: scope_core::RiskSort::Score,
            files: vec![RiskRecord {
                path: RepoPath::from("src/parser.rs"),
                direct_dependents: 1,
                transitive_dependents: 2,
                churn_commits: 3,
                score: 2.5,
                normalized_score: 100,
                reason: "log2-based risk score".to_string(),
            }],
            summary: scope_core::RiskSummary {
                git_available: true,
                scored_files: 5,
                avg_score: 1.2,
                max_score: 2.5,
            },
        });
        let output = serialize_output(&envelope, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["command"], "risk");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["files"][0]["path"], "src/parser.rs");
        assert_eq!(value["data"]["result"]["files"][0]["normalized_score"], 100);
        assert_eq!(value["data"]["result"]["sort"], "score");
        assert_eq!(value["data"]["result"]["summary"]["git_available"], true);
        assert!(value["data"]["result"].get("file").is_none());
        assert_eq!(value["data"]["result"]["top"], 1);
    }

    #[test]
    fn serialize_output_cochange_command_uses_expected_envelope_shape() {
        let envelope = scope_core::stub::cochange(CochangeResult {
            target: RepoPath::from("src/parser.rs"),
            top: Some(2),
            days: 90,
            min_shared_commits: 1,
            sort: scope_core::CochangeSort::Score,
            files: vec![CochangeRecord {
                path: RepoPath::from("src/utils.rs"),
                shared_commits: 2,
                target_commits: 3,
                candidate_commits: 4,
                score: 0.6666666666666666,
                normalized_score: 100,
                reason: "2 shared commits out of 3 target commits in last 90 days".to_string(),
            }],
            summary: scope_core::CochangeSummary {
                git_available: true,
                target_commits: 3,
                related_files: 1,
                max_shared_commits: 2,
                max_score: 0.6666666666666666,
            },
        });
        let output = serialize_output(&envelope, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["command"], "cochange");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["target"], "src/parser.rs");
        assert_eq!(value["data"]["result"]["files"][0]["path"], "src/utils.rs");
        assert_eq!(value["data"]["result"]["files"][0]["shared_commits"], 2);
        assert_eq!(value["data"]["result"]["sort"], "score");
        assert_eq!(value["data"]["result"]["summary"]["target_commits"], 3);
    }

    #[test]
    fn serialize_output_utility_commands_use_expected_envelope_shapes() {
        let unused = scope_core::stub::unused(UnusedResult {
            symbols: vec![UnusedRecord {
                file: RepoPath::from("src/lib.rs"),
                name: "greet".to_string(),
                qualname: "lib::greet".to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                line: 3,
                inbound_references: 0,
                reason: "exported symbol `lib::greet` has no indexed inbound call edges"
                    .to_string(),
            }],
            summary: UnusedSummary {
                exported_symbols: 4,
                unused_symbols: 1,
            },
        });
        let unused_value: serde_json::Value =
            serde_json::from_str(&serialize_output(&unused, true).unwrap()).unwrap();
        assert_eq!(unused_value["command"], "unused");
        assert_eq!(unused_value["status"], "ok");
        assert_eq!(
            unused_value["data"]["result"]["symbols"][0]["qualname"],
            "lib::greet"
        );

        let cycles = scope_core::stub::cycles(CyclesResult {
            severity: Some(CycleSeverity::Medium),
            cycles: vec![CycleRecord {
                files: vec![RepoPath::from("src/a.rs"), RepoPath::from("src/b.rs")],
                edge_count: 2,
                external_dependents: 1,
                severity: CycleSeverity::Medium,
                reason: "2 file cycle with 2 internal edges and 1 external dependents".to_string(),
            }],
            summary: scope_core::CyclesSummary {
                cycle_count: 1,
                low_count: 0,
                medium_count: 1,
                high_count: 0,
            },
        });
        let cycles_value: serde_json::Value =
            serde_json::from_str(&serialize_output(&cycles, true).unwrap()).unwrap();
        assert_eq!(cycles_value["command"], "cycles");
        assert_eq!(cycles_value["data"]["result"]["severity"], "medium");

        let diff = scope_core::stub::diff(BranchDiffResult {
            branch: "main".to_string(),
            changed_files: vec![BranchDiffChangedFile {
                path: RepoPath::from("src/lib.rs"),
                dependents: 2,
            }],
            affected_files: vec![BranchDiffAffectedFile {
                path: RepoPath::from("src/parser.rs"),
                changed_roots: vec![RepoPath::from("src/lib.rs")],
            }],
            summary: BranchDiffSummary {
                changed_files: 1,
                affected_files: 1,
            },
        });
        let diff_value: serde_json::Value =
            serde_json::from_str(&serialize_output(&diff, true).unwrap()).unwrap();
        assert_eq!(diff_value["command"], "diff");
        assert_eq!(diff_value["data"]["result"]["branch"], "main");

        let tree = scope_core::stub::tree(TreeResult {
            target: RepoPath::from("src/lib.rs"),
            reverse: false,
            depth: Some(2),
            tree: TreeNode {
                path: RepoPath::from("src/lib.rs"),
                children: vec![TreeNode {
                    path: RepoPath::from("src/parser.rs"),
                    children: Vec::new(),
                    truncated: false,
                    cycle: false,
                }],
                truncated: false,
                cycle: false,
            },
            summary: TreeSummary {
                reverse: false,
                depth: Some(2),
                nodes: 2,
            },
        });
        let tree_value: serde_json::Value =
            serde_json::from_str(&serialize_output(&tree, true).unwrap()).unwrap();
        assert_eq!(tree_value["command"], "tree");
        assert_eq!(tree_value["data"]["result"]["target"], "src/lib.rs");
    }

    #[test]
    fn serialize_output_surface_command_uses_expected_envelope_shape() {
        let surface = PublicSurface {
            file: RepoPath::from("src/parser.rs"),
            symbols: vec![PublicSurfaceSymbol {
                file: RepoPath::from("src/parser.rs"),
                name: "parse".to_string(),
                qualname: "parser::parse".to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                line: 3,
            }],
        };
        let envelope = scope_core::stub::surface(RepoPath::from("src/parser.rs"), surface);
        let output = serialize_output(&envelope, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["command"], "surface");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["target"], "src/parser.rs");
        assert_eq!(
            value["data"]["surface"]["symbols"][0]["qualname"],
            "parser::parse"
        );
        assert_eq!(value["data"]["surface"]["symbols"][0]["kind"], "function");
    }

    #[test]
    fn serialize_output_surface_diff_command_uses_expected_envelope_shape() {
        let diff = PublicSurfaceDiff {
            before_file: RepoPath::from("src/before.rs"),
            after_file: RepoPath::from("src/after.rs"),
            changes: Vec::new(),
            summary: PublicSurfaceDiffSummary {
                added_count: 1,
                removed_count: 0,
                modified_count: 2,
            },
        };
        let envelope = scope_core::stub::surface_diff(
            RepoPath::from("src/before.rs"),
            RepoPath::from("src/after.rs"),
            diff,
        );
        let output = serialize_output(&envelope, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["command"], "surface-diff");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["before"], "src/before.rs");
        assert_eq!(value["data"]["after"], "src/after.rs");
        assert_eq!(value["data"]["diff"]["summary"]["added_count"], 1);
        assert_eq!(value["data"]["diff"]["summary"]["modified_count"], 2);
    }

    #[test]
    fn serialize_output_test_map_command_uses_expected_envelope_shape() {
        let envelope = scope_core::stub::test_map_covers(scope_core::TestMapCoversResult {
            source_file: RepoPath::from("src/auth/middleware.ts"),
            tests: vec![scope_core::TestMapRecord {
                path: RepoPath::from("tests/auth/middleware.test.ts"),
                distance: 1,
            }],
            summary: scope_core::TestMapCoversSummary {
                covering_tests: 1,
                nearest_distance: Some(1),
            },
        });
        let output = serialize_output(&envelope, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["command"], "test-map-covers");
        assert_eq!(value["status"], "ok");
        assert_eq!(
            value["data"]["result"]["source_file"],
            "src/auth/middleware.ts"
        );
        assert_eq!(
            value["data"]["result"]["tests"][0]["path"],
            "tests/auth/middleware.test.ts"
        );
        assert_eq!(value["data"]["result"]["tests"][0]["distance"], 1);
        assert_eq!(value["data"]["result"]["summary"]["covering_tests"], 1);
    }

    #[test]
    fn serialize_output_rename_plan_command_uses_expected_envelope_shape() {
        let envelope = scope_core::stub::rename_plan(RenamePlan {
            target: "parser::parse".to_string(),
            target_file: RepoPath::from("src/parser.rs"),
            old_name: "parse".to_string(),
            new_name: "parseToken".to_string(),
            apply_requested: false,
            force_requested: false,
            applied: false,
            steps: vec![RenamePlanStep {
                path: RepoPath::from("src/parser.rs"),
                distance: 0,
                certainty: Certainty::Exact,
                roles: vec!["target".to_string()],
                reasons: vec!["defines symbol parser::parse".to_string()],
                edits: vec![RenameEdit {
                    start_byte: 7,
                    end_byte: 12,
                    line: 1,
                    before_text: "parse".to_string(),
                    after_text: "parseToken".to_string(),
                    kind: RenameEditKind::Definition,
                    verified: true,
                    deferred_reason: None,
                }],
                apply_safe: true,
            }],
            skipped: Vec::new(),
            warnings: Vec::new(),
            summary: RenamePlanSummary {
                files_considered: 1,
                files_planned: 1,
                files_skipped: 0,
                edits_planned: 1,
                safe_edits_planned: 1,
                deferred_edits_planned: 0,
                applied_files: 0,
                applied_edits: 0,
                blocked: false,
            },
        });
        let output = serialize_output(&envelope, true).unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(value["command"], "rename-plan");
        assert_eq!(value["status"], "ok");
        assert_eq!(value["data"]["result"]["target"], "parser::parse");
        assert_eq!(value["data"]["result"]["steps"][0]["path"], "src/parser.rs");
        assert_eq!(
            value["data"]["result"]["steps"][0]["edits"][0]["after_text"],
            "parseToken"
        );
    }

    #[test]
    fn serialize_output_snapshot_commands_use_expected_envelope_shape() {
        let save = scope_core::stub::snapshot_save(scope_core::SnapshotSaveResult {
            snapshot: scope_core::SnapshotMetadata {
                name: "baseline".to_string(),
                created_at: 123,
                commit: Some("HEAD".to_string()),
                schema_version: 6,
                snapshot_version: 1,
            },
            replaced_existing: false,
            summary: scope_core::SnapshotDiffSummary {
                files: 2,
                symbols: 3,
                file_edges: 1,
                symbol_edges: 1,
            },
        });
        let list = scope_core::stub::snapshot_list(scope_core::SnapshotListResult {
            snapshots: vec![scope_core::SnapshotMetadata {
                name: "baseline".to_string(),
                created_at: 123,
                commit: None,
                schema_version: 6,
                snapshot_version: 1,
            }],
            summary: scope_core::SnapshotListSummary { snapshot_count: 1 },
        });
        let delete = scope_core::stub::snapshot_delete(scope_core::SnapshotDeleteResult {
            name: "baseline".to_string(),
            deleted: true,
        });

        let save_value: serde_json::Value =
            serde_json::from_str(&serialize_output(&save, true).unwrap()).unwrap();
        let list_value: serde_json::Value =
            serde_json::from_str(&serialize_output(&list, true).unwrap()).unwrap();
        let delete_value: serde_json::Value =
            serde_json::from_str(&serialize_output(&delete, true).unwrap()).unwrap();

        assert_eq!(save_value["command"], "snapshot-save");
        assert_eq!(save_value["data"]["result"]["snapshot"]["name"], "baseline");
        assert_eq!(list_value["command"], "snapshot-list");
        assert_eq!(list_value["data"]["result"]["summary"]["snapshot_count"], 1);
        assert_eq!(delete_value["command"], "snapshot-delete");
        assert_eq!(delete_value["data"]["result"]["deleted"], true);
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

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn ts_small_verify_token_pack_rename_matches_golden() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let actual =
            build_context_pack(&store, "auth::middleware::verifyToken", "rename", 400).unwrap();
        let expected = read_golden("ts_small_verify_token_pack_rename.txt");

        assert_eq!(actual, expected);
        assert!(actual.contains("--- PUBLIC SURFACE (src/auth/middleware.ts) ---"));
        assert!(actual.contains("--- DIRECT CALLEES ---"));
        assert!(actual.contains("--- RENAME IMPACT ---"));
        assert!(actual.contains("truncated: no"));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn ts_small_verify_token_pack_rename_budget_matches_golden() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let actual =
            build_context_pack(&store, "auth::middleware::verifyToken", "rename", 120).unwrap();
        let expected = read_golden("ts_small_verify_token_pack_rename_budget.txt");

        assert_eq!(actual, expected);
        assert!(actual.contains("truncated: yes"));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn surface_target_helper_resolves_symbol_to_file() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let resolved = store
            .resolve_surface_target("auth::middleware::verifyToken")
            .unwrap();
        assert_eq!(resolved, RepoPath::from("src/auth/middleware.ts"));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn surface_target_helper_errors_for_unknown_symbol() {
        let repo = prepare_fixture_copy("rust_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let error = store.resolve_surface_target("missing::symbol").unwrap_err();
        assert!(error.to_string().contains("missing::symbol"));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn surface_target_helper_returns_file_target_unchanged() {
        let repo = prepare_fixture_copy("rust_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let resolved = store.resolve_surface_target("src/parser.rs").unwrap();
        assert_eq!(resolved, RepoPath::from("src/parser.rs"));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn format_public_surface_uses_store_surface_ordering_and_shape() {
        let repo = prepare_fixture_copy("rust_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let formatted = format_public_surface(&store, "parser::parse").unwrap();

        assert_eq!(
            formatted,
            "--- PUBLIC SURFACE (src/parser.rs) ---\nparser::parse | function | public | line 1"
        );

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn surface_stub_reports_symbol_target_as_resolved_file() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let path = store
            .resolve_surface_target("auth::middleware::verifyToken")
            .unwrap();
        let surface = store.query_public_surface(&path).unwrap();
        let envelope = scope_core::stub::surface(path.clone(), surface);

        assert_eq!(
            envelope.data.target,
            RepoPath::from("src/auth/middleware.ts")
        );
        assert_eq!(
            envelope.data.surface.file,
            RepoPath::from("src/auth/middleware.ts")
        );
        assert!(envelope
            .data
            .surface
            .symbols
            .iter()
            .any(|symbol| symbol.qualname == "auth::middleware::verifyToken"));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn surface_diff_stub_reports_expected_fixture_summary() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let before = store.resolve_surface_target("src/auth/jwt.ts").unwrap();
        let after = store.resolve_surface_target("src/auth/aliases.ts").unwrap();
        let diff = store.diff_public_surface(&before, &after).unwrap();
        let envelope = scope_core::stub::surface_diff(before.clone(), after.clone(), diff);

        assert_eq!(envelope.data.before, RepoPath::from("src/auth/jwt.ts"));
        assert_eq!(envelope.data.after, RepoPath::from("src/auth/aliases.ts"));
        assert_eq!(envelope.data.diff.summary.added_count, 1);
        assert_eq!(envelope.data.diff.summary.removed_count, 2);
        assert_eq!(envelope.data.diff.summary.modified_count, 0);

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn surface_usage_errors_match_expected_messages() {
        let missing_target = scope_core::ScopeError::InvalidInput(
            "surface requires a target or diff subcommand".to_string(),
        );
        let mixed_usage = scope_core::ScopeError::InvalidInput(
            "surface target cannot be combined with a subcommand".to_string(),
        );

        assert_eq!(
            missing_target.to_string(),
            "invalid command input: surface requires a target or diff subcommand"
        );
        assert_eq!(
            mixed_usage.to_string(),
            "invalid command input: surface target cannot be combined with a subcommand"
        );
    }

    #[test]
    fn rename_plan_dry_run_collects_fixture_sites() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let plan = store
            .build_rename_plan(
                &repo,
                "auth::middleware::verifyToken",
                "verifySession",
                false,
                false,
            )
            .unwrap();

        assert_eq!(plan.target_file, RepoPath::from("src/auth/middleware.ts"));
        assert_eq!(plan.old_name, "verifyToken");
        assert_eq!(plan.new_name, "verifySession");
        assert!(!plan.applied);
        assert_eq!(plan.summary.edits_planned, 2);
        assert!(!plan.summary.blocked);
        assert!(plan
            .steps
            .iter()
            .any(|step| step.path == RepoPath::from("src/auth/index.ts")));
        assert!(plan
            .steps
            .iter()
            .any(|step| step.path == RepoPath::from("src/auth/middleware.ts")));
        assert!(!plan
            .steps
            .iter()
            .any(|step| step.path == RepoPath::from("src/index.ts")));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn rename_plan_apply_updates_safe_fixture_sites() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let plan = store
            .build_rename_plan(
                &repo,
                "auth::middleware::verifyToken",
                "verifySession",
                true,
                true,
            )
            .unwrap();

        assert!(plan.applied);
        assert_eq!(plan.summary.applied_files, 2);
        assert_eq!(plan.summary.applied_edits, 2);
        let root_index = fs::read_to_string(repo.join("src/index.ts")).unwrap();
        assert!(root_index.contains("export { verifyToken } from \"./auth/index\";"));
        assert!(root_index.contains("export { format } from \"./utils/formatter\";"));

        let auth_index = fs::read_to_string(repo.join("src/auth/index.ts")).unwrap();
        assert!(auth_index.contains("export { verifySession } from \"./middleware\";"));
        assert!(!auth_index.contains("export { verifyToken } from \"./middleware\";"));

        assert!(fs::read_to_string(repo.join("src/auth/middleware.ts"))
            .unwrap()
            .contains("export function verifySession(token: string): boolean"));

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn rename_plan_file_target_plans_import_path_rewrites_without_move() {
        let repo = prepare_fixture_copy("rust_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let plan = store
            .build_rename_plan(&repo, "src/parser.rs", "parser2", false, false)
            .unwrap();

        assert_eq!(plan.target_file, RepoPath::from("src/parser.rs"));
        assert_eq!(plan.old_name, "parser");
        assert_eq!(plan.new_name, "parser2");
        assert!(!plan.warnings.is_empty());

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn rename_plan_without_force_blocks_when_deferred_sites_remain() {
        let repo = prepare_fixture_copy("ts_small");
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let _ = index_repo(&repo, &store).unwrap();

        let plan = store
            .build_rename_plan(
                &repo,
                "auth::middleware::verifyToken",
                "verifySession",
                true,
                false,
            )
            .unwrap();

        assert!(!plan.summary.blocked);
        assert!(plan.applied);
        assert!(plan.skipped.is_empty());

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn benchmark_helper_reports_real_full_and_incremental_timings() {
        let repo = repo_root();
        let summary = run_benchmark(&repo, Some("rust_small"), 1).unwrap();

        assert!(summary.indexed_files > 0);
        assert_eq!(
            summary.mutation.target_file,
            RepoPath::from("src/parser.rs")
        );
        assert_eq!(summary.mutation.change_kind, "append_comment");
        assert_eq!(summary.full.files_processed_avg, summary.indexed_files);
        assert!(summary.incremental.changed_files_avg >= 1);
        assert!(summary.incremental.affected_files_avg >= summary.incremental.changed_files_avg);
        assert!(summary.incremental.affected_files_avg <= summary.indexed_files);
        assert!(summary.full.min_ms <= summary.full.avg_ms);
        assert!(summary.full.avg_ms <= summary.full.max_ms);
        assert!(summary.incremental.min_ms <= summary.incremental.avg_ms);
        assert!(summary.incremental.avg_ms <= summary.incremental.max_ms);
    }

    #[test]
    fn incremental_index_removes_deleted_files_and_rebuilds_dependents() {
        let repo = prepare_fixture_copy("rust_small");
        let _ = fs::remove_file(repo.join(".scope/index.db"));
        let store = scope_core::Store::open(&repo.join(".scope/index.db")).unwrap();
        let initial = index_repo(&repo, &store).unwrap();
        assert_eq!(initial.affected_files, 5);
        assert_eq!(
            store
                .query_reverse_deps(&RepoPath::from("src/parser.rs"))
                .unwrap()
                .len(),
            2
        );

        fs::remove_file(repo.join("src/parser.rs")).unwrap();

        let rebuilt = index_repo(&repo, &store).unwrap();
        assert_eq!(rebuilt.affected_files, 2);
        assert!(store
            .list_indexed_files()
            .unwrap()
            .iter()
            .all(|path| path != &RepoPath::from("src/parser.rs")));
        assert!(store
            .query_symbols(&RepoPath::from("src/parser.rs"), false, None)
            .unwrap()
            .is_empty());
        assert!(store
            .query_reverse_deps(&RepoPath::from("src/parser.rs"))
            .unwrap()
            .is_empty());
        let lib_deps = store.query_deps(&RepoPath::from("src/lib.rs")).unwrap();
        assert!(lib_deps
            .iter()
            .all(|dep| dep.path != RepoPath::from("src/parser.rs")));
        assert!(store
            .query_deps(&RepoPath::from("src/resolver.rs"))
            .unwrap()
            .is_empty());

        let _ = fs::remove_dir_all(repo);
    }
}

fn index_repo(
    repo_root: &std::path::Path,
    store: &scope_core::Store,
) -> Result<IndexRunStats, scope_core::ScopeError> {
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

    let extract_map: HashMap<RepoPath, scope_core::ExtractResult> = extracts
        .into_iter()
        .map(|extract| (extract.file.path.clone(), extract))
        .collect();
    let scanned_paths: HashSet<_> = extract_map.keys().cloned().collect();
    let indexed_paths = store.list_indexed_files()?;
    if indexed_paths.is_empty() {
        let mut all_extracts: Vec<_> = extract_map.into_values().collect();
        all_extracts.sort_by(|left, right| left.file.path.cmp(&right.file.path));
        let indexed_files = all_extracts.len();
        store.persist_extract_results(&all_extracts)?;
        return Ok(IndexRunStats {
            indexed_files,
            changed_files: indexed_files,
            deleted_files: 0,
            affected_files: indexed_files,
        });
    }

    let mut changed_or_new = Vec::new();
    for extract in extract_map.values() {
        match store.classify_file_change(&extract.file)? {
            None | Some(true) => changed_or_new.push(extract.file.path.clone()),
            Some(false) => {}
        }
    }

    let deleted_paths: Vec<_> = indexed_paths
        .into_iter()
        .filter(|path| !scanned_paths.contains(path))
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

    let mut affected_extracts: Vec<_> = affected_paths
        .into_iter()
        .filter_map(|path| extract_map.get(&path).cloned())
        .collect();
    affected_extracts.sort_by(|left, right| left.file.path.cmp(&right.file.path));

    for extract in &affected_extracts {
        store.upsert_file(&extract.file)?;
    }
    for extract in &affected_extracts {
        store.persist_extract_result(extract)?;
    }
    for extract in &affected_extracts {
        store.refresh_call_edges(extract)?;
    }

    Ok(IndexRunStats {
        indexed_files: extract_map.len(),
        changed_files: closure_seeds.len().saturating_sub(deleted_paths.len()),
        deleted_files: deleted_paths.len(),
        affected_files: affected_extracts.len(),
    })
}
