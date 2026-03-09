use std::path::{Path, PathBuf};

use crate::{ScopeError, ScopeResult};

#[derive(Debug, Clone, Default)]
pub struct BootstrapOptions {
    pub repo_root_override: Option<PathBuf>,
    pub db_override: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub repo_root: PathBuf,
    pub scope_dir: PathBuf,
    pub db_path: PathBuf,
}

pub fn find_repo_root(start: &Path) -> ScopeResult<PathBuf> {
    let mut current = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };

    loop {
        if has_repo_marker(&current) {
            return Ok(current);
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return Ok(start.to_path_buf()),
        }
    }
}

pub fn discover_runtime_paths(cwd: &Path, options: &BootstrapOptions) -> ScopeResult<RuntimePaths> {
    let repo_root = match &options.repo_root_override {
        Some(path) => path.clone(),
        None => find_repo_root(cwd)?,
    };

    let scope_dir = repo_root.join(".scope");
    let db_path = options
        .db_override
        .clone()
        .unwrap_or_else(|| scope_dir.join("index.db"));

    Ok(RuntimePaths {
        repo_root,
        scope_dir,
        db_path,
    })
}

pub fn ensure_scope_dir(scope_dir: &Path) -> ScopeResult<()> {
    std::fs::create_dir_all(scope_dir).map_err(|error| ScopeError::io(scope_dir, error))
}

fn has_repo_marker(path: &Path) -> bool {
    [".git", ".scope", "Cargo.toml", "package.json"]
        .into_iter()
        .any(|marker| path.join(marker).exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("scope-{prefix}-{nanos}"))
    }

    #[test]
    fn finds_repo_root_from_nested_directory() {
        let root = unique_temp_dir("repo-root");
        let nested = root.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();

        let found = find_repo_root(&nested).unwrap();
        assert_eq!(found, root);

        std::fs::remove_dir_all(found).unwrap();
    }

    #[test]
    fn defaults_to_scope_db_under_repo_root() {
        let cwd = unique_temp_dir("runtime-paths");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(cwd.join("package.json"), "{}\n").unwrap();

        let paths = discover_runtime_paths(&cwd, &BootstrapOptions::default()).unwrap();
        assert_eq!(paths.repo_root, cwd);
        assert_eq!(paths.scope_dir, paths.repo_root.join(".scope"));
        assert_eq!(paths.db_path, paths.scope_dir.join("index.db"));

        std::fs::remove_dir_all(paths.repo_root).unwrap();
    }
}
