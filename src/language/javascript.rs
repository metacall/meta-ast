use crate::language::{RawSymbol, impl_language};
use once_cell::sync::Lazy;

static JS_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_javascript::LANGUAGE.into(),
        r#"
(function_declaration
  "async"? @async
  name: (identifier) @name
  parameters: (formal_parameters) @signature
) @kind.function

(generator_function_declaration
  "async"? @async
  name: (identifier) @name
  parameters: (formal_parameters) @signature
) @kind.function

(class_declaration
  name: (identifier) @name
) @kind.class

(method_definition
  "async"? @async
  name: [
    (property_identifier)
    (identifier)
  ] @name
  parameters: (formal_parameters) @signature
) @kind.method

(export_statement
  [
    (function_declaration
      "async"? @async
      name: (identifier) @name
      parameters: (formal_parameters) @signature
    ) @kind.function
    (class_declaration
      name: (identifier) @name
    ) @kind.class
  ]
) @visibility.public
"#,
    )
    .expect("Failed to parse JavaScript query")
});

fn extract<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    _cursor: &mut tree_sitter::TreeCursor<'a>,
) -> Vec<RawSymbol<'a>> {
    super::common::extract_with_query(tree, source, &JS_QUERY)
}

impl_language!(
    JavaScript,
    tree_sitter_javascript::LANGUAGE.into(),
    extract,
    &["js", "mjs", "cjs"]
);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::{SymbolKind, Visibility};

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_function_declaration() {
        let src = b"function hello() {}";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_async_function() {
        let src = b"async function fetch() {}";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_class_and_methods() {
        let src = b"class Foo {\n  constructor() {}\n  bar() {}\n}";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Method))
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn extract_exported_class() {
        let src = b"export class Foo { bar() {} }";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(class.visibility, Some(Visibility::Public));
    }

    #[test]
    fn js_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/javascript/functions.js"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
        insta::assert_json_snapshot!(symbols);
    }
}
