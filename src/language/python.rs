use crate::language::{RawSymbol, impl_language};
use once_cell::sync::Lazy;

static PYTHON_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
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
    )
    .expect("Failed to parse Python query")
});

fn extract<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    _cursor: &mut tree_sitter::TreeCursor<'a>,
) -> Vec<RawSymbol<'a>> {
    super::common::extract_with_query(tree, source, &PYTHON_QUERY)
}

impl_language!(
    Python,
    tree_sitter_python::LANGUAGE.into(),
    extract,
    &["py", "pyi"]
);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn python_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
    }

    #[test]
    fn extract_simple_function() {
        let tree = parse(b"def hello(): pass");
        let mut cursor = tree.walk();
        let symbols = extract(&tree, b"def hello(): pass", &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_async_function() {
        let tree = parse(b"async def fetch(): pass");
        let mut cursor = tree.walk();
        let symbols = extract(&tree, b"async def fetch(): pass", &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_class_and_methods() {
        let src =
            "class Foo:\n    def __init__(self):\n        pass\n    def bar(self):\n        pass\n";
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
        let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(foo.kind, SymbolKind::Class));
        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert!(matches!(bar.kind, SymbolKind::Method));
    }

    #[test]
    fn extract_decorated_function() {
        let src = "@decorator\ndef decorated_func(x):\n    return x * 2\n";
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
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
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].docstring.as_deref(), Some("Say hello."));
    }

    #[test]
    fn python_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/python/simple_functions.py"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
        insta::assert_json_snapshot!(symbols);
    }
}
