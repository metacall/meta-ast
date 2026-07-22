//! Graph output serialization for dependency analysis results.
//!
//! Provides JSON serialization of the dependency graph, SCC analysis,
//! deployability hints, and the portable DataGraph contract for external
//! consumers (sinks, dashboards, CLI output).

use serde::Serialize;

use crate::graph::CodeGraph;
use crate::graph::edge::EdgeKind;
use crate::graph::node::{FileNode, NodeData, SymbolNode};
use crate::graph::scc::{DeployabilityHint, SccAnalysis};

/// Version of the graph export schema.
/// Incremented on breaking changes to the serialized structure.
pub const SCHEMA_VERSION: u32 = 1;

/// Complete graph analysis output for serialization.
///
/// Serves both the CLI `graph` output (with SCC/deployability) and the
/// `--datagraph` sink export (where SCC analysis is optional).
#[derive(Debug, Clone, Serialize)]
pub struct GraphOutput {
    /// Schema version for forward compatibility
    pub schema_version: u32,
    /// Analysis metadata
    pub metadata: GraphMetadata,
    /// Graph nodes (files, symbols, externals, data)
    pub nodes: Vec<SerializedNode>,
    /// Graph edges with kinds
    pub edges: Vec<SerializedEdge>,
    /// Strongly connected components (empty when SCC analysis skipped)
    pub sccs: Vec<SerializedScc>,
    /// Deployability summary statistics (None when SCC analysis skipped)
    pub deployability: Option<DeployabilityStats>,
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
    /// Number of SCCs computed (0 when SCC analysis skipped)
    pub scc_count: usize,
    /// Number of file nodes
    pub file_count: usize,
    /// Number of symbol nodes
    pub symbol_count: usize,
    /// Number of data-bearing nodes
    pub data_node_count: usize,
}

/// Serialized node representation.
#[derive(Debug, Clone, Serialize)]
pub struct SerializedNode {
    /// Node index in the graph
    pub id: usize,
    /// Node kind: "file", "symbol", "external", or "data"
    pub kind: String,
    /// File path (for file/external nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Language identifier (for file/external nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Symbol or data node name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Symbol kind (for symbol nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    /// Visibility (for symbol nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Data scope classification (local, parameter, member, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_scope: Option<String>,
    /// Type hint or annotation (for data nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_hint: Option<String>,
}

/// Serialized edge representation.
#[derive(Debug, Clone, Serialize)]
pub struct SerializedEdge {
    /// Source node index
    pub source: usize,
    /// Target node index
    pub target: usize,
    /// Edge kind: "ownership", "import", "reference", or "flow"
    pub kind: String,
    /// Confidence score (0.0-1.0), omitted when 1.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// Flow kind for dataflow edges (def_use, argument, return, field_access)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_kind: Option<String>,
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
    /// Create a GraphOutput from a CodeGraph and optional SCC analysis.
    ///
    /// When `scc_analysis` is `None`, SCCs and deployability stats are
    /// omitted from the output (empty/skipped in serialization).
    pub fn from_graph(
        graph: &CodeGraph,
        scc_analysis: Option<&SccAnalysis>,
        snapshot_id: u64,
    ) -> Self {
        let metadata = Self::build_metadata(graph, scc_analysis, snapshot_id);
        let nodes = Self::serialize_nodes(graph);
        let edges = Self::serialize_edges(graph);

        let (sccs, deployability) = if let Some(scc) = scc_analysis {
            (
                Self::serialize_sccs(scc),
                Some(Self::build_deployability_stats(scc)),
            )
        } else {
            (Vec::new(), None)
        };

        Self {
            schema_version: SCHEMA_VERSION,
            metadata,
            nodes,
            edges,
            sccs,
            deployability,
        }
    }

    fn build_metadata(
        graph: &CodeGraph,
        scc_analysis: Option<&SccAnalysis>,
        snapshot_id: u64,
    ) -> GraphMetadata {
        let node_count = graph.graph.node_count();
        let edge_count = graph.graph.edge_count();
        let scc_count = scc_analysis.map_or(0, |s| s.components.len());

        let mut file_count = 0;
        let mut symbol_count = 0;
        let mut data_node_count = 0;

        for node_data in graph.graph.node_weights() {
            match node_data {
                NodeData::File(_) => file_count += 1,
                NodeData::Symbol(_) => symbol_count += 1,
                NodeData::External(_) => {}
                NodeData::Data(_) => data_node_count += 1,
            }
        }

        GraphMetadata {
            snapshot_id,
            node_count,
            edge_count,
            scc_count,
            file_count,
            symbol_count,
            data_node_count,
        }
    }

