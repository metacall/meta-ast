mod interface;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = interface::args::Cli::parse();

    match cli {
        interface::args::Cli::Inspect(args) => {
            let files = meta_ast::input::discover_files(&args.path, None)?;

            let result = meta_ast::extractor::extract(&files);

            for diag in &result.diagnostics {
                eprintln!(
                    "[{:?}] {}: {}",
                    diag.severity,
                    diag.path.display(),
                    diag.message
                );
            }

            let json = meta_ast::output::inspect::to_inspect_json(&result.symbols)?;

            match args.output {
                Some(path) => std::fs::write(&path, &json)?,
                None => println!("{json}"),
            }

            Ok(())
        }
    }
}
