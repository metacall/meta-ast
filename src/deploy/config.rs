//! MetaCall configuration file parsing.
//!
//! Mirrors the schema read by `loader_load_from_configuration` in MetaCall:
//! a `language_id`, an optional `path`, and a `scripts` array. Relative
//! `path` values resolve against the configuration file directory.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::error::{Diagnostic, Severity};

#[derive(Debug, Clone)]
pub(crate) struct LoadConfiguration {
    pub language_id: Option<String>,
    pub context_path: Option<PathBuf>,
    pub scripts: Vec<String>,
}

pub(crate) fn parse_load_configuration(bytes: &[u8]) -> Result<LoadConfiguration, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("invalid MetaCall configuration JSON: {error}"))?;
    let language_id = value
        .get("language_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string);
    let context_path = value
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let scripts = value
        .get("scripts")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(LoadConfiguration {
        language_id,
        context_path,
        scripts,
    })
}

/// MetaCall configuration file read, in bytes.
///
/// Configurations are small JSON documents; anything larger is reported
/// instead of buffered without bound.
pub(crate) const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Join a user-supplied reference onto the root, refusing escapes.
///
/// Absolute references and `..` escapes return `None`: the caller reports
/// the reference instead of reading outside the project. The check is
/// lexical; a symlink inside the root pointing out still follows it.
pub(crate) fn join_contained(root: &Path, reference: &str) -> Option<PathBuf> {
    let reference_path = Path::new(reference);
    if reference_path.is_absolute() {
        return None;
    }
    let joined = root.join(reference_path);
    normalize(&joined)
        .starts_with(normalize(root))
        .then_some(joined)
}

/// Read a configuration file with the size cap enforced during the read.
///
/// Both failure modes arrive as a message the caller attaches to the call
/// site that named the file.
pub(crate) fn read_config_file(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|error| format!("unreadable MetaCall configuration: {error}"))?;
    let mut bounded = file.take(MAX_CONFIG_BYTES + 1);
    let mut bytes = Vec::new();
    bounded
        .read_to_end(&mut bytes)
        .map_err(|error| format!("unreadable MetaCall configuration: {error}"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(format!(
            "MetaCall configuration is over the {MAX_CONFIG_BYTES} byte limit"
        ));
    }
    Ok(bytes)
}

/// True when a script reference escapes its resolution base.
///
/// Absolute references outside the base and `..` escapes never resolve to
/// project files through the lookup strategies, so the caller reports them
/// instead of minting an external node named after the escape.
pub(crate) fn escapes_base(base: &Path, script: &str) -> bool {
    let script_path = Path::new(script);
    let base = normalize(base);
    let joined = if script_path.is_absolute() {
        normalize(script_path)
    } else {
        normalize(&base.join(script_path))
    };
    !joined.starts_with(&base)
}

/// One read and parse of each configuration file per phase.
///
/// Load-edge injection and client-call resolution read the same
/// configurations in separate phases, so each phase keeps one cache: N
/// sites sharing a file read and parse it once, and one failure reports
/// once without relying on end-of-run dedup.
#[derive(Debug, Default)]
pub(crate) struct ConfigCache {
    entries: HashMap<PathBuf, Result<LoadConfiguration, String>>,
}

impl ConfigCache {
    /// Read, parse, and memoize one configuration file.
    pub(crate) fn get(&mut self, file: &Path) -> Result<LoadConfiguration, String> {
        if let Some(entry) = self.entries.get(file) {
            return entry.clone();
        }
        let parsed = read_config_file(file).and_then(|bytes| parse_load_configuration(&bytes));
        self.entries.insert(file.to_path_buf(), parsed.clone());
        parsed
    }
}

