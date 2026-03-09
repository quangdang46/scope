mod cli;

use clap::Parser;
use cli::{ChangeType, Cli, Commands};
use scope_core::SymbolKind;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), scope_core::ScopeError> {
    let cli = Cli::parse();

    let output = match cli.command {
        Commands::Index(args) => serde_json::to_string_pretty(&scope_core::stub::index(
            args.repo_root,
            args.no_git,
            args.watch,
        )),
        Commands::Deps(args) => serde_json::to_string_pretty(&scope_core::stub::deps(
            args.file,
            args.reverse,
            args.transitive,
            args.depth,
        )),
        Commands::Symbols(args) => serde_json::to_string_pretty(&scope_core::stub::symbols(
            args.file,
            args.public_only,
            args.kind.map(symbol_kind_name),
        )),
        Commands::Calls(args) => {
            serde_json::to_string_pretty(&scope_core::stub::calls(args.symbol, args.transitive))
        }
        Commands::Callers(args) => {
            serde_json::to_string_pretty(&scope_core::stub::callers(args.symbol, args.transitive))
        }
        Commands::Impact(args) => serde_json::to_string_pretty(&scope_core::stub::impact(
            args.target,
            change_type_name(args.change_type),
            args.depth,
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
