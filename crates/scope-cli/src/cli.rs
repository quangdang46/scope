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
    #[arg(long, global = true)]
    pub compact: bool,
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
    /// Trace entry points that can reach a configured sensitive capability
    Audit(AuditArgs),
    /// Query the public API surface for a file or symbol target
    Surface(SurfaceArgs),
    /// Report Martin instability scores from indexed file dependencies
    Stability(StabilityArgs),
    /// Report churn-weighted risk scores from indexed file dependencies
    Risk(RiskArgs),
    /// Report files that frequently change with a target file across recent commits
    Cochange(CochangeArgs),
    /// Build static test coverage topology from file imports
    TestMap(TestMapArgs),
    /// Build a conservative rename execution plan for a file or symbol target
    RenamePlan(RenamePlanArgs),
    /// Save, list, or delete stored architectural snapshots
    Snapshot(SnapshotArgs),
    /// Compare two saved architectural snapshots
    DiffSnapshot(DiffSnapshotArgs),
    /// Simulate extraction of symbols into a new file without changing the index
    Simulate(SimulateArgs),
    /// Report exported symbols with no indexed inbound references
    Unused,
    /// Report circular file dependency chains
    Cycles(CyclesArgs),
    /// Report blast radius for files changed relative to a git branch/ref
    Diff(DiffArgs),
    /// Render a recursive dependency tree for an indexed file
    Tree(TreeArgs),
    /// Suggest decomposition clusters for a large indexed file
    Split(SplitArgs),
    /// Compare a file against structurally similar indexed files
    Mirror(MirrorArgs),
    /// Detect entry points and analyze file reachability
    Entry(EntryArgs),
    /// Start a local HTTP API server with a minimal embedded UI
    Serve(ServeArgs),
    /// Explore the indexed graph with a composable query language
    Query(QueryArgs),
    /// Inspect repository and index health
    Doctor(DoctorArgs),
    /// Benchmark full versus incremental indexing on an isolated repo copy
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
pub struct AuditArgs {
    #[arg(long)]
    pub capability: String,
}

