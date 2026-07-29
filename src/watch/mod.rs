//! File-system watch mode with incremental re-analysis.
//!
//! Re-parsing an entire codebase on every edit is slow and wasteful. The `watch`
//! module monitors project directories for source file changes and runs incremental
//! re-analysis.
//!
//! ## Architecture
//!
//! - **BLAKE3 Fingerprinting** ([`cache`]): Hashing file bytes with BLAKE3 detects real content changes beyond file modification timestamps.
//! - **Zero-Allocation Cache Sharing** ([`cache`]): Unchanged files reuse their `Arc<FileExtraction>` pointers. Re-analysis avoids deep vector cloning.
//! - **Incremental Diffing** ([`reanalyze`]): Computes precise `ChangeSet` diffs (added, modified, removed, unchanged) between ticks.
//! - **Debounced OS Watcher** ([`watcher`]): Integrates `notify-debouncer-mini` to aggregate rapid file edits before triggering re-analysis.

pub mod cache;
pub mod config;
pub mod reanalyze;
pub mod state;
pub mod watcher;

pub use config::WatchConfig;
pub use reanalyze::incremental_reanalyze;
pub use state::{ChangeSet, WatchState};
pub use watcher::run_watch;
