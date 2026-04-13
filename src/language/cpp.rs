use crate::language::{RawSymbol, impl_language};
use once_cell::sync::Lazy;

use super::common::extract_with_query;

static CPP_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_cpp::LANGUAGE.into(),
        r#"
        (function_definition
          declarator: (function_declarator
            declarator: (_) @name
            parameters: (parameter_list) @signature
          )
        ) @kind.function

        (class_specifier
            name: (type_identifier) @name) @kind.class

        (struct_specifier
            name: (type_identifier) @name) @kind.struct

        (enum_specifier
            name: (type_identifier) @name) @kind.enum

        (namespace_definition
            name: (namespace_identifier) @name) @kind.namespace

        (type_definition
            declarator: (type_identifier) @name) @kind.type_alias
        "#,
    )
    .expect("Failed to parse C++ query")
});

fn extract<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    _cursor: &mut tree_sitter::TreeCursor<'a>,
) -> Vec<RawSymbol<'a>> {
    extract_with_query(tree, source, &CPP_QUERY)
}

impl_language!(
    Cpp,
    tree_sitter_cpp::LANGUAGE.into(),
    extract,
    &["cc", "cpp", "cxx"]
);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_class() {
        let src = b"class Foo { public: void bar() {} };";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(foo.kind, SymbolKind::Class));
    }

    #[test]
    fn extract_method() {
        let src = b"class Foo { public: void bar() {} };";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert!(matches!(bar.kind, SymbolKind::Method));
    }

    #[test]
    fn extract_namespace() {
        let src = b"namespace math { int add(int a, int b) { return a + b; } }";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let ns = symbols.iter().find(|s| s.name == "math").unwrap();
        assert!(matches!(ns.kind, SymbolKind::Namespace));
    }

    #[test]
    fn cpp_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/cpp/classes.cpp"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
        insta::assert_json_snapshot!(symbols);
    }
}
