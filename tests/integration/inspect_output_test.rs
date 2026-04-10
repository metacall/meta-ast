use std::path::Path;

#[test]
fn python_output_schema_validation() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let json = meta_ast::output::inspect::to_inspect_json(&result.symbols).unwrap();

    let output: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert!(output["funcs"].is_array(), "funcs should be array");
    assert!(output["classes"].is_array(), "classes should be array");
    assert!(output["objects"].is_array(), "objects should be array");

    for func in output["funcs"].as_array().unwrap() {
        assert!(func["name"].is_string(), "func must have name");
        assert!(
            func["source_range"].is_object(),
            "func must have source_range"
        );
        assert!(func["async"].is_boolean(), "func must have async field");
    }

    for class in output["classes"].as_array().unwrap() {
        assert!(class["name"].is_string(), "class must have name");
        assert!(
            class["source_range"].is_object(),
            "class must have source_range"
        );
    }

    for obj in output["objects"].as_array().unwrap() {
        assert!(obj["name"].is_string(), "object must have name");
        assert!(
            obj["source_range"].is_object(),
            "object must have source_range"
        );
    }
}

#[test]
fn all_language_outputs_valid() {
    let root = Path::new("tests/fixtures");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let json = meta_ast::output::inspect::to_inspect_json(&result.symbols).unwrap();

    let output: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert!(output.is_object());
    assert!(output["funcs"].is_array());
    assert!(output["classes"].is_array());
    assert!(output["objects"].is_array());
}

#[test]
fn insta_python_inspect_snapshot() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let output = meta_ast::output::inspect::symbols_to_inspect_output(&result.symbols);
    insta::assert_json_snapshot!(output);
}

#[test]
fn insta_all_languages_snapshot() {
    let root = Path::new("tests/fixtures");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let output = meta_ast::output::inspect::symbols_to_inspect_output(&result.symbols);
    insta::assert_json_snapshot!(output);
}
