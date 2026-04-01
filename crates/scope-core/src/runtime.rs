use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

use crate::{load_arch_config, ArchConfig, ScopeError, ScopeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOperation {
    Report,
    Gate,
    SimulateExtract,
    DiffSnapshot,
    Audit,
    TestMapBuild,
    TestMapCovers,
    TestMapCoveredBy,
    TestMapUncovered,
    EntryList,
    EntryCone,
    EntryReaches,
    EntryUnreachable,
    ArchCheck,
}

impl RuntimeOperation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Gate => "gate",
            Self::SimulateExtract => "simulate-extract",
            Self::DiffSnapshot => "diff-snapshot",
            Self::Audit => "audit",
            Self::TestMapBuild => "test-map-build",
            Self::TestMapCovers => "test-map-covers",
            Self::TestMapCoveredBy => "test-map-covered-by",
            Self::TestMapUncovered => "test-map-uncovered",
            Self::EntryList => "entry-list",
            Self::EntryCone => "entry-cone",
            Self::EntryReaches => "entry-reaches",
            Self::EntryUnreachable => "entry-unreachable",
            Self::ArchCheck => "arch-check",
        }
    }

    pub fn is_expensive(self) -> bool {
        matches!(
            self,
            Self::Report | Self::Gate | Self::SimulateExtract | Self::DiffSnapshot
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePolicy {
    pub allow_expensive_operations: bool,
    pub allow_cross_workspace_roots: bool,
}

impl Default for RuntimePolicy {
    fn default() -> Self {
        Self {
            allow_expensive_operations: true,
            allow_cross_workspace_roots: true,
        }
    }
}

impl RuntimePolicy {
    pub fn from_env() -> Self {
        Self {
            allow_expensive_operations: env_flag("SCOPE_RUNTIME_ALLOW_EXPENSIVE_OPS", true),
            allow_cross_workspace_roots: env_flag("SCOPE_RUNTIME_ALLOW_CROSS_ROOTS", true),
        }
    }

    pub fn deny_expensive() -> Self {
        Self {
            allow_expensive_operations: false,
            ..Self::default()
        }
    }

    pub fn deny_cross_workspace_roots() -> Self {
        Self {
            allow_cross_workspace_roots: false,
            ..Self::default()
        }
    }

    pub fn evaluate_operation(&self, operation: RuntimeOperation) -> ScopeResult<()> {
        if operation.is_expensive() && !self.allow_expensive_operations {
            return Err(ScopeError::InvalidInput(format!(
                "runtime policy denied `{}`: expensive runtime operations are disabled for this session",
                operation.name()
            )));
        }
        Ok(())
    }

    pub fn evaluate_repo_root(
        &self,
        session_root: &Path,
        requested_root: &Path,
    ) -> ScopeResult<()> {
        if self.allow_cross_workspace_roots {
            return Ok(());
        }

        let session_root = normalize_path(session_root);
        let requested_root = normalize_path(requested_root);
        if requested_root == session_root || requested_root.starts_with(&session_root) {
            return Ok(());
        }

        Err(ScopeError::InvalidInput(format!(
            "runtime policy denied repo_root `{}`: cross-workspace roots are disabled for this session (session root: `{}`)",
            requested_root.display(),
            session_root.display()
        )))
    }
}

#[derive(Debug, Default)]
pub struct RuntimeMetadataCache {
    arch_configs: Mutex<HashMap<PathBuf, CachedArchConfig>>,
}

impl RuntimeMetadataCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn invalidate_arch_config(&self, repo_root: &Path) -> ScopeResult<()> {
        let mut cache = self
            .arch_configs
            .lock()
            .map_err(|error| ScopeError::Internal(format!("runtime cache poisoned: {error}")))?;
        cache.remove(repo_root);
        Ok(())
    }

    pub fn load_arch_config(
        &self,
        repo_root: &Path,
        policy: &RuntimePolicy,
        operation: RuntimeOperation,
    ) -> ScopeResult<ArchConfig> {
        policy.evaluate_operation(operation)?;

        let config_path = arch_config_path(repo_root);
        let state = current_file_state(&config_path)?;
        let mut cache = self
            .arch_configs
            .lock()
            .map_err(|error| ScopeError::Internal(format!("runtime cache poisoned: {error}")))?;

        if let Some(cached) = cache.get(repo_root) {
            if cached.state == state {
                return Ok(cached.config.clone());
            }
        }

        let config = load_arch_config(repo_root)?;
        cache.insert(
            repo_root.to_path_buf(),
            CachedArchConfig {
                state,
                config: config.clone(),
            },
        );
        Ok(config)
    }
}

#[derive(Debug, Clone)]
struct CachedArchConfig {
    state: ArchConfigFileState,
    config: ArchConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchConfigFileState {
    exists: bool,
    modified: Option<SystemTime>,
    size_bytes: Option<u64>,
}

fn arch_config_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".scope/arch.toml")
}

