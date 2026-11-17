use std::str::FromStr;

use clap::{Parser, Subcommand};

/// Text printed at the bottom of `meta-ast --help`.
const AFTER_HELP: &str = "\
Exit status:
  0  success (diagnostics below --fail-on do not change it)
  1  analysis or I/O failure, including a tripped --fail-on policy
  2  usage or configuration error

Logging:
  RUST_LOG wins when it is set. Otherwise -v/-vv/-vvv select info/debug/
  trace, -q silences logging entirely (analysis diagnostics still print),
  and --no-banner skips the startup banner.
";

/// Polyglot static analyzer that builds symbol surfaces and cross-file dependency graphs.
#[derive(Parser)]
#[command(
    name = "meta-ast",
    version,
    about = "Polyglot static analyzer",
    arg_required_else_help = true,
    after_help = AFTER_HELP
)]
pub struct Cli {
    /// Flags shared by every subcommand, accepted before or after it
    #[command(flatten)]
    pub globals: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

/// The analysis commands.
#[derive(Subcommand)]
pub enum Command {
    /// Inspect a project directory/file and extract all symbol definitions
    ///
    /// Examples:
    ///   meta-ast inspect ./my-project
    ///   meta-ast inspect ./my-project/main.py -f yaml -o symbols.yaml
    ///   meta-ast inspect ./my-project --language python
    Inspect(InspectArgs),

    /// Build a cross-file dependency graph and analyze Strongly Connected Components (SCCs)
    ///
    /// Examples:
    ///   meta-ast graph ./my-project
    ///   meta-ast graph ./my-project --html -o project_graph.html
    ///   meta-ast graph ./my-project -f yaml -o graph.yaml
    Graph(GraphArgs),

    /// Scan cross-language call sites and generate MetaCall deployment manifests
    ///
    /// Examples:
    ///   meta-ast deploy ./my-project --out ./deploy-dir
    ///   meta-ast deploy ./my-project --check
    #[cfg(feature = "metacall-deploy")]
    Deploy(DeployArgs),
}

/// Flags available on every subcommand, anywhere on the command line.
#[derive(Debug, Parser)]
pub struct GlobalArgs {
    /// Raise log verbosity: info (-v), debug (-vv), trace (-vvv)
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Silence tracing output; analysis diagnostics still print
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Skip the startup banner
    #[arg(long, global = true)]
    pub no_banner: bool,
}

impl GlobalArgs {
    /// The tracing filter the flags select when `RUST_LOG` is unset.
    ///
    /// A quiet run silences tracing completely; analysis diagnostics are
    /// rendered separately and are never affected.
    pub fn log_level(&self) -> &'static str {
        if self.quiet {
            return "off";
        }
        match self.verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    }
}

fn parse_format(s: &str) -> Result<crate::output::OutputFormat, String> {
    match s.to_lowercase().as_str() {
        "json" => Ok(crate::output::OutputFormat::Json),
        "yaml" | "yml" => Ok(crate::output::OutputFormat::Yaml),
        _ => Err(format!("invalid format '{s}': expected 'json' or 'yaml'")),
    }
}

fn parse_language(s: &str) -> Result<crate::language::LangId, String> {
    let normalized = s.to_lowercase();
    crate::language::LangId::from_str(&normalized).map_err(|_| {
        let all = crate::language::LangId::all();
        let names: Vec<_> = all.iter().map(|l| l.as_ref()).collect();
        format!(
            "invalid language '{s}': expected one of {}",
            names.join(", ")
        )
    })
}

#[derive(Parser)]
pub struct InspectArgs {
    /// Root directory or source file to inspect
    #[arg(value_name = "PATH")]
    pub path: std::path::PathBuf,

    /// Output file path (prints to stdout if omitted)
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<std::path::PathBuf>,

    /// Only analyze files detected as this language
    #[arg(short, long, value_name = "LANG", value_parser = parse_language)]
    pub language: Option<crate::language::LangId>,

