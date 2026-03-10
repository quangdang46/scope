use std::path::{Path, PathBuf};

use crate::{
    Certainty, DiagnosticSeverity, ExportRecord, ExtractResult, FileRecord, ImportPath,
    ImportRecord, ModuleRecord, ParseDiagnostic, ParseStatus, RepoPath, ScanEntry, Span,
    SupportedLanguage, SymbolKind, SymbolRecord, Visibility,
};
use crate::model::CallSiteRecord;

pub trait Adapter: Send + Sync {
    /// File extensions handled by this adapter, without leading dots.
    fn extensions(&self) -> &[&str];

    /// Stable language identifier used by storage and query layers.
    fn language_id(&self) -> &'static str;

    /// Parse a scanned source file into the normalized intermediate model.
    ///
    /// Implementations should avoid panicking and should instead surface
    /// partial results and diagnostics through `ExtractResult`.
    fn extract(&self, entry: &ScanEntry, source: &str) -> ExtractResult;
}

pub fn adapter_for_language(language: SupportedLanguage) -> Option<&'static dyn Adapter> {
    match language {
        SupportedLanguage::Rust => Some(&RustAdapter),
        SupportedLanguage::TypeScript | SupportedLanguage::JavaScript => Some(&TsJsAdapter),
        SupportedLanguage::Python | SupportedLanguage::Ruby | SupportedLanguage::Go => None,
    }
}

pub struct RustAdapter;

pub struct TsJsAdapter;

impl Adapter for RustAdapter {
    fn extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn language_id(&self) -> &'static str {
        "rust"
    }

    fn extract(&self, entry: &ScanEntry, source: &str) -> ExtractResult {
        let mut imports = Vec::new();
        let mut modules = Vec::new();
        let mut exports = Vec::new();
        let mut symbols = Vec::new();
        let mut call_sites = Vec::new();

        let repo_root = infer_repo_root(entry);
        let crate_root = repo_root.join("src");
        let module_dir = rust_module_dir(entry);

        let mut byte_offset = 0usize;
        let mut brace_depth = 0i32;
        let mut current_function_qualname: Option<String> = None;
        for (line_index, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            let span = line_span(line, byte_offset, line_index);
            let scan_line = strip_line_comment(line);
            let line_open_braces = scan_line.bytes().filter(|byte| *byte == b'{').count() as i32;
            let line_close_braces = scan_line.bytes().filter(|byte| *byte == b'}').count() as i32;

            if let Some(module) = parse_module_declaration(trimmed, &entry.path, &module_dir, &span)
            {
                let exported_module_name = module.name.clone();
                let module_qualname = qualified_symbol_name(&entry.path, &module.name);

                if let Some(declared_path) = &module.declared_path {
                    if !repo_root.join(&declared_path.0).exists() {
                        modules.push(ModuleRecord {
                            declared_path: None,
                            certainty: Certainty::Heuristic,
                            ..module
                        });
                    } else {
                        modules.push(module.clone());
                    }
                } else {
                    modules.push(module.clone());
                }

                if trimmed.starts_with("pub mod ") {
                    exports.push(ExportRecord {
                        file: entry.path.clone(),
                        name: exported_module_name.clone(),
                        qualname: None,
                        kind: SymbolKind::Module,
                        visibility: Visibility::Public,
                        span: span.clone(),
                    });
                }

                symbols.push(SymbolRecord {
                    file: entry.path.clone(),
                    name: exported_module_name,
                    qualname: module_qualname,
                    kind: SymbolKind::Module,
                    visibility: visibility_from_item(trimmed),
                    exported: trimmed.starts_with("pub mod "),
                    span: span.clone(),
                });
            }

            let function_qualname_for_line = current_function_qualname.as_deref();
            call_sites.extend(extract_call_sites_from_line(
                line,
                byte_offset,
                line_index,
                &entry.path,
                function_qualname_for_line,
            ));

            let declared_function_qualname = parse_top_level_symbol(trimmed, &entry.path, &span).map(|symbol| {
                let qualname = if symbol.kind == SymbolKind::Function {
                    Some(symbol.qualname.clone())
                } else {
                    None
                };
                symbols.push(symbol);
                qualname
            }).flatten();

            if let Some(import_record) =
                parse_use_declaration(trimmed, &entry.path, &crate_root, &module_dir, &span)
            {
                if trimmed.starts_with("pub use ") {
                    let exported_name = import_export_name(trimmed);
                    exports.push(ExportRecord {
                        file: entry.path.clone(),
                        name: exported_name,
                        qualname: None,
                        kind: SymbolKind::Module,
                        visibility: Visibility::Public,
                        span: span.clone(),
                    });
                }
                imports.push(import_record);
            }

            let previous_brace_depth = brace_depth;
            brace_depth += line_open_braces - line_close_braces;

            if previous_brace_depth == 0 && current_function_qualname.is_none() && line_open_braces > line_close_braces {
                current_function_qualname = declared_function_qualname;
            }

            if brace_depth <= 0 {
                brace_depth = 0;
                current_function_qualname = None;
            }

            byte_offset += line.len() + 1;
        }

        ExtractResult {
            file: FileRecord {
                path: entry.path.clone(),
                language: self.language_id().to_string(),
                parse_status: ParseStatus::Ok,
                is_barrel: false,
                content_hash: None,
                mtime_unix_seconds: None,
                size_bytes: None,
            },
            imports,
            modules,
            exports,
            symbols,
            call_sites,
            parse_diagnostics: Vec::new(),
        }
    }
}

