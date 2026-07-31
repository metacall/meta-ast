//! Persistent watch state and change tracking.

use crate::model::SnapshotId;
use crate::watch::cache::IncrementalCache;

/// Metrics summarizing file changes between analysis snapshots.
#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    /// Number of newly discovered source files.
    pub files_added: usize,
    /// Number of deleted or missing source files.
    pub files_removed: usize,
    /// Number of modified files re-parsed in this tick.
    pub files_modified: usize,
    /// Number of untouched files reusing cached extractions.
    pub files_unchanged: usize,
}

/// Mutable state preserved across incremental re-analysis passes.
pub struct WatchState {
    pub(crate) cache: IncrementalCache,
    pub(crate) snapshot_counter: u32,
}

impl WatchState {
    /// Create a new empty WatchState.
    pub fn new() -> Self {
        Self {
            cache: IncrementalCache::new(),
            snapshot_counter: 0,
        }
    }

    /// Allocate the next monotonic snapshot ID for re-analysis.
    pub(crate) fn next_snapshot_id(&mut self) -> SnapshotId {
        self.snapshot_counter += 1;
        SnapshotId::new(self.snapshot_counter)
            .expect("watch snapshot counter exhausted (> u32::MAX)")
    }
}

impl Default for WatchState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_id_allocation_is_monotonic() {
        let mut state = WatchState::new();
        let s1 = state.next_snapshot_id();
        let s2 = state.next_snapshot_id();
        assert_eq!(s1.to_raw(), 1);
        assert_eq!(s2.to_raw(), 2);
    }
}
