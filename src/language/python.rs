use crate::language::LanguageSpec;
use std::sync::LazyLock;

static PYTHON_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_python::LANGUAGE.into(),
        r#"
(function_definition
  "async"? @async
  name: (identifier) @name
  parameters: (parameters) @signature
  body: (block (expression_statement (string) @docstring)?)
) @kind.function

(class_definition
  name: (identifier) @name
  body: (block (expression_statement (string) @docstring)?)
) @kind.class

(decorated_definition
  definition: [
    (function_definition
      "async"? @async
      name: (identifier) @name
      parameters: (parameters) @signature
      body: (block (expression_statement (string) @docstring)?)
    ) @kind.function
    (class_definition
      name: (identifier) @name
      body: (block (expression_statement (string) @docstring)?)
    ) @kind.class
  ]
)
"#,
        "Python",
    )
});

fn python_query() -> &'static tree_sitter::Query {
    &PYTHON_QUERY
}

const PYTHON_IMPORT_QUERY_STR: &str = r#"
(import_statement
  (dotted_name) @import.path)
(import_statement
  (aliased_import
    name: (dotted_name) @import.path
    alias: (identifier) @import.alias))
(import_from_statement
  module_name: (dotted_name) @import.path
  name: (_) @import.symbol)
(import_from_statement
  module_name: (dotted_name) @import.path
  (aliased_import name: (_) @import.symbol alias: (identifier) @import.alias))
(import_from_statement
  module_name: (dotted_name) @import.path
  (wildcard_import) @import.star)
"#;

const PYTHON_REFERENCE_QUERY_STR: &str = r#"
(call
  function: (identifier) @reference.name)
(call
  function: (attribute
    attribute: (identifier) @reference.name))
"#;

static PYTHON_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_python::LANGUAGE.into(),
        &format!(
            "{}\n{}",
            PYTHON_IMPORT_QUERY_STR, PYTHON_REFERENCE_QUERY_STR
        ),
        "Python combined import+ref",
    )
});

fn python_import_ref_query() -> &'static tree_sitter::Query {
    &PYTHON_IMPORT_REF_QUERY
}

pub const PYTHON_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["py", "pyi"],
    grammar_fn: || tree_sitter_python::LANGUAGE.into(),
    query_fn: python_query,
    import_ref_query_fn: python_import_ref_query,
    class_like_parents: &["class_definition"],
    ancestor_visibility_rules: &[],
};

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Python)).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn python_grammar_loads() {
        let _ = grammar_for(LangId::Python);
    }

    #[test]
    fn extract_simple_function() {
        let tree = parse(b"def hello(): pass");
        let symbols = extract_symbols_for(LangId::Python, &tree, b"def hello(): pass");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_async_function() {
        let tree = parse(b"async def fetch(): pass");
        let symbols = extract_symbols_for(LangId::Python, &tree, b"async def fetch(): pass");
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_class_and_methods() {
        let src =
            "class Foo:\n    def __init__(self):\n        pass\n    def bar(self):\n        pass\n";
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Python, &tree, src.as_bytes());
        let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(foo.kind, SymbolKind::Class));
        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert!(matches!(bar.kind, SymbolKind::Method));
    }

    #[test]
    fn extract_decorated_function() {
        let src = "@decorator\ndef decorated_func(x):\n    return x * 2\n";
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Python, &tree, src.as_bytes());
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "decorated_func");
    }

    #[test]
    fn extract_function_with_docstring() {
        let src = br#"def greet():
    """Say hello."""
    pass
"#;
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Python, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].docstring.as_deref(), Some("Say hello."));
    }

    #[test]
    fn docstring_does_not_eat_content_starting_with_quote() {
        let src = br#"def f():
    """"Quoted" at start."""
    pass
"#;
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Python, &tree, src);
        assert_eq!(symbols.len(), 1);
        let ds = symbols[0].docstring.as_deref().unwrap();
        assert!(
            ds.starts_with('"'),
            "docstring content should start with a quote char, got: {ds:?}"
        );
        assert!(
            ds.contains("Quoted"),
            "docstring should contain 'Quoted', got: {ds:?}"
        );
    }

    #[test]
    fn docstring_does_not_eat_content_ending_with_quote() {
        let src = br#"def f():
    '''She said "hello"'''
    pass
"#;
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Python, &tree, src);
        assert_eq!(symbols.len(), 1);
        let ds = symbols[0].docstring.as_deref().unwrap();
        assert!(
            ds.contains("hello"),
            "docstring should contain 'hello', got: {ds:?}"
        );
        assert!(
            ds.contains(r#"""#),
            "docstring should preserve inner double quotes, got: {ds:?}"
        );
    }

    #[test]
    fn docstring_multiline_strips_delimiters_not_content() {
        let src = br#"def f():
    """
    She said "hi".
    """
    pass
"#;
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Python, &tree, src);
        assert_eq!(symbols.len(), 1);
        let ds = symbols[0].docstring.as_deref().unwrap();
        assert!(
            !ds.starts_with('"'),
            "docstring should not start with delimiter quote, got: {ds:?}"
        );
        assert!(
            ds.contains(r#""hi""#),
            "docstring should preserve inner quotes, got: {ds:?}"
        );
    }

    #[test]
    fn python_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/python/simple_functions.py"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Python, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
