use std::path::Path;

use crate::{
    config::{discover_runtime_paths, ensure_scope_dir, BootstrapOptions, RuntimePaths},
    store::Store,
    tracing::{init_tracing, Verbosity},
    ScopeResult,
};

#[derive(Debug)]
pub struct AppContext {
    pub paths: RuntimePaths,
    pub store: Store,
}

pub fn bootstrap(cwd: &Path, options: &BootstrapOptions, verbosity: Verbosity) -> ScopeResult<AppContext> {
    init_tracing(verbosity)?;
    let paths = discover_runtime_paths(cwd, options)?;
    ensure_scope_dir(&paths.scope_dir)?;
    let store = Store::open(&paths.db_path)?;

    Ok(AppContext { paths, store })
}
