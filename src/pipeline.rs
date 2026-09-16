use std::path::Path;
use std::sync::Arc;

use crate::error::Diagnostic;
use crate::graph::{CodeGraph, GraphBuilder, SccAnalysis};
use crate::input;
use crate::language::LangId;
use crate::model::{FileExtraction, SnapshotId};

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
    /// Flattened scope cache from the same pass. Resolves a name for a file id.
    pub scope: crate::graph::resolver::FlattenedScopeCache,
    /// One record per resolved use site, in extraction and reference order.
    pub references: Vec<crate::graph::resolver::ResolvedReference>,
}

/// Assemble the graph, the scope cache, the resolved references, the client
/// call edges and the SCC analysis from a set of extractions.
///
/// Both the one-shot and the incremental entry point call this, so the analysis
/// of a tree cannot depend on which entry point produced it. The returned
/// diagnostics are in the canonical order.
pub fn build_analysis(
    extractions: Vec<Arc<FileExtraction>>,
    root: &Path,
    snapshot_id: SnapshotId,
) -> (GraphAnalysis, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let parts =
        GraphBuilder::from_extractions_detailed(&extractions, root, snapshot_id, &mut diagnostics);
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    (
        GraphAnalysis {
            graph: parts.graph,
            scc: parts.scc,
            snapshot_id,
            extractions,
            scope: parts.scope,
            references: parts.references,
        },
        diagnostics,
    )
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
    let (files, mut discovery_diagnostics) =
        input::discover_files_with_diagnostics(root, languages)?;
    let extraction = crate::extractor::extract(&files);
    let mut diagnostics: Vec<Diagnostic> = extraction
        .files
        .iter()
        .flat_map(|f| f.diagnostics.iter().cloned())
        .collect();
    diagnostics.append(&mut discovery_diagnostics);

    let arc_extractions: Vec<_> = extraction.files.into_iter().map(Arc::new).collect();

    let (analysis, mut graph_diagnostics) = build_analysis(arc_extractions, root, snapshot_id);
    diagnostics.append(&mut graph_diagnostics);
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    Ok((analysis, diagnostics))
}

/// Build a SnapshotMeta for the current analysis run.
pub fn snapshot_meta(snapshot_id: SnapshotId) -> SnapshotMeta {
    SnapshotMeta {
        id: snapshot_id,
        datagraph_schema_version: crate::output::graph::SCHEMA_VERSION,
    }
}
