//! OS filesystem event watcher loop.
//!
//! Listens for file-system events using `notify-debouncer-mini`, triggers
//! incremental re-analysis, and invokes the change callback.

use std::path::PathBuf;

use crate::input;
use crate::pipeline::GraphAnalysis;
use crate::watch::config::WatchConfig;
use crate::watch::reanalyze::incremental_reanalyze;
use crate::watch::state::{ChangeSet, WatchState};

/// Start a debounced file-system watcher on `root` and re-analyse on changes.
///
/// The `on_change` closure is called after initial analysis and after
/// every subsequent incremental re-analysis. Typical usage: emit serialized
/// graph output on each change.
///
/// This function blocks until the watcher encounters an unrecoverable error
/// or the underlying channel disconnects.
pub fn run_watch(
    root: PathBuf,
    config: WatchConfig,
    mut on_change: impl FnMut(&GraphAnalysis, &ChangeSet) -> Result<(), anyhow::Error>,
) -> anyhow::Result<()> {
    use notify_debouncer_mini::{DebounceEventResult, new_debouncer};

    let mut state = WatchState::new();

    tracing::info!(root = %root.display(), "Running initial analysis");
    let (analysis, change_set, diags) = incremental_reanalyze(&root, &mut state)?;

    for d in &diags {
        tracing::warn!(
            path = %d.path.display(),
            severity = ?d.severity,
            "{}",
            d.message,
        );
    }

    on_change(&analysis, &change_set)?;

    let (tx, rx) = std::sync::mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(config.debounce, move |res| {
        let _ = tx.send(res);
    })?;

    debouncer.watcher().watch(
        &root,
        notify_debouncer_mini::notify::RecursiveMode::Recursive,
    )?;

    tracing::info!(
        root = %root.display(),
        debounce_ms = config.debounce.as_millis(),
        "Watching for file changes",
    );

    for res in rx {
        match res {
            Ok(events) => {
                let relevant = events
                    .iter()
                    .any(|e| input::detect_language(&e.path).is_some() || e.path.is_dir());
                if !relevant && !events.is_empty() {
                    continue;
                }

                tracing::debug!(count = events.len(), "Debounced change detected");
                match incremental_reanalyze(&root, &mut state) {
                    Ok((analysis, change_set, diags)) => {
                        for d in &diags {
                            tracing::warn!(
                                path = %d.path.display(),
                                severity = ?d.severity,
                                "{}",
                                d.message,
                            );
                        }
                        if let Err(e) = on_change(&analysis, &change_set) {
                            tracing::error!("Emit error: {e}");
                        }
                    }
                    Err(e) => tracing::error!("Re-analysis error: {e}"),
                }
            }
            Err(e) => {
                tracing::error!("Watch error: {e}");
            }
        }
    }

    Ok(())
}
