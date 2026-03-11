use std::sync::{Mutex, Once, OnceLock};

use tracing_subscriber::EnvFilter;

use crate::{ScopeError, ScopeResult};

static TRACING_INIT: Once = Once::new();
static TRACING_RESULT: OnceLock<Mutex<Option<ScopeResult<()>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
}

pub fn init_tracing(verbosity: Verbosity) -> ScopeResult<()> {
    let result_cell = TRACING_RESULT.get_or_init(|| Mutex::new(None));
    TRACING_INIT.call_once(|| {
        let filter = match verbosity {
            Verbosity::Quiet => EnvFilter::new("error"),
            Verbosity::Normal => {
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
            }
            Verbosity::Verbose => {
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug"))
            }
        };

        let result = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .try_init()
            .map_err(|error| ScopeError::Tracing(error.to_string()))
            .or_else(|error| match error {
                ScopeError::Tracing(message)
                    if message.contains("a global default trace dispatcher has already been set") =>
                {
                    Ok(())
                }
                other => Err(other),
            });

        *result_cell.lock().expect("tracing init mutex poisoned") = Some(result);
    });

    let guard = result_cell.lock().expect("tracing init mutex poisoned");
    match guard.as_ref() {
        Some(Ok(())) | None => Ok(()),
        Some(Err(ScopeError::Tracing(message))) => Err(ScopeError::Tracing(message.clone())),
        Some(Err(other)) => Err(ScopeError::Tracing(other.to_string())),
    }
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
