use std::path::Path;

#[test]
fn end_to_end_python_project() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(!files.is_empty(), "should find Python fixture files");

    for (_, lang) in &files {
        assert_eq!(*lang, meta_ast::language::LangId::Python);
    }

    let result = meta_ast::extractor::extract(&files);
    assert!(
        !result.symbols.is_empty(),
        "should extract symbols from Python fixtures"
    );

    for symbol in &result.symbols {
        assert!(!symbol.name.is_empty());
        assert_eq!(symbol.language, meta_ast::language::LangId::Python);
    }

    let json = meta_ast::output::inspect::serialize_inspect(
        &result.symbols,
        &meta_ast::output::OutputFormat::Json,
    )
    .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["funcs"].is_array());
    assert!(parsed["classes"].is_array());
    assert!(parsed["objects"].is_array());

    for func in parsed["funcs"].as_array().unwrap() {
        assert!(func["name"].is_string(), "func must have name");
        assert!(
            func["source_range"].is_object(),
            "func must have source_range"
        );
        assert!(func["async"].is_boolean(), "func must have async field");
    }

    for class in parsed["classes"].as_array().unwrap() {
        assert!(class["name"].is_string(), "class must have name");
        assert!(
            class["source_range"].is_object(),
            "class must have source_range"
        );
    }

    for obj in parsed["objects"].as_array().unwrap() {
        assert!(obj["name"].is_string(), "object must have name");
        assert!(
            obj["source_range"].is_object(),
            "object must have source_range"
        );
    }
}

#[test]
fn end_to_end_mixed_languages() {
    let root = Path::new("tests/fixtures");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(
        files.len() > 5,
        "should find files across multiple languages"
    );

    let langs: std::collections::HashSet<_> = files.iter().map(|(_, l)| *l).collect();
    assert!(
        langs.len() >= 3,
        "should find at least 3 different languages"
    );

    let result = meta_ast::extractor::extract(&files);
    assert!(!result.symbols.is_empty());

    let json = meta_ast::output::inspect::serialize_inspect(
        &result.symbols,
        &meta_ast::output::OutputFormat::Json,
    )
    .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["funcs"].is_array());
}

#[test]
fn pipeline_idempotent() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();

    let result1 = meta_ast::extractor::extract(&files);
    let result2 = meta_ast::extractor::extract(&files);

    let names1: Vec<_> = result1
        .symbols
        .iter()
        .map(|s| format!("{:?}", s.kind))
        .collect();
    let names2: Vec<_> = result2
        .symbols
        .iter()
        .map(|s| format!("{:?}", s.kind))
        .collect();

    assert_eq!(
        names1, names2,
        "same input should produce same symbol kinds"
    );

    let names1: Vec<_> = result1.symbols.iter().map(|s| &s.name).collect();
    let names2: Vec<_> = result2.symbols.iter().map(|s| &s.name).collect();
    assert_eq!(
        names1, names2,
        "same input should produce same symbol names"
    );
}

#[test]
fn json_output_has_required_structure() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let json = meta_ast::output::inspect::serialize_inspect(
        &result.symbols,
        &meta_ast::output::OutputFormat::Json,
    )
    .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let obj = parsed.as_object().expect("root should be object");

    assert!(obj.contains_key("funcs"), "must have funcs key");
    assert!(obj.contains_key("classes"), "must have classes key");
    assert!(obj.contains_key("objects"), "must have objects key");
    assert_eq!(obj.len(), 3, "should have exactly 3 top-level keys");
}
