use clap::Parser;

/// Polyglot static analyzer that builds symbol surfaces and cross-file dependency graphs.
#[derive(Parser)]
#[command(name = "meta-ast", version, about = "Polyglot static analyzer")]
pub enum Cli {
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

fn parse_format(s: &str) -> Result<crate::output::OutputFormat, String> {
    match s.to_lowercase().as_str() {
        "json" => Ok(crate::output::OutputFormat::Json),
        "yaml" | "yml" => Ok(crate::output::OutputFormat::Yaml),
        _ => Err(format!("invalid format '{s}': expected 'json' or 'yaml'")),
    }
}

#[derive(Parser)]
pub struct InspectArgs {
    /// Root directory or source file to inspect
    pub path: std::path::PathBuf,

    /// Output file path (prints to stdout if omitted)
    #[arg(short, long)]
    pub output: Option<std::path::PathBuf>,

    /// Override automatic language detection and force a specific language
    #[arg(short, long)]
    pub language: Option<String>,

    /// Output format for the extracted symbols
    #[arg(short = 'f', long, default_value = "json", value_parser = parse_format)]
    pub format: crate::output::OutputFormat,
}

#[derive(Parser)]
pub struct GraphArgs {
    /// Root directory to analyze
    pub path: std::path::PathBuf,

    /// Output file path (prints to stdout if omitted)
    #[arg(short, long)]
    pub output: Option<std::path::PathBuf>,

    /// Override automatic language detection and force a specific language
    #[arg(short, long)]
    pub language: Option<String>,

    /// Output serialization format for the graph structure
    #[arg(short = 'f', long, default_value = "json", value_parser = parse_format)]
    pub format: crate::output::OutputFormat,

    /// Generate an interactive HTML dashboard with graph visualization
    #[arg(long)]
    pub html: bool,
}

#[cfg(feature = "metacall-deploy")]
#[derive(Parser)]
pub struct DeployArgs {
    /// Root directory of the project to analyze
    pub path: std::path::PathBuf,

    /// Output format for generated manifests
    #[arg(short = 'f', long, default_value = "json", value_parser = parse_format)]
    pub format: crate::output::OutputFormat,

    /// Check mode: diff generated manifests against existing metacall.json
    #[arg(long)]
    pub check: bool,

    /// Output directory for generated manifests and mesh annotation
    #[arg(short, long, default_value = ".")]
    pub out: std::path::PathBuf,
}
