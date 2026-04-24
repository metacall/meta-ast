use clap::Parser;

#[derive(Parser)]
#[command(name = "meta-ast", version, about = "Polyglot static analyzer")]
pub enum Cli {
    Inspect(InspectArgs),
    Graph(GraphArgs),
}

#[derive(Parser)]
pub struct InspectArgs {
    pub path: std::path::PathBuf,

    #[arg(short, long)]
    pub output: Option<std::path::PathBuf>,

    #[arg(short, long)]
    pub language: Option<String>,
}

#[derive(Parser)]
pub struct GraphArgs {
    pub path: std::path::PathBuf,

    #[arg(short, long)]
    pub output: Option<std::path::PathBuf>,

    #[arg(short, long)]
    pub language: Option<String>,
}