    /// Output format for the extracted symbols
    #[arg(
        short = 'f',
        long,
        value_name = "FMT",
        default_value = "json",
        value_parser = parse_format
    )]
    pub format: crate::output::OutputFormat,

    /// Diagnostic severity that makes the run exit with status 1
    #[arg(long, value_enum, default_value_t = crate::interface::report::FailOn::Error)]
    pub fail_on: crate::interface::report::FailOn,

    /// How to render analysis diagnostics on stderr
    #[arg(long, value_enum, default_value_t = crate::interface::report::DiagnosticFormat::Text)]
    pub diagnostics: crate::interface::report::DiagnosticFormat,
}

#[derive(Parser)]
pub struct GraphArgs {
    /// Root directory to analyze
    #[arg(value_name = "PATH")]
    pub path: std::path::PathBuf,

    /// Output file path (defaults to stdout, or `<path>.html` with --html)
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<std::path::PathBuf>,

    /// Only analyze files detected as this language
    #[arg(short, long, value_name = "LANG", value_parser = parse_language)]
    pub language: Option<crate::language::LangId>,

    /// Output serialization format for the graph structure
    #[arg(
        short = 'f',
        long,
        value_name = "FMT",
        default_value = "json",
        value_parser = parse_format
    )]
    pub format: crate::output::OutputFormat,

    /// Diagnostic severity that makes the run exit with status 1
    #[arg(long, value_enum, default_value_t = crate::interface::report::FailOn::Error)]
    pub fail_on: crate::interface::report::FailOn,

    /// How to render analysis diagnostics on stderr
    #[arg(long, value_enum, default_value_t = crate::interface::report::DiagnosticFormat::Text)]
    pub diagnostics: crate::interface::report::DiagnosticFormat,

    /// Generate an interactive HTML dashboard with graph visualization
    #[arg(long)]
    pub html: bool,

    /// Open the written dashboard in the default browser (needs --html)
    #[arg(long, requires = "html")]
    pub open: bool,

    /// Also emit a portable datagraph export (requires --features dataflow)
    #[cfg(feature = "dataflow")]
    #[arg(long)]
    pub datagraph: bool,

    /// Output file for the datagraph export (defaults to the graph output name plus `.datagraph.json`)
    #[cfg(feature = "dataflow")]
    #[arg(long, value_name = "FILE")]
    pub datagraph_output: Option<std::path::PathBuf>,

    /// Enter watch mode: monitor the project and re-analyze on file changes
    #[cfg(feature = "watch")]
    #[arg(long)]
    pub watch: bool,

    /// Debounce duration in milliseconds for watch mode (default: 200)
    #[cfg(feature = "watch")]
    #[arg(long, value_name = "MS", default_value = "200")]
    pub watch_debounce: u64,
}

#[cfg(feature = "metacall-deploy")]
#[derive(Parser)]
pub struct DeployArgs {
    /// Root directory of the project to analyze
    #[arg(value_name = "PATH")]
    pub path: std::path::PathBuf,

    /// Output format for generated manifests
    #[arg(
        short = 'f',
        long,
        value_name = "FMT",
        default_value = "json",
        value_parser = parse_format
    )]
    pub format: crate::output::OutputFormat,

    /// Check mode: verify cut fairness and diff the generated manifests
    /// against the committed metacall.pods and metacall.mesh documents
    #[arg(long)]
    pub check: bool,

    /// Output directory for generated manifests and mesh annotation
    #[arg(short, long, value_name = "DIR", default_value = ".")]
    pub out: std::path::PathBuf,

    /// Maximum number of files in a single pod before rebalancing is triggered
    #[arg(
        long,
        value_name = "COUNT",
        default_value_t = crate::deploy::cut::DEFAULT_MAX_POD_SIZE
    )]
    pub max_pod_size: usize,

    /// Diagnostic severity that makes the run exit with status 1
    #[arg(long, value_enum, default_value_t = crate::interface::report::FailOn::Error)]
    pub fail_on: crate::interface::report::FailOn,

    /// How to render analysis diagnostics on stderr
    #[arg(long, value_enum, default_value_t = crate::interface::report::DiagnosticFormat::Text)]
    pub diagnostics: crate::interface::report::DiagnosticFormat,
}

