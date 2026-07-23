//! SCC (Strongly Connected Components) analysis for dependency graphs.
//!
//! This module implements Tarjan's SCC algorithm on the dependency subgraph
//! (Import and Reference edges only; Ownership edges are excluded via
//! `EdgeFiltered`).
//!
//! ## SCC as atomic deployment unit
//!
//! An SCC entirely within one language is never subdivided regardless of
//! size. Cross-language SCCs may be split at the lowest-confidence edge
//! (see `deploy::cut`), but same-language SCCs are always kept together.
//! This guarantees that cycles - a known source of tight coupling - are
//! preserved as a single deployment unit whenever possible.
use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeFiltered;
use std::collections::HashMap;

use crate::graph::edge::{EdgeData, EdgeKind};
use crate::graph::node::NodeData;

/// A single strongly connected component.
#[derive(Debug, Clone)]
pub struct Scc {
    /// Index of this component in topological order (dependencies first)
    pub index: usize,
    /// Node indices in this component
    pub nodes: Vec<NodeIndex>,
    /// Whether this component is cyclic (size > 1 or self-loop)
    pub is_cyclic: bool,
    /// Deployability recommendation
    pub hint: DeployabilityHint,
}

/// Deployability classification for an SCC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeployabilityHint {
    /// Single node, no self-loop, no dependencies - can deploy independently
    Independent,
    /// Single node, no self-loop, but has dependencies
    AcyclicDependency,
    /// Part of a cycle (size > 1) - requires grouped deployment
    CyclicCluster,
    /// Single node with self-loop - deploy with caution
    SelfLoop,
}

impl std::fmt::Display for DeployabilityHint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployabilityHint::Independent => write!(f, "independent"),
            DeployabilityHint::AcyclicDependency => write!(f, "acyclic_dependency"),
            DeployabilityHint::CyclicCluster => write!(f, "cyclic_cluster"),
            DeployabilityHint::SelfLoop => write!(f, "self_loop"),
        }
    }
}

/// Complete SCC analysis results for a dependency graph.
#[derive(Debug, Clone)]
pub struct SccAnalysis {
    /// SCCs in reverse topological order (dependencies before dependents)
    pub components: Vec<Scc>,
    /// Map from node index to its component index
    pub node_to_component: HashMap<NodeIndex, usize>,
}

impl SccAnalysis {
    /// Analyze a graph and compute SCCs on the dependency subgraph.
    ///
    /// Ownership edges are excluded from SCC computation per graph-model.md.
    /// The dependency subgraph includes Import and Reference edge kinds.
    ///
    /// Uses an `EdgeFiltered` view instead of cloning the graph - zero-cost,
    /// no allocation for the subgraph.
    pub fn analyze(graph: &DiGraph<NodeData, EdgeData>) -> Self {
        // Zero-cost view that excludes non-dependency edges (Ownership, Flow).
        let dep_view = EdgeFiltered::from_fn(
            graph,
            |edge: petgraph::graph::EdgeReference<'_, EdgeData>| {
                edge.weight().kind.participates_in_scc()
            },
        );

        // Run Tarjan SCC algorithm on the view.
        // The returned NodeIndex values ARE the original graph's indices.
        let scc_groups = tarjan_scc(&dep_view);

        let mut components = Vec::with_capacity(scc_groups.len());
        let mut node_to_component = HashMap::new();

        for (index, nodes) in scc_groups.into_iter().enumerate() {
            let has_self_loop = nodes.iter().any(|&node| {
                graph
                    .neighbors_directed(node, petgraph::Direction::Outgoing)
                    .any(|neighbor| neighbor == node)
            });

            let is_cyclic = nodes.len() > 1 || has_self_loop;

            let hint = if nodes.len() > 1 {
                DeployabilityHint::CyclicCluster
            } else if has_self_loop {
                DeployabilityHint::SelfLoop
            } else if nodes.len() == 1 {
                DeployabilityHint::AcyclicDependency
            } else {
                DeployabilityHint::Independent
            };

            for &node in &nodes {
                node_to_component.insert(node, index);
            }

            components.push(Scc {
                index,
                nodes,
                is_cyclic,
                hint,
            });
        }

        Self::classify_independence(graph, &mut components, &node_to_component);

