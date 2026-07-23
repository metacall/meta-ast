//! Integration tests for the datagraph pipeline.
//!
//! Validates GraphOutput serialization, sink adapters,
//! and the full pipeline with dataflow extraction on fixtures.

use std::path::Path;

use meta_ast::graph::GraphBuilder;
use meta_ast::model::SnapshotId;
use meta_ast::output::graph::{GraphOutput, SCHEMA_VERSION};

#[test]
fn datagraph_export_from_pipeline() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(!files.is_empty());

    let result = meta_ast::extractor::extract(&files);
    let mut diags = Vec::new();
    let (graph, _scc) =
        GraphBuilder::from_extractions(&result.files, root, SnapshotId(1), &mut diags);

    let export = GraphOutput::from_graph(&graph, None, 1);
    assert_eq!(export.schema_version, SCHEMA_VERSION);
    assert!(
        export.metadata.node_count > 0,
        "datagraph should contain nodes"
    );
    assert!(
        export.metadata.edge_count > 0,
        "datagraph should contain ownership edges"
    );
    assert_eq!(export.nodes.len(), export.metadata.node_count);
    assert_eq!(export.edges.len(), export.metadata.edge_count);
}

#[test]
fn datagraph_export_file_node_fields() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let mut diags = Vec::new();
    let (graph, _scc) =
        GraphBuilder::from_extractions(&result.files, root, SnapshotId(1), &mut diags);

    let export = GraphOutput::from_graph(&graph, None, 1);
    let file_nodes: Vec<_> = export.nodes.iter().filter(|n| n.kind == "file").collect();
    assert!(!file_nodes.is_empty(), "should have file nodes");
    for node in file_nodes {
        assert!(node.path.is_some(), "file node must have path");
        assert!(node.language.is_some(), "file node must have language");
        assert!(node.name.is_none(), "file node should not have name");
        assert!(
            node.data_scope.is_none(),
            "file node should not have data_scope"
        );
    }
}

#[test]
fn datagraph_export_symbol_node_fields() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let mut diags = Vec::new();
    let (graph, _scc) =
        GraphBuilder::from_extractions(&result.files, root, SnapshotId(1), &mut diags);

    let export = GraphOutput::from_graph(&graph, None, 1);
    let symbol_nodes: Vec<_> = export.nodes.iter().filter(|n| n.kind == "symbol").collect();
    assert!(!symbol_nodes.is_empty(), "should have symbol nodes");
    for node in symbol_nodes {
        assert!(node.name.is_some(), "symbol node must have name");
        assert!(
            node.symbol_kind.is_some(),
            "symbol node must have symbol_kind"
        );
        assert!(
            node.data_scope.is_none(),
            "symbol node should not have data_scope"
        );
        assert!(
            node.type_hint.is_none(),
            "symbol node should not have type_hint"
        );
    }
}

#[test]
fn datagraph_export_edges_have_required_fields() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let mut diags = Vec::new();
    let (graph, _scc) =
        GraphBuilder::from_extractions(&result.files, root, SnapshotId(1), &mut diags);

    let export = GraphOutput::from_graph(&graph, None, 1);
    for edge in &export.edges {
        assert!(edge.source <= export.metadata.node_count);
        assert!(edge.target <= export.metadata.node_count);
        assert!(!edge.kind.is_empty(), "edge kind must not be empty");
        let valid_kinds = ["ownership", "import", "reference", "flow"];
        assert!(
            valid_kinds.contains(&edge.kind.as_str()),
            "unexpected edge kind: {}",
            edge.kind
        );
    }
}

#[test]
fn datagraph_export_json_roundtrip() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let mut diags = Vec::new();
    let (graph, _scc) =
        GraphBuilder::from_extractions(&result.files, root, SnapshotId(1), &mut diags);

    let export = GraphOutput::from_graph(&graph, None, 1);
    let json = serde_json::to_string_pretty(&export).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["metadata"]["snapshot_id"], 1);
    assert!(parsed["nodes"].is_array());
    assert!(parsed["edges"].is_array());
    assert_eq!(
        parsed["metadata"]["node_count"].as_u64().unwrap() as usize,
        export.metadata.node_count
    );
    assert_eq!(
        parsed["metadata"]["edge_count"].as_u64().unwrap() as usize,
        export.metadata.edge_count
    );
}

#[test]
fn snapshot_meta_has_schema_version() {
    let meta = meta_ast::pipeline::snapshot_meta(SnapshotId(42));
    assert_eq!(meta.id, SnapshotId(42));
    assert_eq!(meta.datagraph_schema_version, SCHEMA_VERSION);
}

#[cfg(feature = "dataflow")]
#[test]
fn dataflow_pipeline_extracts_on_python_fixtures() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(!files.is_empty());

    let result = meta_ast::extractor::extract(&files);
    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;
    for file in &result.files {
        total_nodes += file.data_nodes.len();
        total_edges += file.flow_edges.len();
    }
    assert!(total_nodes > 0, "Python fixtures should extract DataNodes");
    assert!(total_edges > 0, "Python fixtures should extract FlowEdges");
}

#[cfg(feature = "dataflow")]
#[test]
fn sink_json_writes_datagraph_to_file() {
    use meta_ast::sink::GraphSink;
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let mut diags = Vec::new();
    let (graph, _scc) =
        GraphBuilder::from_extractions(&result.files, root, SnapshotId(1), &mut diags);
    let export = GraphOutput::from_graph(&graph, None, 1);

    let temp = std::env::temp_dir().join("meta_ast_dg_integration.json");
    let sink = meta_ast::sink::JsonSink::new(Some(temp.clone()));
    sink.emit(&export).unwrap();

    let content = std::fs::read_to_string(&temp).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["nodes"].is_array());
    assert!(parsed["edges"].is_array());

    let has_data_node = parsed["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n["kind"] == "data");
    assert!(
        has_data_node,
        "exported datagraph should contain data nodes"
    );
    let _ = std::fs::remove_file(&temp);
}
