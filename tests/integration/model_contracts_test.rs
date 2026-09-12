//! Construction contracts for the extraction, deploy and output types.
//!
//! The scans here are narrow on purpose: each one looks for the exact
//! expression that a second copy of a rule would be written with.

use std::path::{Path, PathBuf};

use meta_ast::LangId;
use meta_ast::model::FileExtraction;

fn read_source(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every gated field of a file extraction is set through the one constructor.
/// A struct literal elsewhere would silently miss a feature-gated field.
#[test]
fn file_extraction_is_constructed_through_one_constructor() {
    let mut sources = Vec::new();
    rust_sources(&repository_root().join("src"), &mut sources);

    let mut literals = Vec::new();
    for path in sources {
        if path.ends_with("src/model/mod.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let has_literal = source.lines().any(|line| line.trim() == "FileExtraction {");
        if has_literal {
            literals.push(path);
        }
    }
    assert!(
        literals.is_empty(),
        "a file extraction must be built through FileExtraction::empty, found literals in {literals:?}"
    );

    let model = read_source("src/model/mod.rs");
    assert!(
        model.contains("pub fn empty("),
        "the constructor stays public"
    );
    assert!(
        model.contains("#[cfg(feature = \"metacall-deploy\")]\n            call_sites:"),
        "the gated field is set inside the constructor"
    );
}

#[test]
fn gated_fields_start_empty() {
    let extraction = FileExtraction::empty(PathBuf::from("a.py"), LangId::Python);
    assert!(extraction.symbols.is_empty());
    assert!(extraction.imports.is_empty());
    assert!(extraction.references.is_empty());
    assert!(extraction.diagnostics.is_empty());
    assert_eq!(extraction.ast_node_count, 0);

    #[cfg(feature = "metacall-deploy")]
    assert!(extraction.call_sites.is_empty());
    #[cfg(feature = "dataflow")]
    {
        assert!(extraction.data_nodes.is_empty());
        assert!(extraction.flow_edges.is_empty());
    }
}

/// The public output records are closed for construction outside the crate, so
/// a new field cannot break a consumer silently.
#[test]
fn public_output_records_are_non_exhaustive() {
    let graph_output = read_source("src/output/graph.rs");
    for name in [
        "pub struct GraphOutput {",
        "pub struct GraphMetadata {",
        "pub struct SerializedNode {",
        "pub struct SerializedEdge {",
        "pub struct SerializedScc {",
        "pub struct DeployabilityStats {",
    ] {
        let marker = format!("#[non_exhaustive]\n{name}");
        assert!(
            graph_output.contains(&marker),
            "{name} must be non_exhaustive"
        );
    }
}
