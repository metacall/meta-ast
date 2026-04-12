use crate::language::{RawSymbol, impl_language};
use crate::model::{SymbolKind, Visibility};

use super::common::{field_text, has_child_kind, source_range_from_node};

fn extract_signature<'a>(
    node: &tree_sitter::Node<'a>,
    source: &'a [u8],
) -> Option<std::borrow::Cow<'a, str>> {
    field_text(node, "parameters", source).map(std::borrow::Cow::from)
}

fn extract_named_symbol<'a>(
    node: &tree_sitter::Node<'a>,
    source: &'a [u8],
    kind: SymbolKind,
    visibility: Option<Visibility>,
) -> Option<RawSymbol<'a>> {
    let name = field_text(node, "name", source)?.into();
    let is_async = has_child_kind(node, "async");
    let signature = extract_signature(node, source);

    Some(RawSymbol {
        name,
        kind,
        source_range: source_range_from_node(node),
        visibility,
        signature,
        docstring: None,
        is_async,
    })
}

fn extract_class_with_methods<'a>(
    node: &tree_sitter::Node<'a>,
    source: &'a [u8],
    visibility: Option<Visibility>,
) -> Vec<RawSymbol<'a>> {
    let mut symbols = Vec::new();

    if let Some(sym) = extract_named_symbol(node, source, SymbolKind::Class, visibility) {
        symbols.push(sym);
    }

    if let Some(body) = node.child_by_field_name("body") {
        for child in body.children(&mut body.walk()) {
            if child.kind() == "method_definition"
                && let Some(method_sym) =
                    extract_named_symbol(&child, source, SymbolKind::Method, visibility)
            {
                symbols.push(method_sym);
            }
        }
    }

    symbols
}

fn extract<'a>(tree: &'a tree_sitter::Tree, source: &'a [u8]) -> Vec<RawSymbol<'a>> {
    let mut symbols = Vec::new();
    let root = tree.root_node();

    fn visit<'a>(
        node: tree_sitter::Node<'a>,
        source: &'a [u8],
        symbols: &mut Vec<RawSymbol<'a>>,
        exported: bool,
    ) {
        if node.is_error() || node.is_missing() {
            return;
        }

        let visibility = if exported {
            Some(Visibility::Public)
        } else {
            None
        };

        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(sym) =
                    extract_named_symbol(&node, source, SymbolKind::Function, visibility)
                {
                    symbols.push(sym);
                }
            }
            "function" => {
                if node.child_by_field_name("name").is_some()
                    && let Some(sym) =
                        extract_named_symbol(&node, source, SymbolKind::Function, visibility)
                {
                    symbols.push(sym);
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                symbols.extend(extract_class_with_methods(&node, source, visibility));
            }
            "interface_declaration" => {
                if let Some(sym) =
                    extract_named_symbol(&node, source, SymbolKind::Interface, visibility)
                {
                    symbols.push(sym);
                }
            }
            "type_alias_declaration" => {
                if let Some(sym) =
                    extract_named_symbol(&node, source, SymbolKind::TypeAlias, visibility)
                {
                    symbols.push(sym);
                }
            }
            "enum_declaration" => {
                if let Some(sym) = extract_named_symbol(&node, source, SymbolKind::Enum, visibility)
                {
                    symbols.push(sym);
                }
            }
            "export_statement" => {
                for child in node.children(&mut node.walk()) {
                    if child.kind() != "export" {
                        visit(child, source, symbols, true);
                    }
                }
            }
            _ => {
                for child in node.children(&mut node.walk()) {
                    visit(child, source, symbols, exported);
                }
            }
        }
    }

    visit(root, source, &mut symbols, false);
    symbols
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
    fn ts_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
    }

    #[test]
    fn extract_interface() {
        let src = b"interface Foo { bar(): void; }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Foo");
        assert!(matches!(symbols[0].kind, SymbolKind::Interface));
    }

    #[test]
    fn extract_type_alias() {
        let src = b"type Point = { x: number; };";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Point");
        assert!(matches!(symbols[0].kind, SymbolKind::TypeAlias));
    }

    #[test]
    fn extract_enum() {
        let src = b"enum Dir { A, B }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Dir");
        assert!(matches!(symbols[0].kind, SymbolKind::Enum));
    }

    #[test]
    fn extract_class_with_methods() {
        let src = b"class Foo {\n  constructor() {}\n  bar(): void {}\n}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Method))
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn ts_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/typescript/interfaces.ts"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract(&tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