fn current_file_state(path: &Path) -> ScopeResult<ArchConfigFileState> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(ArchConfigFileState {
            exists: true,
            modified: metadata.modified().ok(),
            size_bytes: Some(metadata.len()),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ArchConfigFileState {
            exists: false,
            modified: None,
            size_bytes: None,
        }),
        Err(error) => Err(ScopeError::io(path, error)),
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn env_flag(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "0" | "false" | "no" | "off" => false,
            "1" | "true" | "yes" | "on" => true,
            _ => default,
        },
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        thread,
        time::{Duration, UNIX_EPOCH},
    };

    use super::*;

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("scope-runtime-{prefix}-{nanos}"))
    }

    fn write_arch(repo_root: &Path, source: &str) {
        let scope_dir = repo_root.join(".scope");
        fs::create_dir_all(&scope_dir).expect("scope dir should exist");
        fs::write(scope_dir.join("arch.toml"), source).expect("arch config should be written");
    }

    #[test]
    fn reloads_arch_config_when_file_metadata_changes() {
        let repo_root = unique_temp_dir("reload");
        fs::create_dir_all(&repo_root).expect("repo root should exist");
        write_arch(
            &repo_root,
            r#"
[[gate]]
metric = "cycles"
severity = "warning"
message = "first"
skip = true
"#,
        );

        let cache = RuntimeMetadataCache::new();
        let policy = RuntimePolicy::default();

        let first = cache
            .load_arch_config(&repo_root, &policy, RuntimeOperation::Gate)
            .expect("first config load should succeed");
        assert_eq!(first.gates.len(), 1);
        assert_eq!(first.gates[0].message.as_deref(), Some("first"));

        thread::sleep(Duration::from_millis(5));
        write_arch(
            &repo_root,
            r#"
[[gate]]
metric = "cycles"
severity = "warning"
message = "second"
skip = true
"#,
        );

        let second = cache
            .load_arch_config(&repo_root, &policy, RuntimeOperation::Gate)
            .expect("second config load should succeed");
        assert_eq!(second.gates[0].message.as_deref(), Some("second"));

        let _ = fs::remove_dir_all(repo_root);
    }

    #[test]
    fn policy_denies_expensive_operations_with_readable_reason() {
        let repo_root = unique_temp_dir("policy");
        fs::create_dir_all(&repo_root).expect("repo root should exist");
        write_arch(
            &repo_root,
            r#"
[[gate]]
metric = "cycles"
severity = "warning"
message = "blocked"
skip = true
"#,
        );

        let cache = RuntimeMetadataCache::new();
        let error = cache
            .load_arch_config(
                &repo_root,
                &RuntimePolicy::deny_expensive(),
                RuntimeOperation::Report,
            )
            .expect_err("policy should deny report");
        assert!(error
            .to_string()
            .contains("runtime policy denied `report`: expensive runtime operations are disabled"));

        let _ = fs::remove_dir_all(repo_root);
    }

    #[test]
    fn policy_can_deny_cross_workspace_roots() {
        let session_root = unique_temp_dir("session");
        let other_root = unique_temp_dir("other");
        fs::create_dir_all(&session_root).expect("session root should exist");
        fs::create_dir_all(&other_root).expect("other root should exist");

        let error = RuntimePolicy::deny_cross_workspace_roots()
            .evaluate_repo_root(&session_root, &other_root)
            .expect_err("cross-workspace root should be denied");
        assert!(error
            .to_string()
            .contains("runtime policy denied repo_root"));

        let _ = fs::remove_dir_all(session_root);
        let _ = fs::remove_dir_all(other_root);
    }
}