impl Adapter for TsJsAdapter {
    fn extensions(&self) -> &[&str] {
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"]
    }

    fn language_id(&self) -> &'static str {
        "typescript_javascript"
    }

    fn extract(&self, entry: &ScanEntry, source: &str) -> ExtractResult {
        let repo_root = infer_repo_root(entry);
        let mut imports = Vec::new();
        let mut exports = Vec::new();
        let mut symbols = Vec::new();
        let mut call_sites = Vec::new();
        let mut parse_diagnostics = Vec::new();
        let mut byte_offset = 0usize;
        let mut brace_depth = 0i32;
        let mut current_function_qualname: Option<String> = None;
        let mut saw_non_export_statement = false;

        for (line_index, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            let span = line_span(line, byte_offset, line_index);
            let scan_line = strip_line_comment(line);
            let line_open_braces = scan_line.bytes().filter(|byte| *byte == b'{').count() as i32;
            let line_close_braces = scan_line.bytes().filter(|byte| *byte == b'}').count() as i32;

            if !trimmed.is_empty() && !trimmed.starts_with("export ") {
                saw_non_export_statement = true;
            }

            if let Some(import_record) = parse_ts_js_import(trimmed, &entry.path, &repo_root, &span) {
                imports.push(import_record);
            }
            if let Some(diagnostic) = parse_ts_js_dynamic_pattern(trimmed, &entry.path, &span) {
                parse_diagnostics.push(diagnostic);
            }

            if let Some(export_record) = parse_ts_js_named_export(trimmed, &entry.path, &span) {
                exports.push(export_record);
            }
            if let Some(symbol) = parse_ts_js_reexport_function_binding(trimmed, &entry.path, &span) {
                symbols.push(symbol);
            }

            let function_qualname_for_line = current_function_qualname.as_deref();
            call_sites.extend(extract_call_sites_from_line(
                line,
                byte_offset,
                line_index,
                &entry.path,
                function_qualname_for_line,
            ));

            let declared_function_qualname = parse_ts_js_top_level_symbol(trimmed, &entry.path, &span)
                .map(|symbol| {
                    let qualname = if symbol.kind == SymbolKind::Function {
                        Some(symbol.qualname.clone())
                    } else {
                        None
                    };
                    if symbol.exported {
                        exports.push(ExportRecord {
                            file: entry.path.clone(),
                            name: symbol.name.clone(),
                            qualname: Some(symbol.qualname.clone()),
                            kind: symbol.kind.clone(),
                            visibility: symbol.visibility.clone(),
                            span: symbol.span.clone(),
                        });
                    }
                    symbols.push(symbol);
                    qualname
                })
                .flatten();

            let previous_brace_depth = brace_depth;
            brace_depth += line_open_braces - line_close_braces;

            if previous_brace_depth == 0
                && current_function_qualname.is_none()
                && line_open_braces > line_close_braces
            {
                current_function_qualname = declared_function_qualname;
            }

            if brace_depth <= 0 {
                brace_depth = 0;
                current_function_qualname = None;
            }

            byte_offset += line.len() + 1;
        }

        let export_lines = source
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && !trimmed.starts_with("//")
            })
            .all(|line| line.trim().starts_with("export "));

        let parse_status = if parse_diagnostics.is_empty() {
            ParseStatus::Ok
        } else {
            ParseStatus::Partial
        };

        ExtractResult {
            file: FileRecord {
                path: entry.path.clone(),
                language: self.language_id().to_string(),
                parse_status,
                is_barrel: export_lines && !saw_non_export_statement,
                content_hash: None,
                mtime_unix_seconds: None,
                size_bytes: None,
            },
            imports,
            modules: Vec::new(),
            exports,
            symbols,
            call_sites,
            parse_diagnostics,
        }
    }
}

fn parse_top_level_symbol(trimmed: &str, file: &RepoPath, span: &Span) -> Option<SymbolRecord> {
    let body = strip_visibility_prefix(trimmed);
    let (prefix, kind) = [
        ("fn ", SymbolKind::Function),
        ("struct ", SymbolKind::Struct),
        ("enum ", SymbolKind::Enum),
        ("trait ", SymbolKind::Trait),
        ("const ", SymbolKind::Constant),
        ("static ", SymbolKind::Static),
        ("type ", SymbolKind::TypeAlias),
    ]
    .into_iter()
    .find(|(prefix, _)| body.starts_with(prefix))?;

    let name = identifier_after_prefix(body, prefix)?;
    let semantics = visibility_semantics(trimmed);
    Some(SymbolRecord {
        file: file.clone(),
        name: name.clone(),
        qualname: qualified_symbol_name(file, &name),
        kind,
        visibility: semantics.visibility,
        exported: semantics.exported,
        span: span.clone(),
    })
}

fn identifier_after_prefix(trimmed: &str, prefix: &str) -> Option<String> {
    let remainder = trimmed.strip_prefix(prefix)?.trim_start();
    let identifier: String = remainder
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();

    if identifier.is_empty() {
        None
    } else {
        Some(identifier)
    }
}

fn qualified_symbol_name(file: &RepoPath, name: &str) -> String {
    let module = file
        .0
        .strip_prefix("src/")
        .unwrap_or(&file.0)
        .trim_end_matches(".rs")
        .trim_end_matches(".ts")
        .trim_end_matches(".tsx")
        .trim_end_matches(".js")
        .trim_end_matches(".jsx")
        .trim_end_matches(".mjs")
        .trim_end_matches(".cjs")
        .replace("/mod", "")
        .replace("/index", "")
        .replace('/', "::");

    if module.is_empty() {
        name.to_string()
    } else {
        format!("{module}::{name}")
    }
}

