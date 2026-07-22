//! Pod partitioning via Union-Find over same-language dependency edges.
//!
//! A pod is a set of files connected by Import+Reference edges where
//! all files share the same language. Cross-language edges are never
//! unioned, becoming inter-pod edges by construction.
//!
//! After partitioning, each pod can be deployed as a MetaCall runtime
//! instance. Inter-pod edges represent MetaCall RPC boundaries (either
//! explicit `metacall_load_from_*` calls or implicit reference edges
//! that cross language boundaries).

use std::collections::HashMap;

use petgraph::unionfind::UnionFind;

use crate::graph::{CodeGraph, EdgeKind, NodeData};
use crate::language::LangId;
use crate::model::FileId;

/// A pod - a set of same-language files that are connected by
/// dependency edges (Import + Reference) within the same language.
#[derive(Debug, Clone)]
pub struct Pod {
    pub id: usize,
    pub files: Vec<FileId>,
    pub language: LangId,
}

/// An inter-pod edge representing a cross-boundary dependency.
#[derive(Debug, Clone)]
pub struct InterPodEdge {
    pub from_pod: usize,
    pub to_pod: usize,
    pub from_file: FileId,
    pub to_file: FileId,
    pub kind: EdgeKind,
    pub confidence: f32,
    pub is_cross_language: bool,
}

/// Result of pod partitioning.
#[derive(Debug, Clone)]
pub struct PodPartition {
    pub pods: Vec<Pod>,
    pub inter_pod_edges: Vec<InterPodEdge>,
    pub file_languages: HashMap<FileId, LangId>,
}

/// Resolve the owning `FileId` for a graph node.
///
/// - `FileNode` -> its own id.
/// - `SymbolNode` -> the `file_id` it belongs to.
/// - `ExternalNode` -> `None` (externals aren't partitioned).
pub(crate) fn node_to_file_id(node: &NodeData) -> Option<FileId> {
    match node {
        NodeData::File(f) => Some(f.id),
        NodeData::Symbol(s) => Some(s.file_id),
        NodeData::External(_) => None,
        NodeData::Data(_) => None,
    }
}

/// Partition files into same-language pods using Union-Find.
///
/// Algorithm:
/// 1. Map every `FileId` to a contiguous `u32` index.
/// 2. For each Import or Reference edge where both endpoint files share
///    the same language, union their indices.
/// 3. `into_labeling()` yields the canonical representative per element;
///    group by representative to form pods.
/// 4. A second pass collects cross-pod (and cross-language) edges.
pub fn partition_into_pods(graph: &CodeGraph) -> PodPartition {
    // Build file->language mapping and contiguous index assignment
    let file_ids: Vec<FileId> = graph.file_to_index.keys().copied().collect();
    let file_idx: HashMap<FileId, usize> = file_ids
        .iter()
        .enumerate()
        .map(|(i, &fid)| (fid, i))
        .collect();

    let mut file_languages: HashMap<FileId, LangId> = HashMap::with_capacity(file_ids.len());
    for &fid in &file_ids {
        if let Some(node) = graph.file_node(fid) {
            file_languages.insert(fid, node.language);
        }
    }

    // Union same-language Import+Reference edges.
    let n = file_ids.len();
    let mut uf = UnionFind::new(n);

    for edge_idx in graph.graph.edge_indices() {
        let weight = &graph.graph[edge_idx];
        if !weight.participates_in_scc() {
            // Ownership edges are excluded - same rule as SCC analysis.
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

        let src_lang = file_languages.get(&src_fid);
        let dst_lang = file_languages.get(&dst_fid);

        // Only union files in the same language.
        if src_lang == dst_lang && src_lang.is_some() {
            let i = file_idx[&src_fid] as u32;
            let j = file_idx[&dst_fid] as u32;
            uf.union(i, j);
        }
    }

    // Extract pods from the labeling.
    let labeling = uf.into_labeling();
    let mut rep_to_pod: HashMap<u32, usize> = HashMap::new();
    let mut pods: Vec<Pod> = Vec::new();

    for (fidx, &rep) in labeling.iter().enumerate() {
        let pod_id = *rep_to_pod.entry(rep).or_insert_with(|| {
            let id = pods.len();
            let lang = file_languages[&file_ids[fidx]];
            pods.push(Pod {
                id,
                files: Vec::new(),
                language: lang,
            });
            id
        });
        pods[pod_id].files.push(file_ids[fidx]);
    }

    // Collect inter-pod edges. Deduplicate by (from_pod, to_pod).
    // When both Import and Reference edges exist for the same pod pair,
    // fuse their confidences via multiplication (combined structural + deploy weight).
    #[derive(Default)]
    struct FusedConfidence {
        import: Option<f32>,
        reference: Option<f32>,
    }

    let mut dedup: HashMap<(usize, usize), (FusedConfidence, bool, FileId, FileId)> =
        HashMap::new();

    for edge_idx in graph.graph.edge_indices() {
        let weight = graph.graph[edge_idx];
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

        let src_rep = labeling[file_idx[&src_fid]];
        let dst_rep = labeling[file_idx[&dst_fid]];
        if src_rep == dst_rep {
            continue; // intra-pod edge, not inter-pod.
        }

        let src_pod = rep_to_pod[&src_rep];
        let dst_pod = rep_to_pod[&dst_rep];
        let src_lang = file_languages.get(&src_fid);
        let dst_lang = file_languages.get(&dst_fid);
        let is_cross_lang = src_lang != dst_lang;

        let key = (src_pod, dst_pod);
        let entry = dedup
            .entry(key)
            .or_insert_with(|| (FusedConfidence::default(), is_cross_lang, src_fid, dst_fid));

        // Update cross-language flag: any cross-language edge makes it cross-language.
        if is_cross_lang {
            entry.1 = true;
        }

        match weight.kind {
            EdgeKind::Import => {
                entry.0.import = Some(
                    entry
                        .0
                        .import
                        .map_or(weight.confidence, |c| c.max(weight.confidence)),
                );
            }
            EdgeKind::Reference => {
                entry.0.reference = Some(
                    entry
                        .0
                        .reference
                        .map_or(weight.confidence, |c| c.max(weight.confidence)),
                );
            }
            _ => {}
        }
    }

    let mut inter_pod_edges: Vec<InterPodEdge> = Vec::with_capacity(dedup.len());
    for ((from_pod, to_pod), (fused, is_cross_lang, src_fid, dst_fid)) in dedup {
        let (kind, confidence) = match (fused.import, fused.reference) {
            (Some(imp), Some(rf)) => {
                // Multiply structural confidence (scope resolution) by deploy confidence (scanner).
                (EdgeKind::Import, (imp * rf).clamp(0.0, 1.0))
            }
            (Some(imp), None) => (EdgeKind::Import, imp),
            (None, Some(rf)) => (EdgeKind::Reference, rf),
            (None, None) => continue,
        };
        inter_pod_edges.push(InterPodEdge {
            from_pod,
            to_pod,
            from_file: src_fid,
            to_file: dst_fid,
            kind,
            confidence,
            is_cross_language: is_cross_lang,
        });
    }

    PodPartition {
        pods,
        inter_pod_edges,
        file_languages,
    }
}
