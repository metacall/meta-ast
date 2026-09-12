use std::process::ExitCode;

use clap::Parser;
use meta_ast::interface::args::Cli;
use meta_ast::interface::report::report_diagnostics;
use meta_ast::model::SnapshotId;

#[cfg(feature = "watch")]
use meta_ast::watch::{WatchConfig, run_watch};

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
        Cli::Inspect(args) => {
            let languages = args.language.map(|l| [l]);
            let files = meta_ast::input::discover_files(
                &args.path,
                languages.as_ref().map(|a| a.as_slice()),
            )?;

            let result = meta_ast::extractor::extract_with_options(
                &files,
                &meta_ast::extractor::ExtractOptions {
                    skip_imports_and_refs: true,
                },
            );

            let mut diagnostics = Vec::new();
            let mut symbols = Vec::new();
            for file in result.files {
                diagnostics.extend(file.diagnostics.iter().cloned());
                symbols.extend(file.symbols);
            }

            report_diagnostics(&diagnostics, args.fail_on)?;

            let config = meta_ast::output::emitter::EmitConfig {
                output: args.output,
                format: args.format,
                html: false,
                open_browser: false,
            };

            meta_ast::output::emitter::emit_inspect(&mut symbols, &config)?;

            Ok(ExitCode::SUCCESS)
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
                    languages: args.language.map(|l| vec![l]),
                };

                let output = args.output.clone();
                let html = args.html;
                let format = args.format;
                let fail_on = args.fail_on;

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

                    let _ = fail_on;

                    Ok(())
                })
                .map(|()| ExitCode::SUCCESS);
            }

            let snapshot_id = SnapshotId::from(std::num::NonZeroU32::MIN);
            let languages = args.language.map(|l| [l]);
            let (analysis, diags) = meta_ast::pipeline::analyze_graph(
                &args.path,
                snapshot_id,
                languages.as_ref().map(|a| a.as_slice()),
            )?;

            report_diagnostics(&diags, args.fail_on)?;

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

                // The datagraph has its own path. Reusing -o would make the two
                // exports overwrite each other, and deriving only the file name
                // would drop the graph output's directory.
                let output_path = match (&args.datagraph_output, &args.output) {
                    (Some(explicit), _) => explicit.clone(),
                    (None, Some(graph_output)) => {
                        let stem = graph_output
                            .file_stem()
                            .map(|stem| stem.to_string_lossy().to_string())
                            .unwrap_or_else(|| "datagraph".to_string());
                        graph_output.with_file_name(format!("{stem}.datagraph.json"))
                    }
                    (None, None) => std::path::PathBuf::from("datagraph.json"),
                };
                tracing::info!(path = %output_path.display(), "Writing datagraph export");
                let sink = meta_ast::sink::JsonSink::new(Some(output_path));
                sink.emit(&export)?;
            }

            Ok(ExitCode::SUCCESS)
        }

        #[cfg(feature = "metacall-deploy")]
        Cli::Deploy(args) => {
            let config = meta_ast::deploy::DeployConfig {
                root: args.path,
                out: args.out,
                format: args.format,
                check: args.check,
                max_pod_size: args.max_pod_size,
            };
            meta_ast::deploy::run_deploy(config)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
