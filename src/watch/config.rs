//! Configuration for watch mode execution.

use std::path::PathBuf;
use std::time::Duration;

use crate::language::LangId;
use crate::output::OutputFormat;

/// Configuration parameters governing the debounced file-system watcher.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Debounce duration before triggering re-analysis after file changes.
    pub debounce: Duration,
    /// Serialization format for graph output.
    pub format: OutputFormat,
    /// Optional output file path for serialized results.
    pub output: Option<PathBuf>,
    /// Generate an interactive HTML dashboard.
    pub html: bool,
    /// Automatically open the browser when generating an HTML dashboard.
    pub open_browser: bool,
    /// Optional language filter applied to discovered files.
    pub languages: Option<Vec<LangId>>,
}

impl WatchConfig {
    /// Create a new WatchConfig with a default 200 ms debounce duration and JSON output.
    pub fn new() -> Self {
        Self {
            debounce: Duration::from_millis(200),
            format: OutputFormat::Json,
            output: None,
            html: false,
            open_browser: true,
            languages: None,
        }
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
        assert_eq!(cfg.format, OutputFormat::Json);
        assert!(cfg.output.is_none());
        assert!(!cfg.html);
        assert!(cfg.open_browser);
        assert!(cfg.languages.is_none());
    }
}