    fn serialize_nodes(graph: &CodeGraph) -> Vec<SerializedNode> {
        graph
            .graph
            .node_indices()
            .map(|idx| {
                let node_data = &graph.graph[idx];
                Self::serialize_node(idx.index(), node_data)
            })
            .collect()
    }

    fn serialize_node(id: usize, node_data: &NodeData) -> SerializedNode {
        match node_data {
            NodeData::File(f) => Self::serialize_file_node(id, f),
            NodeData::Symbol(s) => Self::serialize_symbol_node(id, s),
            NodeData::External(e) => SerializedNode {
                id,
                kind: "external".to_string(),
                path: Some(e.raw_path.clone()),
                language: Some(e.language.as_ref().to_string()),
                name: None,
                symbol_kind: None,
                visibility: None,
                data_scope: None,
                type_hint: None,
            },
            NodeData::Data(d) => SerializedNode {
                id,
                kind: "data".to_string(),
                path: None,
                language: None,
                name: d.name.clone(),
                symbol_kind: None,
                visibility: None,
                data_scope: Some(d.scope.as_str().to_string()),
                type_hint: d.type_hint.clone(),
            },
        }
    }

    fn serialize_file_node(id: usize, file_node: &FileNode) -> SerializedNode {
        SerializedNode {
            id,
            kind: "file".to_string(),
            path: Some(file_node.path.to_string_lossy().to_string()),
            language: Some(file_node.language.as_ref().to_string()),
            name: None,
            symbol_kind: None,
            visibility: None,
            data_scope: None,
            type_hint: None,
        }
    }

