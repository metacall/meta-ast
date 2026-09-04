use meta_ast::language::LangId;
use meta_ast::model::{LineColumn, SourceRange, Symbol, SymbolId, SymbolKind, Visibility};
use meta_ast::output::OutputFormat;
use std::path::PathBuf;

fn sample_symbol(id: u32, name: &str, kind: SymbolKind) -> Symbol {
    Symbol {
        id: SymbolId::new(id).unwrap(),
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

    let mut builder = GraphBuilder::new(SnapshotId::new(1).unwrap());
    let file_path = PathBuf::from("test.py");
    builder.add_file(file_path.clone(), LangId::Python);
    let sym = sample_symbol(1, "main", SymbolKind::Function);
    builder.add_symbol(&sym).unwrap();

    let graph = builder.build();
    let scc = SccAnalysis::analyze(graph.graph());

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

// ---------------------------------------------------------------------------
// Structured JSON error output tests (issue #62)
// ---------------------------------------------------------------------------

#[test]
fn json_error_output_nonexistent_path() {
    let io_err = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "path does not exist: /invalid/path",
    );
    let err: anyhow::Error = meta_ast::error::Error::Io(io_err).into();
    let json_str = meta_ast::output::format_json_error(&err);

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("format_json_error should produce valid JSON");

    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["error"]["kind"], "IoError");
    assert!(
        parsed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("path does not exist"),
        "message should contain the IO error text"
    );
    assert!(
        parsed["diagnostics"].is_array(),
        "diagnostics must be an array"
    );
    let diags = parsed["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["severity"], "Error");
}

#[test]
fn json_error_output_parse_error() {
    let err: anyhow::Error = meta_ast::error::Error::Parse {
        path: PathBuf::from("broken.py"),
        message: "unexpected token at line 42".into(),
    }
    .into();
    let json_str = meta_ast::output::format_json_error(&err);

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("format_json_error should produce valid JSON");

    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["error"]["kind"], "ParseError");
    assert!(
        parsed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("broken.py"),
        "message should reference the file path"
    );
    let diags = parsed["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["path"], "broken.py");
    assert_eq!(diags[0]["severity"], "Error");
    assert!(
        diags[0]["message"]
            .as_str()
            .unwrap()
            .contains("unexpected token"),
        "diagnostic message should contain parse error detail"
    );
}

#[test]
fn json_error_output_unknown_error() {
    let err = anyhow::anyhow!("something completely unexpected");
    let json_str = meta_ast::output::format_json_error(&err);

    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("format_json_error should produce valid JSON");

    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["error"]["kind"], "UnknownError");
    assert!(
        parsed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("something completely unexpected"),
        "message should contain the original error text"
    );
    assert!(
        parsed["diagnostics"].as_array().unwrap().is_empty(),
        "unknown errors should have no diagnostics"
    );
}
