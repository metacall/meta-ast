use meta_ast::graph::builder::GraphBuilder;
use meta_ast::graph::scc::SccAnalysis;
use meta_ast::language::LangId;
use meta_ast::model::{
    LineColumn, SnapshotId, SourceRange, Symbol, SymbolId, SymbolKind, Visibility,
};
use std::path::PathBuf;

fn sample_symbol(id: u32, name: &str, path: &str) -> Symbol {
    Symbol {
        id: SymbolId(id),
        name: name.to_string(),
        kind: SymbolKind::Function,
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

fn build_sample_graph() -> (meta_ast::graph::CodeGraph, SccAnalysis) {
    let mut builder = GraphBuilder::new(SnapshotId(1));
    let _file_id = builder.add_file(PathBuf::from("src/main.rs"), LangId::Rust);
    let sym1 = sample_symbol(1, "main", "src/main.rs");
    let sym2 = sample_symbol(2, "helper", "src/main.rs");
    builder.add_symbol(&sym1).unwrap();
    builder.add_symbol(&sym2).unwrap();
    builder.add_reference(sym1.id, sym2.id);

    let graph = builder.build();
    let scc = SccAnalysis::analyze(&graph.graph);
    (graph, scc)
}

#[test]
fn to_graph_html_contains_doctype() {
    let (graph, scc) = build_sample_graph();
    let html = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, false).unwrap();
    assert!(
        html.starts_with("<!DOCTYPE html>"),
        "should start with DOCTYPE"
    );
}

#[test]
fn to_graph_html_contains_title() {
    let (graph, scc) = build_sample_graph();
    let html = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, false).unwrap();
    assert!(
        html.contains("<title>Meta-AST Graph Dashboard</title>"),
        "should contain title"
    );
}

#[test]
fn to_graph_html_data_placeholder_replaced() {
    let (graph, scc) = build_sample_graph();
    let html = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, false).unwrap();
    assert!(!html.contains("__DATA__"), "__DATA__ should be substituted");
    assert!(
        !html.contains("__CDN_SCRIPT__"),
        "__CDN_SCRIPT__ should be substituted"
    );
}

#[test]
fn to_graph_html_json_data_valid() {
    let (graph, scc) = build_sample_graph();
    let html = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, false).unwrap();

    let start = html.find("var DATA=").expect("should contain var DATA=") + 9;
    let end = html[start..]
        .find(';')
        .expect("should find semicolon after DATA");
    let json_str = &html[start..start + end];

    let parsed: serde_json::Value =
        serde_json::from_str(json_str).expect("DATA should be valid JSON");
    assert!(parsed.get("nodes").is_some(), "should have nodes");
    assert!(parsed.get("edges").is_some(), "should have edges");
    assert!(parsed.get("sccs").is_some(), "should have sccs");
}

#[test]
fn to_graph_html_cdn_link() {
    let (graph, scc) = build_sample_graph();
    let html = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, false).unwrap();
    assert!(
        html.contains("cdnjs.cloudflare.com/ajax/libs/cytoscape"),
        "should contain CDN link"
    );
}

#[test]
fn to_graph_html_empty_graph() {
    let builder = GraphBuilder::new(SnapshotId(1));
    let graph = builder.build();
    let scc = SccAnalysis::analyze(&graph.graph);
    let html = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, false).unwrap();
    assert!(
        html.contains("\"node_count\":0"),
        "empty graph should have node_count 0"
    );
}

#[cfg(not(feature = "embed-cytoscape"))]
#[test]
fn to_graph_html_self_contained_without_feature() {
    let (graph, scc) = build_sample_graph();
    let result = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, true);
    assert!(
        result.is_err(),
        "self_contained without feature should fail"
    );
}

#[cfg(feature = "embed-cytoscape")]
#[test]
fn to_graph_html_self_contained_with_feature() {
    let (graph, scc) = build_sample_graph();
    let result = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, true);
    assert!(result.is_ok(), "self_contained with feature should succeed");
    let html = result.unwrap();
    assert!(
        html.contains("<script>") && html.contains("cytoscape"),
        "self-contained HTML should embed cytoscape inline"
    );
}

#[test]
fn to_graph_html_cytoscape_container_div() {
    let (graph, scc) = build_sample_graph();
    let html = meta_ast::output::dashboard::to_graph_html(&graph, &scc, 1, false).unwrap();
    assert!(
        html.contains("<div id=\"cy\">"),
        "should contain cy container div"
    );
}
