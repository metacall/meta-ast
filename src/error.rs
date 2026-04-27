use std::path::PathBuf;

use crate::model::SourceRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub severity: Severity,
    pub message: String,
    pub source_range: Option<SourceRange>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("query error ({language}): {message}")]
    Query {
        language: crate::language::LangId,
        message: String,
    },

    #[error("config: {0}")]
    Config(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LangId;
    use std::path::PathBuf;

    #[test]
    fn io_error_from_std_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn parse_error_display_format() {
        let err = Error::Parse {
            path: PathBuf::from("foo.bar"),
            message: "bad syntax".into(),
        };
        let displayed = err.to_string();
        assert!(displayed.contains("foo.bar"), "{displayed}");
        assert!(displayed.contains("bad syntax"), "{displayed}");
        assert!(displayed.starts_with("parse error in"), "{displayed}");
    }

    #[test]
    fn query_error_display_format() {
        let err = Error::Query {
            language: LangId::Rust,
            message: "no match".into(),
        };
        let displayed = err.to_string();
        assert!(displayed.contains("rust"), "{displayed}");
        assert!(displayed.contains("no match"), "{displayed}");
        assert!(displayed.starts_with("query error"), "{displayed}");
    }

    #[test]
    fn config_error_display() {
        let err = Error::Config("bad setting".into());
        let displayed = err.to_string();
        assert!(displayed.starts_with("config:"), "{displayed}");
        assert!(displayed.contains("bad setting"), "{displayed}");
    }

    #[test]
    fn diagnostic_construction() {
        let sr = SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: crate::model::LineColumn { line: 1, column: 0 },
            end: crate::model::LineColumn {
                line: 1,
                column: 10,
            },
        };
        let d = Diagnostic {
            path: PathBuf::from("test.rs"),
            severity: Severity::Error,
            message: "undefined variable".into(),
            source_range: Some(sr.clone()),
        };
        assert_eq!(d.path, PathBuf::from("test.rs"));
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "undefined variable");
        assert!(d.source_range.is_some());
        let range = d.source_range.unwrap();
        assert_eq!(range.byte_start, 0);
        assert_eq!(range.byte_end, 10);
    }

    #[test]
    fn severity_variants() {
        assert_ne!(Severity::Warning, Severity::Error);
        let v = [Severity::Warning, Severity::Error];
        assert_eq!(v.len(), 2);
    }
}
