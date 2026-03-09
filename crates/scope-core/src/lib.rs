pub mod adapters;
pub mod bootstrap;
pub mod config;
pub mod error;
pub mod json;
pub mod model;
pub mod scanner;
pub mod store;
pub mod stub;
pub mod tracing;

pub use adapters::{Adapter, RustAdapter};
pub use bootstrap::{bootstrap, AppContext};
pub use config::{BootstrapOptions, RuntimePaths};
pub use error::{ScopeError, ScopeResult};
pub use json::{JsonEnvelope, JsonStatus, SCHEMA_VERSION};
pub use model::{
    Certainty, DependencyRecord, DiagnosticSeverity, EdgeKind, ExportRecord, ExtractResult,
    FileRecord, ImportPath, ImportRecord, ModuleRecord, NodeKind, ParseDiagnostic, ParseStatus,
    RepoPath, Span, SymbolKind, SymbolRecord, TraversalRecord, Visibility,
};
pub use scanner::{scan_repo, ScanConfig, ScanEntry, SupportedLanguage};
pub use store::{DatabaseInfo, Store, INDEX_SCHEMA_VERSION};
pub use tracing::Verbosity;
