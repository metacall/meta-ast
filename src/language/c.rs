use crate::language::{RawSymbol, impl_language};
use once_cell::sync::Lazy;

use super::common::extract_with_query;

static C_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_c::LANGUAGE.into(),
        r#"
        (function_definition
            declarator: [
                (function_declarator
                    declarator: (identifier) @name
                    parameters: (parameter_list) @signature)
                (function_declarator
                    declarator: (parenthesized_declarator
                        (identifier) @name)
                    parameters: (parameter_list) @signature)
                (pointer_declarator
                    declarator: (function_declarator
                        declarator: (identifier) @name
                        parameters: (parameter_list) @signature))
            ]) @kind.function

        (struct_specifier
            name: (type_identifier) @name) @kind.struct

        (enum_specifier
            name: (type_identifier) @name) @kind.enum

        (type_definition
            declarator: (type_identifier) @name) @kind.type_alias
        "#,
    )
    .expect("Failed to parse C query")
});

fn extract<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    _cursor: &mut tree_sitter::TreeCursor<'a>,
) -> Vec<RawSymbol<'a>> {
    extract_with_query(tree, source, &C_QUERY)
}

impl_language!(C, tree_sitter_c::LANGUAGE.into(), extract, &["c"]);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn c_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .unwrap();
    }

    #[test]
    fn extract_function() {
        let src = b"int add(int a, int b) { return a + b; }";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "add");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
        assert_eq!(symbols[0].signature.as_deref(), Some("(int a, int b)"));
    }

    #[test]
    fn extract_struct() {
        let src = b"struct Point { int x; int y; };";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let s = symbols.iter().find(|s| s.name == "Point").unwrap();
        assert!(matches!(s.kind, SymbolKind::Struct));
    }

    #[test]
    fn extract_enum() {
        let src = b"enum Color { RED, GREEN };";
        let tree = parse(src);
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src, &mut cursor);
        let s = symbols.iter().find(|s| s.name == "Color").unwrap();
        assert!(matches!(s.kind, SymbolKind::Enum));
    }

    #[test]
    fn c_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/c/functions.c"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let mut cursor = tree.walk();
        let symbols = extract(&tree, src.as_bytes(), &mut cursor);
        insta::assert_json_snapshot!(symbols);
    }
}
