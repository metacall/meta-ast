use crate::language::{RawSymbol, impl_language};
use once_cell::sync::Lazy;

static TS_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
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
  (type_identifier) @name
) @kind.class

(abstract_class_declaration
  (type_identifier) @name
) @kind.class

(interface_declaration
  (type_identifier) @name
) @kind.interface

(enum_declaration
  (identifier) @name
) @kind.enum

(type_alias_declaration
  (type_identifier) @name
) @kind.type_alias

(method_definition
  "async"? @async
  name: (_) @name
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
      (type_identifier) @name
    ) @kind.class
    (abstract_class_declaration
      (type_identifier) @name
    ) @kind.class
    (interface_declaration
      (type_identifier) @name
    ) @kind.interface
    (enum_declaration
      (identifier) @name
    ) @kind.enum
    (type_alias_declaration
      (type_identifier) @name
    ) @kind.type_alias
  ]
) @visibility.public
"#,
    )
    .expect("Failed to parse TypeScript query")
});

fn extract<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    _cursor: &mut tree_sitter::TreeCursor<'a>,
) -> Vec<RawSymbol<'a>> {
    super::common::extract_with_query(tree, source, &TS_QUERY)
}

impl_language!(
    TypeScript,
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    extract,
    &["ts", "cts", "mts"]
);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_interface() {
        let src = b"interface Foo { bar(): void; }";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Foo");
        assert!(matches!(symbols[0].kind, SymbolKind::Interface));
    }

    #[test]
    fn extract_type_alias() {
        let src = b"type Point = { x: number; };";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Point");
        assert!(matches!(symbols[0].kind, SymbolKind::TypeAlias));
    }

    #[test]
    fn extract_enum() {
        let src = b"enum Dir { A, B }";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Dir");
        assert!(matches!(symbols[0].kind, SymbolKind::Enum));
    }

    #[test]
    fn ts_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/typescript/interfaces.ts"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
        insta::assert_json_snapshot!(symbols);
    }
}
