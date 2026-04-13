use crate::language::{RawSymbol, impl_language};
use once_cell::sync::Lazy;

static TSX_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
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
    .expect("Failed to parse TSX query")
});

fn extract<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    _cursor: &mut tree_sitter::TreeCursor<'a>,
) -> Vec<RawSymbol<'a>> {
    super::common::extract_with_query(tree, source, &TSX_QUERY)
}

impl_language!(
    Tsx,
    tree_sitter_typescript::LANGUAGE_TSX.into(),
    extract,
    &["tsx"]
);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::{SymbolKind, Visibility};

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_tsx_function() {
        let src = b"function App(): JSX.Element { return <div/>; }";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "App");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_tsx_exported_class() {
        let src = b"export class Foo extends React.Component { render() { return <div/>; } }";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        assert_eq!(class.visibility, Some(Visibility::Public));
    }

    #[test]
    fn tsx_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tsx/components.tsx"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
        insta::assert_json_snapshot!(symbols);
    }
}
