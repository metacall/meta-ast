//! Diagnostic reporting and the exit policy.
//!
//! An analysis run reports every diagnostic it collected and then decides the
//! process status from the requested policy. The reporting is separate from the
//! serialization, so `--fail-on` never changes the emitted document.
//!
//! Two rendering formats are available:
//!
//! * `text` (default): one `error: path:line: message` line per diagnostic,
//!   written straight to `stderr`, plus a summary when the policy is not
//!   tripped (a tripped policy already fails with a counted error).
//! * `json`: a single machine-readable document with the counts and every
//!   diagnostic, so tooling can consume the analysis failures.

use crate::error::{Diagnostic, Error, Severity};

/// How many diagnostics make a run fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum FailOn {
    /// Never fail on diagnostics.
    Never,
    /// Fail when the run reports an error diagnostic.
    #[default]
    Error,
    /// Fail when the run reports an error or a warning diagnostic.
    Warning,
}

impl FailOn {
    /// True when the collected diagnostics trip the policy.
    pub fn tripped(self, errors: usize, warnings: usize) -> bool {
        match self {
            FailOn::Never => false,
            FailOn::Error => errors > 0,
            FailOn::Warning => errors > 0 || warnings > 0,
        }
    }
}

/// How diagnostics are rendered on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
#[value(rename_all = "lower")]
pub enum DiagnosticFormat {
    /// One `error: path:line: message` line per diagnostic plus a summary.
    #[default]
    Text,
    /// A single JSON document with a summary and every diagnostic.
    Json,
}

/// One human-readable line: `error: path:line: message`.
///
/// The line is 1-indexed to match editor gutters; a diagnostic without a
/// source position omits the column entirely.
pub fn render_text(diagnostic: &Diagnostic) -> String {
    let label = severity_label(diagnostic.severity);
    match diagnostic.source_range.as_ref() {
        Some(range) => format!(
            "{label}: {}:{}: {}",
            diagnostic.path.display(),
            range.start.line + 1,
            diagnostic.message
        ),
        None => format!(
            "{label}: {}: {}",
            diagnostic.path.display(),
            diagnostic.message
        ),
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// The machine-readable report document (`--diagnostics json`).
#[derive(Debug, serde::Serialize)]
struct JsonReport<'a> {
    summary: JsonSummary,
    diagnostics: &'a [Diagnostic],
}

#[derive(Debug, serde::Serialize)]
struct JsonSummary {
    errors: usize,
    warnings: usize,
}

/// Log every diagnostic and apply the policy.
///
/// Text rendering, the default: watch mode and every pre-existing caller.
pub fn report_diagnostics(diagnostics: &[Diagnostic], fail_on: FailOn) -> Result<(), Error> {
    report_diagnostics_with(diagnostics, fail_on, DiagnosticFormat::Text)
}

/// Log every diagnostic in the requested format and apply the policy.
///
/// Returns [`Error::Diagnostics`] when the policy is tripped, so the caller can
/// map it to the analysis exit code.
pub fn report_diagnostics_with(
    diagnostics: &[Diagnostic],
    fail_on: FailOn,
    format: DiagnosticFormat,
) -> Result<(), Error> {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    let warnings = diagnostics.len() - errors;

    match format {
        DiagnosticFormat::Text => {
            for diagnostic in diagnostics {
                eprintln!("{}", render_text(diagnostic));
            }
            // A tripped policy fails with a counted message, so the summary
            // would repeat it verbatim.
            if (errors > 0 || warnings > 0) && !fail_on.tripped(errors, warnings) {
                eprintln!("{errors} error(s) and {warnings} warning(s) reported");
            }
        }
        DiagnosticFormat::Json => {
            let report = JsonReport {
                summary: JsonSummary { errors, warnings },
                diagnostics,
            };
            // Diagnostics are plain data with no non-string map keys, so
            // serialization is an invariant rather than a runtime failure.
            let json = serde_json::to_string(&report).expect("diagnostics serialize to JSON");
            eprintln!("{json}");
        }
    }

    if fail_on.tripped(errors, warnings) {
        return Err(Error::Diagnostics { errors, warnings });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LineColumn, SourceRange};
    use std::path::PathBuf;

    fn diagnostic(severity: Severity) -> Diagnostic {
        Diagnostic {
            path: PathBuf::from("a.py"),
            severity,
            message: "problem".into(),
            source_range: None,
        }
    }

    fn range_at(line: usize) -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: LineColumn { line, column: 0 },
            end: LineColumn { line, column: 10 },
        }
    }

    #[test]
    fn never_accepts_errors() {
        let diagnostics = [diagnostic(Severity::Error)];
        assert!(report_diagnostics(&diagnostics, FailOn::Never).is_ok());
    }

    #[test]
    fn error_policy_ignores_warnings() {
        let diagnostics = [diagnostic(Severity::Warning)];
        assert!(report_diagnostics(&diagnostics, FailOn::Error).is_ok());
    }

    #[test]
    fn error_policy_fails_on_an_error() {
        let diagnostics = [diagnostic(Severity::Warning), diagnostic(Severity::Error)];
        let error = report_diagnostics(&diagnostics, FailOn::Error).unwrap_err();
        assert!(matches!(
            error,
            Error::Diagnostics {
                errors: 1,
                warnings: 1
            }
        ));
    }

    #[test]
    fn warning_policy_fails_on_a_warning() {
        let diagnostics = [diagnostic(Severity::Warning)];
        assert!(report_diagnostics(&diagnostics, FailOn::Warning).is_err());
    }

    #[test]
    fn clean_run_never_fails() {
        assert!(report_diagnostics(&[], FailOn::Warning).is_ok());
    }

    #[test]
    fn text_rendering_carries_severity_path_line_and_message() {
        let mut diagnostic = diagnostic(Severity::Error);
        diagnostic.path = PathBuf::from("src/a.py");
        diagnostic.source_range = Some(range_at(11));
        assert_eq!(render_text(&diagnostic), "error: src/a.py:12: problem");
    }

    #[test]
    fn text_rendering_omits_the_line_without_a_range() {
        let mut diagnostic = diagnostic(Severity::Warning);
        diagnostic.path = PathBuf::from("src/a.py");
        assert_eq!(render_text(&diagnostic), "warning: src/a.py: problem");
    }

    #[test]
    fn json_report_carries_the_summary_and_entries() {
        let diagnostics = [diagnostic(Severity::Warning), diagnostic(Severity::Error)];
        let report = JsonReport {
            summary: JsonSummary {
                errors: 1,
                warnings: 1,
            },
            diagnostics: &diagnostics,
        };

        let json = serde_json::to_string(&report).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["summary"]["errors"], 1);
        assert_eq!(value["summary"]["warnings"], 1);
        assert_eq!(value["diagnostics"].as_array().unwrap().len(), 2);
        assert_eq!(value["diagnostics"][0]["path"], "a.py");
        assert_eq!(value["diagnostics"][1]["severity"], "Error");
    }

    #[test]
    fn json_format_never_changes_the_policy() {
        let diagnostics = [diagnostic(Severity::Error)];
        let text = report_diagnostics_with(&diagnostics, FailOn::Error, DiagnosticFormat::Text);
        let json = report_diagnostics_with(&diagnostics, FailOn::Error, DiagnosticFormat::Json);
        assert!(matches!(text, Err(Error::Diagnostics { errors: 1, .. })));
        assert!(matches!(json, Err(Error::Diagnostics { errors: 1, .. })));
    }
}