#[derive(Debug, clap::Args)]
pub struct SurfaceArgs {
    #[command(subcommand)]
    pub command: Option<SurfaceCommand>,
    pub target: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum SurfaceCommand {
    /// Compare public API surface between two indexed files or symbol targets
    Diff(SurfaceDiffArgs),
}

#[derive(Debug, clap::Args)]
pub struct SurfaceDiffArgs {
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum StabilitySortArg {
    Instability,
    FanIn,
    FanOut,
    Path,
}

#[derive(Debug, clap::Args)]
pub struct StabilityArgs {
    #[arg(long)]
    pub file: Option<String>,
    #[arg(long)]
    pub flag_threshold: Option<f64>,
    #[arg(long, value_enum, default_value_t = StabilitySortArg::Instability)]
    pub sort: StabilitySortArg,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum RiskSortArg {
    Score,
    Churn,
    Dependents,
    Path,
}

#[derive(Debug, clap::Args)]
pub struct RiskArgs {
    #[arg(long)]
    pub file: Option<String>,
    #[arg(long, default_value_t = 90)]
    pub days: u32,
    #[arg(long)]
    pub threshold: Option<f64>,
    #[arg(long)]
    pub top: Option<usize>,
    #[arg(long, value_enum, default_value_t = RiskSortArg::Score)]
    pub sort: RiskSortArg,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum CochangeSortArg {
    Score,
    SharedCommits,
    Path,
}

#[derive(Debug, clap::Args)]
pub struct CochangeArgs {
    pub target: String,
    #[arg(long, default_value_t = 90)]
    pub days: u32,
    #[arg(long, default_value_t = 1)]
    pub min_shared_commits: usize,
    #[arg(long)]
    pub top: Option<usize>,
    #[arg(long, value_enum, default_value_t = CochangeSortArg::Score)]
    pub sort: CochangeSortArg,
}

#[derive(Debug, clap::Args)]
pub struct TestMapArgs {
    #[command(subcommand)]
    pub command: TestMapCommand,
}

#[derive(Debug, Subcommand)]
pub enum TestMapCommand {
    /// Detect test files and summarize static coverage topology
    Build,
    /// Show which tests statically cover a source file
    Covers(TestMapTargetArgs),
    /// Show which source files are statically covered by a test file
    CoveredBy(TestMapTargetArgs),
    /// List indexed non-test files with no static test coverage
    Uncovered,
}

#[derive(Debug, clap::Args)]
pub struct TestMapTargetArgs {
    pub target: String,
}

#[derive(Debug, clap::Args)]
pub struct RenamePlanArgs {
    pub target: String,
    #[arg(long = "to")]
    pub new_name: String,
    #[arg(long)]
    pub apply: bool,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: SnapshotCommand,
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    /// Save the current indexed graph as a named snapshot
    Save(SnapshotSaveArgs),
    /// List saved snapshots
    List,
    /// Delete a saved snapshot by name
    Delete(SnapshotDeleteArgs),
}

#[derive(Debug, clap::Args)]
pub struct SnapshotSaveArgs {
    #[arg(long)]
    pub name: String,
    #[arg(long)]
    pub commit: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct SnapshotDeleteArgs {
    pub name: String,
}

#[derive(Debug, clap::Args)]
pub struct DiffSnapshotArgs {
    pub before: String,
    pub after: String,
}

#[derive(Debug, clap::Args)]
pub struct SimulateArgs {
    #[command(subcommand)]
    pub command: SimulateCommand,
}

#[derive(Debug, Subcommand)]
pub enum SimulateCommand {
    /// Simulate extracting symbols into a new file without persisting changes
    Extract(SimulateExtractArgs),
}

#[derive(Debug, clap::Args)]
pub struct SimulateExtractArgs {
    pub symbols: String,
    #[arg(long = "into")]
    pub into_file: String,
}

#[derive(Debug, Clone, ValueEnum)]
pub enum CycleSeverityArg {
    Low,
    Medium,
    High,
}

#[derive(Debug, clap::Args)]
pub struct CyclesArgs {
    #[arg(long, value_enum)]
    pub severity: Option<CycleSeverityArg>,
}

#[derive(Debug, clap::Args)]
pub struct DiffArgs {
    pub branch: String,
}

#[derive(Debug, clap::Args)]
pub struct TreeArgs {
    pub path: String,
    #[arg(long)]
    pub reverse: bool,
    #[arg(long)]
    pub depth: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub struct SplitArgs {
    pub target: String,
    #[arg(long)]
    pub clusters: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub struct MirrorArgs {
    pub target: String,
    #[arg(long)]
    pub other: Option<String>,
    #[arg(long)]
    pub threshold: Option<u32>,
    #[arg(long)]
    pub top: Option<usize>,
}

#[derive(Debug, clap::Args)]
pub struct EntryArgs {
    #[command(subcommand)]
    pub command: EntryCommand,
}

#[derive(Debug, Subcommand)]
pub enum EntryCommand {
    /// List detected entry points
    List,
    /// Show files reachable from an entry point
    Cone(EntryTargetArgs),
    /// Show which entry points can reach a target file
    Reaches(EntryTargetArgs),
    /// Show indexed files reachable from no detected entry point
    Unreachable(EntryUnreachableArgs),
}

#[derive(Debug, clap::Args)]
pub struct EntryTargetArgs {
    pub target: String,
}

#[derive(Debug, clap::Args)]
pub struct EntryUnreachableArgs {
    #[arg(long)]
    pub min_age_days: Option<u64>,
}

#[derive(Debug, clap::Args)]
pub struct ServeArgs {
    #[arg(long, default_value_t = 7777)]
    pub port: u16,
    #[arg(long)]
    pub open: bool,
    #[arg(long)]
    pub no_ui: bool,
}

#[derive(Debug, clap::Args)]
pub struct QueryArgs {
    #[arg(long)]
    pub expr: Option<String>,
}

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
