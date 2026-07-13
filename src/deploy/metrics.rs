//! File- and pod-level deployment metrics.
//!
//! Metrics are derived from data already collected during extraction
//! (`FileExtraction`) and graph construction, so no extra passes are
//! needed. The default metric is AST node count (a proxy for
//! computational surface area); the schema is open for future
//! telemetry-based metrics.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::deploy::pod::PodPartition;
use crate::graph::{CodeGraph, NodeData};
use crate::model::{FileExtraction, FileId};

/// Per-file deployment metrics, derived from extraction results.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileMetrics {
    pub ast_node_count: usize,
    pub symbol_count: usize,
    pub import_count: usize,
    pub reference_count: usize,
}

/// Aggregated per-pod deployment metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PodMetrics {
    pub total_ast_nodes: usize,
    pub file_count: usize,
    pub symbol_count: usize,
}

/// Compute per-file metrics from extraction results, keyed by file path.
pub fn compute_file_metrics(extractions: &[FileExtraction]) -> HashMap<PathBuf, FileMetrics> {
    let mut metrics = HashMap::new();
    for file in extractions {
        metrics.insert(
            file.path.clone(),
            FileMetrics {
                ast_node_count: file.ast_node_count,
                symbol_count: file.symbols.len(),
                import_count: file.imports.len(),
                reference_count: file.references.len(),
            },
        );
    }
    metrics
}

/// Compute per-pod metrics from pod partition and extraction data.
pub fn compute_pod_metrics(
    partition: &PodPartition,
    file_metrics: &HashMap<PathBuf, FileMetrics>,
    graph: &CodeGraph,
) -> Vec<PodMetrics> {
    // Build a reverse mapping from FileId -> Path using the graph's file nodes.
    let mut fid_to_path: HashMap<FileId, PathBuf> = HashMap::new();
    for (&fid, &idx) in &graph.file_to_index {
        if let Some(NodeData::File(f)) = graph.graph.node_weight(idx) {
            fid_to_path.insert(fid, f.path.clone());
        }
    }

    let mut pod_metrics = Vec::with_capacity(partition.pods.len());
    for pod in &partition.pods {
        let mut total_ast_nodes = 0usize;
        let file_count = pod.files.len();
        let mut symbol_count = 0usize;

        for &fid in &pod.files {
            if let Some(path) = fid_to_path.get(&fid)
                && let Some(fm) = file_metrics.get(path)
            {
                total_ast_nodes += fm.ast_node_count;
                symbol_count += fm.symbol_count;
            }
        }

        pod_metrics.push(PodMetrics {
            total_ast_nodes,
            file_count,
            symbol_count,
        });
    }
    pod_metrics
}
