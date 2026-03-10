use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct RepoPath(pub String);

impl From<&str> for RepoPath {
    fn from(value: &str) -> Self {
        Self(value.replace('\\', "/"))
    }
}

impl From<String> for RepoPath {
    fn from(value: String) -> Self {
        Self(value.replace('\\', "/"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Span {
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParseStatus {
    Ok,
    Partial,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    Exact,
    Resolved,
    Heuristic,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Local,
    Module,
    Package,
    Public,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Class,
    Enum,
    TypeAlias,
    Module,
    Namespace,
    Constant,
    Static,
    Interface,
    Trait,
    Variable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Import,
    Call,
    Export,
    Define,
    Contain,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Symbol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    pub path: RepoPath,
    pub language: String,
    pub parse_status: ParseStatus,
    pub is_barrel: bool,
    pub content_hash: Option<String>,
    pub mtime_unix_seconds: Option<i64>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportPath {
    Relative(RepoPath),
    External(String),
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportRecord {
    pub file: RepoPath,
    pub raw_text: String,
    pub import_path: ImportPath,
    pub span: Span,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleRecord {
    pub file: RepoPath,
    pub name: String,
    pub declared_path: Option<RepoPath>,
    pub is_inline: bool,
    pub span: Span,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportRecord {
    pub file: RepoPath,
    pub name: String,
    pub qualname: Option<String>,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SymbolRecord {
    pub file: RepoPath,
    pub name: String,
    pub qualname: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub exported: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CallSiteRecord {
    pub file: RepoPath,
    pub caller_qualname: Option<String>,
    pub callee_name: String,
    pub callee_qualname: Option<String>,
    pub is_method: bool,
    pub span: Span,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub path: RepoPath,
    pub message: String,
    pub span: Option<Span>,
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractResult {
    pub file: FileRecord,
    pub imports: Vec<ImportRecord>,
    pub modules: Vec<ModuleRecord>,
    pub exports: Vec<ExportRecord>,
    pub symbols: Vec<SymbolRecord>,
    pub call_sites: Vec<CallSiteRecord>,
    pub parse_diagnostics: Vec<ParseDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyRecord {
    pub kind: NodeKind,
    pub path: RepoPath,
    pub edge_kind: EdgeKind,
    pub certainty: Certainty,
    pub import_text: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraversalRecord {
    pub kind: NodeKind,
    pub path: Option<RepoPath>,
    pub qualname: Option<String>,
    pub edge_kind: EdgeKind,
    pub certainty: Certainty,
    pub reason: String,
    pub distance: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ContextFileRole {
    Target,
    DefinesTargetSymbol,
    DirectCaller,
    DirectCallee,
    Importer,
    Dependency,
    NearbyContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextFileRecord {
    pub path: RepoPath,
    pub score: u32,
    pub estimated_tokens: usize,
    pub distance: u32,
    pub certainty: Certainty,
    pub reasons: Vec<String>,
    pub roles: Vec<ContextFileRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextSummary {
    pub targets_count: usize,
    pub must_read_count: usize,
    pub should_read_count: usize,
    pub skipped_count: usize,
    pub estimated_tokens: usize,
    pub budget: Option<usize>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextResult {
    pub targets: Vec<String>,
    pub change_type: String,
    pub budget: Option<usize>,
    pub must_read: Vec<ContextFileRecord>,
    pub should_read: Vec<ContextFileRecord>,
    pub summary: ContextSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicSurfaceSymbol {
    pub file: RepoPath,
    pub name: String,
    pub qualname: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicSurface {
    pub file: RepoPath,
    pub symbols: Vec<PublicSurfaceSymbol>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicSurfaceChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicSurfaceChange {
    pub kind: PublicSurfaceChangeKind,
    pub before: Option<PublicSurfaceSymbol>,
    pub after: Option<PublicSurfaceSymbol>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicSurfaceDiffSummary {
    pub added_count: usize,
    pub removed_count: usize,
    pub modified_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicSurfaceDiff {
    pub before_file: RepoPath,
    pub after_file: RepoPath,
    pub changes: Vec<PublicSurfaceChange>,
    pub summary: PublicSurfaceDiffSummary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ImpactChangeType {
    Body,
    Signature,
    Rename,
    Delete,
    Visibility,
    SideEffect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImpactTraversalRule {
    pub change_type: ImpactChangeType,
    pub summary: String,
    pub traversal_strategy: String,
    pub primary_blast_radius: String,
    pub allowed_edge_kinds: Vec<EdgeKind>,
    pub include_transitive: bool,
    pub default_max_distance: Option<u32>,
    pub include_re_exports: bool,
    pub include_importers: bool,
    pub include_callers: bool,
    pub include_visibility_boundary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchLayer {
    pub name: String,
    pub pattern: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchRule {
    pub from: String,
    pub may_not_import: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ArchConfig {
    #[serde(default, rename = "layer")]
    pub layers: Vec<ArchLayer>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<ArchRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchFileEdge {
    pub from_file: RepoPath,
    pub to_file: RepoPath,
    pub edge_kind: EdgeKind,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchViolation {
    pub from_file: RepoPath,
    pub to_file: RepoPath,
    pub from_layer: String,
    pub to_layer: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchCheckResult {
    pub config_path: RepoPath,
    pub checked_edges: usize,
    pub checked_layered_edges: usize,
    pub violations: Vec<ArchViolation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StabilityCategory {
    Stable,
    StableAbstraction,
    Balanced,
    UnstableAndCentral,
    HealthyLeaf,
    Isolated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StabilityRecord {
    pub path: RepoPath,
    pub fan_in: usize,
    pub fan_out: usize,
    pub instability: f64,
    pub category: StabilityCategory,
    pub flagged: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StabilitySummary {
    pub avg_instability: f64,
    pub flagged_count: usize,
    pub stable_count: usize,
    pub stable_abstraction_count: usize,
    pub balanced_count: usize,
    pub healthy_leaf_count: usize,
    pub isolated_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StabilitySort {
    Instability,
    FanIn,
    FanOut,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StabilityResult {
    pub file: Option<RepoPath>,
    pub flag_threshold: Option<f64>,
    pub sort: StabilitySort,
    pub files: Vec<StabilityRecord>,
    pub summary: StabilitySummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RiskSort {
    Score,
    Churn,
    Dependents,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskRecord {
    pub path: RepoPath,
    pub direct_dependents: usize,
    pub transitive_dependents: usize,
    pub churn_commits: usize,
    pub score: f64,
    pub normalized_score: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskSummary {
    pub git_available: bool,
    pub scored_files: usize,
    pub avg_score: f64,
    pub max_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskResult {
    pub file: Option<RepoPath>,
    pub top: Option<usize>,
    pub days: u32,
    pub sort: RiskSort,
    pub files: Vec<RiskRecord>,
    pub summary: RiskSummary,
}
