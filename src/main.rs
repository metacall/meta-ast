mod interface;

use clap::Parser;
use meta_ast::graph::{GraphBuilder, SccAnalysis};
use meta_ast::model::SnapshotId;

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

        interface::args::Cli::Graph(args) => {
            let files = meta_ast::input::discover_files(&args.path, None)?;

            // Create snapshot ID for this analysis
            let snapshot_id = SnapshotId(1);

            // Build the dependency graph
            let mut builder = GraphBuilder::new(snapshot_id);

            // Add all files as nodes
            for (path, lang) in &files {
                builder.add_file(path.clone(), *lang);
            }

            // Extract symbols and add them to the graph
            let result = meta_ast::extractor::extract(&files);

            for diag in &result.diagnostics {
                eprintln!(
                    "[{:?}] {}: {}",
                    diag.severity,
                    diag.path.display(),
                    diag.message
                );
            }

            // Add symbols to the graph (ownership edges created automatically)
            for symbol in &result.symbols {
                builder.add_symbol(symbol);
            }

            // Build the graph
            let graph = builder.build();

            // Run SCC analysis on the dependency subgraph
            let scc_analysis = SccAnalysis::analyze(&graph.graph);

            // Output results
            let json = meta_ast::output::graph::to_graph_json(
                &graph,
                &scc_analysis,
                snapshot_id.0 as u64,
            )?;

            match args.output {
                Some(path) => std::fs::write(&path, &json)?,
                None => println!("{json}"),
            }

            Ok(())
        }
    }
}
