pub mod dashboard;
pub mod emitter;
pub mod graph;
pub mod inspect;
pub mod shard;

use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` by renaming a temporary file beside it.
///
/// A crash between truncate and flush leaves a partial document, and every
/// writer here produces output that is either complete or useless. The
/// temporary file lives in the target directory, so the rename never crosses
/// a mount point. The temporary name is unique per call, so concurrent emits
/// to one path never share a file: the last rename wins with whole content.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("output path has no file name: {}", path.display()),
        )
    })?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = std::ffi::OsString::from(".");
    temporary.push(file_name);
    temporary.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary_path = directory.join(temporary);

    let mut file = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(&temporary_path)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary_path, path) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error);
    }
    sync_directory(directory);
    Ok(())
}

/// Flush the directory entry so the rename survives a crash.
///
/// A failed flush leaves complete content behind with a durability window,
/// so it warns instead of failing the write.
#[cfg(unix)]
fn sync_directory(directory: &Path) {
    let directory = if directory.as_os_str().is_empty() {
        Path::new(".")
    } else {
        directory
    };
    if let Ok(file) = std::fs::File::open(directory)
        && let Err(error) = file.sync_all()
    {
        tracing::warn!(path = %directory.display(), %error, "directory flush failed");
    }
}

/// No directory handle exists on Windows; the rename alone is atomic.
#[cfg(not(unix))]
fn sync_directory(_directory: &Path) {}

/// Supported output serialization formats.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum OutputFormat {
    Json,
    Yaml,
}

impl OutputFormat {
    /// Serialize a value to the chosen format.
    pub fn serialize<T: Serialize>(&self, value: &T) -> anyhow::Result<String> {
        match self {
            Self::Json => Ok(serde_json::to_string_pretty(value)?),
            Self::Yaml => Ok(yaml_serde::to_string(value)?),
        }
    }

    /// File extension for this format, without the dot.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
        }
    }
}

/// The document a command writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputKind {
    /// The interactive dashboard.
    Html,
    /// The serialized graph or symbol list.
    Text,
}

/// Resolve where a command writes its document.
///
/// An explicit path always wins. The dashboard then lands beside the analyzed
/// path as `<stem>.html`, and serialized text goes to stdout.
pub fn default_output_path(
    kind: OutputKind,
    output: Option<PathBuf>,
    root: &Path,
) -> Option<PathBuf> {
    match (kind, output) {
        (_, Some(path)) => Some(path),
        (OutputKind::Html, None) => Some(root.with_extension("html")),
        (OutputKind::Text, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_files(directory: &Path) -> Vec<String> {
        std::fs::read_dir(directory)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.contains(".tmp."))
            .collect()
    }

    #[test]
    fn explicit_output_wins_over_the_default() {
        let given = PathBuf::from("reports/graph.html");
        assert_eq!(
            default_output_path(OutputKind::Html, Some(given.clone()), Path::new("demo")),
            Some(given)
        );
    }

    #[test]
    fn html_defaults_beside_the_analyzed_path() {
        assert_eq!(
            default_output_path(OutputKind::Html, None, Path::new("demo")),
            Some(PathBuf::from("demo.html"))
        );
        assert_eq!(
            default_output_path(OutputKind::Html, None, Path::new("demo/src/main.py")),
            Some(PathBuf::from("demo/src/main.html"))
        );
    }

    #[test]
    fn text_without_a_path_goes_to_stdout() {
        assert_eq!(
            default_output_path(OutputKind::Text, None, Path::new("demo")),
            None
        );
    }

    #[test]
    fn write_atomic_replaces_content_without_leftovers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.json");

        write_atomic(&path, b"first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");

        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(
            temporary_files(directory.path()).is_empty(),
            "no temporary file survives: {:?}",
            temporary_files(directory.path())
        );
    }

    #[test]
    fn write_atomic_failure_creates_no_file() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("nested").join("graph.json");

        assert!(write_atomic(&missing, b"payload").is_err());
        assert!(!missing.exists());
        assert!(temporary_files(directory.path()).is_empty());
    }

    #[test]
    fn write_atomic_rejects_directory_paths() {
        let directory = tempfile::tempdir().unwrap();

        assert!(write_atomic(directory.path(), b"payload").is_err());
        assert!(temporary_files(directory.path()).is_empty());
    }

    #[test]
    fn write_atomic_concurrent_same_path_keeps_whole_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("graph.json");

        std::thread::scope(|scope| {
            for thread in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    let payload = format!("payload-{thread:04}");
                    let payload = payload.repeat(64);
                    write_atomic(path, payload.as_bytes()).unwrap();
                });
            }
        });

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            (0..8).any(|thread| content == format!("payload-{thread:04}").repeat(64)),
            "the file holds one complete payload, got {} bytes",
            content.len()
        );
        assert!(temporary_files(directory.path()).is_empty());
    }
}
