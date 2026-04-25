//! Graph output serialization for dependency analysis results.
//!
//! Provides JSON serialization of the dependency graph, SCC analysis,
//! and deployability hints for external consumers.

use petgraph::graph::NodeIndex;
use serde::Serialize;

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::{FileNode, NodeData, SymbolNode};
use crate::graph::scc::{DeployabilityHint, SccAnalysis};

/// Complete graph analysis output for serialization.
#[derive(Debug, Clone, Serialize)]
pub struct GraphOutput {
    /// Analysis metadata
    pub metadata: GraphMetadata,
    /// Graph nodes (files and symbols)
    pub nodes: Vec<SerializedNode>,
    /// Graph edges with kinds
    pub edges: Vec<SerializedEdge>,
    /// Strongly connected components
    pub sccs: Vec<SerializedScc>,
    /// Deployability summary statistics
    pub deployability: DeployabilityStats,
}

/// Metadata about the graph analysis.
#[derive(Debug, Clone, Serialize)]
pub struct GraphMetadata {
    /// Snapshot identifier for this analysis
    pub snapshot_id: u64,
    /// Total number of nodes
    pub node_count: usize,
    /// Total number of edges
    pub edge_count: usize,
    /// Number of SCCs computed
    pub scc_count: usize,
    /// Number of file nodes
    pub file_count: usize,
    /// Number of symbol nodes
    pub symbol_count: usize,
}

/// Serialized node representation.
#[derive(Debug, Clone, Serialize)]
pub struct SerializedNode {
    /// Node index in the graph
    pub id: usize,
    /// Node kind: "file" or "symbol"
    pub kind: String,
    /// File path (for file nodes) or containing file path (for symbol nodes)
    pub path: Option<String>,
    /// Language identifier (for file nodes)
    pub language: Option<String>,
    /// Symbol name (for symbol nodes)
    pub name: Option<String>,
    /// Symbol kind (for symbol nodes)
    #[serde(rename = "symbol_kind", skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    /// Visibility (for symbol nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

/// Serialized edge representation.
#[derive(Debug, Clone, Serialize)]
pub struct SerializedEdge {
    /// Source node index
    pub source: usize,
    /// Target node index
    pub target: usize,
    /// Edge kind: "ownership", "import", or "reference"
    pub kind: String,
    /// Confidence score (0.0 - 1.0) for cross-language resolution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// Serialized SCC representation.
#[derive(Debug, Clone, Serialize)]
pub struct SerializedScc {
    /// Component index
    pub index: usize,
    /// Node indices in this component
    pub nodes: Vec<usize>,
    /// Whether this component contains cycles
    pub is_cyclic: bool,
    /// Deployability hint
    pub hint: String,
    /// Component size
    pub size: usize,
}

/// Deployability statistics summary.
#[derive(Debug, Clone, Serialize)]
pub struct DeployabilityStats {
    /// Number of cyclic clusters (size > 1 or self-loop)
    pub cyclic_clusters: usize,
    /// Number of independent deployable units (size = 1, no self-loop)
    pub independent_units: usize,
    /// Number of self-loops (size = 1 with self-loop)
    pub self_loops: usize,
    /// Total number of components
    pub total_components: usize,
}

impl GraphOutput {
    /// Create a new GraphOutput from a CodeGraph and its SCC analysis.
    pub fn from_graph(graph: &CodeGraph, scc_analysis: &SccAnalysis, snapshot_id: u64) -> Self {
        let metadata = Self::build_metadata(graph, scc_analysis, snapshot_id);
        let nodes = Self::serialize_nodes(graph);
        let edges = Self::serialize_edges(graph);
        let sccs = Self::serialize_sccs(scc_analysis);
        let deployability = Self::build_deployability_stats(scc_analysis);

        Self {
            metadata,
            nodes,
            edges,
            sccs,
            deployability,
        }
    }

    fn build_metadata(
        graph: &CodeGraph,
        scc_analysis: &SccAnalysis,
        snapshot_id: u64,
    ) -> GraphMetadata {
        let node_count = graph.graph.node_count();
        let edge_count = graph.graph.edge_count();
        let scc_count = scc_analysis.components.len();

        let mut file_count = 0;
        let mut symbol_count = 0;

        for node_data in graph.graph.node_weights() {
            match node_data {
                NodeData::File(_) => file_count += 1,
                NodeData::Symbol(_) => symbol_count += 1,
            }
        }

        GraphMetadata {
            snapshot_id,
            node_count,
            edge_count,
            scc_count,
            file_count,
            symbol_count,
        }
    }

