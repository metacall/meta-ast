use std::path::Path;

use crate::error::Diagnostic;
use crate::graph::{CodeGraph, GraphBuilder, SccAnalysis};
use crate::input;
use crate::model::SnapshotId;

/// Result of the full graph analysis pipeline.
pub struct GraphAnalysis {
    pub graph: CodeGraph,
    pub scc: SccAnalysis,
    pub snapshot_id: SnapshotId,
}

/// Run the full graph analysis pipeline on a path.
///
/// Discovers files, extracts symbols/imports/references in parallel,
/// builds the dependency graph, resolves cross-file references,
/// and computes SCC analysis.
pub fn analyze_graph(
    root: &Path,
    snapshot_id: SnapshotId,
) -> anyhow::Result<(GraphAnalysis, Vec<Diagnostic>)> {
    let files = input::discover_files(root, None)?;
    let extraction = crate::extractor::extract(&files);
    let mut diagnostics: Vec<Diagnostic> = extraction
        .files
        .iter()
        .flat_map(|f| f.diagnostics.iter().cloned())
        .collect();

    let (graph, scc) =
        GraphBuilder::from_extractions(&extraction.files, root, snapshot_id, &mut diagnostics);

    Ok((
        GraphAnalysis {
            graph,
            scc,
            snapshot_id,
        },
        diagnostics,
    ))
}
