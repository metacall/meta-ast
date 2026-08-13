use std::path::Path;
use std::sync::Arc;

use crate::error::Diagnostic;
use crate::graph::{CodeGraph, GraphBuilder, SccAnalysis};
use crate::input;
use crate::language::LangId;
use crate::model::SnapshotId;

/// Metadata about a snapshot analysis run.
#[derive(Debug, Clone)]
pub struct SnapshotMeta {
    pub id: SnapshotId,
    pub datagraph_schema_version: u32,
}

/// Result of the full graph analysis pipeline.
pub struct GraphAnalysis {
    pub graph: CodeGraph,
    pub scc: SccAnalysis,
    pub snapshot_id: SnapshotId,
    pub extractions: Vec<Arc<crate::model::FileExtraction>>,
}

/// Run the full graph analysis pipeline on a path.
///
/// Discovers files, extracts symbols/imports/references in parallel,
/// builds the dependency graph, resolves cross-file references,
/// and computes SCC analysis.
pub fn analyze_graph(
    root: &Path,
    snapshot_id: SnapshotId,
    languages: Option<&[LangId]>,
) -> anyhow::Result<(GraphAnalysis, Vec<Diagnostic>)> {
    let files = input::discover_files(root, languages)?;
    let extraction = crate::extractor::extract(&files);
    let mut diagnostics: Vec<Diagnostic> = extraction
        .files
        .iter()
        .flat_map(|f| f.diagnostics.iter().cloned())
        .collect();

    let arc_extractions: Vec<_> = extraction.files.into_iter().map(Arc::new).collect();

    let (graph, scc) =
        GraphBuilder::from_extractions(&arc_extractions, root, snapshot_id, &mut diagnostics);

    Ok((
        GraphAnalysis {
            graph,
            scc,
            snapshot_id,
            extractions: arc_extractions,
        },
        diagnostics,
    ))
}

/// Build a SnapshotMeta for the current analysis run.
pub fn snapshot_meta(snapshot_id: SnapshotId) -> SnapshotMeta {
    SnapshotMeta {
        id: snapshot_id,
        datagraph_schema_version: crate::output::graph::SCHEMA_VERSION,
    }
}