    fn serialize_nodes(graph: &CodeGraph) -> Vec<SerializedNode> {
        graph
            .graph
            .node_indices()
            .map(|idx| {
                let node_data = &graph.graph[idx];
                Self::serialize_node(idx, node_data)
            })
            .collect()
    }

    fn serialize_node(idx: NodeIndex, node_data: &NodeData) -> SerializedNode {
        match node_data {
            NodeData::File(file_node) => Self::serialize_file_node(idx, file_node),
            NodeData::Symbol(symbol_node) => Self::serialize_symbol_node(idx, symbol_node),
        }
    }

    fn serialize_file_node(idx: NodeIndex, file_node: &FileNode) -> SerializedNode {
        SerializedNode {
            id: idx.index(),
            kind: "file".to_string(),
            path: Some(file_node.path.to_string_lossy().to_string()),
            language: Some(file_node.language.as_ref().to_string()),
            name: None,
            symbol_kind: None,
            visibility: None,
        }
    }

    fn serialize_symbol_node(idx: NodeIndex, symbol_node: &SymbolNode) -> SerializedNode {
        let visibility = symbol_node.visibility.map(|v| format!("{:?}", v));

        SerializedNode {
            id: idx.index(),
            kind: "symbol".to_string(),
            path: None,
            language: None,
            name: Some(symbol_node.name.clone()),
            symbol_kind: Some(format!("{:?}", symbol_node.kind)),
            visibility,
        }
    }

    fn serialize_edges(graph: &CodeGraph) -> Vec<SerializedEdge> {
        graph
            .graph
            .edge_indices()
            .filter_map(|edge_idx| {
                let (source, target) = graph.graph.edge_endpoints(edge_idx)?;
                let edge_data = graph.graph.edge_weight(edge_idx)?;
                let confidence = if edge_data.confidence < 1.0 {
                    Some(edge_data.confidence)
                } else {
                    None
                };

                let kind_str = match edge_data.kind {
                    EdgeKind::Ownership => "ownership",
                    EdgeKind::Import => "import",
                    EdgeKind::Reference => "reference",
                };

                Some(SerializedEdge {
                    source: source.index(),
                    target: target.index(),
                    kind: kind_str.to_string(),
                    confidence,
                })
            })
            .collect()
    }

    fn serialize_sccs(scc_analysis: &SccAnalysis) -> Vec<SerializedScc> {
        scc_analysis
            .components
            .iter()
            .map(|scc| {
                let nodes: Vec<usize> = scc.nodes.iter().map(|n| n.index()).collect();
                let hint_str = match scc.hint {
                    DeployabilityHint::Independent => "Independent",
                    DeployabilityHint::AcyclicDependency => "AcyclicDependency",
                    DeployabilityHint::CyclicCluster => "CyclicCluster",
                    DeployabilityHint::SelfLoop => "SelfLoop",
                };

                SerializedScc {
                    index: scc.index,
                    nodes,
                    is_cyclic: scc.is_cyclic,
                    hint: hint_str.to_string(),
                    size: scc.nodes.len(),
                }
            })
            .collect()
    }

    fn build_deployability_stats(scc_analysis: &SccAnalysis) -> DeployabilityStats {
        let mut cyclic_clusters = 0;
        let mut independent_units = 0;
        let mut self_loops = 0;

        for scc in &scc_analysis.components {
            match scc.hint {
                DeployabilityHint::Independent | DeployabilityHint::AcyclicDependency => {
                    independent_units += 1;
                }
                DeployabilityHint::CyclicCluster => {
                    cyclic_clusters += 1;
                }
                DeployabilityHint::SelfLoop => {
                    self_loops += 1;
                }
            }
        }

        DeployabilityStats {
            cyclic_clusters,
            independent_units,
            self_loops,
            total_components: scc_analysis.components.len(),
        }
    }
}

/// Serialize graph output to the specified format.
pub fn serialize_graph(
    graph: &CodeGraph,
    scc_analysis: &SccAnalysis,
    snapshot_id: u64,
    format: &crate::output::OutputFormat,
) -> anyhow::Result<String> {
    let output = GraphOutput::from_graph(graph, scc_analysis, snapshot_id);
    format.serialize(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::builder::GraphBuilder;
    use crate::language::LangId;
    use crate::model::{
        LineColumn, SourceRange, Symbol, SymbolId, SymbolKind, Visibility, ids::SnapshotId,
    };
    use crate::output::OutputFormat;
    use std::path::PathBuf;

    fn sample_symbol(id: u32, name: &str, kind: SymbolKind, path: &str) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            kind,
            language: LangId::Rust,
            file_path: PathBuf::from(path),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 10,
                start: LineColumn { line: 1, column: 0 },
                end: LineColumn {
                    line: 1,
                    column: 10,
                },
            },
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        }
    }

