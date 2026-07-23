//! Cut-edge detection and annotation for deployment planning.
//!
//! When an SCC straddles a language boundary or a same-language
//! pod exceeds the maximum size threshold, the weakest edge
//! (lowest confidence) is cut and annotated. The cut edge represents
//! a point where a direct call must be converted to an RPC stub;
//! the petgraph itself is never mutated - only the deployment plan
//! is annotated.

use std::collections::HashSet;

use crate::deploy::pod::{Pod, PodPartition, node_to_file_id};
use crate::graph::scc::SccAnalysis;
use crate::graph::{CodeGraph, NodeData};
use crate::language::LangId;
use crate::model::FileId;

/// Reason the edge was selected for cutting.
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub enum CutReason {
    CrossLanguageScc,
    OversizedPod { pod_size: usize, max_size: usize },
}

/// Annotation attached to a cut edge in the manifest.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CutAnnotation {
    pub from_file: String,
    pub to_file: String,
    pub cut_reason: CutReason,
    pub original_confidence: f32,
}

/// A recorded cut edge that must appear as an RPC stub in the manifest.
#[derive(Debug, Clone)]
pub struct CutEdge {
    pub from_pod: usize,
    pub to_pod: usize,
    pub annotation: CutAnnotation,
}

/// Default maximum pod size (in files) before rebalancing is triggered.
pub const DEFAULT_MAX_POD_SIZE: usize = 20;

/// Detect cross-language SCC edges that must be cut.
///
/// For each SCC spanning multiple languages, find the single lowest-
/// confidence cross-language internal edge and mark it for RPC conversion.
/// Pod IDs are resolved from the partition so the manifest generator
/// can place cuts correctly.
pub fn find_cross_language_cuts(
    scc: &SccAnalysis,
    graph: &CodeGraph,
    file_languages: &std::collections::HashMap<FileId, LangId>,
    partition: &PodPartition,
) -> Vec<CutEdge> {
    // Build FileId -> pod_id lookup for resolving cut pod membership.
    let mut file_to_pod: std::collections::HashMap<FileId, usize> =
        std::collections::HashMap::new();
    for pod in &partition.pods {
        for &fid in &pod.files {
            file_to_pod.insert(fid, pod.id);
        }
    }

    let mut cuts = Vec::new();

    for comp in &scc.components {
        if !comp.is_cyclic {
            continue;
        }

        // Build a HashSet of component nodes for O(1) membership tests.
        let comp_nodes: HashSet<_> = comp.nodes.iter().copied().collect();

        // Determine languages present in this component. Import edges connect
        // File->File (and File->External) directly, so a cross-language cycle can
        // be composed entirely of File nodes with no Symbol nodes; match every
        // NodeData variant that carries a language to avoid missing such cycles.
        let mut langs = HashSet::new();
        for &node_idx in &comp.nodes {
            match graph.graph.node_weight(node_idx) {
                Some(NodeData::Symbol(s)) => {
                    if let Some(&lang) = file_languages.get(&s.file_id) {
                        langs.insert(lang);
                    }
                }
                Some(NodeData::File(f)) => {
                    langs.insert(f.language);
                }
                Some(NodeData::External(e)) => {
                    langs.insert(e.language);
                }
                Some(NodeData::Data(_)) => {}
                None => {}
            }
        }
        if langs.len() <= 1 {
            continue;
        }

        // Find the lowest-confidence cross-language edge inside this SCC.
        let mut best_edge: Option<(FileId, FileId, f32)> = None;
        for edge_idx in graph.graph.edge_indices() {
            let weight = &graph.graph[edge_idx];
            if !weight.participates_in_scc() {
                continue;
            }
            let Some((u, v)) = graph.graph.edge_endpoints(edge_idx) else {
                continue;
            };
            if !comp_nodes.contains(&u) || !comp_nodes.contains(&v) {
                continue;
            }
            let Some(src_fid) = node_to_file_id(&graph.graph[u]) else {
                continue;
            };
            let Some(dst_fid) = node_to_file_id(&graph.graph[v]) else {
                continue;
            };
            if file_languages.get(&src_fid) != file_languages.get(&dst_fid) {
                let conf = weight.confidence;
                if conf < best_edge.map_or(f32::MAX, |(_, _, c)| c) {
                    best_edge = Some((src_fid, dst_fid, conf));
                }
            }
        }

        if let Some((src, dst, conf)) = best_edge {
            let from_pod = file_to_pod.get(&src).copied().unwrap_or(0);
            let to_pod = file_to_pod.get(&dst).copied().unwrap_or(0);
            cuts.push(CutEdge {
                from_pod,
                to_pod,
                annotation: CutAnnotation {
                    from_file: src.0.to_string(),
                    to_file: dst.0.to_string(),
                    cut_reason: CutReason::CrossLanguageScc,
                    original_confidence: conf,
                },
            });
        }
    }

    cuts
}

