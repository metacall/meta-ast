//! Manifest validation and cut-edge fairness checks.
//!
//! Verifies that every edge cut in the deployment plan has a
//! corresponding RPC stub entry in the manifest, so a forced split
//! can never silently drop a call.

use std::collections::HashSet;

use crate::deploy::cut::CutEdge;
use crate::deploy::manifest::PodManifest;

/// Verify that every cut edge has a corresponding manifest entry.
///
/// Principles (aligned with ADR 0003):
/// - Every cut edge must appear in `manifest.edges[]` with cut annotations.
/// - No non-cut edge may carry cut annotations.
/// - Every cut edge must have `kind: "rpc_stub"`.
pub fn check_cut_fairness(manifest: &PodManifest, cuts: &[CutEdge]) -> Vec<String> {
    let mut diagnostics = Vec::new();

    let cut_pairs: HashSet<(usize, usize)> = cuts.iter().map(|c| (c.from_pod, c.to_pod)).collect();

    for cut in cuts {
        let present = manifest.edges.iter().any(|e| {
            e.from_pod == cut.from_pod && e.to_pod == cut.to_pod && !e.cut_annotations.is_empty()
        });
        if !present {
            diagnostics.push(format!(
                "cut edge ({}, {}) missing from manifest or missing cut annotation",
                cut.from_pod, cut.to_pod
            ));
        }

        // A stub proves the directed boundary call is preserved: the caller
        // side opens the client, so a stub in the reverse direction alone
        // does not cover this cut.
        let has_rpc_stub = manifest
            .edges
            .iter()
            .any(|e| e.kind == "rpc_stub" && e.from_pod == cut.from_pod && e.to_pod == cut.to_pod);
        if !has_rpc_stub {
            diagnostics.push(format!(
                "cut edge ({}, {}) has no corresponding 'rpc_stub' entry",
                cut.from_pod, cut.to_pod
            ));
        }
    }

    for edge in &manifest.edges {
        if !edge.cut_annotations.is_empty() && !cut_pairs.contains(&(edge.from_pod, edge.to_pod)) {
            diagnostics.push(format!(
                "edge ({}, {}) has cut annotations but is not in the cut list",
                edge.from_pod, edge.to_pod
            ));
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::cut::{CutAnnotation, CutReason, PortablePath};
    use crate::deploy::manifest::{GlobalMetrics, ManifestEdge};

    fn cut(from_pod: usize, to_pod: usize) -> CutEdge {
        CutEdge {
            from_pod,
            to_pod,
            annotation: CutAnnotation {
                from_file: PortablePath::anchor(crate::model::FileId::new(1).unwrap()),
                to_file: PortablePath::anchor(crate::model::FileId::new(2).unwrap()),
                cut_reason: CutReason::CrossLanguageScc,
                original_confidence: 0.6,
            },
        }
    }

    fn manifest_with(edges: Vec<ManifestEdge>) -> PodManifest {
        PodManifest {
            version: "1.1".into(),
            deployments: Vec::new(),
            edges,
            metrics: GlobalMetrics::default(),
        }
    }

    fn annotated_edge(from_pod: usize, to_pod: usize) -> ManifestEdge {
        ManifestEdge {
            from_pod,
            to_pod,
            kind: "import".into(),
            confidence: 0.6,
            is_cross_language: true,
            cut_annotations: vec![cut(0, 0).annotation],
        }
    }

    fn stub(from_pod: usize, to_pod: usize) -> ManifestEdge {
        ManifestEdge {
            from_pod,
            to_pod,
            kind: "rpc_stub".into(),
            confidence: 0.6,
            is_cross_language: true,
            cut_annotations: vec![cut(0, 0).annotation],
        }
    }

    #[test]
    fn directed_stub_passes() {
        let manifest = manifest_with(vec![annotated_edge(0, 1), stub(0, 1)]);
        assert!(check_cut_fairness(&manifest, &[cut(0, 1)]).is_empty());
    }

    #[test]
    fn reversed_only_stub_fails() {
        let mut reversed = stub(1, 0);
        reversed.cut_annotations.clear();
        let manifest = manifest_with(vec![annotated_edge(0, 1), reversed]);
        let diagnostics = check_cut_fairness(&manifest, &[cut(0, 1)]);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].contains("rpc_stub"),
            "the stub direction must cover the cut: {diagnostics:?}"
        );
    }

    #[test]
    fn stray_annotations_are_flagged() {
        let manifest = manifest_with(vec![annotated_edge(0, 1)]);
        let diagnostics = check_cut_fairness(&manifest, &[]);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(
            diagnostics[0].contains("not in the cut list"),
            "{diagnostics:?}"
        );
    }
}