    #[test]
    fn graph_output_has_required_keys() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let _file_id = builder.add_file(PathBuf::from("src/main.rs"), LangId::Rust);
        let symbol = sample_symbol(1, "main", SymbolKind::Function, "src/main.rs");
        let _sym_idx = builder.add_symbol(&symbol);

        let graph = builder.build();
        let scc_result = SccAnalysis::analyze(&graph.graph);

        let output = GraphOutput::from_graph(&graph, &scc_result, 1);

        assert_eq!(output.metadata.file_count, 1);
        assert_eq!(output.metadata.symbol_count, 1);
        assert!(!output.nodes.is_empty());
    }

    #[test]
    fn serialized_node_kinds() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let _file_id = builder.add_file(PathBuf::from("src/lib.rs"), LangId::Rust);
        let symbol = sample_symbol(1, "lib_fn", SymbolKind::Function, "src/lib.rs");
        let _sym_idx = builder.add_symbol(&symbol);

        let graph = builder.build();
        let scc_result = SccAnalysis::analyze(&graph.graph);

        let json = serialize_graph(&graph, &scc_result, 1, &OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Verify structure
        assert!(parsed.get("metadata").is_some());
        assert!(parsed.get("nodes").is_some());
        assert!(parsed.get("edges").is_some());
        assert!(parsed.get("sccs").is_some());
        assert!(parsed.get("deployability").is_some());

        // Verify node kinds exist
        let nodes = parsed["nodes"].as_array().unwrap();
        let file_nodes: Vec<_> = nodes.iter().filter(|n| n["kind"] == "file").collect();
        let symbol_nodes: Vec<_> = nodes.iter().filter(|n| n["kind"] == "symbol").collect();

        assert_eq!(file_nodes.len(), 1);
        assert_eq!(symbol_nodes.len(), 1);
        assert!(file_nodes[0]["language"].is_string());
        assert!(symbol_nodes[0]["name"].is_string());
    }

    #[test]
    fn empty_graph_produces_empty_output() {
        let builder = GraphBuilder::new(SnapshotId(1));
        let graph = builder.build();
        let scc_result = SccAnalysis::analyze(&graph.graph);

        let output = GraphOutput::from_graph(&graph, &scc_result, 1);
        let json = serde_json::to_string(&output).unwrap();

        assert!(json.contains("\"node_count\":0"));
        assert!(json.contains("\"sccs\":[]"));
    }

    #[test]
    fn scc_serialization_contains_hint() {
        // This test verifies SCC hints are serialized correctly
        // We test through the public API that hints make it into output
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let _file_id = builder.add_file(PathBuf::from("src/a.rs"), LangId::Rust);
        let sym1 = sample_symbol(1, "func_a", SymbolKind::Function, "src/a.rs");
        let sym2 = sample_symbol(2, "func_b", SymbolKind::Function, "src/a.rs");
        builder.add_symbol(&sym1);
        builder.add_symbol(&sym2);

        // Create a cycle via reference edges between the symbols
        builder.add_reference(sym1.id, sym2.id);
        builder.add_reference(sym2.id, sym1.id);

        let graph = builder.build();
        let scc_result = SccAnalysis::analyze(&graph.graph);

        let json = serialize_graph(&graph, &scc_result, 1, &OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Find an SCC with cyclic=true
        let sccs = parsed["sccs"].as_array().unwrap();
        let cyclic_scc = sccs.iter().find(|s| s["is_cyclic"].as_bool().unwrap());

        assert!(
            cyclic_scc.is_some(),
            "Expected to find a cyclic SCC in the output"
        );
        assert_eq!(
            cyclic_scc.unwrap()["hint"],
            "CyclicCluster",
            "Cyclic SCC should have CyclicCluster hint"
        );
    }
}
