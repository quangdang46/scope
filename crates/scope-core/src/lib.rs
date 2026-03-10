pub mod adapters;
pub mod arch;
pub mod bootstrap;
pub mod config;
pub mod error;
pub mod json;
pub mod model;
pub mod scanner;
pub mod store;
pub mod stub;
pub mod tracing;

pub use adapters::{adapter_for_language, Adapter, RustAdapter, TsJsAdapter};
pub use arch::{arch_check, load_arch_config};
pub use bootstrap::{bootstrap, AppContext};
pub use config::{BootstrapOptions, RuntimePaths};
pub use error::{ScopeError, ScopeResult};
pub use json::{JsonEnvelope, JsonStatus, SCHEMA_VERSION};
pub use model::{
    ArchCheckResult, ArchConfig, ArchFileEdge, ArchLayer, ArchRule, ArchViolation, CallSiteRecord,
    Certainty, ContextFileRecord, ContextFileRole, ContextResult, ContextSummary,
    DependencyRecord, DiagnosticSeverity, EdgeKind, ExportRecord, ExtractResult, FileRecord,
    ImpactChangeType, ImpactTraversalRule, ImportPath, ImportRecord, ModuleRecord, NodeKind,
    ParseDiagnostic, ParseStatus, PublicSurface, PublicSurfaceChange, PublicSurfaceChangeKind,
    PublicSurfaceDiff, PublicSurfaceDiffSummary, PublicSurfaceSymbol, RepoPath, Span,
    StabilityRecord, StabilityResult, SymbolKind, SymbolRecord, TraversalRecord, Visibility,
};
pub use scanner::{scan_repo, ScanConfig, ScanEntry, SupportedLanguage};
pub use store::{
    DatabaseInfo, IndexHealthStats, ParseStatusCounts, Store, INDEX_SCHEMA_VERSION,
};
pub use tracing::Verbosity;
