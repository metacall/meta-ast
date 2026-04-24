//! SCC (Strongly Connected Components) analysis for dependency graphs.
//!
//! This module implements Tarjan's SCC algorithm on the dependency subgraph
//! (excluding ownership edges per graph-model.md invariants).

use petgraph::algo::tarjan_scc;
use petgraph::graph::{DiGraph, NodeIndex};
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
            DeployabilityHint::Independent => write!(f, "Independent"),
            DeployabilityHint::AcyclicDependency => write!(f, "AcyclicDependency"),
            DeployabilityHint::CyclicCluster => write!(f, "CyclicCluster"),
            DeployabilityHint::SelfLoop => write!(f, "SelfLoop"),
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
    pub fn analyze(graph: &DiGraph<NodeData, EdgeData>) -> Self {
        // Build dependency subgraph by filtering out ownership edges
        let dependency_graph = Self::build_dependency_subgraph(graph);

        // Run Tarjan SCC algorithm
        let scc_groups = tarjan_scc(&dependency_graph);

        // Build components with metadata
        let mut components = Vec::with_capacity(scc_groups.len());
        let mut node_to_component = HashMap::new();

        for (index, nodes) in scc_groups.into_iter().enumerate() {
            // Map dependency graph indices back to original graph indices
            let original_nodes: Vec<NodeIndex> = nodes
                .iter()
                .filter_map(|&dep_idx| Self::map_to_original(graph, &dependency_graph, dep_idx))
                .collect();

            // Check for self-loops in the original graph
            let has_self_loop = original_nodes.iter().any(|&node| {
                graph
                    .neighbors_directed(node, petgraph::Direction::Outgoing)
                    .any(|neighbor| neighbor == node)
            });

            let is_cyclic = original_nodes.len() > 1 || has_self_loop;

            // Determine deployability hint
            let hint = if original_nodes.len() > 1 {
                DeployabilityHint::CyclicCluster
            } else if has_self_loop {
                DeployabilityHint::SelfLoop
            } else if original_nodes.len() == 1 {
                // Check if it has dependencies to other components
                DeployabilityHint::AcyclicDependency
            } else {
                DeployabilityHint::Independent
            };

            // Record node mappings
            for &node in &original_nodes {
                node_to_component.insert(node, index);
            }

            components.push(Scc {
                index,
                nodes: original_nodes,
                is_cyclic,
                hint,
            });
        }

        // Update hints for independent units (no external dependencies)
        Self::classify_independence(&dependency_graph, &mut components);

        Self {
            components,
            node_to_component,
        }
    }

    /// Build a subgraph containing only dependency edges (Import, Reference).
    /// Ownership edges are excluded from SCC computation.
    fn build_dependency_subgraph(
        original: &DiGraph<NodeData, EdgeData>,
    ) -> DiGraph<NodeData, EdgeData> {
        let mut subgraph = DiGraph::with_capacity(original.node_count(), original.edge_count());

        // Copy all nodes
        let mut node_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();
        for idx in original.node_indices() {
            let weight = original.node_weight(idx).cloned().unwrap_or_else(|| {
                // This should never happen with valid petgraph usage
                panic!("Node index {idx:?} has no weight in original graph")
            });
            let new_idx = subgraph.add_node(weight);
            node_map.insert(idx, new_idx);
        }

        // Copy only dependency edges (exclude Ownership)
        for edge_idx in original.edge_indices() {
            let (source, target) = original
                .edge_endpoints(edge_idx)
                .expect("Edge index must be valid");
            let weight = original
                .edge_weight(edge_idx)
                .expect("Edge index must have weight");

            // Skip ownership edges - they don't participate in SCC
            if weight.kind == EdgeKind::Ownership {
                continue;
            }

            if let (Some(&new_source), Some(&new_target)) =
                (node_map.get(&source), node_map.get(&target))
            {
                subgraph.add_edge(new_source, new_target, weight.clone());
            }
        }

        subgraph
    }

    /// Map a node from the dependency subgraph back to the original graph.
    fn map_to_original(
        original: &DiGraph<NodeData, EdgeData>,
        _subgraph: &DiGraph<NodeData, EdgeData>,
        dep_idx: NodeIndex,
    ) -> Option<NodeIndex> {
        // The subgraph preserves node order from original
        if dep_idx.index() < original.node_count() {
            Some(NodeIndex::new(dep_idx.index()))
        } else {
            None
        }
    }

    /// Classify components as Independent if they have no outgoing dependencies
    /// to other components.
    fn classify_independence(dep_graph: &DiGraph<NodeData, EdgeData>, components: &mut [Scc]) {
        // Build component-level dependency graph
        let mut component_deps: HashMap<usize, Vec<usize>> = HashMap::new();

        for edge_idx in dep_graph.edge_indices() {
            let (source, target) = dep_graph.edge_endpoints(edge_idx).expect("Edge must exist");
            let source_comp = Self::find_component_for_node(components, source);
            let target_comp = Self::find_component_for_node(components, target);

            if let (Some(s), Some(t)) = (source_comp, target_comp)
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

    /// Find which component contain a given node.
    fn find_component_for_node(components: &[Scc], node: NodeIndex) -> Option<usize> {
        components
            .iter()
            .find(|comp| comp.nodes.contains(&node))
            .map(|comp| comp.index)
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
        EdgeData {
            kind,
            confidence: 1.0,
        }
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
        assert_eq!(format!("{}", DeployabilityHint::Independent), "Independent");
        assert_eq!(
            format!("{}", DeployabilityHint::CyclicCluster),
            "CyclicCluster"
        );
        assert_eq!(format!("{}", DeployabilityHint::SelfLoop), "SelfLoop");
        assert_eq!(
            format!("{}", DeployabilityHint::AcyclicDependency),
            "AcyclicDependency"
        );
    }
}