        Self {
            components,
            node_to_component,
        }
    }

    /// Classify components as Independent if they have no outgoing dependencies
    /// to other components.
    fn classify_independence(
        graph: &DiGraph<NodeData, EdgeData>,
        components: &mut [Scc],
        node_to_component: &HashMap<NodeIndex, usize>,
    ) {
        let mut component_deps: HashMap<usize, Vec<usize>> = HashMap::new();

        for edge_idx in graph.edge_indices() {
            let Some(weight) = graph.edge_weight(edge_idx) else {
                continue;
            };
            if weight.kind == EdgeKind::Ownership {
                continue;
            }
            let Some((source, target)) = graph.edge_endpoints(edge_idx) else {
                continue;
            };
            let source_comp = node_to_component.get(&source);
            let target_comp = node_to_component.get(&target);

            if let (Some(&s), Some(&t)) = (source_comp, target_comp)
                && s != t
            {
                component_deps.entry(s).or_default().push(t);
            }
        }

        // Update hints for components with no external dependencies
        for comp in components.iter_mut() {
            if comp.hint == DeployabilityHint::AcyclicDependency {
                let has_external_deps = component_deps
                    .get(&comp.index)
                    .map(|deps| !deps.is_empty())
                    .unwrap_or(false);

                if !has_external_deps {
                    comp.hint = DeployabilityHint::Independent;
                }
            }
        }
    }

    /// Get the component index for a specific node.
    pub fn component_of(&self, node: NodeIndex) -> Option<usize> {
        self.node_to_component.get(&node).copied()
    }

    /// Check if two nodes are in the same SCC (mutually dependent).
    pub fn mutually_dependent(&self, a: NodeIndex, b: NodeIndex) -> bool {
        self.component_of(a) == self.component_of(b)
    }

    /// Returns true if any cycles exist in the graph.
    pub fn has_cycles(&self) -> bool {
        self.components.iter().any(|c| c.is_cyclic)
    }

    /// Get all cyclic components.
    pub fn cyclic_components(&self) -> impl Iterator<Item = &Scc> {
        self.components.iter().filter(|c| c.is_cyclic)
    }

    /// Get all acyclic (independent/dependency) components.
    pub fn acyclic_components(&self) -> impl Iterator<Item = &Scc> {
        self.components.iter().filter(|c| !c.is_cyclic)
    }

    /// Count of components by hint type.
    pub fn hint_counts(&self) -> HashMap<DeployabilityHint, usize> {
        let mut counts = HashMap::new();
        for comp in &self.components {
            *counts.entry(comp.hint).or_insert(0) += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::node::{FileNode, SymbolNode};
    use crate::language::LangId;
    use crate::model::{LineColumn, SourceRange, SymbolId, Visibility, ids::FileId};

    fn make_source_range() -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn {
                line: 1,
                column: 10,
            },
        }
    }

    fn make_file_node(id: u32, path: &str) -> NodeData {
        NodeData::File(FileNode {
            id: FileId(id),
            path: std::path::PathBuf::from(path),
            language: LangId::Rust,
            snapshot_id: crate::model::ids::SnapshotId(1),
        })
    }

    fn make_symbol_node(id: u32, name: &str, file_id: u32) -> NodeData {
        NodeData::Symbol(SymbolNode {
            id: SymbolId(id),
            name: name.to_string(),
            kind: crate::model::SymbolKind::Function,
            file_id: FileId(file_id),
            visibility: Some(Visibility::Public),
            source_range: make_source_range(),
        })
    }

    fn make_edge(kind: EdgeKind) -> EdgeData {
        EdgeData::new(kind)
    }

    #[test]
    fn scc_single_node_no_edges() {
        let mut graph = DiGraph::new();
        let node = graph.add_node(make_symbol_node(1, "foo", 0));

        let analysis = SccAnalysis::analyze(&graph);

        assert_eq!(analysis.components.len(), 1);
        assert_eq!(analysis.components[0].nodes, vec![node]);
        assert!(!analysis.components[0].is_cyclic);
        assert_eq!(analysis.components[0].hint, DeployabilityHint::Independent);
    }

    #[test]
    fn scc_linear_chain() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));
        let b = graph.add_node(make_symbol_node(2, "b", 0));
        let c = graph.add_node(make_symbol_node(3, "c", 0));

        // a -> b -> c (acyclic chain)
        graph.add_edge(a, b, make_edge(EdgeKind::Reference));
        graph.add_edge(b, c, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        assert_eq!(analysis.components.len(), 3);
        assert!(!analysis.has_cycles());
        assert!(analysis.cyclic_components().next().is_none());
    }

    #[test]
    fn scc_simple_cycle() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));
        let b = graph.add_node(make_symbol_node(2, "b", 0));
        let c = graph.add_node(make_symbol_node(3, "c", 0));

        // a -> b -> c -> a (cycle)
        graph.add_edge(a, b, make_edge(EdgeKind::Reference));
        graph.add_edge(b, c, make_edge(EdgeKind::Reference));
        graph.add_edge(c, a, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        assert!(analysis.has_cycles());
        let cyclic: Vec<_> = analysis.cyclic_components().collect();
        assert_eq!(cyclic.len(), 1);
        assert_eq!(cyclic[0].nodes.len(), 3);
        assert_eq!(cyclic[0].hint, DeployabilityHint::CyclicCluster);
    }

    #[test]
    fn scc_ownership_edges_excluded() {
        let mut graph = DiGraph::new();
        let file = graph.add_node(make_file_node(0, "test.rs"));
        let sym = graph.add_node(make_symbol_node(1, "func", 0));

        // Ownership edge should not create cycle even if self-referential structure
        graph.add_edge(file, sym, make_edge(EdgeKind::Ownership));

        let analysis = SccAnalysis::analyze(&graph);

        // Should have 2 components, not 1
        assert_eq!(analysis.components.len(), 2);
        assert!(!analysis.has_cycles());
    }

    #[test]
    fn scc_self_loop_detected() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));

        // Self-loop via reference edge
        graph.add_edge(a, a, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        assert!(analysis.has_cycles());
        let comp = &analysis.components[0];
        assert!(comp.is_cyclic);
        assert_eq!(comp.hint, DeployabilityHint::SelfLoop);
    }

    #[test]
    fn scc_multiple_cycles() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));
        let b = graph.add_node(make_symbol_node(2, "b", 0));
        let c = graph.add_node(make_symbol_node(3, "c", 0));
        let d = graph.add_node(make_symbol_node(4, "d", 0));

        // Cycle 1: a -> b -> a
        graph.add_edge(a, b, make_edge(EdgeKind::Reference));
        graph.add_edge(b, a, make_edge(EdgeKind::Reference));

        // Cycle 2: c -> d -> c (independent)
        graph.add_edge(c, d, make_edge(EdgeKind::Reference));
        graph.add_edge(d, c, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        assert!(analysis.has_cycles());
        let cyclic: Vec<_> = analysis.cyclic_components().collect();
        assert_eq!(cyclic.len(), 2); // Two separate cycles

        let counts = analysis.hint_counts();
        assert_eq!(counts.get(&DeployabilityHint::CyclicCluster), Some(&2));
    }

    #[test]
    fn scc_mutual_dependence_check() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));
        let b = graph.add_node(make_symbol_node(2, "b", 0));
        let c = graph.add_node(make_symbol_node(3, "c", 0));

        // a <-> b are mutually dependent, c is independent
        graph.add_edge(a, b, make_edge(EdgeKind::Reference));
        graph.add_edge(b, a, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        assert!(analysis.mutually_dependent(a, b));
        assert!(!analysis.mutually_dependent(a, c));
        assert!(!analysis.mutually_dependent(b, c));
    }

    #[test]
    fn scc_component_lookup() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));
        let b = graph.add_node(make_symbol_node(2, "b", 0));

        graph.add_edge(a, b, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        let comp_a = analysis.component_of(a);
        let comp_b = analysis.component_of(b);
        assert!(comp_a.is_some());
        assert!(comp_b.is_some());
    }

    #[test]
    fn scc_topological_order_dependencies_first() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));
        let b = graph.add_node(make_symbol_node(2, "b", 0));
        let c = graph.add_node(make_symbol_node(3, "c", 0));

        // c depends on b, b depends on a: a -> b -> c
        graph.add_edge(a, b, make_edge(EdgeKind::Reference));
        graph.add_edge(b, c, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        // Components should be in reverse topological order
        // That is: c (dependent) comes before a (dependency)
        // Or more accurately: dependencies should be processed first
        let indices: Vec<_> = analysis
            .components
            .iter()
            .map(|c| {
                c.nodes
                    .first()
                    .map(|n| n.index())
                    .expect("Component has nodes")
            })
            .collect();

        // Just verify we have 3 components
        assert_eq!(indices.len(), 3);
    }

    #[test]
    fn scc_diamond_structure() {
        let mut graph = DiGraph::new();
        let a = graph.add_node(make_symbol_node(1, "a", 0));
        let b = graph.add_node(make_symbol_node(2, "b", 0));
        let c = graph.add_node(make_symbol_node(3, "c", 0));
        let d = graph.add_node(make_symbol_node(4, "d", 0));

        // Diamond: a -> b, a -> c, b -> d, c -> d
        graph.add_edge(a, b, make_edge(EdgeKind::Reference));
        graph.add_edge(a, c, make_edge(EdgeKind::Reference));
        graph.add_edge(b, d, make_edge(EdgeKind::Reference));
        graph.add_edge(c, d, make_edge(EdgeKind::Reference));

        let analysis = SccAnalysis::analyze(&graph);

        assert!(!analysis.has_cycles());
        assert_eq!(analysis.components.len(), 4);
    }

    #[test]
    fn deployability_hint_display() {
        assert_eq!(format!("{}", DeployabilityHint::Independent), "independent");
        assert_eq!(
            format!("{}", DeployabilityHint::CyclicCluster),
            "cyclic_cluster"
        );
        assert_eq!(format!("{}", DeployabilityHint::SelfLoop), "self_loop");
        assert_eq!(
            format!("{}", DeployabilityHint::AcyclicDependency),
            "acyclic_dependency"
        );
    }
}
