//! Startup banner printed once per process invocation.
//!
//! The banner renders the project wordmark in ASCII art on `stderr`.
//! Printing to `stderr` (not `stdout`) keeps piped machine output clean:
//! `meta-ast inspect ./p | jq .` stays parseable.
//!

pub const BANNER: &str = r#"
                   __                         __
   ____ ___  ___  / /_____ _      ____ ______/ /_
  / __ `__ \/ _ \/ __/ __ `/_____/ __ `/ ___/ __/
 / / / / / /  __/ /_/ /_/ /_____/ /_/ (__  ) /_
/_/ /_/ /_/\___/\__/\__,_/      \__,_/____/\__/

"#;

/// Print the startup banner to `stderr`.
///
/// The banner is printed on every invocation before command dispatch.
/// It is followed by a short tagline line, so a bare `meta-ast`
/// invocation still shows the project identity without requiring
/// `--help`.
pub fn print_banner() {
    eprintln!("{BANNER}");
    eprintln!(
        "Polyglot static analyzer - symbol extraction, cross-language dependency graphs, SCC analysis"
    );
}

#[cfg(test)]
mod tests {
    use super::BANNER;

    #[test]
    fn banner_is_ascii_only() {
        assert!(BANNER.is_ascii());
    }

    #[test]
    fn banner_renders_wordmark_glyphs() {
        assert!(
            BANNER.contains("/__"),
            "banner must contain figlet glyph strokes"
        );
        assert!(BANNER.contains('_'));
    }

    #[test]
    fn banner_has_stable_width() {
        let max_width = BANNER.lines().map(|l| l.len()).max().unwrap_or(0);
        assert_eq!(max_width, 49, "banner width must stay at 49 columns");
    }
}
