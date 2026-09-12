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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cut_annotations: Vec<CutAnnotation>,
}

/// Global aggregate metrics for the entire manifest.
#[derive(Debug, Clone, Serialize, Default)]
pub struct GlobalMetrics {
    pub total_pods: usize,
    pub cross_language_edges: usize,
    pub total_ast_nodes: usize,
    /// Pod files the graph or the metrics pass could not resolve.
    #[serde(default)]
    pub dropped_files: usize,
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
    let mut dropped_files = 0usize;

    for (i, pod) in partition.pods.iter().enumerate() {
        let tag = crate::deploy::tags::metacall_tag(pod.language);
        let mut dropped = 0usize;
        let files: Vec<String> = pod
            .files
            .iter()
            .filter_map(|fid| match graph.file_node(*fid) {
                Some(file) => Some(crate::input::portable_path(&file.path)),
                None => {
                    dropped += 1;
                    tracing::warn!(
                        pod = pod.id,
                        file = fid.to_raw(),
                        "pod file has no graph node"
                    );
                    None
                }
            })
            .collect();

        let deps = dependencies.get(&pod.id).cloned().unwrap_or_default();
        let pm = match pod_metrics.get(i).cloned() {
            Some(metrics) => metrics,
            None => {
                dropped += 1;
                tracing::warn!(pod = pod.id, "pod has no metrics entry");
                PodMetrics {
                    total_ast_nodes: 0,
                    file_count: 0,
                    symbol_count: 0,
                }
            }
        };
        dropped_files += dropped;

        deployments.push(PodDeployment {
            id: pod.id,
            language: tag.to_string(),
            files,
            metrics: pm,
            dependencies: deps,
        });
    }

