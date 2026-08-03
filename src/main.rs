use clap::Parser;
use meta_ast::interface::args::Cli;
use meta_ast::model::SnapshotId;

#[cfg(feature = "watch")]
use meta_ast::watch::{WatchConfig, run_watch};

fn main() -> anyhow::Result<()> {
    meta_ast::interface::banner::print_banner();

    meta_ast::language::validate_queries();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli {
        Cli::Inspect(args) => {
            let files = meta_ast::input::discover_files(&args.path, None)?;

            let result = meta_ast::extractor::extract_with_options(
                &files,
                &meta_ast::extractor::ExtractOptions {
                    skip_imports_and_refs: true,
                },
            );

            let mut symbols: Vec<_> = result
                .files
                .into_iter()
                .flat_map(|f| {
                    for diag in &f.diagnostics {
                        tracing::warn!(
                            path = %diag.path.display(),
                            severity = ?diag.severity,
                            "{}", diag.message
                        );
                    }
                    f.symbols
                })
                .collect();

            let config = meta_ast::output::emitter::EmitConfig {
                output: args.output,
                format: args.format,
                html: false,
                open_browser: false,
            };

            meta_ast::output::emitter::emit_inspect(&mut symbols, &config)?;

            Ok(())
        }

        Cli::Graph(args) => {
            #[cfg(feature = "watch")]
            if args.watch {
                let watch_config = WatchConfig {
                    debounce: std::time::Duration::from_millis(args.watch_debounce),
                    format: args.format,
                    output: args.output.clone(),
                    html: args.html,
                    open_browser: false,
                };

                let output = args.output.clone();
                let html = args.html;
                let format = args.format;

                return run_watch(args.path, watch_config, move |analysis, change_set| {
                    let emit_config = meta_ast::output::emitter::EmitConfig {
                        output: output.clone(),
                        format,
                        html,
                        open_browser: false,
                    };
                    meta_ast::output::emitter::emit_graph(analysis, &emit_config)?;

                    let file_count = analysis.graph.file_count();
                    let sym_count = analysis.graph.symbol_count();
                    let scc_count = analysis.scc.components.len();
                    let cyclic = analysis
                        .scc
                        .components
                        .iter()
                        .filter(|c| c.is_cyclic)
                        .count();
                    tracing::info!(
                        snapshot = analysis.snapshot_id.to_raw(),
                        files = file_count,
                        symbols = sym_count,
                        edges = analysis.graph.edge_count(),
                        sccs = scc_count,
                        cyclic = cyclic,
                        added = change_set.files_added,
                        removed = change_set.files_removed,
                        modified = change_set.files_modified,
                        unchanged = change_set.files_unchanged,
                        "Re-analyzed",
                    );

                    Ok(())
                });
            }

            let snapshot_id = SnapshotId::new(1).unwrap();
            let (analysis, diags) = meta_ast::pipeline::analyze_graph(&args.path, snapshot_id)?;

            for diag in &diags {
                tracing::warn!(
                    path = %diag.path.display(),
                    severity = ?diag.severity,
                    "{}", diag.message
                );
            }

            let default_html_output = if args.html && args.output.is_none() {
                let name = args
                    .path
                    .file_stem()
                    .map(|s: &std::ffi::OsStr| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "project".to_string());
                Some(std::path::PathBuf::from(format!("{}.metast", name)))
            } else {
                args.output.clone()
            };

            let config = meta_ast::output::emitter::EmitConfig {
                output: default_html_output,
                format: args.format,
                html: args.html,
                open_browser: true,
            };

            meta_ast::output::emitter::emit_graph(&analysis, &config)?;

            #[cfg(feature = "dataflow")]
            if args.datagraph {
                use meta_ast::sink::GraphSink;

                let export = meta_ast::output::graph::GraphOutput::from_graph(
                    &analysis.graph,
                    Some(&analysis.scc),
                    analysis.snapshot_id.to_raw() as u64,
                );
                tracing::info!(
                    schema_version = export.schema_version,
                    node_count = export.metadata.node_count,
                    "Exporting datagraph"
                );

                let output_path = args
                    .output
                    .unwrap_or_else(|| std::path::PathBuf::from("datagraph.json"));
                let sink = meta_ast::sink::JsonSink::new(Some(output_path));
                sink.emit(&export)?;
            }

            Ok(())
        }

        #[cfg(feature = "metacall-deploy")]
        Cli::Deploy(args) => {
            let config = meta_ast::deploy::DeployConfig {
                root: args.path,
                out: args.out,
                format: args.format,
                check: args.check,
            };
            meta_ast::deploy::run_deploy(config)
        }
    }
}
