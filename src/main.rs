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

            let config = meta_ast::output::emitter::EmitConfig {
                output: args.output,
                format: args.format,
                html: false,
                open_browser: false,
                self_contained: false,
            };

            meta_ast::output::emitter::emit_inspect(&symbols, &config)?;

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

            let default_html_output = if args.html && args.output.is_none() {
                let name = args
                    .path
                    .file_stem()
                    .map(|s: &std::ffi::OsStr| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "project".to_string());
                Some(std::path::PathBuf::from(format!("{}.metast", name)))
            } else {
                args.output
            };

            let config = meta_ast::output::emitter::EmitConfig {
                output: default_html_output,
                format: args.format,
                html: args.html,
                open_browser: true,
                self_contained: args.self_contained,
            };

            meta_ast::output::emitter::emit_graph(&analysis, &config)?;

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
