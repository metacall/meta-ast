//! Import resolution at the graph level: discovery, extraction, edges.

use std::path::{Path, PathBuf};

use meta_ast::error::Severity;
use meta_ast::extractor::extract;
use meta_ast::graph::{CodeGraph, GraphBuilder, NodeData, edge::EdgeKind};
use meta_ast::input::discover_files;
use meta_ast::model::SnapshotId;

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meta_ast_import_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
}

fn analyze(root: &Path) -> (CodeGraph, Vec<meta_ast::Diagnostic>) {
    let files = discover_files(root, None).unwrap();
    let extractions = extract(&files).files;
    let mut diagnostics = Vec::new();
    let snapshot_id = SnapshotId::new(1).unwrap();
    let (graph, _scc) =
        GraphBuilder::from_extractions(&extractions, root, snapshot_id, &mut diagnostics);
    (graph, diagnostics)
}

fn import_edge_exists(graph: &CodeGraph, from: &str, to: &str) -> bool {
    let file_id = |suffix: &str| {
        graph
            .files()
            .find(|(_, file)| file.path.to_string_lossy().ends_with(suffix))
            .map(|(id, _)| id)
    };
    let (Some(from_id), Some(to_id)) = (file_id(from), file_id(to)) else {
        return false;
    };
    let (Some(from_idx), Some(to_idx)) =
        (graph.file_node_index(from_id), graph.file_node_index(to_id))
    else {
        return false;
    };
    graph
        .edges_of_kind(EdgeKind::Import)
        .any(|(source, target)| source == from_idx && target == to_idx)
}

fn external_paths(graph: &CodeGraph) -> Vec<String> {
    graph
        .graph()
        .node_indices()
        .filter_map(|index| match &graph.graph()[index] {
            NodeData::External(node) => Some(node.raw_path.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn python_relative_import_creates_an_edge_to_the_submodule() {
    let root = project("py_relative_edge");
    write(&root, "pkg/__init__.py", "");
    write(&root, "pkg/util.py", "def helper(): pass\n");
    write(&root, "pkg/mod.py", "from . import util\n");

    let (graph, _diagnostics) = analyze(&root);

    assert!(
        import_edge_exists(&graph, "pkg/mod.py", "pkg/util.py"),
        "externals: {:?}",
        external_paths(&graph)
    );
    assert!(external_paths(&graph).is_empty());
}

#[test]
fn python_double_dot_import_resolves_to_the_parent_package() {
    let root = project("py_double_dot");
    write(&root, "pkg/__init__.py", "");
    write(&root, "pkg/mod.py", "from ..util import helper\n");
    write(&root, "util.py", "def helper(): pass\n");

    let (graph, _diagnostics) = analyze(&root);

    assert!(
        import_edge_exists(&graph, "pkg/mod.py", "util.py"),
        "externals: {:?}",
        external_paths(&graph)
    );
}

#[test]
fn python_third_party_import_becomes_a_bare_external_node() {
    let root = project("py_third_party");
    write(&root, "app.py", "import requests\n");

    let (graph, _diagnostics) = analyze(&root);

    assert!(
        external_paths(&graph).contains(&"requests".to_string()),
        "externals: {:?}",
        external_paths(&graph)
    );
}

#[test]
fn c_system_include_becomes_a_bare_external_node() {
    let root = project("c_system_include");
    write(
        &root,
        "src/a.c",
        "#include <stdio.h>\nint main(void) { return 0; }\n",
    );

    let (graph, _diagnostics) = analyze(&root);

    assert!(
        external_paths(&graph).contains(&"stdio.h".to_string()),
        "externals: {:?}",
        external_paths(&graph)
    );
}

#[test]
fn unresolved_relative_import_emits_a_warning() {
    let root = project("py_unresolved_relative");
    write(&root, "app.py", "from .missing import thing\n");

    let (graph, diagnostics) = analyze(&root);

    assert!(
        diagnostics
            .iter()
            .any(|d| d.severity == Severity::Warning && d.path.ends_with("app.py")),
        "diagnostics: {diagnostics:?}"
    );
    assert!(
        !external_paths(&graph)
            .iter()
            .any(|path| path.contains("missing")),
        "externals: {:?}",
        external_paths(&graph)
    );
}

#[test]
fn python_absolute_first_party_import_still_resolves() {
    let root = project("py_absolute_first_party");
    write(&root, "pkg/__init__.py", "");
    write(&root, "pkg/util.py", "def helper(): pass\n");
    write(&root, "pkg/mod.py", "from pkg.util import helper\n");

    let (graph, _diagnostics) = analyze(&root);

    assert!(
        import_edge_exists(&graph, "pkg/mod.py", "pkg/util.py"),
        "externals: {:?}",
        external_paths(&graph)
    );
}
