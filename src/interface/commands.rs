//! Command implementations for the CLI.
//!
//! The entry point parses arguments and dispatches here. Each command owns its
//! configuration, its pipeline call, its diagnostic policy and its emission,
//! so the entry point never grows a second responsibility.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[cfg(feature = "metacall-deploy")]
use crate::interface::args::DeployArgs;
use crate::interface::args::{GraphArgs, InspectArgs};
use crate::interface::report::report_diagnostics;
use crate::model::SnapshotId;
use crate::output::emitter::{EmitConfig, emit_graph, emit_inspect};

/// Extract symbols and print or write the inspection document.
pub fn inspect(args: InspectArgs) -> anyhow::Result<ExitCode> {
    let languages = args.language.map(|language| [language]);
    let files = crate::input::discover_files(&args.path, languages.as_ref().map(|a| a.as_slice()))?;

    let result = crate::extractor::extract_with_options(
        &files,
        &crate::extractor::ExtractOptions {
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

    let config = EmitConfig::from(&args);
    emit_inspect(&mut symbols, &config)?;

    Ok(ExitCode::SUCCESS)
}

/// Build the dependency graph, report its diagnostics and emit it.
pub fn graph(args: GraphArgs) -> anyhow::Result<ExitCode> {
    #[cfg(feature = "watch")]
    if args.watch {
        return watch(&args);
    }

    let snapshot_id = SnapshotId::from(std::num::NonZeroU32::MIN);
    let languages = args.language.map(|language| [language]);
    let (analysis, diagnostics) = crate::pipeline::analyze_graph(
        &args.path,
        snapshot_id,
        languages.as_ref().map(|a| a.as_slice()),
    )?;

    report_diagnostics(&diagnostics, args.fail_on)?;

    let mut config = EmitConfig::from(&args);
    if config.html && config.output.is_none() {
        config.output = Some(default_dashboard_path(&args.path));
    }
    emit_graph(&analysis, &config)?;

    #[cfg(feature = "dataflow")]
    if args.datagraph {
        emit_datagraph(&analysis, &args)?;
    }

    Ok(ExitCode::SUCCESS)
}

/// Scan for cross-language call sites and write the deployment manifests.
#[cfg(feature = "metacall-deploy")]
pub fn deploy(args: DeployArgs) -> anyhow::Result<ExitCode> {
    let config = crate::deploy::DeployConfig {
        root: args.path,
        out: args.out,
        format: args.format,
        check: args.check,
        max_pod_size: args.max_pod_size,
    };

    let diagnostics = crate::deploy::run_deploy(config)?;
    report_diagnostics(&diagnostics, args.fail_on)?;

    Ok(ExitCode::SUCCESS)
}

/// Default dashboard path: the input name with a `.metast` suffix.
fn default_dashboard_path(path: &Path) -> PathBuf {
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    PathBuf::from(format!("{name}.metast"))
}

/// Watch the project and emit a fresh document on every rebuild.
#[cfg(feature = "watch")]
fn watch(args: &GraphArgs) -> anyhow::Result<ExitCode> {
    let config = crate::watch::WatchConfig {
        debounce: std::time::Duration::from_millis(args.watch_debounce),
        emit: EmitConfig::from(args),
        fail_on: args.fail_on,
        languages: args.language.map(|language| vec![language]),
    };

    let emit = config.emit.clone();
    crate::watch::run_watch(args.path.clone(), config, move |analysis, change_set| {
        emit_graph(analysis, &emit)?;
        log_rebuild(analysis, change_set);
        Ok(())
    })?;

    Ok(ExitCode::SUCCESS)
}

/// Report one watch rebuild at info level.
#[cfg(feature = "watch")]
fn log_rebuild(
    analysis: &crate::pipeline::GraphAnalysis,
    change_set: &crate::reanalyze::ChangeSet,
) {
    let cyclic = analysis
        .scc
        .components
        .iter()
        .filter(|component| component.is_cyclic)
        .count();
    tracing::info!(
        snapshot = analysis.snapshot_id.to_raw(),
        files = analysis.graph.file_count(),
        symbols = analysis.graph.symbol_count(),
        edges = analysis.graph.edge_count(),
        sccs = analysis.scc.components.len(),
        cyclic = cyclic,
        added = change_set.files_added,
        removed = change_set.files_removed,
        modified = change_set.files_modified,
        unchanged = change_set.files_unchanged,
        "Re-analyzed",
    );
}

/// Write the portable datagraph export next to the graph output.
#[cfg(feature = "dataflow")]
fn emit_datagraph(
    analysis: &crate::pipeline::GraphAnalysis,
    args: &GraphArgs,
) -> anyhow::Result<()> {
    use crate::sink::GraphSink;

    let export = crate::output::graph::GraphOutput::from_graph(
        &analysis.graph,
        Some(&analysis.scc),
        analysis.snapshot_id.to_raw() as u64,
    );
    tracing::info!(
        schema_version = export.schema_version,
        node_count = export.metadata.node_count,
        "Exporting datagraph"
    );

    // The datagraph has its own path. Reusing -o would make the two exports
    // overwrite each other, and deriving only the file name would drop the
    // graph output's directory.
    let output_path = match (&args.datagraph_output, &args.output) {
        (Some(explicit), _) => explicit.clone(),
        (None, Some(graph_output)) => {
            let stem = graph_output
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "datagraph".to_string());
            graph_output.with_file_name(format!("{stem}.datagraph.json"))
        }
        (None, None) => PathBuf::from("datagraph.json"),
    };
    tracing::info!(path = %output_path.display(), "Writing datagraph export");
    let sink = crate::sink::JsonSink::new(Some(output_path));
    sink.emit(&export)
}