#[derive(Clone)]
struct VisibilitySemantics {
    visibility: Visibility,
    exported: bool,
}

fn visibility_from_item(trimmed: &str) -> Visibility {
    visibility_semantics(trimmed).visibility
}

fn visibility_semantics(trimmed: &str) -> VisibilitySemantics {
    if trimmed.starts_with("pub(crate)") {
        VisibilitySemantics {
            visibility: Visibility::Package,
            exported: true,
        }
    } else if trimmed.starts_with("pub(super)") {
        VisibilitySemantics {
            visibility: Visibility::Module,
            exported: false,
        }
    } else if trimmed.starts_with("pub(self)") {
        VisibilitySemantics {
            visibility: Visibility::Local,
            exported: false,
        }
    } else if trimmed.starts_with("pub(in ") {
        VisibilitySemantics {
            visibility: Visibility::Unknown,
            exported: false,
        }
    } else if trimmed.starts_with("pub ") || trimmed == "pub" {
        VisibilitySemantics {
            visibility: Visibility::Public,
            exported: true,
        }
    } else {
        VisibilitySemantics {
            visibility: Visibility::Local,
            exported: false,
        }
    }
}

fn strip_visibility_prefix(trimmed: &str) -> &str {
    if let Some(remainder) = trimmed.strip_prefix("pub(crate) ") {
        remainder
    } else if let Some(remainder) = trimmed.strip_prefix("pub(super) ") {
        remainder
    } else if let Some(remainder) = trimmed.strip_prefix("pub(self) ") {
        remainder
    } else if let Some(remainder) = trimmed.strip_prefix("pub(in ") {
        remainder
            .split_once(") ")
            .map(|(_, tail)| tail)
            .unwrap_or(remainder)
    } else if let Some(remainder) = trimmed.strip_prefix("pub ") {
        remainder
    } else {
        trimmed
    }
}

fn parse_ts_js_import(
    trimmed: &str,
    file: &RepoPath,
    repo_root: &Path,
    span: &Span,
) -> Option<ImportRecord> {
    let specifier = quoted_specifier(trimmed)?;
    let import_path = normalize_ts_js_import_path(file, repo_root, &specifier);
    let certainty = match import_path {
        ImportPath::Relative(_) => Certainty::Exact,
        ImportPath::External(_) => Certainty::Heuristic,
        ImportPath::Unresolved => Certainty::Heuristic,
    };

    Some(ImportRecord {
        file: file.clone(),
        raw_text: trimmed.to_string(),
        import_path,
        span: span.clone(),
        certainty,
    })
}

fn parse_ts_js_named_export(trimmed: &str, file: &RepoPath, span: &Span) -> Option<ExportRecord> {
    let name = ts_js_exported_name(trimmed)?;

    Some(ExportRecord {
        file: file.clone(),
        name,
        qualname: None,
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        span: span.clone(),
    })
}

fn parse_ts_js_reexport_function_binding(
    trimmed: &str,
    file: &RepoPath,
    span: &Span,
) -> Option<SymbolRecord> {
    let export = parse_ts_js_named_export(trimmed, file, span)?;
    Some(SymbolRecord {
        file: file.clone(),
        name: export.name.clone(),
        qualname: qualified_symbol_name(file, &export.name),
        kind: SymbolKind::Function,
        visibility: Visibility::Public,
        exported: true,
        span: span.clone(),
    })
}

