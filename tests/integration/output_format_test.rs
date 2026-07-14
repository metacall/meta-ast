use meta_ast::language::LangId;
use meta_ast::model::{LineColumn, SourceRange, Symbol, SymbolId, SymbolKind, Visibility};
use meta_ast::output::OutputFormat;
use std::path::PathBuf;

fn sample_symbol(id: u32, name: &str, kind: SymbolKind) -> Symbol {
    Symbol {
        id: SymbolId(id),
        name: name.to_string(),
        kind,
        language: LangId::Python,
        file_path: PathBuf::from("test.py"),
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
        signature: Some("fn test()".into()),
        docstring: Some("A test function".into()),
        is_async: false,
    }
}

#[test]
fn yaml_serialize_empty_inspect() {
    let yaml =
        meta_ast::output::inspect::serialize_inspect(&mut Vec::new(), &OutputFormat::Yaml).unwrap();
    let parsed: yaml_serde::Value = yaml_serde::from_str(&yaml).unwrap();
    assert!(parsed["funcs"].is_sequence());
    assert!(parsed["classes"].is_sequence());
    assert!(parsed["objects"].is_sequence());
}

#[test]
fn yaml_serialize_inspect_with_symbols() {
    let mut symbols = vec![
        sample_symbol(1, "func_a", SymbolKind::Function),
        sample_symbol(2, "MyClass", SymbolKind::Class),
    ];
    let yaml =
        meta_ast::output::inspect::serialize_inspect(&mut symbols, &OutputFormat::Yaml).unwrap();

    let parsed: yaml_serde::Value = yaml_serde::from_str(&yaml).unwrap();
    let funcs = parsed["funcs"].as_sequence().unwrap();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0]["name"], "func_a");

    let classes = parsed["classes"].as_sequence().unwrap();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0]["name"], "MyClass");
}

#[test]
fn yaml_serialize_graph_output() {
    use meta_ast::graph::builder::GraphBuilder;
    use meta_ast::graph::scc::SccAnalysis;
    use meta_ast::model::SnapshotId;

    let mut builder = GraphBuilder::new(SnapshotId(1));
    let file_path = PathBuf::from("test.py");
    builder.add_file(file_path.clone(), LangId::Python);
    let sym = sample_symbol(1, "main", SymbolKind::Function);
    builder.add_symbol(&sym).unwrap();

    let graph = builder.build();
    let scc = SccAnalysis::analyze(&graph.graph);

    let yaml =
        meta_ast::output::graph::serialize_graph(&graph, &scc, 1, &OutputFormat::Yaml).unwrap();

    let parsed: yaml_serde::Value = yaml_serde::from_str(&yaml).unwrap();
    assert!(parsed["metadata"].is_mapping());
    assert!(parsed["nodes"].is_sequence());
    assert!(parsed["edges"].is_sequence());
    assert!(parsed["sccs"].is_sequence());
}

#[test]
fn output_format_json_not_equal_yaml() {
    assert_ne!(OutputFormat::Json, OutputFormat::Yaml);
    assert_eq!(OutputFormat::Json, OutputFormat::Json);
    assert_eq!(OutputFormat::Yaml, OutputFormat::Yaml);
}

#[test]
fn yaml_json_semantic_equivalence() {
    let mut symbols = vec![
        sample_symbol(1, "func_a", SymbolKind::Function),
        sample_symbol(2, "MyClass", SymbolKind::Class),
    ];

    let json =
        meta_ast::output::inspect::serialize_inspect(&mut symbols.clone(), &OutputFormat::Json)
            .unwrap();
    let yaml =
        meta_ast::output::inspect::serialize_inspect(&mut symbols, &OutputFormat::Yaml).unwrap();

    let json_parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let yaml_parsed: yaml_serde::Value = yaml_serde::from_str(&yaml).unwrap();

    assert_eq!(json_parsed["funcs"].as_array().unwrap().len(), 1);
    assert_eq!(yaml_parsed["funcs"].as_sequence().unwrap().len(), 1);
    assert_eq!(json_parsed["funcs"][0]["name"], "func_a");
    assert_eq!(yaml_parsed["funcs"][0]["name"], "func_a");
}
