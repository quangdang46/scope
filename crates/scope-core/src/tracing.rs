use std::sync::OnceLock;

use tracing_subscriber::EnvFilter;

use crate::{ScopeError, ScopeResult};

static TRACING_INIT: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

pub fn init_tracing(verbosity: Verbosity) -> ScopeResult<()> {
    if TRACING_INIT.get().is_some() {
        return Ok(());
    }

    let filter = match verbosity {
        Verbosity::Quiet => EnvFilter::new("error"),
        Verbosity::Normal => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        Verbosity::Verbose => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init()
        .map_err(|error| ScopeError::Tracing(error.to_string()))?;

    let _ = TRACING_INIT.set(());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracing_initialization_is_idempotent() {
        init_tracing(Verbosity::Quiet).unwrap();
        init_tracing(Verbosity::Verbose).unwrap();
    }
}
