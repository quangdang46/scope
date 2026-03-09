use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Import,
    Call,
    Export,
    Define,
    Contain,
    Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    pub callee_name: String,
    pub callee_qualname: Option<String>,
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
