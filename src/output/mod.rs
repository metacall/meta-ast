pub mod dashboard;
pub mod emitter;
pub mod graph;
pub mod inspect;
pub mod shard;

use serde::Serialize;

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

// ---------------------------------------------------------------------------
// Structured JSON error output (issue #62)
// ---------------------------------------------------------------------------

/// Classifies errors into machine-readable kinds for the JSON error envelope.
///
/// Variants are derived from [`crate::error::Error`].  `UnknownError` is the
/// fallback for errors that cannot be downcast to a known variant.
#[derive(Debug, Clone, Serialize)]
pub enum ErrorKind {
    IoError,
    ParseError,
    QueryError,
    ConfigError,
    InvalidSourceUri,
    GraphError,
    UnknownError,
}

/// The `error` object inside the JSON error envelope.
#[derive(Debug, Clone, Serialize)]
pub struct JsonErrorDetail {
    pub kind: ErrorKind,
    pub message: String,
}

/// Top-level JSON error envelope emitted when `--format json` is active
/// and a fatal error occurs.
///
/// ```json
/// {
///   "status": "error",
///   "error": { "kind": "IoError", "message": "..." },
///   "diagnostics": [ ... ]
/// }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct JsonErrorOutput {
    pub status: String,
    pub error: JsonErrorDetail,
    pub diagnostics: Vec<crate::error::Diagnostic>,
}

/// Build a structured JSON error string from an [`anyhow::Error`].
///
/// Attempts to downcast to [`crate::error::Error`] to determine the
/// [`ErrorKind`] and extract diagnostics.  Falls back to `UnknownError`
/// for errors that do not originate from the crate's error type.
pub fn format_json_error(err: &anyhow::Error) -> String {
    use crate::error::{Diagnostic, Error, Severity};
    use std::path::PathBuf;

    let (kind, message, diagnostics) = match err.downcast_ref::<Error>() {
        Some(Error::Io(io_err)) => {
            let msg = format!("{io_err}");
            let diag = Diagnostic {
                path: PathBuf::from("<unknown>"),
                severity: Severity::Error,
                message: msg.clone(),
                source_range: None,
            };
            (ErrorKind::IoError, msg, vec![diag])
        }
        Some(Error::Parse { path, message }) => {
            let diag = Diagnostic {
                path: path.clone(),
                severity: Severity::Error,
                message: message.clone(),
                source_range: None,
            };
            (ErrorKind::ParseError, err.to_string(), vec![diag])
        }
        Some(Error::Query { language, message }) => {
            let msg = format!("query error ({language}): {message}");
            (ErrorKind::QueryError, msg, vec![])
        }
        Some(Error::Config(msg)) => (ErrorKind::ConfigError, msg.clone(), vec![]),
        Some(Error::InvalidSourceUri { uri, message }) => {
            let msg = format!("invalid source URI '{uri}': {message}");
            (ErrorKind::InvalidSourceUri, msg, vec![])
        }
        Some(Error::Graph(msg)) => (ErrorKind::GraphError, msg.clone(), vec![]),
        None => {
            if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
                let msg = format!("{io_err}");
                let path = if let Some(stripped) = msg.strip_prefix("path does not exist: ") {
                    PathBuf::from(stripped.trim())
                } else {
                    PathBuf::from("<unknown>")
                };
                let diag = Diagnostic {
                    path,
                    severity: Severity::Error,
                    message: msg.clone(),
                    source_range: None,
                };
                (ErrorKind::IoError, msg, vec![diag])
            } else {
                (ErrorKind::UnknownError, err.to_string(), vec![])
            }
        }
    };

    let output = JsonErrorOutput {
        status: "error".to_string(),
        error: JsonErrorDetail { kind, message },
        diagnostics,
    };

    // Serialization of our own simple structs should never fail, but if it
    // does, fall back to a hand-written JSON string.
    serde_json::to_string_pretty(&output).unwrap_or_else(|e| {
        format!(
            r#"{{"status":"error","error":{{"kind":"UnknownError","message":"failed to serialize error: {e}"}},"diagnostics":[]}}"#
        )
    })
}
