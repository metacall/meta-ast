//! Semantic pins for extraction results the graph depends on.
//!
//! Each case asserts one concrete fact (a symbol kind or an edge) instead of a
//! snapshot, so a change in the extraction contract fails here with a readable
//! message rather than as a snapshot diff.

use std::path::{Path, PathBuf};

use meta_ast::extractor::ExtractOptions;
use meta_ast::model::SnapshotId;
use meta_ast::{EdgeKind, ExtractionResult, FileExtraction, GraphAnalysis, SymbolKind};

fn fixtures(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn extract_root(root: &Path) -> ExtractionResult {
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(!files.is_empty(), "{} holds no source file", root.display());
    meta_ast::extractor::extract_with_options(
        &files,
        &ExtractOptions {
            skip_imports_and_refs: false,
        },
    )
}

fn file<'a>(result: &'a ExtractionResult, name: &str) -> &'a FileExtraction {
    result
        .files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|found| found == name))
        .unwrap_or_else(|| panic!("{name} is not in the extraction result"))
}

fn symbol_kind(file: &FileExtraction, name: &str) -> Option<SymbolKind> {
    file.symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .map(|symbol| symbol.kind)
}

fn analyze(root: &Path) -> GraphAnalysis {
    let snapshot_id = SnapshotId::new(1);
    assert!(snapshot_id.is_some(), "1 is a valid snapshot id");
    let (analysis, _) =
        meta_ast::pipeline::analyze_graph(root, snapshot_id.unwrap(), None).unwrap();
    analysis
}

fn import_edges(analysis: &GraphAnalysis) -> Vec<(String, String)> {
    let raw = analysis.graph.graph();
    let mut edges = Vec::new();
    for edge in raw.edge_indices() {
        let Some(weight) = raw.edge_weight(edge) else {
            continue;
        };
        if weight.kind != EdgeKind::Import {
            continue;
        }
        let Some((source, target)) = raw.edge_endpoints(edge) else {
            continue;
        };
        let name = |index| {
            raw.node_weight(index)
                .and_then(|node| node.as_file())
                .and_then(|node| node.path.file_name())
                .map_or_else(String::new, |found| found.to_string_lossy().to_string())
        };
        edges.push((name(source), name(target)));
    }
    edges
}

#[test]
fn a_module_level_assignment_is_a_constant() {
    let result = extract_root(&fixtures("import_hygiene"));
    let constants = file(&result, "constants.py");
    assert_eq!(
        symbol_kind(constants, "LIMIT"),
        Some(SymbolKind::Constant),
        "a module level assignment is a constant"
    );
}

#[test]
fn a_function_expression_assigned_to_a_const_is_a_function() {
    let root = std::env::temp_dir().join("meta_ast_semantics_function_expression");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("app.js"),
        "export const compute = function (value) { return value; };\n",
    )
    .unwrap();

    let result = extract_root(&root);
    let app = file(&result, "app.js");
    assert_eq!(
        symbol_kind(app, "compute"),
        Some(SymbolKind::Function),
        "a function expression bound to a const is a function"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_c_header_prototype_is_a_declaration() {
    let result = extract_root(&fixtures("multi"));
    let header = file(&result, "utils.h");
    assert_eq!(
        symbol_kind(header, "multiply"),
        Some(SymbolKind::Declaration),
        "a C prototype without a body is a declaration"
    );
}

#[test]
fn a_cpp_header_prototype_is_a_declaration() {
    let result = extract_root(&fixtures("multi"));
    let header = file(&result, "utils.hpp");
    assert_eq!(
        symbol_kind(header, "divide"),
        Some(SymbolKind::Declaration),
        "a C++ prototype without a body is a declaration"
    );
}

#[test]
fn a_relative_import_links_the_two_files() {
    let analysis = analyze(&fixtures("import_hygiene"));
    let edges = import_edges(&analysis);
    assert!(
        edges
            .iter()
            .any(|(source, target)| source == "app.js" && target == "helper.js"),
        "a relative import links the importing file to the imported file, saw {edges:?}"
    );
}
