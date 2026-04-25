use clap::Parser;
use meta_ast::graph::{GraphBuilder, SccAnalysis};
use meta_ast::interface::args::Cli;
use meta_ast::model::SnapshotId;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli {
        Cli::Inspect(args) => {
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

            let content =
                meta_ast::output::inspect::serialize_inspect(&result.symbols, &args.format)?;

            match args.output {
                Some(path) => std::fs::write(&path, &content)?,
                None => println!("{content}"),
            }

            Ok(())
        }

        Cli::Graph(args) => {
            let files = meta_ast::input::discover_files(&args.path, None)?;

            let snapshot_id = SnapshotId(1);

            let mut builder = GraphBuilder::new(snapshot_id);

            for (path, lang) in &files {
                builder.add_file(path.clone(), *lang);
            }

            let result = meta_ast::extractor::extract(&files);

            for diag in &result.diagnostics {
                eprintln!(
                    "[{:?}] {}: {}",
                    diag.severity,
                    diag.path.display(),
                    diag.message
                );
            }

            for symbol in &result.symbols {
                builder.add_symbol(symbol);
            }

            let graph = builder.build();
            let scc_analysis = SccAnalysis::analyze(&graph.graph);

            if args.html {
                let html = meta_ast::output::dashboard::to_graph_html(
                    &graph,
                    &scc_analysis,
                    snapshot_id.0 as u64,
                    args.self_contained,
                )?;
                let path = args.output.unwrap_or_else(|| {
                    let name = args
                        .path
                        .file_stem()
                        .map(|s: &std::ffi::OsStr| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "project".to_string());
                    std::path::PathBuf::from(format!("{}.metast", name))
                });
                let path_str = path.to_string_lossy().to_string();
                std::fs::write(&path, &html)?;
                if let Err(e) = webbrowser::open(&path_str) {
                    eprintln!("Warning: could not open browser: {e}");
                }
            } else {
                let content = meta_ast::output::graph::serialize_graph(
                    &graph,
                    &scc_analysis,
                    snapshot_id.0 as u64,
                    &args.format,
                )?;
                match args.output {
                    Some(path) => std::fs::write(&path, &content)?,
                    None => println!("{content}"),
                }
            }

            Ok(())
        }
    }
}
