//! OS filesystem event watcher loop.
//!
//! Listens for file-system events using `notify-debouncer-mini`, triggers
//! incremental re-analysis, and invokes the change callback.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::input;
use crate::interface::report::report_diagnostics;
use crate::language::LangId;
use crate::pipeline::GraphAnalysis;
use crate::reanalyze::{ChangeSet, WatchState, incremental_reanalyze};
use crate::watch::config::WatchConfig;

/// How long the loop waits for events before it checks the stop flag again.
const IDLE_POLL: Duration = Duration::from_millis(100);

/// Start a debounced file-system watcher on `root` and re-analyse on changes.
///
/// The `on_change` closure is called after initial analysis and after
/// every subsequent incremental re-analysis. Typical usage: emit serialized
/// graph output on each change.
///
/// This function blocks until the watcher channel disconnects, or until a
/// failed tick is the only outcome left. Interrupts terminate the process.
/// Use [`run_watch_until`] to stop the loop from another thread.
pub fn run_watch(
    root: PathBuf,
    config: WatchConfig,
    on_change: impl FnMut(&GraphAnalysis, &ChangeSet) -> Result<(), anyhow::Error>,
) -> anyhow::Result<()> {
    let stop = AtomicBool::new(false);
    run_watch_until(root, config, &stop, on_change)
}

/// Like [`run_watch`], but the loop also stops when `stop` is set.
///
/// A tick that fails to emit or to re-analyse is counted and reported once the
/// loop ends, so a long watch run cannot hide a broken output path.
pub fn run_watch_until(
    root: PathBuf,
    config: WatchConfig,
    stop: &AtomicBool,
    mut on_change: impl FnMut(&GraphAnalysis, &ChangeSet) -> Result<(), anyhow::Error>,
) -> anyhow::Result<()> {
    use notify_debouncer_mini::{DebounceEventResult, new_debouncer};

    let mut state = WatchState::new();

    let languages = config.languages.as_deref();
    let debounce = config.debounce();
    let mut failures = 0usize;

    tracing::info!(root = %root.display(), "Running initial analysis");
    let (analysis, change_set, diagnostics) = incremental_reanalyze(&root, languages, &mut state)?;

    // The policy is reported per tick and ends the run only once the loop stops:
    // a long watch run must not abort on a diagnostic it already reported.
    if report_diagnostics(&diagnostics, config.fail_on).is_err() {
        failures += 1;
    }

    on_change(&analysis, &change_set)?;

    let (tx, rx) = std::sync::mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(debounce, move |res| {
        let _ = tx.send(res);
    })?;

    debouncer.watcher().watch(
        &root,
        notify_debouncer_mini::notify::RecursiveMode::Recursive,
    )?;

    tracing::info!(
        root = %root.display(),
        debounce_ms = debounce.as_millis(),
        "Watching for file changes",
    );

    while !stop.load(Ordering::Relaxed) {
        let res = match rx.recv_timeout(IDLE_POLL) {
            Ok(res) => res,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };

        match res {
            Ok(events) => {
                let relevant =
                    has_relevant_path(events.iter().map(|event| event.path.as_path()), languages);
                if !relevant {
                    continue;
                }

                tracing::debug!(count = events.len(), "Debounced change detected");
                match incremental_reanalyze(&root, languages, &mut state) {
                    Ok((analysis, change_set, diagnostics)) => {
                        if report_diagnostics(&diagnostics, config.fail_on).is_err() {
                            failures += 1;
                        }
                        if let Err(e) = on_change(&analysis, &change_set) {
                            failures += 1;
                            tracing::error!("Emit error: {e}");
                        }
                    }
                    Err(e) => {
                        failures += 1;
                        tracing::error!("Re-analysis error: {e}");
                    }
                }
            }
            Err(e) => {
                failures += 1;
                tracing::error!("Watch error: {e}");
            }
        }
    }

    if failures > 0 {
        anyhow::bail!("watch stopped after {failures} failed tick(s)");
    }

    Ok(())
}

/// A batch matters only when it touches a source file of a wanted language.
///
/// Directory metadata events fire on every child change and carry no source
/// path, so they never justify a full re-analysis on their own.
fn has_relevant_path<'a>(
    paths: impl Iterator<Item = &'a Path>,
    languages: Option<&[LangId]>,
) -> bool {
    paths.into_iter().any(|path| {
        input::detect_language(path)
            .is_some_and(|lang| languages.is_none_or(|langs| langs.contains(&lang)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_events_are_not_relevant() {
        let paths = [Path::new("/tmp/project"), Path::new("/tmp/project/sub")];
        assert!(!has_relevant_path(paths.iter().copied(), None));
    }

    #[test]
    fn source_events_are_relevant() {
        let paths = [Path::new("/tmp/project"), Path::new("/tmp/project/main.py")];
        assert!(has_relevant_path(paths.iter().copied(), None));
    }

    #[test]
    fn language_filter_applies_to_events() {
        let paths = [Path::new("/tmp/project/main.py")];
        assert!(!has_relevant_path(
            paths.iter().copied(),
            Some(&[LangId::Rust])
        ));
        assert!(has_relevant_path(
            paths.iter().copied(),
            Some(&[LangId::Python])
        ));
    }
}
