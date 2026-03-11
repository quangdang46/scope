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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotFileRecord {
    pub path: RepoPath,
    pub language: String,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotSymbolRecord {
    pub file: RepoPath,
    pub name: String,
    pub qualname: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub exported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SnapshotEdgeRecord {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotGraph {
    pub schema_version: u32,
    pub snapshot_version: u32,
    pub created_at: i64,
    pub files: Vec<SnapshotFileRecord>,
    pub symbols: Vec<SnapshotSymbolRecord>,
    pub file_edges: Vec<SnapshotEdgeRecord>,
    pub symbol_edges: Vec<SnapshotEdgeRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SnapshotMetadata {
    pub name: String,
    pub created_at: i64,
    pub commit: Option<String>,
    pub schema_version: u32,
    pub snapshot_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotStoredRecord {
    pub metadata: SnapshotMetadata,
    pub graph: SnapshotGraph,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotSaveResult {
    pub snapshot: SnapshotMetadata,
    pub replaced_existing: bool,
    pub summary: SnapshotDiffSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotListResult {
    pub snapshots: Vec<SnapshotMetadata>,
    pub summary: SnapshotListSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotListSummary {
    pub snapshot_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotDeleteResult {
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEdgeDelta {
    pub file_edges_added: usize,
    pub file_edges_removed: usize,
    pub symbol_edges_added: usize,
    pub symbol_edges_removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotCentralityDelta {
    pub path: RepoPath,
    pub before_fan_in: usize,
    pub after_fan_in: usize,
    pub delta: isize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotStabilityDelta {
    pub before_avg_instability: f64,
    pub after_avg_instability: f64,
    pub delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotDiffSummary {
    pub files: usize,
    pub symbols: usize,
    pub file_edges: usize,
    pub symbol_edges: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotDiffResult {
    pub before: SnapshotMetadata,
    pub after: SnapshotMetadata,
    pub edge_delta: SnapshotEdgeDelta,
    pub added_file_edges: Vec<SnapshotEdgeRecord>,
    pub removed_file_edges: Vec<SnapshotEdgeRecord>,
    pub added_symbol_edges: Vec<SnapshotEdgeRecord>,
    pub removed_symbol_edges: Vec<SnapshotEdgeRecord>,
    pub newly_central_files: Vec<SnapshotCentralityDelta>,
    pub introduced_violations: Vec<ArchViolation>,
    pub resolved_violations: Vec<ArchViolation>,
    pub stability: SnapshotStabilityDelta,
    pub surface_diff: PublicSurfaceDiff,
    pub summary: SnapshotDiffSummary,
    pub omitted: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenameEditKind {
    Definition,
    ImportSpecifier,
    ImportPath,
    DeferredCallSite,
    DeferredReexport,
    DeferredDynamicImport,
    DeferredUnknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenameEdit {
    pub start_byte: u32,
    pub end_byte: u32,
    pub line: u32,
    pub before_text: String,
    pub after_text: String,
    pub kind: RenameEditKind,
    pub verified: bool,
    pub deferred_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenamePlanStep {
    pub path: RepoPath,
    pub distance: u32,
    pub certainty: Certainty,
    pub roles: Vec<String>,
    pub reasons: Vec<String>,
    pub edits: Vec<RenameEdit>,
    pub apply_safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenamePlanSummary {
    pub files_considered: usize,
    pub files_planned: usize,
    pub files_skipped: usize,
    pub edits_planned: usize,
    pub safe_edits_planned: usize,
    pub deferred_edits_planned: usize,
    pub applied_files: usize,
    pub applied_edits: usize,
    pub blocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenamePlan {
    pub target: String,
    pub target_file: RepoPath,
    pub old_name: String,
    pub new_name: String,
    pub apply_requested: bool,
    pub force_requested: bool,
    pub applied: bool,
    pub steps: Vec<RenamePlanStep>,
    pub skipped: Vec<RenamePlanStep>,
    pub warnings: Vec<String>,
    pub summary: RenamePlanSummary,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryPointConfig {
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityConfig {
    pub name: String,
    pub pattern: Option<String>,
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub expected_callers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ArchConfig {
    #[serde(default, rename = "layer")]
    pub layers: Vec<ArchLayer>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<ArchRule>,
    #[serde(default, rename = "entry_point")]
    pub entry_points: Vec<EntryPointConfig>,
    #[serde(default, rename = "capability")]
    pub capabilities: Vec<CapabilityConfig>,
    #[serde(default, rename = "tests")]
    pub tests: TestConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchFileEdge {
    pub from_file: RepoPath,
    pub to_file: RepoPath,
    pub edge_kind: EdgeKind,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
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
pub enum EntryPointDetection {
    Config,
    ZeroInDegree,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryPointRecord {
    pub file: RepoPath,
    pub detection: EntryPointDetection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryReachableRecord {
    pub file: RepoPath,
    pub distance: u32,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryListSummary {
    pub entry_points: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryListResult {
    pub entry_points: Vec<EntryPointRecord>,
    pub summary: EntryListSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryConeSummary {
    pub reachable_files: usize,
    pub max_distance: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryConeResult {
    pub entry: RepoPath,
    pub reachable: Vec<EntryReachableRecord>,
    pub summary: EntryConeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryReachesSummary {
    pub reaching_entry_points: usize,
    pub nearest_distance: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryReachesResult {
    pub target: RepoPath,
    pub entry_points: Vec<EntryReachableRecord>,
    pub summary: EntryReachesSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryUnreachableRecord {
    pub file: RepoPath,
    pub last_modified_days_ago: Option<u64>,
    pub exported_symbols: usize,
    pub certainty: Certainty,
    pub certainty_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EntryUnreachableResult {
    pub entry_points: Vec<EntryPointRecord>,
    pub total_files: usize,
    pub reachable_files: usize,
    pub unreachable_files: usize,
    pub min_age_days: Option<u64>,
    pub unreachable: Vec<EntryUnreachableRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditCapabilitySource {
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditEntryReachRecord {
    pub entry_point: RepoPath,
    pub distance: u32,
    pub certainty: Certainty,
    pub expected: bool,
    pub path: Vec<RepoPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditSummary {
    pub capability_sources: usize,
    pub reaching_entry_points: usize,
    pub expected_entry_points: usize,
    pub unexpected_entry_points: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditResult {
    pub capability: String,
    pub capability_sources: Vec<AuditCapabilitySource>,
    pub entry_points: Vec<EntryPointRecord>,
    pub reaches: Vec<AuditEntryReachRecord>,
    pub summary: AuditSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateMetric {
    LayerViolations,
    Cycles,
    MaxFileFanIn,
    ParseErrors,
    UnreachableFiles,
    UnusedExports,
    HealthScore,
    HealthScoreDelta,
    ImportsUnresolvedPct,
    ImportsResolvedPct,
    PublicSurfaceRemoved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateConfig {
    pub metric: GateMetric,
    pub max: Option<f64>,
    pub min: Option<f64>,
    pub max_delta: Option<f64>,
    pub min_delta: Option<f64>,
    pub severity: GateSeverity,
    pub message: Option<String>,
    #[serde(default)]
    pub skip: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GatesConfig {
    #[serde(default, rename = "gate")]
    pub gates: Vec<GateConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthReportMetrics {
    pub total_files: usize,
    pub total_symbols: usize,
    pub total_imports: usize,
    pub unresolved_imports: usize,
    pub imports_unresolved_pct: f64,
    pub imports_resolved_pct: f64,
    pub parse_errors: usize,
    pub layer_violations: usize,
    pub cycles: usize,
    pub max_file_fan_in: usize,
    pub avg_instability: f64,
    pub unreachable_files: usize,
    pub unused_exports: usize,
    pub public_surface_removed: usize,
    pub health_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthReportComparison {
    pub target: String,
    pub baseline_health_score: f64,
    pub health_score_delta: f64,
    pub baseline_layer_violations: usize,
    pub layer_violations_delta: isize,
    pub baseline_cycles: usize,
    pub cycles_delta: isize,
    pub baseline_unreachable_files: usize,
    pub unreachable_files_delta: isize,
    pub baseline_public_surface_removed: usize,
    pub public_surface_removed_delta: isize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthReportResult {
    pub generated_at: i64,
    pub compare: Option<HealthReportComparison>,
    pub metrics: HealthReportMetrics,
    pub risk_hotspots: Vec<RiskRecord>,
    pub arch_violations: Vec<ArchViolation>,
    pub cycles_detail: Vec<CycleRecord>,
    pub unreachable_detail: Vec<EntryUnreachableRecord>,
    pub unused_export_detail: Vec<UnusedRecord>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Pass,
    Warning,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateEvaluation {
    pub metric: GateMetric,
    pub status: GateStatus,
    pub severity: GateSeverity,
    pub current_value: f64,
    pub baseline_value: Option<f64>,
    pub delta: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_delta: Option<f64>,
    pub max_delta: Option<f64>,
    pub message: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateSummary {
    pub total: usize,
    pub passed: usize,
    pub warnings: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateResult {
    pub compare: Option<String>,
    pub report: HealthReportResult,
    pub summary: GateSummary,
    pub evaluations: Vec<GateEvaluation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestConfig {
    pub patterns: Vec<String>,
    pub exclude_patterns: Vec<String>,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            patterns: vec![
                "**/*.test.*".to_string(),
                "**/*.spec.*".to_string(),
                "tests/**".to_string(),
                "**/test_*.rs".to_string(),
                "**/__tests__/**".to_string(),
            ],
            exclude_patterns: vec!["**/__mocks__/**".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapRecord {
    pub path: RepoPath,
    pub distance: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapBuildSummary {
    pub test_files: usize,
    pub covered_source_files: usize,
    pub uncovered_source_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapBuildResult {
    pub tests: Vec<RepoPath>,
    pub summary: TestMapBuildSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapCoversSummary {
    pub covering_tests: usize,
    pub nearest_distance: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapCoversResult {
    pub source_file: RepoPath,
    pub tests: Vec<TestMapRecord>,
    pub summary: TestMapCoversSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapCoveredBySummary {
    pub covered_source_files: usize,
    pub nearest_distance: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapCoveredByResult {
    pub test_file: RepoPath,
    pub covered_files: Vec<TestMapRecord>,
    pub summary: TestMapCoveredBySummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapUncoveredSummary {
    pub source_files_considered: usize,
    pub uncovered_source_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestMapUncoveredResult {
    pub files: Vec<RepoPath>,
    pub summary: TestMapUncoveredSummary,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CochangeSort {
    Score,
    SharedCommits,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CochangeRecord {
    pub path: RepoPath,
    pub shared_commits: usize,
    pub target_commits: usize,
    pub candidate_commits: usize,
    pub score: f64,
    pub normalized_score: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CochangeSummary {
    pub git_available: bool,
    pub target_commits: usize,
    pub related_files: usize,
    pub max_shared_commits: usize,
    pub max_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CochangeResult {
    pub target: RepoPath,
    pub top: Option<usize>,
    pub days: u32,
    pub min_shared_commits: usize,
    pub sort: CochangeSort,
    pub files: Vec<CochangeRecord>,
    pub summary: CochangeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnusedRecord {
    pub file: RepoPath,
    pub name: String,
    pub qualname: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub line: u32,
    pub inbound_references: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnusedSummary {
    pub exported_symbols: usize,
    pub unused_symbols: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnusedResult {
    pub symbols: Vec<UnusedRecord>,
    pub summary: UnusedSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CycleSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CycleRecord {
    pub files: Vec<RepoPath>,
    pub edge_count: usize,
    pub external_dependents: usize,
    pub severity: CycleSeverity,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CyclesSummary {
    pub cycle_count: usize,
    pub low_count: usize,
    pub medium_count: usize,
    pub high_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CyclesResult {
    pub severity: Option<CycleSeverity>,
    pub cycles: Vec<CycleRecord>,
    pub summary: CyclesSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchDiffChangedFile {
    pub path: RepoPath,
    pub dependents: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchDiffAffectedFile {
    pub path: RepoPath,
    pub changed_roots: Vec<RepoPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchDiffSummary {
    pub changed_files: usize,
    pub affected_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchDiffResult {
    pub branch: String,
    pub changed_files: Vec<BranchDiffChangedFile>,
    pub affected_files: Vec<BranchDiffAffectedFile>,
    pub summary: BranchDiffSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeNode {
    pub path: RepoPath,
    pub children: Vec<TreeNode>,
    pub truncated: bool,
    pub cycle: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeSummary {
    pub reverse: bool,
    pub depth: Option<usize>,
    pub nodes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeResult {
    pub target: RepoPath,
    pub reverse: bool,
    pub depth: Option<usize>,
    pub tree: TreeNode,
    pub summary: TreeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitClusterMember {
    pub qualname: String,
    pub name: String,
    pub kind: SymbolKind,
    pub exported: bool,
    pub inbound_calls: usize,
    pub inbound_files: usize,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitCluster {
    pub id: usize,
    pub members: Vec<SplitClusterMember>,
    pub cohesion_score: f64,
    pub suggested_name: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitSummary {
    pub target_symbols: usize,
    pub exported_symbols: usize,
    pub clusters: usize,
    pub isolated_symbols: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitResult {
    pub target: RepoPath,
    pub requested_clusters: Option<usize>,
    pub warnings: Vec<String>,
    pub clusters: Vec<SplitCluster>,
    pub summary: SplitSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MirrorSignature {
    pub language: String,
    pub imports: Vec<String>,
    pub exported_symbol_kinds: Vec<String>,
    pub inbound_neighbor_count: usize,
    pub outbound_neighbor_count: usize,
    pub exported_symbol_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MirrorMatch {
    pub path: RepoPath,
    pub score: f64,
    pub normalized_score: u32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MirrorSummary {
    pub candidates_considered: usize,
    pub matches_returned: usize,
    pub threshold: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MirrorResult {
    pub target: RepoPath,
    pub other: Option<RepoPath>,
    pub threshold: Option<u32>,
    pub top: Option<usize>,
    pub target_signature: MirrorSignature,
    pub other_signature: Option<MirrorSignature>,
    pub similarity_score: Option<f64>,
    pub matches: Vec<MirrorMatch>,
    pub summary: MirrorSummary,
}
