//! Configuration for watch mode execution.

use std::time::Duration;

use crate::interface::report::FailOn;
use crate::language::LangId;
use crate::output::emitter::EmitConfig;

/// Floor for the debounce duration. Zero turns the watcher into a hot loop on
/// chatty file systems.
pub const MIN_DEBOUNCE: Duration = Duration::from_millis(50);

/// Configuration parameters governing the debounced file-system watcher.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Debounce duration before triggering re-analysis after file changes.
    pub debounce: Duration,
    /// Emission configuration applied to every rebuild.
    pub emit: EmitConfig,
    /// Diagnostic severity that fails the run when the watcher stops.
    pub fail_on: FailOn,
    /// Optional language filter applied to discovered files.
    pub languages: Option<Vec<LangId>>,
}

impl WatchConfig {
    /// Create a new WatchConfig with a default 200 ms debounce duration and JSON output.
    pub fn new() -> Self {
        Self {
            debounce: Duration::from_millis(200),
            emit: EmitConfig {
                output: None,
                format: crate::output::OutputFormat::Json,
                html: false,
                open_browser: false,
            },
            fail_on: FailOn::Error,
            languages: None,
        }
    }

    /// Effective debounce duration, never below [`MIN_DEBOUNCE`].
    pub fn debounce(&self) -> Duration {
        self.debounce.max(MIN_DEBOUNCE)
    }
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_config_default_values() {
        let cfg = WatchConfig::default();
        assert_eq!(cfg.debounce, Duration::from_millis(200));
        assert_eq!(cfg.debounce(), Duration::from_millis(200));
        assert_eq!(cfg.emit.format, crate::output::OutputFormat::Json);
        assert!(cfg.emit.output.is_none());
        assert!(!cfg.emit.html);
        assert!(!cfg.emit.open_browser);
        assert_eq!(cfg.fail_on, FailOn::Error);
        assert!(cfg.languages.is_none());
    }

    #[test]
    fn debounce_has_a_floor() {
        let cfg = WatchConfig {
            debounce: Duration::ZERO,
            ..WatchConfig::new()
        };
        assert_eq!(cfg.debounce(), MIN_DEBOUNCE);
    }
}
