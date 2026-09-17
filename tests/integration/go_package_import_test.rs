//! The Go package import contract.
//!
//! A Go import names a package, which is a directory. The graph model has file
//! targets and string targets only. A package path resolves to a file only
//! when that file exists on disk; otherwise the import becomes an external
//! node named after the package path itself. These tests pin that rule and
//! the invariants that must hold either way: no file node without a file on
//! disk, and no absolute path in a target name.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use meta_ast::extractor::extract;
use meta_ast::graph::{CodeGraph, GraphBuilder, NodeData, edge::EdgeKind};
use meta_ast::input::{discover_files, portable_path};
use meta_ast::model::SnapshotId;

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meta_ast_go_package_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &Path, relative: &str, body: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
}

fn analyze(root: &Path) -> CodeGraph {
    let files = discover_files(root, None).unwrap();
    let extractions = extract(&files).files;
    let mut diagnostics = Vec::new();
    let snapshot_id = SnapshotId::new(1);
    assert!(snapshot_id.is_some(), "1 is a valid snapshot id");
    let (graph, _scc) =
        GraphBuilder::from_extractions(&extractions, root, snapshot_id.unwrap(), &mut diagnostics);
    graph
}

/// A module with a package import, the module root import, and one member file.
fn go_module(name: &str) -> PathBuf {
    let root = project(name);
    write(&root, "go.mod", "module myproject\n\ngo 1.22\n");
    write(
        &root,
        "main.go",
        "package main\n\nimport (\n\t\"myproject\"\n\t\"myproject/internal/util\"\n)\n\nfunc main() {\n\tutil.Helper()\n}\n",
    );
    write(
        &root,
        "internal/util/helper.go",
        "package util\n\nfunc Helper() {}\n",
    );
    root
}

fn external_names(graph: &CodeGraph) -> BTreeSet<String> {
    graph
        .graph()
        .node_indices()
        .filter_map(|index| match &graph.graph()[index] {
            NodeData::External(node) => Some(node.raw_path.clone()),
            _ => None,
        })
        .collect()
}

fn import_edge_exists(graph: &CodeGraph, from_suffix: &str, to_suffix: &str) -> bool {
    let node_for = |suffix: &str| {
        graph
            .files()
            .find(|(_, file)| portable_path(&file.path).ends_with(suffix))
            .and_then(|(id, _)| graph.file_node_index(id))
    };
    let (Some(from), Some(to)) = (node_for(from_suffix), node_for(to_suffix)) else {
        return false;
    };
    graph
        .edges_of_kind(EdgeKind::Import)
        .any(|(source, target)| source == from && target == to)
}

#[test]
fn module_root_import_becomes_a_package_named_external_node() {
    let root = go_module("root");
    let graph = analyze(&root);

    let names = external_names(&graph);
    assert!(
        names.contains("myproject"),
        "the module root import is targetless today, so it keeps the package path as its name: {names:?}"
    );
}

#[test]
fn subpackage_import_without_its_file_becomes_a_specifier_named_external_node() {
    let root = go_module("subpackage");
    let graph = analyze(&root);

    let names = external_names(&graph);
    assert!(
        names.contains("myproject/internal/util"),
        "a package path without its file keeps the specifier as its name: {names:?}"
    );
    assert!(
        names.iter().all(|name| !Path::new(name).is_absolute()),
        "no external node names a phantom absolute file path: {names:?}"
    );
}

#[test]
fn a_package_import_reaches_no_member_file() {
    let root = go_module("members");
    let graph = analyze(&root);

    assert!(
        !import_edge_exists(&graph, "main.go", "internal/util/helper.go"),
        "the import stops at the specifier-named external node, so the member file stays unconnected"
    );
}

#[test]
fn no_file_node_names_a_path_without_a_file() {
    let root = go_module("file_nodes");
    let graph = analyze(&root);

    for (_, file) in graph.files() {
        let path = file.path.clone();
        assert!(
            root.join(&path).exists() || path.exists(),
            "a file node must correspond to a file on disk: {}",
            path.display()
        );
    }
}

#[test]
fn a_workspace_without_a_module_file_keeps_the_specifier_as_its_name() {
    let root = project("no_module");
    write(
        &root,
        "main.go",
        "package main\n\nimport \"example.com/remote/pkg\"\n\nfunc main() {}\n",
    );

    let graph = analyze(&root);
    let names = external_names(&graph);

    assert!(
        names.contains("example.com/remote/pkg"),
        "without a module file the specifier itself is the target name: {names:?}"
    );
}
