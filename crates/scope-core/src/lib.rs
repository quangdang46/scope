pub mod adapters;
pub mod arch;
pub mod bootstrap;
pub mod config;
pub mod error;
pub mod indexer;
pub mod install;
pub mod json;
pub mod model;
pub mod query_lang;
pub mod scanner;
pub mod serve;
pub mod snapshot;
pub mod store;
pub mod stub;
pub mod tracing;

pub use adapters::{adapter_for_language, Adapter, RustAdapter, TsJsAdapter};
pub use arch::{
    arch_check, arch_check_edges, arch_explain, arch_init, load_arch_config, validate_gate_config,
};
pub use bootstrap::{bootstrap, AppContext};
pub use config::{load_tsconfig, BootstrapOptions, RuntimePaths};
pub use error::{ScopeError, ScopeResult};
pub use indexer::{index_repo, IndexRunStats};
pub use install::{
    auto_install_user_mcp, install_mcp_for_hosts, supported_install_hosts, McpInstallHostResult,
    McpInstallReport, McpInstallStatus,
};
pub use json::{JsonEnvelope, JsonStatus, SCHEMA_VERSION};
pub use model::{
    ArchCheckResult, ArchConfig, ArchExplainResult, ArchFileEdge, ArchInitResult, ArchLayer,
    ArchRule, ArchViolation, AuditCapabilitySource, AuditEntryReachRecord, AuditResult,
    AuditSummary, BranchDiffAffectedFile, BranchDiffChangedFile, BranchDiffResult,
    BranchDiffSummary, CallSiteRecord, CapabilityConfig, Certainty, CochangeRecord, CochangeResult,
    CochangeSort, CochangeSummary, ContextFileRecord, ContextFileRole, ContextResult,
    ContextSummary, CycleRecord, CycleSeverity, CyclesResult, CyclesSummary, DependencyRecord,
    DiagnosticSeverity, EdgeKind, EntryConeResult, EntryConeSummary, EntryListResult,
    EntryListSummary, EntryPointConfig, EntryPointDetection, EntryPointRecord,
    EntryReachableRecord, EntryReachesResult, EntryReachesSummary, EntryUnreachableRecord,
    EntryUnreachableResult, ExportRecord, ExtractResult, FileRecord, GateConfig, GateEvaluation,
    GateMetric, GateResult, GateSeverity, GateStatus, GateSummary, GatesConfig,
    HealthReportComparison, HealthReportMetrics, HealthReportResult, ImpactChangeType,
    ImpactTraversalRule, ImportPath, ImportRecord, MirrorMatch, MirrorResult, MirrorSignature,
    MirrorSummary, ModuleRecord, NodeKind, ParseDiagnostic, ParseStatus, PublicSurface,
    PublicSurfaceChange, PublicSurfaceChangeKind, PublicSurfaceDiff, PublicSurfaceDiffSummary,
    PublicSurfaceSymbol, RenameEdit, RenameEditKind, RenamePlan, RenamePlanStep, RenamePlanSummary,
    RepoPath, RiskRecord, RiskResult, RiskSort, RiskSummary, SimulateExtractResult,
    SimulateExtraction, SimulateFileStabilityDelta, SimulateGraphDelta, SimulateRecommendation,
    SnapshotCentralityDelta, SnapshotCycleDelta, SnapshotDeleteResult, SnapshotDiffResult,
    SnapshotDiffSummary, SnapshotEdgeDelta, SnapshotEdgeRecord, SnapshotFileRecord, SnapshotGraph,
    SnapshotListResult, SnapshotListSummary, SnapshotMetadata, SnapshotSaveResult,
    SnapshotStabilityDelta, SnapshotStoredRecord, SnapshotSymbolRecord, Span, SplitCluster,
    SplitClusterMember, SplitResult, SplitSummary, StabilityCategory, StabilityRecord,
    StabilityResult, StabilitySort, StabilitySummary, SymbolKind, SymbolRecord, TestConfig,
    TestMapBuildResult, TestMapBuildSummary, TestMapCoveredByResult, TestMapCoveredBySummary,
    TestMapCoversResult, TestMapCoversSummary, TestMapRecord, TestMapUncoveredResult,
    TestMapUncoveredSummary, TraversalRecord, TreeNode, TreeResult, TreeSummary, TsConfig,
    UnusedRecord, UnusedResult, UnusedSummary, Visibility,
};
pub use query_lang::{
    execute_query, parse_query_statement, QueryExpr, QuerySession, QuerySource, QueryStatement,
    QueryStep, QueryValue,
};
pub use scanner::{scan_repo, ScanConfig, ScanEntry, SupportedLanguage};
pub use serve::{run_server, ServeOptions};
pub use store::{
    validate_cochange_args, DatabaseInfo, IndexHealthStats, ParseStatusCounts, Store,
    INDEX_SCHEMA_VERSION,
};
pub use stub::{
    audit, benchmark, callers, calls, context, cycles, deps, diff, diff_snapshot, doctor, explain,
    gate, impact, index, mcp_stub_message, query, rename_plan, render_markdown_benchmark_report,
    render_markdown_report, report, scaffolded_gate, scaffolded_report, simulate_extract,
    snapshot_delete, snapshot_list, snapshot_save, stability, surface, surface_diff,
    test_map_build, test_map_covered_by, test_map_covers, test_map_uncovered, tree, unused, why,
};
pub use tracing::Verbosity;
