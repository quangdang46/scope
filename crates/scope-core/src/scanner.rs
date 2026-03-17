use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

use crate::{RepoPath, ScopeError, ScopeResult};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanConfig {
    pub include_hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Ruby,
    Go,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEntry {
    pub path: RepoPath,
    pub absolute_path: PathBuf,
    pub language: SupportedLanguage,
}

pub fn scan_repo(root: &Path, config: &ScanConfig) -> ScopeResult<Vec<ScanEntry>> {
    let mut builder = WalkBuilder::new(root);
    builder.hidden(!config.include_hidden);
    builder.git_ignore(true);
    builder.git_exclude(true);
    builder.git_global(true);
    builder.require_git(false);
    builder.add_custom_ignore_filename(".scopeignore");

    let walker = builder.build();
    let mut entries = Vec::new();

    for result in walker {
        let dir_entry = result.map_err(|error| ScopeError::io(root, error))?;
        let path = dir_entry.path();

        if !dir_entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }

        let Some(language) = supported_language(path) else {
            continue;
        };

        let relative = path
            .strip_prefix(root)
            .map_err(|error| ScopeError::io(path, error))?;

        entries.push(ScanEntry {
            path: RepoPath::from(normalize_relative_path(relative)),
            absolute_path: path.to_path_buf(),
            language,
        });
    }

    entries.sort_by(|left, right| left.path.0.cmp(&right.path.0));
    Ok(entries)
}

fn supported_language(path: &Path) -> Option<SupportedLanguage> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("rs") => Some(SupportedLanguage::Rust),
        Some("ts") | Some("tsx") => Some(SupportedLanguage::TypeScript),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => Some(SupportedLanguage::JavaScript),
        Some("py") => Some(SupportedLanguage::Python),
        Some("rb") => Some(SupportedLanguage::Ruby),
        Some("go") => Some(SupportedLanguage::Go),
        _ => None,
    }
}

fn normalize_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("scope-scanner-{prefix}-{nanos}"))
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn includes_supported_source_files_and_normalizes_paths() {
        let root = unique_temp_dir("supported");
        write_file(&root.join("src/lib.rs"), "pub fn greet() {}\n");
        write_file(&root.join("web/app.ts"), "export const app = true;\n");
        write_file(&root.join("README.md"), "ignored\n");

        let entries = scan_repo(&root, &ScanConfig::default()).unwrap();
        let paths: Vec<_> = entries.into_iter().map(|entry| entry.path.0).collect();

        assert_eq!(paths, vec!["src/lib.rs", "web/app.ts"]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn respects_gitignore_files() {
        let root = unique_temp_dir("gitignore");
        write_file(&root.join(".gitignore"), "ignored/\n");
        write_file(&root.join("src/lib.rs"), "pub fn greet() {}\n");
        write_file(&root.join("ignored/skip.rs"), "pub fn skip() {}\n");

        let entries = scan_repo(&root, &ScanConfig::default()).unwrap();
        let paths: Vec<_> = entries.into_iter().map(|entry| entry.path.0).collect();

        assert_eq!(paths, vec!["src/lib.rs"]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn respects_scopeignore_files() {
        let root = unique_temp_dir("scopeignore");
        write_file(&root.join(".scopeignore"), "vendor/\n");
        write_file(&root.join("src/lib.rs"), "pub fn greet() {}\n");
        write_file(
            &root.join("vendor/generated.ts"),
            "export const generated = true;\n",
        );

        let entries = scan_repo(&root, &ScanConfig::default()).unwrap();
        let paths: Vec<_> = entries.into_iter().map(|entry| entry.path.0).collect();

        assert_eq!(paths, vec!["src/lib.rs"]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn excludes_hidden_files_by_default() {
        let root = unique_temp_dir("hidden-default");
        write_file(&root.join(".hidden.rs"), "pub fn hidden() {}\n");
        write_file(&root.join("src/lib.rs"), "pub fn greet() {}\n");

        let entries = scan_repo(&root, &ScanConfig::default()).unwrap();
        let paths: Vec<_> = entries.into_iter().map(|entry| entry.path.0).collect();

        assert_eq!(paths, vec!["src/lib.rs"]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn can_include_hidden_files_when_requested() {
        let root = unique_temp_dir("hidden-include");
        write_file(&root.join(".hidden.rs"), "pub fn hidden() {}\n");
        write_file(&root.join("src/lib.rs"), "pub fn greet() {}\n");

        let entries = scan_repo(
            &root,
            &ScanConfig {
                include_hidden: true,
            },
        )
        .unwrap();
        let paths: Vec<_> = entries.into_iter().map(|entry| entry.path.0).collect();

        assert_eq!(paths, vec![".hidden.rs", "src/lib.rs"]);

        fs::remove_dir_all(root).unwrap();
    }
}
