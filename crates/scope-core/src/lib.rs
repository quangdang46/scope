pub mod adapters;
pub mod arch;
pub mod bootstrap;
pub mod config;
pub mod error;
pub mod json;
pub mod model;
pub mod scanner;
pub mod serve;
pub mod snapshot;
pub mod store;
pub mod stub;
pub mod tracing;

pub use adapters::{adapter_for_language, Adapter, RustAdapter, TsJsAdapter};
pub use arch::{arch_check, arch_check_edges, load_arch_config};
pub use bootstrap::{bootstrap, AppContext};
pub use config::{BootstrapOptions, RuntimePaths};
pub use error::{ScopeError, ScopeResult};
pub use json::{JsonEnvelope, JsonStatus, SCHEMA_VERSION};
pub use model::{
    ArchCheckResult, ArchConfig, ArchFileEdge, ArchLayer, ArchRule, ArchViolation,
    AuditCapabilitySource, AuditEntryReachRecord, AuditResult, AuditSummary,
    BranchDiffAffectedFile, BranchDiffChangedFile, BranchDiffResult, BranchDiffSummary,
    CallSiteRecord, CapabilityConfig, Certainty, CochangeRecord, CochangeResult, CochangeSort,
    CochangeSummary, ContextFileRecord, ContextFileRole, ContextResult, ContextSummary,
    CycleRecord, CycleSeverity, CyclesResult, CyclesSummary, DependencyRecord, DiagnosticSeverity,
    EdgeKind, EntryConeResult, EntryConeSummary, EntryListResult, EntryListSummary,
    EntryPointConfig, EntryPointDetection, EntryPointRecord, EntryReachableRecord,
    EntryReachesResult, EntryReachesSummary, EntryUnreachableRecord, EntryUnreachableResult,
    ExportRecord, ExtractResult, FileRecord, GateConfig, GateEvaluation, GateMetric, GateResult,
    GateSeverity, GateStatus, GateSummary, GatesConfig, HealthReportComparison,
    HealthReportMetrics, HealthReportResult, ImpactChangeType, ImpactTraversalRule, ImportPath,
    ImportRecord, MirrorMatch, MirrorResult, MirrorSignature, MirrorSummary, ModuleRecord,
    NodeKind, ParseDiagnostic, ParseStatus, PublicSurface, PublicSurfaceChange,
    PublicSurfaceChangeKind, PublicSurfaceDiff, PublicSurfaceDiffSummary, PublicSurfaceSymbol,
    RenameEdit, RenameEditKind, RenamePlan, RenamePlanStep, RenamePlanSummary, RepoPath,
    RiskRecord, RiskResult, RiskSort, RiskSummary, SnapshotCentralityDelta, SnapshotDeleteResult,
    SnapshotDiffResult, SnapshotDiffSummary, SnapshotEdgeDelta, SnapshotEdgeRecord,
    SnapshotFileRecord, SnapshotGraph, SnapshotListResult, SnapshotListSummary, SnapshotMetadata,
    SnapshotSaveResult, SnapshotStabilityDelta, SnapshotStoredRecord, SnapshotSymbolRecord, Span,
    SplitCluster, SplitClusterMember, SplitResult, SplitSummary, StabilityCategory,
    StabilityRecord, StabilityResult, StabilitySort, StabilitySummary, SymbolKind, SymbolRecord,
    TestConfig, TestMapBuildResult, TestMapBuildSummary, TestMapCoveredByResult,
    TestMapCoveredBySummary, TestMapCoversResult, TestMapCoversSummary, TestMapRecord,
    TestMapUncoveredResult, TestMapUncoveredSummary, TraversalRecord, TreeNode, TreeResult,
    TreeSummary, UnusedRecord, UnusedResult, UnusedSummary, Visibility,
};
pub use scanner::{scan_repo, ScanConfig, ScanEntry, SupportedLanguage};
pub use serve::{run_server, ServeOptions};
pub use store::{DatabaseInfo, IndexHealthStats, ParseStatusCounts, Store, INDEX_SCHEMA_VERSION};
pub use tracing::Verbosity;
