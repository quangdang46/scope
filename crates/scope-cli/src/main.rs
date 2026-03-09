mod cli;

use std::env;

use clap::Parser;
use cli::{ChangeType, Cli, Commands};
use scope_core::{
    scan_repo, Adapter, BootstrapOptions, DatabaseInfo, RustAdapter, ScanConfig, SupportedLanguage,
    SymbolKind, Verbosity,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), scope_core::ScopeError> {
    let cli = Cli::parse();
    let cwd = env::current_dir().map_err(|error| scope_core::ScopeError::io(".", error))?;
    let verbosity = verbosity(&cli);

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
        Commands::Impact(args) => serde_json::to_string_pretty(&scope_core::stub::impact(
            args.target,
            change_type_name(args.change_type),
            args.depth,
        )),
        Commands::Explain(args) => serde_json::to_string_pretty(&scope_core::stub::explain(
            args.target,
            args.to,
            args.depth,
        )),
        Commands::Doctor(args) => serde_json::to_string_pretty(&scope_core::stub::doctor(args.fix)),
        Commands::Benchmark(args) => serde_json::to_string_pretty(&scope_core::stub::benchmark(
            args.fixture,
            args.iterations,
        )),
    }
    .map_err(|error| scope_core::ScopeError::Serialization(error.to_string()))?;

    println!("{output}");
    Ok(())
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

fn index_repo(
    repo_root: &std::path::Path,
    store: &scope_core::Store,
) -> Result<usize, scope_core::ScopeError> {
    let adapter = RustAdapter;
    let entries = scan_repo(repo_root, &ScanConfig::default())?;
    let mut indexed_files = 0usize;

    for entry in entries {
        if entry.language != SupportedLanguage::Rust {
            continue;
        }

        if !scope_core::adapters::supports_path(&adapter, &entry.absolute_path) {
            continue;
        }

        let source = std::fs::read_to_string(&entry.absolute_path)
            .map_err(|error| scope_core::ScopeError::io(&entry.absolute_path, error))?;
        let extract = adapter.extract(&entry, &source);
        store.persist_extract_result(&extract)?;
        indexed_files += 1;
    }

    let entries = scan_repo(repo_root, &ScanConfig::default())?;
    for entry in entries {
        if entry.language != SupportedLanguage::Rust {
            continue;
        }

        if !scope_core::adapters::supports_path(&adapter, &entry.absolute_path) {
            continue;
        }

        let source = std::fs::read_to_string(&entry.absolute_path)
            .map_err(|error| scope_core::ScopeError::io(&entry.absolute_path, error))?;
        let extract = adapter.extract(&entry, &source);
        store.refresh_call_edges(&extract)?;
    }

    Ok(indexed_files)
}
