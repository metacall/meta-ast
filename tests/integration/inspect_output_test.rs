use std::path::Path;

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
