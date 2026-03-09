use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "scope")]
#[command(about = "Local static analysis engine for dependency and impact queries")]
pub struct Cli {
    #[arg(long, global = true)]
    pub repo_root: Option<PathBuf>,
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,
    #[arg(long, global = true)]
    pub verbose: bool,
    #[arg(long, global = true)]
    pub quiet: bool,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Build or refresh the repository index
    Index(IndexArgs),
    /// Query file dependencies
    Deps(DepsArgs),
    /// Query symbols defined in a file
    Symbols(SymbolsArgs),
    /// Query what a symbol calls
    Calls(CallsArgs),
    /// Query what calls a symbol
    Callers(CallersArgs),
    /// Estimate static impact for a change target
    Impact(ImpactArgs),
}

#[derive(Debug, clap::Args)]
pub struct IndexArgs {
    pub repo_root: Option<PathBuf>,
    #[arg(long)]
    pub no_git: bool,
    #[arg(long)]
    pub watch: bool,
}

#[derive(Debug, clap::Args)]
pub struct DepsArgs {
    pub file: String,
    #[arg(long)]
    pub reverse: bool,
    #[arg(long)]
    pub transitive: bool,
    #[arg(long)]
    pub depth: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub struct SymbolsArgs {
    pub file: String,
    #[arg(long)]
    pub public_only: bool,
    #[arg(long)]
    pub kind: Option<SymbolKind>,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Method,
    Module,
    Constant,
    Variable,
}

#[derive(Debug, clap::Args)]
pub struct CallsArgs {
    pub symbol: String,
    #[arg(long)]
    pub transitive: bool,
}

#[derive(Debug, clap::Args)]
pub struct CallersArgs {
    pub symbol: String,
    #[arg(long)]
    pub transitive: bool,
}

#[derive(Debug, clap::Args)]
pub struct ImpactArgs {
    pub target: String,
    #[arg(long, value_enum)]
    pub change_type: ChangeType,
    #[arg(long)]
    pub depth: Option<usize>,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum ChangeType {
    Body,
    Signature,
    Rename,
    Delete,
    Visibility,
    SideEffect,
}
