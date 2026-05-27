use std::collections::HashMap;
use std::path::Path;

use crate::error::Diagnostic;
use crate::graph::{CodeGraph, GraphBuilder, SccAnalysis};
use crate::input;
use crate::model::{FileId, SnapshotId};

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

    let mut builder = GraphBuilder::new(snapshot_id);
    for (path, lang) in &files {
        builder.add_file(path.clone(), *lang);
    }

    for file in &extraction.files {
        for symbol in &file.symbols {
            if let Err(e) = builder.add_symbol(symbol) {
                diagnostics.push(Diagnostic {
                    path: file.path.clone(),
                    severity: crate::error::Severity::Warning,
                    message: format!("failed to add symbol to graph: {e}"),
                    source_range: None,
                });
            }
        }
    }

    let mut path_to_file_id: HashMap<std::path::PathBuf, FileId> = HashMap::new();
    for file in &extraction.files {
        if let Some(fid) = builder.file_id_for_path(&file.path) {
            path_to_file_id.insert(file.path.clone(), fid);
        }
    }

    for file in &extraction.files {
        let Some(&source_fid) = path_to_file_id.get(&file.path) else {
            continue;
        };
        for import in &file.imports {
            if let Some(dir) = file.path.parent()
                && let Some(target) =
                    crate::graph::resolver::normalize_import_path(dir, &import.namespace, file.lang)
            {
                builder.add_import(source_fid, target);
            }
        }
    }

    let import_adjacency = builder.import_adjacency();
    let ctx = crate::graph::resolver::ResolutionContext::from_extractions(
        &extraction.files,
        &path_to_file_id,
        import_adjacency,
    );
    let scope_cache = crate::graph::resolver::FlattenedScopeCache::build(&ctx, &mut diagnostics);

    let ref_edges = crate::graph::resolver::resolve_all_references(
        &extraction.files,
        &path_to_file_id,
        &scope_cache,
        &mut diagnostics,
    );

    for (from, to) in ref_edges {
        builder.add_reference(from, to);
    }

    let graph = builder.build();
    let scc = SccAnalysis::analyze(&graph.graph);

    Ok((
        GraphAnalysis {
            graph,
            scc,
            snapshot_id,
        },
        diagnostics,
    ))
}