/// The CLI owns the mapping from parsed arguments to the emitter configuration.
impl From<&InspectArgs> for crate::output::emitter::EmitConfig {
    fn from(args: &InspectArgs) -> Self {
        Self {
            output: args.output.clone(),
            format: args.format,
            html: false,
            open_browser: false,
        }
    }
}

impl From<&GraphArgs> for crate::output::emitter::EmitConfig {
    fn from(args: &GraphArgs) -> Self {
        Self {
            output: args.output.clone(),
            format: args.format,
            html: args.html,
            open_browser: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::report::DiagnosticFormat;

    #[test]
    fn parse_language_valid_lowercase() {
        assert_eq!(
            parse_language("python"),
            Ok(crate::language::LangId::Python)
        );
        assert_eq!(
            parse_language("typescript"),
            Ok(crate::language::LangId::TypeScript)
        );
    }

    #[test]
    fn parse_language_case_insensitive() {
        assert_eq!(
            parse_language("Python"),
            Ok(crate::language::LangId::Python)
        );
        assert_eq!(
            parse_language("TYPESCRIPT"),
            Ok(crate::language::LangId::TypeScript)
        );
    }

    #[test]
    fn parse_language_invalid_returns_all_names() {
        let err = parse_language("pytho").unwrap_err();
        for id in crate::language::LangId::all() {
            let name: &str = id.as_ref();
            assert!(
                err.contains(name),
                "error {err:?} should list the valid name {name:?}"
            );
        }
    }

    #[test]
    fn parse_language_empty_returns_err() {
        assert!(parse_language("").is_err());
    }

    #[test]
    fn parse_format_rejects_unknown_values() {
        let err = parse_format("toml").unwrap_err();
        assert!(err.contains("json") && err.contains("yaml"), "{err}");
    }

    #[test]
    fn opening_the_dashboard_is_an_opt_in() {
        let parsed = Cli::try_parse_from(["meta-ast", "graph", "demo", "--html", "--open"]);
        assert!(
            parsed.is_ok(),
            "graph --html --open parses: {:?}",
            parsed.as_ref().err()
        );
        assert!(
            matches!(&parsed, Ok(cli) if matches!(cli.command, Command::Graph(ref args) if args.open)),
            "the flag reaches the parsed arguments"
        );
        assert!(
            Cli::try_parse_from(["meta-ast", "graph", "demo", "--open"]).is_err(),
            "--open without --html is a usage error"
        );
    }

    #[test]
    fn global_flags_parse_before_the_subcommand() {
        let parsed =
            Cli::try_parse_from(["meta-ast", "--no-banner", "-v", "inspect", "."]).unwrap();
        assert!(parsed.globals.no_banner);
        assert_eq!(parsed.globals.verbose, 1);
        assert!(matches!(parsed.command, Command::Inspect(_)));
    }

    #[test]
    fn global_flags_parse_after_the_subcommand() {
        let parsed = Cli::try_parse_from(["meta-ast", "inspect", ".", "-vv", "-q"]).unwrap();
        assert_eq!(parsed.globals.verbose, 2);
        assert!(parsed.globals.quiet);
    }

    #[test]
    fn verbosity_maps_to_log_levels() {
        let level = |verbose: u8, quiet: bool| {
            GlobalArgs {
                verbose,
                quiet,
                no_banner: false,
            }
            .log_level()
        };
        assert_eq!(level(0, false), "warn");
        assert_eq!(level(1, false), "info");
        assert_eq!(level(2, false), "debug");
        assert_eq!(level(5, false), "trace");
        assert_eq!(level(3, true), "off", "quiet wins over verbosity");
    }

    #[test]
    fn diagnostics_default_to_text_and_accept_json() {
        let parsed = Cli::try_parse_from(["meta-ast", "inspect", "."]).unwrap();
        assert!(
            matches!(parsed.command, Command::Inspect(ref args) if args.diagnostics == DiagnosticFormat::Text),
            "the default diagnostic format is text"
        );

        let parsed = Cli::try_parse_from(["meta-ast", "inspect", ".", "--diagnostics", "json"]);
        assert!(
            parsed.is_ok(),
            "--diagnostics json parses: {:?}",
            parsed.as_ref().err()
        );
    }
}
