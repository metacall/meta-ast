use std::process::ExitCode;

use clap::Parser;
use meta_ast::interface::args::{Cli, Command, GlobalArgs};
use meta_ast::interface::commands;

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

    install_subscriber(&cli.globals);

    if !cli.globals.no_banner {
        meta_ast::interface::banner::print_banner();
    }
    meta_ast::language::validate_queries();

    match cli.command {
        Command::Inspect(args) => commands::inspect(args),
        Command::Graph(args) => commands::graph(args),
        #[cfg(feature = "metacall-deploy")]
        Command::Deploy(args) => commands::deploy(args),
    }
}

/// `RUST_LOG` wins when it is set; otherwise the global flags pick the level.
fn install_subscriber(globals: &GlobalArgs) {
    use tracing_subscriber::EnvFilter;

    let filter = match std::env::var("RUST_LOG") {
        Ok(_) => EnvFilter::from_default_env(),
        Err(_) => EnvFilter::new(globals.log_level()),
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