fn ts_js_exported_name(trimmed: &str) -> Option<String> {
    let remainder = trimmed.strip_prefix("export {")?;
    let names = remainder.split('}').next()?.trim();
    let first = names.split(',').next()?.trim();
    let alias_target = first.split(" as ").nth(1).unwrap_or(first).trim();
    let name = alias_target
        .split_whitespace()
        .next()?
        .trim()
        .trim_end_matches(',')
        .to_string();

    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn parse_ts_js_dynamic_pattern(
    trimmed: &str,
    file: &RepoPath,
    span: &Span,
) -> Option<ParseDiagnostic> {
    let message = if trimmed.contains("import(") {
        Some("dynamic import is not resolved yet")
    } else if trimmed.contains("require(")
        && !trimmed.contains("require(\"")
        && !trimmed.contains("require('")
    {
        Some("computed require is not resolved yet")
    } else {
        None
    }?;

    Some(ParseDiagnostic {
        path: file.clone(),
        message: message.to_string(),
        span: Some(span.clone()),
        severity: DiagnosticSeverity::Warning,
    })
}

fn parse_ts_js_top_level_symbol(trimmed: &str, file: &RepoPath, span: &Span) -> Option<SymbolRecord> {
    let (body, exported) = if let Some(remainder) = trimmed.strip_prefix("export ") {
        (remainder.trim_start(), true)
    } else {
        (trimmed, false)
    };

    let (prefix, kind) = [
        ("async function ", SymbolKind::Function),
        ("function ", SymbolKind::Function),
        ("const ", SymbolKind::Variable),
        ("let ", SymbolKind::Variable),
        ("class ", SymbolKind::Class),
        ("interface ", SymbolKind::Interface),
        ("type ", SymbolKind::TypeAlias),
    ]
    .into_iter()
    .find(|(prefix, _)| body.starts_with(prefix))?;

    let name = identifier_after_prefix(body, prefix)?;
    let kind = if matches!(kind, SymbolKind::Variable) && !body.contains("=>") {
        return None;
    } else if matches!(kind, SymbolKind::Variable) {
        SymbolKind::Function
    } else {
        kind
    };

    Some(SymbolRecord {
        file: file.clone(),
        name: name.clone(),
        qualname: qualified_symbol_name(file, &name),
        kind,
        visibility: if exported {
            Visibility::Public
        } else {
            Visibility::Local
        },
        exported,
        span: span.clone(),
    })
}

fn quoted_specifier(trimmed: &str) -> Option<String> {
    let quote_index = trimmed.find(['\'', '"'])?;
    let quote = trimmed.as_bytes()[quote_index] as char;
    let remainder = &trimmed[quote_index + 1..];
    let end = remainder.find(quote)?;
    Some(remainder[..end].to_string())
}

fn normalize_ts_js_import_path(file: &RepoPath, repo_root: &Path, specifier: &str) -> ImportPath {
    if !specifier.starts_with('.') {
        return ImportPath::External(specifier.to_string());
    }

    let importer = repo_root.join(&file.0);
    let Some(importer_dir) = importer.parent() else {
        return ImportPath::Unresolved;
    };
    let base = importer_dir.join(specifier);

    for candidate in ts_js_candidates(&base) {
        if candidate.exists() {
            let normalized = candidate.canonicalize().unwrap_or(candidate.clone());
            let normalized_repo_root = repo_root
                .canonicalize()
                .unwrap_or_else(|_| repo_root.to_path_buf());
            return ImportPath::Relative(path_under_explicit_repo(&normalized, &normalized_repo_root));
        }
    }

    ImportPath::Unresolved
}

fn ts_js_candidates(base: &Path) -> Vec<PathBuf> {
    vec![
        base.to_path_buf(),
        base.with_extension("ts"),
        base.with_extension("tsx"),
        base.with_extension("js"),
        base.with_extension("jsx"),
        base.with_extension("mjs"),
        base.with_extension("cjs"),
        base.join("index.ts"),
        base.join("index.tsx"),
        base.join("index.js"),
        base.join("index.jsx"),
        base.join("index.mjs"),
        base.join("index.cjs"),
    ]
}

pub fn supports_path(adapter: &dyn Adapter, path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };

    adapter
        .extensions()
        .iter()
        .any(|candidate| candidate == &extension)
}

fn parse_use_declaration(
    trimmed: &str,
    file: &RepoPath,
    crate_root: &Path,
    module_dir: &Path,
    span: &Span,
) -> Option<ImportRecord> {
    let remainder = trimmed
        .strip_prefix("pub use ")
        .or_else(|| trimmed.strip_prefix("use "))?
        .strip_suffix(';')?
        .trim();

    let raw_text = trimmed.to_string();
    let import_path = normalize_rust_use_path(remainder, crate_root, module_dir);
    let certainty = match import_path {
        ImportPath::Relative(_) => Certainty::Exact,
        ImportPath::External(_) => Certainty::Heuristic,
        ImportPath::Unresolved => Certainty::Heuristic,
    };

    Some(ImportRecord {
        file: file.clone(),
        raw_text,
        import_path,
        span: span.clone(),
        certainty,
    })
}

fn parse_module_declaration(
    trimmed: &str,
    file: &RepoPath,
    module_dir: &Path,
    span: &Span,
) -> Option<ModuleRecord> {
    let remainder = trimmed
        .strip_prefix("pub mod ")
        .or_else(|| trimmed.strip_prefix("mod "))?
        .trim();

    if let Some(name) = remainder.strip_suffix(';') {
        let name = name.trim();
        return Some(ModuleRecord {
            file: file.clone(),
            name: name.to_string(),
            declared_path: resolve_module_file(module_dir, &[name]),
            is_inline: false,
            span: span.clone(),
            certainty: Certainty::Exact,
        });
    }

    if let Some(name) = remainder.strip_suffix('{') {
        return Some(ModuleRecord {
            file: file.clone(),
            name: name.trim().to_string(),
            declared_path: None,
            is_inline: true,
            span: span.clone(),
            certainty: Certainty::Exact,
        });
    }

    None
}

fn normalize_rust_use_path(remainder: &str, crate_root: &Path, module_dir: &Path) -> ImportPath {
    let simplified = simplify_use_path(remainder);
    let segments: Vec<&str> = simplified
        .split("::")
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return ImportPath::Unresolved;
    }

    if remainder.starts_with("crate::") {
        return resolve_from_base(crate_root, &segments[1..]).unwrap_or(ImportPath::Unresolved);
    }

    if remainder.starts_with("self::") {
        return resolve_from_base(module_dir, &segments[1..]).unwrap_or(ImportPath::Unresolved);
    }

    if remainder.starts_with("super::") {
        let base = module_dir.parent().unwrap_or(crate_root);
        return resolve_from_base(base, &segments[1..]).unwrap_or(ImportPath::Unresolved);
    }

    if remainder.starts_with("::") {
        return ImportPath::External(segments[0].to_string());
    }

    ImportPath::External(segments[0].to_string())
}

fn simplify_use_path(remainder: &str) -> String {
    let before_alias = remainder.split(" as ").next().unwrap_or(remainder).trim();
    let before_group = before_alias
        .split("::{")
        .next()
        .unwrap_or(before_alias)
        .trim();
    before_group.trim_end_matches("::*").to_string()
}

