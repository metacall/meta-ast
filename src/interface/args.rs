use clap::Parser;

#[derive(Parser)]
#[command(name = "meta-ast", version, about = "Polyglot static analyzer")]
pub enum Cli {
    Inspect(InspectArgs),
    Graph(GraphArgs),
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
    pub path: std::path::PathBuf,

    #[arg(short, long)]
    pub output: Option<std::path::PathBuf>,

    #[arg(short, long)]
    pub language: Option<String>,

    #[arg(short = 'f', long, default_value = "json", value_parser = parse_format)]
    pub format: crate::output::OutputFormat,
}

#[derive(Parser)]
pub struct GraphArgs {
    pub path: std::path::PathBuf,

    #[arg(short, long)]
    pub output: Option<std::path::PathBuf>,

    #[arg(short, long)]
    pub language: Option<String>,

    #[arg(short = 'f', long, default_value = "json", value_parser = parse_format)]
    pub format: crate::output::OutputFormat,

    /// Generate an interactive HTML dashboard with graph visualization
    #[arg(long)]
    pub html: bool,

    /// Embed Cytoscape.js directly in the HTML (no CDN dependency)
    #[arg(long)]
    pub self_contained: bool,
}
