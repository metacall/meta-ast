use std::process::ExitCode;

use clap::Parser;
use meta_ast::interface::{args::Cli, commands};

fn main() -> ExitCode {
    match run() {
        Ok(status) => status,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(exit_status(&error))
        }
    }
}

/// Usage and configuration problems exit with 2, every other failure with 1.
fn exit_status(error: &anyhow::Error) -> u8 {
    match error.downcast_ref::<meta_ast::Error>() {
        Some(meta_ast::Error::Config(_)) => 2,
        _ => 1,
    }
}

/// Parse first, so help and version stay clean, then install the subscriber
/// before anything can log or panic.
fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    meta_ast::interface::banner::print_banner();
    meta_ast::language::validate_queries();

    match cli {
        Cli::Inspect(args) => commands::inspect(args),
        Cli::Graph(args) => commands::graph(args),
        #[cfg(feature = "metacall-deploy")]
        Cli::Deploy(args) => commands::deploy(args),
    }
}