/// Find the weakest internal edge in an oversized pod and mark it for splitting.
///
/// Greedy approach: for a pod exceeding `max_size`, find the
/// internal edge with the lowest confidence and cut it.
/// Iterates graph edges once (O(edges)) instead of per-file.
pub fn find_oversized_pod_cut(pod: &Pod, graph: &CodeGraph, max_size: usize) -> Option<CutEdge> {
    if pod.files.len() <= max_size {
        return None;
    }

    let files_set: HashSet<FileId> = pod.files.iter().copied().collect();
    let mut best_edge: Option<(FileId, FileId, f32)> = None;

    // Single pass over all edges -- filter to intra-pod edges only.
    for edge_idx in graph.graph.edge_indices() {
        let weight = &graph.graph[edge_idx];
        if !weight.participates_in_scc() {
            continue;
        }
        let Some((u, v)) = graph.graph.edge_endpoints(edge_idx) else {
            continue;
        };
        let Some(src_fid) = node_to_file_id(&graph.graph[u]) else {
            continue;
        };
        let Some(dst_fid) = node_to_file_id(&graph.graph[v]) else {
            continue;
        };
        // Only edges where both endpoints are in this pod.
        if !files_set.contains(&src_fid) || !files_set.contains(&dst_fid) {
            continue;
        }
        let conf = weight.confidence;
        if conf < best_edge.map_or(f32::MAX, |(_, _, c)| c) {
            best_edge = Some((src_fid, dst_fid, conf));
        }
    }

    best_edge.map(|(src, dst, conf)| CutEdge {
        from_pod: pod.id,
        to_pod: pod.id,
        annotation: CutAnnotation {
            from_file: src.0.to_string(),
            to_file: dst.0.to_string(),
            cut_reason: CutReason::OversizedPod {
                pod_size: pod.files.len(),
                max_size,
            },
            original_confidence: conf,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::pod::partition_into_pods;
    use crate::graph::edge::{EdgeData, EdgeKind};
    use crate::graph::node::{FileNode, NodeData};
    use crate::graph::scc::SccAnalysis;
    use crate::language::LangId;
    use crate::model::ids::{FileId, SnapshotId};
    use petgraph::graph::NodeIndex;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Build a pod of `n` same-language files (f0..f{n-1}.py) where each file
    /// imports the next, plus a deliberately weak edge from the first to the
    /// last, to exercise `find_oversized_pod_cut`.
    fn build_chain_pod(n: usize) -> (Pod, CodeGraph) {
        let mut graph = CodeGraph::new(SnapshotId(1));
        let mut fids = Vec::with_capacity(n);
        for i in 0..n {
            let id = FileId(i as u32);
            let idx = graph.graph.add_node(NodeData::File(FileNode::new(
                id,
                PathBuf::from(format!("f{i}.py")),
                LangId::Python,
                SnapshotId(1),
            )));
            graph.file_to_index.insert(id, idx);
            fids.push(id);
        }

        let idx_of = |fid: FileId| -> NodeIndex { *graph.file_to_index.get(&fid).unwrap() };
        for i in 0..n.saturating_sub(1) {
            graph.graph.add_edge(
                idx_of(fids[i]),
                idx_of(fids[i + 1]),
                EdgeData::new(EdgeKind::Import),
            );
        }
        if n >= 2 {
            graph.graph.add_edge(
                idx_of(fids[0]),
                idx_of(fids[n - 1]),
                EdgeData::with_confidence(EdgeKind::Import, 0.3),
            );
        }

        let pod = Pod {
            id: 0,
            files: fids,
            language: LangId::Python,
        };
        (pod, graph)
    }

    #[test]
    fn oversized_pod_cut_fires_below_threshold() {
        let (pod, graph) = build_chain_pod(4);
        let cut = find_oversized_pod_cut(&pod, &graph, 3);
        let cut = cut.expect("oversized pod should produce a cut");
        assert_eq!(cut.from_pod, 0);
        assert_eq!(cut.to_pod, 0);
        match &cut.annotation.cut_reason {
            CutReason::OversizedPod { pod_size, max_size } => {
                assert_eq!(*pod_size, 4);
                assert_eq!(*max_size, 3);
            }
            other => panic!("expected OversizedPod, got {other:?}"),
        }
        assert_eq!(cut.annotation.original_confidence, 0.3);
    }

    #[test]
    fn small_pod_no_oversized_cut() {
        let (pod, graph) = build_chain_pod(2);
        assert!(
            find_oversized_pod_cut(&pod, &graph, 3).is_none(),
            "pod within threshold must not be cut"
        );
    }

    #[test]
    fn cross_language_cycle_produces_scc_cut() {
        let mut graph = CodeGraph::new(SnapshotId(1));
        let py_id = FileId(0);
        let go_id = FileId(1);
        let py_idx = graph.graph.add_node(NodeData::File(FileNode::new(
            py_id,
            PathBuf::from("orch.py"),
            LangId::Python,
            SnapshotId(1),
        )));
        let go_idx = graph.graph.add_node(NodeData::File(FileNode::new(
            go_id,
            PathBuf::from("auth.go"),
            LangId::Go,
            SnapshotId(1),
        )));
        graph.file_to_index.insert(py_id, py_idx);
        graph.file_to_index.insert(go_id, go_idx);

        graph
            .graph
            .add_edge(py_idx, go_idx, EdgeData::new(EdgeKind::Import));
        graph
            .graph
            .add_edge(go_idx, py_idx, EdgeData::new(EdgeKind::Import));

        let scc = SccAnalysis::analyze(&graph.graph);
        let mut file_languages: HashMap<FileId, LangId> = HashMap::new();
        for (&fid, &idx) in &graph.file_to_index {
            if let NodeData::File(f) = &graph.graph[idx] {
                file_languages.insert(fid, f.language);
            }
        }
        let partition = partition_into_pods(&graph);
        let cuts = find_cross_language_cuts(&scc, &graph, &file_languages, &partition);
        let cut = cuts
            .into_iter()
            .find(|c| matches!(c.annotation.cut_reason, CutReason::CrossLanguageScc))
            .expect("cross-language cycle must produce a CrossLanguageScc cut");
        assert!(matches!(
            cut.annotation.cut_reason,
            CutReason::CrossLanguageScc
        ));
    }
}