    // Build inter-pod edges, annotating cuts where applicable. A pod pair can
    // carry more than one cut, so the annotations merge into a list.
    let mut cut_lookup: HashMap<(usize, usize), Vec<CutAnnotation>> = HashMap::new();
    for cut in cuts {
        cut_lookup
            .entry((cut.from_pod, cut.to_pod))
            .or_default()
            .push(cut.annotation.clone());
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
            let annotations = cut_lookup
                .get(&(ip.from_pod, ip.to_pod))
                .cloned()
                .unwrap_or_default();
            ManifestEdge {
                from_pod: ip.from_pod,
                to_pod: ip.to_pod,
                kind,
                confidence: ip.confidence,
                is_cross_language: ip.is_cross_language,
                cut_annotations: annotations,
            }
        })
        .collect();

    // Mark cut edges that weren't already in inter_pod_edges as rpc_stub edges.
    let inter_pod_pairs: std::collections::HashSet<(usize, usize)> = partition
        .inter_pod_edges
        .iter()
        .map(|ip| (ip.from_pod, ip.to_pod))
        .collect();

    // Every cut pair must surface as one `rpc_stub` edge (ADR 0003: a forced
    // split is only safe if the call boundary is explicitly represented). The
    // pair carries every annotation, and a repeated pair does not add a stub.
    for (pair, annotations) in &cut_lookup {
        if edges
            .iter()
            .any(|edge| edge.kind == "rpc_stub" && edge.from_pod == pair.0 && edge.to_pod == pair.1)
        {
            continue;
        }
        let confidence = annotations
            .iter()
            .map(|annotation| annotation.original_confidence)
            .fold(f32::INFINITY, f32::min);
        edges.push(ManifestEdge {
            from_pod: pair.0,
            to_pod: pair.1,
            kind: "rpc_stub".to_string(),
            confidence,
            is_cross_language: annotations.iter().any(|annotation| {
                matches!(
                    annotation.cut_reason,
                    crate::deploy::cut::CutReason::CrossLanguageScc
                )
            }),
            cut_annotations: annotations.clone(),
        });
    }

    edges.sort_by(|a, b| (a.from_pod, a.to_pod, &a.kind).cmp(&(b.from_pod, b.to_pod, &b.kind)));
    let total_ast_nodes = pod_metrics.iter().fold(0usize, |total, metrics| {
        total.saturating_add(metrics.total_ast_nodes)
    });
    let cross_language_edges = edges.iter().filter(|e| e.is_cross_language).count();
    let total_pods = deployments.len();

    PodManifest {
        version: "1.1".to_string(),
        deployments,
        edges,
        metrics: GlobalMetrics {
            total_pods,
            cross_language_edges,
            total_ast_nodes,
            dropped_files,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::cut::{CutAnnotation, CutReason};
    use crate::deploy::metrics::PodMetrics;
    use crate::deploy::pod::{InterPodEdge, Pod, PodPartition};
    use crate::graph::EdgeKind;
    use crate::graph::node::{FileNode, NodeData};
    use crate::language::LangId;
    use crate::model::ids::{FileId, SnapshotId};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn test_partition() -> (PodPartition, CodeGraph) {
        let mut graph = CodeGraph::new(SnapshotId::new(1).unwrap());
        let py = FileId::new(1).unwrap();
        let js = FileId::new(2).unwrap();
        for (fid, path, lang) in [
            (py, "a.py", LangId::Python),
            (js, "b.js", LangId::JavaScript),
        ] {
            let idx = graph.add_node(NodeData::File(FileNode::new(
                fid,
                PathBuf::from(path),
                lang,
                SnapshotId::new(1).unwrap(),
            )));
            graph.file_to_index.insert(fid, idx);
        }

        let partition = PodPartition {
            pods: vec![
                Pod {
                    id: 0,
                    files: vec![py],
                    language: LangId::Python,
                },
                Pod {
                    id: 1,
                    files: vec![js],
                    language: LangId::JavaScript,
                },
            ],
            inter_pod_edges: vec![InterPodEdge {
                from_pod: 0,
                to_pod: 1,
                from_file: py,
                to_file: js,
                kind: EdgeKind::Import,
                confidence: 0.6,
                is_cross_language: true,
            }],
            file_languages: HashMap::from([(py, LangId::Python), (js, LangId::JavaScript)]),
        };
        (partition, graph)
    }

    fn cut(reason: CutReason, confidence: f32) -> CutEdge {
        CutEdge {
            from_pod: 0,
            to_pod: 1,
            annotation: CutAnnotation {
                from_file: "a.py".to_string(),
                to_file: "b.js".to_string(),
                cut_reason: reason,
                original_confidence: confidence,
            },
        }
    }

    /// Two cuts on one pod pair must surface as a single rpc_stub edge and keep
    /// both annotations.
    #[test]
    fn two_cuts_on_one_pod_pair_emit_one_rpc_stub() {
        let (partition, graph) = test_partition();
        let metrics = vec![
            PodMetrics {
                total_ast_nodes: 1,
                file_count: 1,
                symbol_count: 0,
            },
            PodMetrics {
                total_ast_nodes: 1,
                file_count: 1,
                symbol_count: 0,
            },
        ];
        let cuts = vec![
            cut(CutReason::CrossLanguageScc, 0.6),
            cut(
                CutReason::OversizedPod {
                    pod_size: 4,
                    max_size: 3,
                },
                0.3,
            ),
        ];

        let manifest = generate_pod_manifest(&partition, &metrics, &cuts, &HashMap::new(), &graph);

        let stubs: Vec<&ManifestEdge> = manifest
            .edges
            .iter()
            .filter(|e| e.kind == "rpc_stub")
            .collect();
        assert_eq!(
            stubs.len(),
            1,
            "one rpc_stub per pod pair, got {}: {:?}",
            stubs.len(),
            stubs
                .iter()
                .map(|e| (e.from_pod, e.to_pod))
                .collect::<Vec<_>>()
        );

        let json = serde_json::to_string(&manifest).unwrap();
        assert!(
            json.contains("CrossLanguageScc"),
            "the SCC cut annotation must survive"
        );
        assert!(
            json.contains("OversizedPod"),
            "the oversized cut annotation must survive"
        );
    }
}
