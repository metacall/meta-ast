//! File-system watch mode.
//!
//! Only the OS watcher lives behind the `watch` feature. The extraction cache
//! and the incremental re-analysis loop are not feature gated:
//!
//! - [`crate::cache`]: BLAKE3 fingerprinting and extraction reuse.
//! - [`crate::reanalyze`]: incremental diffing with buffer overlays.
//! - [`watcher`]: debounced `notify` watcher that drives re-analysis.
//! - [`config`]: watcher configuration.

pub mod config;
pub mod watcher;

pub use config::WatchConfig;
pub use watcher::run_watch;
