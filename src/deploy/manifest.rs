//! Pod-level deployment manifest generation.
//!
//! pod-based schema: `PodManifest` contains `deployments` (one per pod),
//! `edges` (inter-pod dependency edges with optional cut annotations),
//! and `metrics` (global aggregate).

use std::collections::HashMap;

use serde::Serialize;

use crate::deploy::cut::{CutAnnotation, CutEdge};
use crate::deploy::dependency::DependencyEntry;
use crate::deploy::metrics::PodMetrics;
use crate::deploy::pod::PodPartition;
use crate::graph::CodeGraph;

/// The top-level pod-based deployment manifest.
#[derive(Debug, Clone, Serialize)]
pub struct PodManifest {
    pub version: String,
    pub deployments: Vec<PodDeployment>,
    pub edges: Vec<ManifestEdge>,
    pub metrics: GlobalMetrics,
}

/// A single pod deployment - an independently deployable unit.
#[derive(Debug, Clone, Serialize)]
pub struct PodDeployment {
    pub id: usize,
    pub language: String,
    pub files: Vec<String>,
    pub metrics: PodMetrics,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<DependencyEntry>,
}

/// An inter-pod edge in the manifest.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestEdge {
    pub from_pod: usize,
    pub to_pod: usize,
    pub kind: String,
    pub confidence: f32,
    pub is_cross_language: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cut_annotation: Option<CutAnnotation>,
}

/// Global aggregate metrics for the entire manifest.
#[derive(Debug, Clone, Serialize, Default)]
pub struct GlobalMetrics {
    pub total_pods: usize,
    pub cross_language_edges: usize,
    pub total_ast_nodes: usize,
}

/// Generate a `PodManifest` from partition, metrics, cuts, and dependencies.
pub fn generate_pod_manifest(
    partition: &PodPartition,
    pod_metrics: &[PodMetrics],
    cuts: &[CutEdge],
    dependencies: &HashMap<usize, Vec<DependencyEntry>>,
    graph: &CodeGraph,
) -> PodManifest {
    let mut deployments = Vec::with_capacity(partition.pods.len());

    for (i, pod) in partition.pods.iter().enumerate() {
        let tag = crate::deploy::tags::metacall_tag(pod.language);
        let files: Vec<String> = pod
            .files
            .iter()
            .filter_map(|fid| {
                graph
                    .file_node(*fid)
                    .map(|f| f.path.to_string_lossy().replace('\\', "/"))
            })
            .collect();

        let deps = dependencies.get(&pod.id).cloned().unwrap_or_default();
        let pm = pod_metrics.get(i).cloned().unwrap_or(PodMetrics {
            total_ast_nodes: 0,
            file_count: 0,
            symbol_count: 0,
        });

        deployments.push(PodDeployment {
            id: pod.id,
            language: tag.to_string(),
            files,
            metrics: pm,
            dependencies: deps,
        });
    }

    // Build inter-pod edges, annotating cuts where applicable.
    let mut cut_lookup: HashMap<(usize, usize), &CutAnnotation> = HashMap::new();
    for cut in cuts {
        cut_lookup.insert((cut.from_pod, cut.to_pod), &cut.annotation);
    }

    let mut edges: Vec<ManifestEdge> = partition
        .inter_pod_edges
        .iter()
        .map(|ip| {
            let kind = match ip.kind {
                crate::graph::EdgeKind::Import => "import".to_string(),
                crate::graph::EdgeKind::Reference => "reference".to_string(),
                crate::graph::EdgeKind::Ownership => "ownership".to_string(),
                crate::graph::EdgeKind::Flow => "flow".to_string(),
            };
            let annotation = cut_lookup
                .get(&(ip.from_pod, ip.to_pod))
                .map(|a| (*a).clone());
            ManifestEdge {
                from_pod: ip.from_pod,
                to_pod: ip.to_pod,
                kind,
                confidence: ip.confidence,
                is_cross_language: ip.is_cross_language,
                cut_annotation: annotation,
            }
        })
        .collect();

    // Mark cut edges that weren't already in inter_pod_edges as rpc_stub edges.
    let inter_pod_pairs: std::collections::HashSet<(usize, usize)> = partition
        .inter_pod_edges
        .iter()
        .map(|ip| (ip.from_pod, ip.to_pod))
        .collect();

    for cut in cuts {
        // Every cut must surface as a `rpc_stub` edge (ADR 0003: a forced
        // split is only safe if the call boundary is explicitly represented).
        // This holds even when an `import` edge for the same pod pair already
        // exists - the import edge keeps its annotation, and a distinct
        // `rpc_stub` edge records the split boundary.
        edges.push(ManifestEdge {
            from_pod: cut.from_pod,
            to_pod: cut.to_pod,
            kind: "rpc_stub".to_string(),
            confidence: cut.annotation.original_confidence,
            is_cross_language: matches!(
                cut.annotation.cut_reason,
                crate::deploy::cut::CutReason::CrossLanguageScc
            ),
            cut_annotation: Some(cut.annotation.clone()),
        });

        // Also annotate the matching import/reference edge so the existing
        // cross-language link carries the cut reason.
        if inter_pod_pairs.contains(&(cut.from_pod, cut.to_pod)) {
            for edge in &mut edges {
                if edge.from_pod == cut.from_pod
                    && edge.to_pod == cut.to_pod
                    && edge.cut_annotation.is_none()
                {
                    edge.cut_annotation = Some(cut.annotation.clone());
                }
            }
        }
    }

    let total_ast_nodes = pod_metrics.iter().map(|m| m.total_ast_nodes).sum();
    let cross_language_edges = edges.iter().filter(|e| e.is_cross_language).count();
    let total_pods = deployments.len();

    PodManifest {
        version: "1.0".to_string(),
        deployments,
        edges,
        metrics: GlobalMetrics {
            total_pods,
            cross_language_edges,
            total_ast_nodes,
        },
    }
}