fn resolve_from_base(base: &Path, segments: &[&str]) -> Option<ImportPath> {
    for candidate_len in (1..=segments.len()).rev() {
        if let Some(path) = resolve_module_file(base, &segments[..candidate_len]) {
            return Some(ImportPath::Relative(path));
        }
    }

    None
}

fn resolve_module_file(base: &Path, segments: &[&str]) -> Option<RepoPath> {
    if segments.is_empty() {
        return None;
    }

    let joined = segments.join("/");
    let file_candidate = base.join(format!("{joined}.rs"));
    if file_candidate.exists() {
        return Some(path_under_repo(&file_candidate, base));
    }

    let mod_candidate = base.join(&joined).join("mod.rs");
    if mod_candidate.exists() {
        return Some(path_under_repo(&mod_candidate, base));
    }

    None
}

fn path_under_repo(path: &Path, base: &Path) -> RepoPath {
    let repo_root = repo_root_from_base(base);
    path_under_explicit_repo(path, &repo_root)
}

fn path_under_explicit_repo(path: &Path, repo_root: &Path) -> RepoPath {
    let relative = path.strip_prefix(repo_root).unwrap_or(path);
    RepoPath::from(relative.to_string_lossy().to_string())
}

fn repo_root_from_base(base: &Path) -> PathBuf {
    if base.file_name().is_some_and(|name| name == "src") {
        base.parent().unwrap_or(base).to_path_buf()
    } else {
        let mut current = base.to_path_buf();
        while current.file_name().is_some_and(|name| name != "src") {
            let Some(parent) = current.parent() else {
                break;
            };
            current = parent.to_path_buf();
        }
        current.parent().unwrap_or(&current).to_path_buf()
    }
}

fn infer_repo_root(entry: &ScanEntry) -> PathBuf {
    let relative = PathBuf::from(&entry.path.0);
    let mut current = entry.absolute_path.clone();

    let depth = relative.components().count();
    for _ in 0..depth {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        }
    }

    current
}

fn rust_module_dir(entry: &ScanEntry) -> PathBuf {
    let repo_root = infer_repo_root(entry);
    let relative = Path::new(&entry.path.0);
    let path_under_repo = repo_root.join(relative);
    let src_root = repo_root.join("src");

    match path_under_repo.file_name().and_then(|value| value.to_str()) {
        Some("lib.rs") | Some("main.rs") => src_root,
        Some("mod.rs") => path_under_repo.parent().unwrap_or(&src_root).to_path_buf(),
        Some(stem) => path_under_repo
            .parent()
            .unwrap_or(&src_root)
            .join(stem.trim_end_matches(".rs")),
        None => src_root,
    }
}

fn import_export_name(trimmed: &str) -> String {
    simplify_use_path(
        trimmed
            .strip_prefix("pub use ")
            .unwrap_or(trimmed)
            .trim_end_matches(';')
            .trim(),
    )
    .split("::")
    .last()
    .unwrap_or("unknown")
    .to_string()
}

fn line_span(line: &str, byte_offset: usize, line_index: usize) -> Span {
    Span {
        start_byte: byte_offset as u32,
        end_byte: (byte_offset + line.len()) as u32,
        start_line: line_index as u32 + 1,
        end_line: line_index as u32 + 1,
    }
}

fn extract_call_sites_from_line(
    line: &str,
    byte_offset: usize,
    line_index: usize,
    file: &RepoPath,
    caller_qualname: Option<&str>,
) -> Vec<CallSiteRecord> {
    let scan_line = strip_line_comment(line);
    let trimmed = scan_line.trim();
    let declared_function_name = declared_function_name_on_line(trimmed);
    let bytes = scan_line.as_bytes();
    let mut call_sites = Vec::new();

    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'(' {
            continue;
        }

        let Some(candidate) = callable_candidate_before_paren(&scan_line, index) else {
            continue;
        };

        if is_non_call_keyword(&candidate.text)
            || looks_like_macro_invocation(&scan_line, candidate.start)
            || declared_function_name.as_deref() == Some(candidate.text.as_str())
        {
            continue;
        }

        let callee_name = candidate
            .text
            .rsplit([':', '.'])
            .next()
            .unwrap_or(candidate.text.as_str())
            .to_string();

        let callee_qualname = if candidate.text.contains("::") {
            Some(candidate.text.clone())
        } else {
            None
        };

        call_sites.push(CallSiteRecord {
            file: file.clone(),
            caller_qualname: caller_qualname.map(ToOwned::to_owned),
            callee_name,
            callee_qualname,
            is_method: candidate.is_method,
            span: Span {
                start_byte: (byte_offset + candidate.start) as u32,
                end_byte: (byte_offset + candidate.end) as u32,
                start_line: line_index as u32 + 1,
                end_line: line_index as u32 + 1,
            },
            certainty: Certainty::Exact,
        });
    }

    call_sites
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CallCandidate {
    text: String,
    start: usize,
    end: usize,
    is_method: bool,
}

