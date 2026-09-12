//! MetaCall configuration file parsing.
//!
//! Mirrors the schema read by `loader_load_from_configuration` in MetaCall:
//! a `language_id`, an optional `path`, and a `scripts` array. Relative
//! `path` values resolve against the configuration file directory.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::error::{Diagnostic, Severity};

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
}
