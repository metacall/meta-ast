pub mod dashboard;
pub mod emitter;
pub mod graph;
pub mod inspect;
pub mod shard;

use serde::Serialize;
use std::io::Write;
use std::path::Path;

/// Write `bytes` to `path` by renaming a temporary file beside it.
///
/// A crash between truncate and flush leaves a partial document, and every
/// writer here produces output that is either complete or useless. The
/// temporary file lives in the target directory, so the rename never crosses
/// a mount point.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = std::ffi::OsString::from(".");
    temporary.push(path.file_name().unwrap_or_default());
    temporary.push(format!(".tmp.{}", std::process::id()));
    let temporary_path = directory.join(temporary);

    let mut file = std::fs::File::create(&temporary_path)?;
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
    Ok(())
}

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
}
