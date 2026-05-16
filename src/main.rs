use clap::Parser;
use meta_ast::interface::args::Cli;
use meta_ast::model::SnapshotId;

fn main() -> anyhow::Result<()> {
    meta_ast::language::validate_queries();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli {
        Cli::Inspect(args) => {
            let files = meta_ast::input::discover_files(&args.path, None)?;

            let result = meta_ast::extractor::extract(&files);

            let symbols: Vec<_> = result
                .files
                .iter()
                .flat_map(|f| f.symbols.iter().cloned())
                .collect();

            for file in &result.files {
                for diag in &file.diagnostics {
                    tracing::warn!(
                        path = %diag.path.display(),
                        severity = ?diag.severity,
                        "{}", diag.message
                    );
                }
            }

            let content = meta_ast::output::inspect::serialize_inspect(&symbols, &args.format)?;

            match args.output {
                Some(path) => std::fs::write(&path, &content)?,
                None => println!("{content}"),
            }

            Ok(())
        }

        Cli::Graph(args) => {
            let snapshot_id = SnapshotId(1);
            let (analysis, diags) = meta_ast::pipeline::analyze_graph(&args.path, snapshot_id)?;

            for diag in &diags {
                tracing::warn!(
                    path = %diag.path.display(),
                    severity = ?diag.severity,
                    "{}", diag.message
                );
            }

            if args.html {
                let html = meta_ast::output::dashboard::to_graph_html(
                    &analysis.graph,
                    &analysis.scc,
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
                    tracing::warn!(error = %e, "could not open browser");
                }
            } else {
                let content = meta_ast::output::graph::serialize_graph(
                    &analysis.graph,
                    &analysis.scc,
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