/// Base directory for script resolution.
///
/// An absolute `path` wins. A relative `path` joins the configuration
/// directory. Without `path`, the caller fallback applies. The result is
/// lexically normalized so `..` components match the discovered file paths.
pub(crate) fn script_base(
    config_file: &Path,
    config: &LoadConfiguration,
    fallback: &Path,
) -> PathBuf {
    let Some(context_path) = &config.context_path else {
        return fallback.to_path_buf();
    };
    if context_path.is_absolute() {
        return normalize(context_path);
    }
    normalize(&config_file.parent().unwrap_or(fallback).join(context_path))
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub(crate) fn config_diagnostic(
    path: &Path,
    range: Option<&crate::model::SourceRange>,
    message: String,
) -> Diagnostic {
    Diagnostic {
        path: path.to_path_buf(),
        severity: Severity::Warning,
        message,
        source_range: range.cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_language_id_path_and_scripts() {
        let bytes = br#"{"language_id":"node","path":"../lib","scripts":["extra.js","other.js"]}"#;
        let parsed = parse_load_configuration(bytes).unwrap();
        assert_eq!(parsed.language_id.as_deref(), Some("node"));
        assert_eq!(parsed.context_path, Some(PathBuf::from("../lib")));
        assert_eq!(parsed.scripts, vec!["extra.js", "other.js"]);
    }

    #[test]
    fn missing_fields_are_empty() {
        let parsed = parse_load_configuration(b"{}").unwrap();
        assert!(parsed.language_id.is_none());
        assert!(parsed.context_path.is_none());
        assert!(parsed.scripts.is_empty());
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_load_configuration(b"not json").is_err());
    }

    #[test]
    fn relative_path_joins_the_config_directory() {
        let config = LoadConfiguration {
            language_id: Some("node".to_string()),
            context_path: Some(PathBuf::from("../lib")),
            scripts: Vec::new(),
        };
        let base = script_base(
            Path::new("/proj/cfg/deploy.json"),
            &config,
            Path::new("/proj"),
        );
        assert_eq!(base, PathBuf::from("/proj/lib"));
    }

    #[test]
    fn absolute_path_wins() {
        let config = LoadConfiguration {
            language_id: None,
            context_path: Some(PathBuf::from("/opt/lib")),
            scripts: Vec::new(),
        };
        let base = script_base(
            Path::new("/proj/cfg/deploy.json"),
            &config,
            Path::new("/proj"),
        );
        assert_eq!(base, PathBuf::from("/opt/lib"));
    }

    #[test]
    fn fallback_applies_without_path() {
        let config = LoadConfiguration {
            language_id: None,
            context_path: None,
            scripts: Vec::new(),
        };
        let base = script_base(
            Path::new("/proj/cfg/deploy.json"),
            &config,
            Path::new("/proj"),
        );
        assert_eq!(base, PathBuf::from("/proj"));
    }

    #[test]
    fn contained_join_accepts_inside_references() {
        let root = Path::new("/proj");
        assert_eq!(
            join_contained(root, "cfg/deploy.json"),
            Some(PathBuf::from("/proj/cfg/deploy.json"))
        );
        assert_eq!(
            join_contained(root, "sub/../deploy.json"),
            Some(PathBuf::from("/proj/sub/../deploy.json")),
            "a dot-dot that stays inside resolves"
        );
    }

    #[test]
    fn contained_join_refuses_escapes() {
        let root = Path::new("/proj");
        assert_eq!(join_contained(root, "/etc/x.json"), None);
        assert_eq!(join_contained(root, "../../etc/x.json"), None);
        assert_eq!(join_contained(root, "../project/x.json"), None);
    }

    #[test]
    fn escapes_base_flags_outside_scripts() {
        let base = Path::new("/proj");
        assert!(!escapes_base(base, "sum.js"));
        assert!(!escapes_base(base, "sub/../sum.js"));
        assert!(!escapes_base(base, "/proj/sum.js"));
        assert!(escapes_base(base, "/etc/passwd"));
        assert!(escapes_base(base, "../../etc/passwd"));
    }

    #[test]
    fn missing_config_is_unreadable() {
        let missing = Path::new("/proj/does-not-exist.json");
        let error = read_config_file(missing).unwrap_err();
        assert!(
            error.contains("unreadable"),
            "the message attaches to a call site: {error}"
        );
    }

    #[test]
    fn oversized_config_is_reported() {
        let dir = std::env::temp_dir().join("meta_ast_config_cap");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.json");
        let chunk = vec![b'x'; 64 * 1024];
        let mut file = std::fs::File::create(&path).unwrap();
        use std::io::Write;
        for _ in 0..=MAX_CONFIG_BYTES / chunk.len() as u64 {
            file.write_all(&chunk).unwrap();
        }
        drop(file);

        let error = read_config_file(&path).unwrap_err();
        assert!(
            error.contains(&MAX_CONFIG_BYTES.to_string()),
            "the error names the cap: {error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_cache_reads_once() {
        let dir = std::env::temp_dir().join("meta_ast_config_cache");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("deploy.json");
        std::fs::write(&path, r#"{"language_id":"node","scripts":["a.js"]}"#).unwrap();

        let mut cache = ConfigCache::default();
        let first = cache.get(&path).unwrap();
        assert_eq!(first.scripts, vec!["a.js"]);

        std::fs::remove_file(&path).unwrap();
        let second = cache.get(&path).unwrap();
        assert_eq!(
            second.scripts, first.scripts,
            "the deleted file still resolves from the cache"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
