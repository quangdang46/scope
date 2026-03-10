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
    /// Explain why a file or symbol appears in impact results
    Explain(ExplainArgs),
    /// Explain the shortest path connecting two files or symbols
    Why(WhyArgs),
    /// Recommend the minimum file set to read before making a change
    Context(ContextArgs),
    /// Generate a budgeted plain-text context pack for an agent
    Pack(PackArgs),
    /// Check architecture rules against indexed file dependencies
    Arch(ArchArgs),
    /// Inspect repository and index health
    Doctor(DoctorArgs),
    /// Run benchmark scaffolding for future performance work
    Benchmark(BenchmarkArgs),
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

#[derive(Debug, Clone, ValueEnum, Default)]
pub enum ChangeType {
    #[default]
    Body,
    Signature,
    Rename,
    Delete,
    Visibility,
    SideEffect,
}

#[derive(Debug, clap::Args)]
pub struct ExplainArgs {
    pub target: String,
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long)]
    pub depth: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub struct WhyArgs {
    pub from: String,
    pub to: String,
    #[arg(long)]
    pub depth: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub struct ContextArgs {
    #[arg(long = "target", required = true)]
    pub targets: Vec<String>,
    #[arg(long, value_enum)]
    pub change_type: ChangeType,
    #[arg(long)]
    pub budget: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub struct PackArgs {
    pub target: String,
    #[arg(long, value_enum, default_value_t = ChangeType::Body)]
    pub change_type: ChangeType,
    #[arg(long)]
    pub budget: usize,
    #[arg(long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct ArchArgs {
    #[command(subcommand)]
    pub command: ArchCommand,
}

#[derive(Debug, Subcommand)]
pub enum ArchCommand {
    /// Check layer rules against direct indexed file edges
    Check(ArchCheckArgs),
}

#[derive(Debug, clap::Args)]
pub struct ArchCheckArgs {}

#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    #[arg(long)]
    pub fix: bool,
}

#[derive(Debug, clap::Args)]
pub struct BenchmarkArgs {
    #[arg(long)]
    pub fixture: Option<String>,
    #[arg(long)]
    pub iterations: Option<u32>,
}
