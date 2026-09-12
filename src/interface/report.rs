//! Diagnostic reporting and the exit policy.
//!
//! An analysis run reports every diagnostic it collected and then decides the
//! process status from the requested policy. The reporting is separate from the
//! serialization, so `--fail-on` never changes the emitted document.

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

/// Log every diagnostic and apply the policy.
///
/// Returns [`Error::Diagnostics`] when the policy is tripped, so the caller can
/// map it to the analysis exit code.
pub fn report_diagnostics(diagnostics: &[Diagnostic], fail_on: FailOn) -> Result<(), Error> {
    let mut errors = 0usize;
    let mut warnings = 0usize;

    for diagnostic in diagnostics {
        let path = diagnostic.path.display().to_string();
        let range = diagnostic
            .source_range
            .as_ref()
            .map(|range| range.start.line + 1);
        match diagnostic.severity {
            Severity::Error => {
                errors += 1;
                tracing::error!(path = %path, line = ?range, "{}", diagnostic.message);
            }
            Severity::Warning => {
                warnings += 1;
                tracing::warn!(path = %path, line = ?range, "{}", diagnostic.message);
            }
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
    use std::path::PathBuf;

    fn diagnostic(severity: Severity) -> Diagnostic {
        Diagnostic {
            path: PathBuf::from("a.py"),
            severity,
            message: "problem".into(),
            source_range: None,
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
}