fn callable_candidate_before_paren(line: &str, paren_index: usize) -> Option<CallCandidate> {
    if paren_index == 0 {
        return None;
    }

    let bytes = line.as_bytes();
    let mut end = paren_index;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }

    if end == 0 {
        return None;
    }

    let mut start = end;
    while start > 0 {
        let byte = bytes[start - 1];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'.') {
            start -= 1;
            continue;
        }
        break;
    }

    if start == end {
        return None;
    }

    let candidate = &line[start..end];
    if !candidate.chars().any(|character| character.is_ascii_alphabetic() || character == '_') {
        return None;
    }

    let candidate = if let Some((_, method_name)) = candidate.rsplit_once('.') {
        if method_name.is_empty() {
            return None;
        }
        CallCandidate {
            text: method_name.to_string(),
            start: end - method_name.len(),
            end,
            is_method: true,
        }
    } else {
        CallCandidate {
            text: candidate.to_string(),
            start,
            end,
            is_method: false,
        }
    };

    if candidate
        .text
        .split("::")
        .any(|segment| !is_valid_rust_identifier(segment))
    {
        return None;
    }

    Some(candidate)
}

fn strip_line_comment(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

fn declared_function_name_on_line(trimmed: &str) -> Option<String> {
    let body = strip_visibility_prefix(trimmed);
    if let Some(name) = if body.starts_with("async fn ") {
        identifier_after_prefix(body, "async fn ")
    } else if body.starts_with("fn ") {
        identifier_after_prefix(body, "fn ")
    } else {
        None
    } {
        return Some(name);
    }

    let body = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim_start();
    if let Some(name) = if body.starts_with("async function ") {
        identifier_after_prefix(body, "async function ")
    } else if body.starts_with("function ") {
        identifier_after_prefix(body, "function ")
    } else if body.starts_with("const ") {
        identifier_after_prefix(body, "const ")
    } else if body.starts_with("let ") {
        identifier_after_prefix(body, "let ")
    } else {
        None
    } {
        return Some(name);
    }

    None
}

fn looks_like_macro_invocation(line: &str, candidate_start: usize) -> bool {
    line[..candidate_start].trim_end().ends_with('!')
}

fn is_non_call_keyword(candidate: &str) -> bool {
    matches!(
        candidate,
        "if"
            | "match"
            | "while"
            | "loop"
            | "for"
            | "fn"
            | "struct"
            | "enum"
            | "trait"
            | "impl"
            | "return"
    )
}

fn is_valid_rust_identifier(segment: &str) -> bool {
    let mut characters = segment.chars();
    let Some(first) = characters.next() else {
        return false;
    };

    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }

    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scanner::SupportedLanguage, RepoPath};

    fn fixture_entry(relative: &str) -> ScanEntry {
        fixture_entry_in("rust_small", relative, SupportedLanguage::Rust)
    }

    fn fixture_entry_in(fixture: &str, relative: &str, language: SupportedLanguage) -> ScanEntry {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let fixture_root = repo_root.join("fixtures").join(fixture);
        let relative_path = Path::new(relative);

        ScanEntry {
            path: RepoPath::from(relative),
            absolute_path: fixture_root.join(relative_path),
            language,
        }
    }

    #[test]
    fn matches_supported_extensions() {
        let adapter = RustAdapter;
        assert!(supports_path(&adapter, Path::new("src/lib.rs")));
        assert!(!supports_path(&adapter, Path::new("src/lib.ts")));

        let ts_js = TsJsAdapter;
        assert!(supports_path(&ts_js, Path::new("src/lib.ts")));
        assert!(supports_path(&ts_js, Path::new("src/lib.js")));
        assert!(!supports_path(&ts_js, Path::new("src/lib.rs")));
    }

    #[test]
    fn extracts_module_declarations_from_rust_small_lib() {
        let adapter = RustAdapter;
        let entry = fixture_entry("src/lib.rs");
        let source = std::fs::read_to_string(&entry.absolute_path).unwrap();

        let result = adapter.extract(&entry, &source);
        let module_paths: Vec<_> = result
            .modules
            .iter()
            .filter_map(|module| module.declared_path.as_ref().map(|path| path.0.clone()))
            .collect();

        assert_eq!(
            module_paths,
            vec!["src/parser.rs", "src/resolver.rs", "src/utils.rs"]
        );
        assert!(result.imports.is_empty());
    }

    #[test]
    fn extracts_ts_js_imports_symbols_calls_and_barrels() {
        let adapter = TsJsAdapter;
        let middleware_entry = fixture_entry_in(
            "ts_small",
            "src/auth/middleware.ts",
            SupportedLanguage::TypeScript,
        );
        let middleware_source = std::fs::read_to_string(&middleware_entry.absolute_path).unwrap();
        let middleware_result = adapter.extract(&middleware_entry, &middleware_source);

        assert_eq!(middleware_result.imports.len(), 1);
        assert_eq!(middleware_result.imports[0].raw_text, "import { verify } from \"./jwt\";");
        assert_eq!(
            middleware_result.imports[0].import_path,
            ImportPath::Relative(RepoPath::from("src/auth/jwt.ts"))
        );
        assert_eq!(middleware_result.symbols[0].name, "verifyToken");
        assert_eq!(middleware_result.symbols[0].qualname, "auth::middleware::verifyToken");
        assert_eq!(middleware_result.call_sites[0].callee_name, "verify");
        assert_eq!(
            middleware_result.call_sites[0].caller_qualname,
            Some("auth::middleware::verifyToken".to_string())
        );
        assert!(!middleware_result.file.is_barrel);

        let index_entry = fixture_entry_in("ts_small", "src/index.ts", SupportedLanguage::TypeScript);
        let index_source = std::fs::read_to_string(&index_entry.absolute_path).unwrap();
        let index_result = adapter.extract(&index_entry, &index_source);

        assert!(index_result.file.is_barrel);
        assert_eq!(index_result.imports.len(), 2);
        assert_eq!(
            index_result
                .imports
                .iter()
                .map(|import| import.import_path.clone())
                .collect::<Vec<_>>(),
            vec![
                ImportPath::Relative(RepoPath::from("src/auth/index.ts")),
                ImportPath::Relative(RepoPath::from("src/utils/formatter.ts")),
            ]
        );

        let alias_entry = fixture_entry_in("ts_small", "src/auth/aliases.ts", SupportedLanguage::TypeScript);
        let alias_source = std::fs::read_to_string(&alias_entry.absolute_path).unwrap();
        let alias_result = adapter.extract(&alias_entry, &alias_source);

        assert!(alias_result.file.is_barrel);
        assert_eq!(alias_result.imports.len(), 1);
        assert_eq!(
            alias_result.imports[0].import_path,
            ImportPath::Relative(RepoPath::from("src/auth/jwt.ts"))
        );
        assert_eq!(
            alias_result
                .symbols
                .iter()
                .map(|symbol| symbol.name.clone())
                .collect::<Vec<_>>(),
            vec!["verifyJwt"]
        );
    }

    #[test]
    fn extracts_crate_relative_use_as_repo_relative_path() {
        let adapter = RustAdapter;
        let entry = fixture_entry("src/resolver.rs");
        let source = std::fs::read_to_string(&entry.absolute_path).unwrap();

        let result = adapter.extract(&entry, &source);

        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].raw_text, "use crate::parser;");
        assert_eq!(
            result.imports[0].import_path,
            ImportPath::Relative(RepoPath::from("src/parser.rs"))
        );
        assert_eq!(result.imports[0].certainty, Certainty::Exact);
    }

    #[test]
    fn resolves_symbol_level_imports_to_owning_module_file() {
        let path = normalize_rust_use_path(
            "crate::parser::parse",
            Path::new("/tmp/project/src"),
            Path::new("/tmp/project/src/resolver"),
        );

        assert_eq!(path, ImportPath::Unresolved);
    }

    #[test]
    fn treats_external_imports_conservatively() {
        let path = normalize_rust_use_path(
            "std::collections::HashMap",
            Path::new("/tmp/project/src"),
            Path::new("/tmp/project/src"),
        );

        assert_eq!(path, ImportPath::External("std".to_string()));
    }

    #[test]
    fn extracts_top_level_symbols_from_rust_small_files() {
        let adapter = RustAdapter;
        let entry = fixture_entry("src/lib.rs");
        let source = std::fs::read_to_string(&entry.absolute_path).unwrap();

        let result = adapter.extract(&entry, &source);
        let names_and_kinds: Vec<_> = result
            .symbols
            .iter()
            .map(|symbol| (symbol.name.clone(), symbol.kind.clone(), symbol.visibility.clone()))
            .collect();

        assert_eq!(
            names_and_kinds,
            vec![
                ("parser".to_string(), SymbolKind::Module, Visibility::Public),
                ("resolver".to_string(), SymbolKind::Module, Visibility::Public),
                ("utils".to_string(), SymbolKind::Module, Visibility::Public),
                ("greet".to_string(), SymbolKind::Function, Visibility::Public),
                ("farewell".to_string(), SymbolKind::Function, Visibility::Public),
            ]
        );
        assert_eq!(result.symbols[0].qualname, "lib::parser");
        assert_eq!(result.symbols[3].qualname, "lib::greet");
        assert_eq!(result.symbols[3].span.start_line, 5);
        assert_eq!(result.symbols[4].span.start_line, 10);
    }

    #[test]
    fn extracts_local_function_symbols_from_nested_files() {
        let adapter = RustAdapter;
        let entry = fixture_entry("src/parser.rs");
        let source = std::fs::read_to_string(&entry.absolute_path).unwrap();

        let result = adapter.extract(&entry, &source);
        let names: Vec<_> = result.symbols.iter().map(|symbol| symbol.name.clone()).collect();
        let visibilities: Vec<_> = result
            .symbols
            .iter()
            .map(|symbol| symbol.visibility.clone())
            .collect();

        assert_eq!(names, vec!["parse", "tokenize"]);
        assert_eq!(visibilities, vec![Visibility::Public, Visibility::Local]);
        assert_eq!(result.symbols[0].qualname, "parser::parse");
        assert_eq!(result.symbols[1].qualname, "parser::tokenize");
        assert_eq!(result.symbols[1].span.start_line, 5);
    }

    #[test]
    fn extracts_direct_call_sites_from_rust_small_fixture_files() {
        let adapter = RustAdapter;
        let entry = fixture_entry("src/lib.rs");
        let source = std::fs::read_to_string(&entry.absolute_path).unwrap();

        let result = adapter.extract(&entry, &source);
        let calls: Vec<_> = result
            .call_sites
            .iter()
            .map(|call| {
                (
                    call.caller_qualname.clone(),
                    call.callee_name.clone(),
                    call.callee_qualname.clone(),
                    call.span.start_line,
                )
            })
            .collect();

        assert_eq!(
            calls,
            vec![
                (
                    Some("lib::greet".to_string()),
                    "parse".to_string(),
                    Some("parser::parse".to_string()),
                    6,
                ),
                (
                    Some("lib::greet".to_string()),
                    "format_output".to_string(),
                    Some("utils::format_output".to_string()),
                    7,
                ),
                (Some("lib::greet".to_string()), "join".to_string(), None, 7),
            ]
        );
    }

    #[test]
    fn extracts_nested_and_method_calls_conservatively() {
        let file = RepoPath::from("src/example.rs");
        let calls = extract_call_sites_from_line(
            "    outer(inner(), value.join(\"::\"));",
            0,
            0,
            &file,
            Some("example::caller"),
        );

        let extracted: Vec<_> = calls
            .into_iter()
            .map(|call| {
                (
                    call.caller_qualname,
                    call.callee_name,
                    call.callee_qualname,
                    call.is_method,
                    call.span.start_byte,
                    call.span.end_byte,
                )
            })
            .collect();

        assert_eq!(
            extracted,
            vec![
                (Some("example::caller".to_string()), "outer".to_string(), None, false, 4, 9),
                (Some("example::caller".to_string()), "inner".to_string(), None, false, 10, 15),
                (Some("example::caller".to_string()), "join".to_string(), None, true, 25, 29),
            ]
        );
    }

    #[test]
    fn attributes_calls_to_enclosing_top_level_functions() {
        let adapter = RustAdapter;
        let entry = fixture_entry("src/parser.rs");
        let source = std::fs::read_to_string(&entry.absolute_path).unwrap();

        let result = adapter.extract(&entry, &source);
        let calls: Vec<_> = result
            .call_sites
            .iter()
            .map(|call| (call.caller_qualname.clone(), call.callee_name.clone()))
            .collect();

        assert_eq!(
            calls,
            vec![
                (Some("parser::parse".to_string()), "tokenize".to_string()),
                (Some("parser::tokenize".to_string()), "split_whitespace".to_string()),
                (Some("parser::tokenize".to_string()), "map".to_string()),
                (Some("parser::tokenize".to_string()), "collect".to_string()),
            ]
        );
    }

    #[test]
    fn skips_macros_comments_and_control_flow_keywords() {
        let file = RepoPath::from("src/example.rs");

        assert!(extract_call_sites_from_line("println!(\"{}\", foo());", 0, 0, &file, None)
            .into_iter()
            .map(|call| call.callee_name)
            .eq(vec!["foo".to_string()]));
        assert!(extract_call_sites_from_line("if condition(foo()) {}", 0, 0, &file, None)
            .into_iter()
            .map(|call| call.callee_name)
            .eq(vec!["condition".to_string(), "foo".to_string()]));
        assert!(extract_call_sites_from_line("// ignored_call()", 0, 0, &file, None).is_empty());
    }

    #[test]
    fn ts_js_dynamic_patterns_surface_partial_diagnostics() {
        let adapter = TsJsAdapter;
        let entry = fixture_entry_in("ts_small", "src/auth/middleware.ts", SupportedLanguage::TypeScript);
        let source = "const auth = await import(path);\nconst plugin = require(factoryName);\n";

        let result = adapter.extract(&entry, source);

        assert_eq!(result.file.parse_status, ParseStatus::Partial);
        assert_eq!(result.parse_diagnostics.len(), 2);
        assert_eq!(result.parse_diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(result.parse_diagnostics[0].message, "dynamic import is not resolved yet");
        assert_eq!(result.parse_diagnostics[1].message, "computed require is not resolved yet");
        assert!(result.imports.is_empty());
    }

    #[test]
    fn normalizes_rust_visibility_semantics_conservatively() {
        assert_eq!(visibility_semantics("pub fn greet() {} ").visibility, Visibility::Public);
        assert!(visibility_semantics("pub fn greet() {} ").exported);

        assert_eq!(
            visibility_semantics("pub(crate) fn greet() {}").visibility,
            Visibility::Package
        );
        assert!(visibility_semantics("pub(crate) fn greet() {}").exported);

        assert_eq!(
            visibility_semantics("pub(super) fn greet() {}").visibility,
            Visibility::Module
        );
        assert!(!visibility_semantics("pub(super) fn greet() {}").exported);

        assert_eq!(
            visibility_semantics("pub(self) fn greet() {}").visibility,
            Visibility::Local
        );
        assert!(!visibility_semantics("pub(self) fn greet() {}").exported);

        assert_eq!(
            visibility_semantics("pub(in crate::internal) fn greet() {}").visibility,
            Visibility::Unknown
        );
        assert!(!visibility_semantics("pub(in crate::internal) fn greet() {}").exported);

        assert_eq!(visibility_semantics("fn greet() {}").visibility, Visibility::Local);
        assert!(!visibility_semantics("fn greet() {}").exported);
    }

    #[test]
    fn parses_non_public_visibility_prefixes_for_symbols() {
        let file = RepoPath::from("src/example.rs");
        let span = Span {
            start_byte: 0,
            end_byte: 24,
            start_line: 1,
            end_line: 1,
        };

        let package_symbol =
            parse_top_level_symbol("pub(crate) fn helper() {}", &file, &span).unwrap();
        assert_eq!(package_symbol.visibility, Visibility::Package);
        assert!(package_symbol.exported);

        let module_symbol =
            parse_top_level_symbol("pub(super) const VALUE: u32 = 1;", &file, &span).unwrap();
        assert_eq!(module_symbol.visibility, Visibility::Module);
        assert!(!module_symbol.exported);

        let unknown_symbol = parse_top_level_symbol(
            "pub(in crate::internal) type Alias = u32;",
            &file,
            &span,
        )
        .unwrap();
        assert_eq!(unknown_symbol.visibility, Visibility::Unknown);
        assert!(!unknown_symbol.exported);
    }
}
