use std::path::{Path, PathBuf};

use crate::{
    Certainty, ExportRecord, ExtractResult, FileRecord, ImportPath, ImportRecord, ModuleRecord,
    ParseStatus, RepoPath, ScanEntry, Span, SymbolKind, Visibility,
};

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

pub struct RustAdapter;

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

        let repo_root = infer_repo_root(entry);
        let crate_root = repo_root.join("src");
        let module_dir = rust_module_dir(entry);

        let mut byte_offset = 0usize;
        for (line_index, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            let span = line_span(line, byte_offset, line_index);

            if let Some(module) = parse_module_declaration(trimmed, &entry.path, &module_dir, &span)
            {
                let exported_module_name = module.name.clone();

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
                        name: exported_module_name,
                        qualname: None,
                        kind: SymbolKind::Module,
                        visibility: Visibility::Public,
                        span: span.clone(),
                    });
                }
            }

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

            byte_offset += line.len() + 1;
        }

        ExtractResult {
            file: FileRecord {
                path: entry.path.clone(),
                language: self.language_id().to_string(),
                parse_status: ParseStatus::Ok,
                is_barrel: false,
            },
            imports,
            modules,
            exports,
            symbols: Vec::new(),
            call_sites: Vec::new(),
            parse_diagnostics: Vec::new(),
        }
    }
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
    let relative = path.strip_prefix(&repo_root).unwrap_or(path);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{scanner::SupportedLanguage, RepoPath};

    fn fixture_entry(relative: &str) -> ScanEntry {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let fixture_root = repo_root.join("fixtures/rust_small");
        let relative_path = Path::new(relative);

        ScanEntry {
            path: RepoPath::from(relative),
            absolute_path: fixture_root.join(relative_path),
            language: SupportedLanguage::Rust,
        }
    }

    #[test]
    fn matches_supported_extensions() {
        let adapter = RustAdapter;
        assert!(supports_path(&adapter, Path::new("src/lib.rs")));
        assert!(!supports_path(&adapter, Path::new("src/lib.ts")));
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
}