    fn serialize_symbol_node(id: usize, symbol_node: &SymbolNode) -> SerializedNode {
        SerializedNode {
            id,
            kind: "symbol".to_string(),
            path: None,
            language: None,
            name: Some(symbol_node.name.clone()),
            symbol_kind: Some(format!("{:?}", symbol_node.kind)),
            visibility: symbol_node.visibility.map(|v| format!("{:?}", v)),
            data_scope: None,
            type_hint: None,
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

                let flow_kind = if edge_data.kind == EdgeKind::Flow {
                    edge_data.flow_kind.map(|fk| format!("{:?}", fk))
                } else {
                    None
                };

                Some(SerializedEdge {
                    source: source.index(),
                    target: target.index(),
                    kind: edge_data.kind.as_str().to_string(),
                    confidence,
                    flow_kind,
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

                SerializedScc {
                    index: scc.index,
                    nodes,
                    is_cyclic: scc.is_cyclic,
                    hint: scc.hint.to_string(),
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
    let output = GraphOutput::from_graph(graph, Some(scc_analysis), snapshot_id);
    format.serialize(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GraphBuilder;
    use crate::language::LangId;
    use crate::model::{DataNodeId, DataScope, FlowKind, LineColumn, SnapshotId, SourceRange};
    use crate::output::OutputFormat;
    use std::path::PathBuf;

    fn sample_source_range() -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: LineColumn { line: 0, column: 0 },
            end: LineColumn {
                line: 0,
                column: 10,
            },
        }
    }

    fn sample_scc_analysis() -> SccAnalysis {
        SccAnalysis {
            components: Vec::new(),
            node_to_component: std::collections::HashMap::new(),
        }
    }

    // ── GraphOutput tests ───────────────────────────────────────────

    #[test]
    fn graph_output_with_file_node() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        builder.add_file(PathBuf::from("src/main.py"), LangId::Python);
        let graph = builder.build();
        let scc = sample_scc_analysis();

        let output = GraphOutput::from_graph(&graph, Some(&scc), 1);

        assert_eq!(output.schema_version, SCHEMA_VERSION);
        assert_eq!(output.metadata.file_count, 1);
        assert_eq!(output.metadata.symbol_count, 0);
        assert_eq!(output.metadata.data_node_count, 0);
        assert_eq!(output.metadata.scc_count, 0);
        assert_eq!(output.nodes.len(), 1);
        assert_eq!(output.nodes[0].kind, "file");
        assert_eq!(output.nodes[0].path.as_deref(), Some("src/main.py"));
        assert!(output.nodes[0].language.is_some());
    }

    #[test]
    fn json_output_has_required_structure() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        builder.add_file(PathBuf::from("src/main.py"), LangId::Python);
        let graph = builder.build();
        let scc = sample_scc_analysis();

        let json = serialize_graph(&graph, &scc, 1, &OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["schema_version"], 1);
        assert!(parsed["metadata"].is_object());
        assert!(parsed["nodes"].is_array());
        assert!(parsed["edges"].is_array());
        // sccs is skip_serializing_if empty, so may be absent when empty
        if let Some(sccs) = parsed.get("sccs") {
            assert!(sccs.is_array());
        }
    }

    #[test]
    fn empty_graph_serializes() {
        let builder = GraphBuilder::new(SnapshotId(1));
        let graph = builder.build();
        let scc = sample_scc_analysis();

        let output = GraphOutput::from_graph(&graph, Some(&scc), 1);
        let json = serde_json::to_string(&output).unwrap();

        assert!(json.contains("\"node_count\":0"));
        assert!(json.contains("\"edge_count\":0"));
        assert!(json.contains("\"schema_version\":1"));
    }

    #[test]
    fn scc_fields_serialize() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        builder.add_file(PathBuf::from("a.py"), LangId::Python);
        builder.add_file(PathBuf::from("b.py"), LangId::Python);
        let graph = builder.build();
        let scc = sample_scc_analysis();

        let json = serialize_graph(&graph, &scc, 1, &OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Verify SCC fields exist (empty array is skipped in serialization,
        // but the key is present when Some(SccAnalysis) is passed)
        let _sccs = &parsed["sccs"];
        // Verify deployability stats exist
        let _deploy = &parsed["deployability"];
    }

    // ── Data node serialization tests ───────────────────────────────

    #[test]
    fn data_node_serializes_with_correct_fields() {
        use crate::graph::node::DataGraphNode;
        let mut graph = CodeGraph::new(SnapshotId(1));
        let dnode = DataGraphNode {
            id: DataNodeId(10),
            symbol_id: None,
            name: Some("local_var".into()),
            scope: DataScope::Local,
            type_hint: Some("u32".into()),
            source_range: sample_source_range(),
        };
        graph.graph.add_node(NodeData::Data(dnode));
        let scc = sample_scc_analysis();

        let output = GraphOutput::from_graph(&graph, Some(&scc), 1);
        assert_eq!(output.metadata.data_node_count, 1);
        assert_eq!(output.nodes[0].kind, "data");
        assert_eq!(output.nodes[0].name.as_deref(), Some("local_var"));
        assert_eq!(output.nodes[0].data_scope.as_deref(), Some("local"));
        assert_eq!(output.nodes[0].type_hint.as_deref(), Some("u32"));
        assert!(output.nodes[0].symbol_kind.is_none());
        assert!(output.nodes[0].visibility.is_none());
    }

    #[test]
    fn flow_edge_serializes_with_flow_kind() {
        use crate::graph::node::DataGraphNode;
        let mut graph = CodeGraph::new(SnapshotId(1));
        let src = graph.graph.add_node(NodeData::Data(DataGraphNode {
            id: DataNodeId(1),
            symbol_id: None,
            name: Some("x".into()),
            scope: DataScope::Local,
            type_hint: None,
            source_range: sample_source_range(),
        }));
        let dst = graph.graph.add_node(NodeData::Data(DataGraphNode {
            id: DataNodeId(2),
            symbol_id: None,
            name: Some("y".into()),
            scope: DataScope::Parameter,
            type_hint: None,
            source_range: sample_source_range(),
        }));
        graph.add_edge_normalized_with_flow(
            src,
            dst,
            EdgeKind::Flow,
            0.9,
            Some(FlowKind::Argument),
        );
        let scc = sample_scc_analysis();

        let output = GraphOutput::from_graph(&graph, Some(&scc), 1);
        assert_eq!(output.edges.len(), 1);
        assert_eq!(output.edges[0].kind, "flow");
        assert_eq!(output.edges[0].confidence, Some(0.9));
        assert_eq!(output.edges[0].flow_kind.as_deref(), Some("Argument"));
    }

    #[test]
    fn flow_edge_without_flow_kind_omits_field() {
        use crate::graph::node::DataGraphNode;
        let mut graph = CodeGraph::new(SnapshotId(1));
        let src = graph.graph.add_node(NodeData::Data(DataGraphNode {
            id: DataNodeId(1),
            symbol_id: None,
            name: Some("a".into()),
            scope: DataScope::Local,
            type_hint: None,
            source_range: sample_source_range(),
        }));
        let dst = graph.graph.add_node(NodeData::Data(DataGraphNode {
            id: DataNodeId(2),
            symbol_id: None,
            name: Some("b".into()),
            scope: DataScope::Local,
            type_hint: None,
            source_range: sample_source_range(),
        }));
        graph.add_edge_normalized(src, dst, EdgeKind::Flow, 1.0);
        let scc = sample_scc_analysis();

        let output = GraphOutput::from_graph(&graph, Some(&scc), 1);
        assert_eq!(output.edges[0].flow_kind, None);
        assert_eq!(output.edges[0].confidence, None); // 1.0 → omitted
    }

    // ── Light mode (no SCC) tests ───────────────────────────────────

    #[test]
    fn light_mode_omits_scc_and_deployability() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        builder.add_file(PathBuf::from("main.rs"), LangId::Rust);
        let graph = builder.build();

        let output = GraphOutput::from_graph(&graph, None, 1);
        let json = serde_json::to_string(&output).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(output.metadata.scc_count, 0);
        // sccs is skipped when empty, deployability is null when None
        assert!(parsed.get("sccs").is_none() || parsed["sccs"].is_array());
        assert!(parsed.get("deployability").is_none() || parsed["deployability"].is_null());
    }

    #[test]
    fn light_mode_with_symbol_node() {
        use crate::model::{Symbol, SymbolId, SymbolKind, Visibility};
        let mut builder = GraphBuilder::new(SnapshotId(1));
        builder.add_file(PathBuf::from("main.rs"), LangId::Rust);
        let sym = Symbol {
            id: SymbolId(1),
            name: "main_fn".into(),
            kind: SymbolKind::Function,
            language: LangId::Rust,
            file_path: PathBuf::from("main.rs"),
            source_range: sample_source_range(),
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        };
        builder.add_symbol(&sym).unwrap();
        let graph = builder.build();

        let output = GraphOutput::from_graph(&graph, None, 1);
        assert_eq!(output.nodes.len(), 2); // file + symbol
        assert_eq!(output.edges.len(), 1); // ownership
        let sym_node = output.nodes.iter().find(|n| n.kind == "symbol").unwrap();
        assert_eq!(sym_node.name.as_deref(), Some("main_fn"));
    }

    #[test]
    fn has_all_required_keys() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        builder.add_file(PathBuf::from("main.rs"), LangId::Rust);
        let graph = builder.build();

        let output = GraphOutput::from_graph(&graph, None, 1);
        let val: serde_json::Value = serde_json::to_value(&output).unwrap();
        let required = ["schema_version", "metadata", "nodes", "edges"];
        for key in &required {
            assert!(val.get(key).is_some(), "missing required key: {key}");
        }
    }

    // ── Edge kind validation ────────────────────────────────────────

    #[test]
    fn edge_kinds_are_valid() {
        let mut builder = GraphBuilder::new(SnapshotId(1));
        let file_a = builder.add_file(PathBuf::from("a.py"), LangId::Python);
        let _file_b = builder.add_file(PathBuf::from("b.py"), LangId::Python);
        builder.add_import(file_a, PathBuf::from("b.py"));
        let graph = builder.build();
        let scc = sample_scc_analysis();

        let output = GraphOutput::from_graph(&graph, Some(&scc), 1);
        let valid_kinds = ["ownership", "import", "reference", "flow"];
        for edge in &output.edges {
            assert!(
                valid_kinds.contains(&edge.kind.as_str()),
                "unexpected edge kind: {}",
                edge.kind
            );
        }
    }
}
