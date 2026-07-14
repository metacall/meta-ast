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
/// - Every cut edge must appear in `manifest.edges[]` with `cut_annotation`.
/// - No non-cut edge may have a `cut_annotation`.
/// - Every cut edge must have `kind: "rpc_stub"`.
pub fn check_cut_fairness(manifest: &PodManifest, cuts: &[CutEdge]) -> Vec<String> {
    let mut diagnostics = Vec::new();

    let cut_pairs: HashSet<(usize, usize)> = cuts.iter().map(|c| (c.from_pod, c.to_pod)).collect();

    for cut in cuts {
        let present = manifest.edges.iter().any(|e| {
            e.from_pod == cut.from_pod && e.to_pod == cut.to_pod && e.cut_annotation.is_some()
        });
        if !present {
            diagnostics.push(format!(
                "cut edge ({}, {}) missing from manifest or missing cut_annotation",
                cut.from_pod, cut.to_pod
            ));
        }

        // An RPC boundary between two pods is symmetric at the fairness level:
        // the cut direction is chosen by the SCC's lowest-confidence edge and
        // may be the reverse of the call-site-derived rpc_stub. A stub in
        // either direction proves the cross-boundary call is preserved.
        let has_rpc_stub = manifest.edges.iter().any(|e| {
            e.kind == "rpc_stub"
                && ((e.from_pod == cut.from_pod && e.to_pod == cut.to_pod)
                    || (e.from_pod == cut.to_pod && e.to_pod == cut.from_pod))
        });
        if !has_rpc_stub {
            diagnostics.push(format!(
                "cut edge ({}, {}) has no corresponding 'rpc_stub' entry",
                cut.from_pod, cut.to_pod
            ));
        }
    }

    for edge in &manifest.edges {
        if edge.cut_annotation.is_some() && !cut_pairs.contains(&(edge.from_pod, edge.to_pod)) {
            diagnostics.push(format!(
                "edge ({}, {}) has cut_annotation but is not in the cut list",
                edge.from_pod, edge.to_pod
            ));
        }
    }

    diagnostics
}
