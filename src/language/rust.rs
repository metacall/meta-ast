use crate::language::{RawSymbol, impl_language};
use crate::model::{SymbolKind, Visibility};

use super::common::source_range_from_node;

fn extract<'a>(tree: &'a tree_sitter::Tree, source: &'a [u8]) -> Vec<RawSymbol<'a>> {
    let mut symbols = Vec::new();
    let root = tree.root_node();

    fn visit<'a>(
        node: tree_sitter::Node<'a>,
        source: &'a [u8],
        symbols: &mut Vec<RawSymbol<'a>>,
        in_impl: bool,
    ) {
        if node.is_error() || node.is_missing() {
            return;
        }

        match node.kind() {
            "function_item" => {
                if let Some(sym) = extract_function(&node, source, in_impl) {
                    symbols.push(sym);
                }
            }
            "struct_item" => {
                if let Some(sym) = extract_named_item(&node, source, SymbolKind::Struct) {
                    symbols.push(sym);
                }
            }
            "enum_item" => {
                if let Some(sym) = extract_named_item(&node, source, SymbolKind::Enum) {
                    symbols.push(sym);
                }
            }
            "trait_item" => {
                if let Some(sym) = extract_named_item(&node, source, SymbolKind::Trait) {
                    symbols.push(sym);
                }
            }
            "impl_item" => {
                for child in node.children(&mut node.walk()) {
                    visit(child, source, symbols, true);
                }
            }
            "const_item" => {
                if let Some(sym) = extract_named_item(&node, source, SymbolKind::Constant) {
                    symbols.push(sym);
                }
            }
            "static_item" => {
                if let Some(sym) = extract_named_item(&node, source, SymbolKind::Constant) {
                    symbols.push(sym);
                }
            }
            "type_item" => {
                if let Some(sym) = extract_named_item(&node, source, SymbolKind::TypeAlias) {
                    symbols.push(sym);
                }
            }
            "mod_item" => {
                if let Some(sym) = extract_named_item(&node, source, SymbolKind::Module) {
                    symbols.push(sym);
                }
            }
            _ => {
                for child in node.children(&mut node.walk()) {
                    visit(child, source, symbols, in_impl);
                }
            }
        }
    }

    visit(root, source, &mut symbols, false);
    symbols
}

fn extract_visibility(node: &tree_sitter::Node) -> Option<Visibility> {
    let has_vis = node
        .children(&mut node.walk())
        .any(|c| c.kind() == "visibility_modifier");
    if has_vis {
        Some(Visibility::Public)
    } else {
        None
    }
}

fn extract_function<'a>(
    node: &tree_sitter::Node<'a>,
    source: &'a [u8],
    in_impl: bool,
) -> Option<RawSymbol<'a>> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.into();

    let is_async = node.children(&mut node.walk()).any(|c| {
        if c.kind() == "function_modifiers" {
            c.children(&mut c.walk()).any(|gc| gc.kind() == "async")
        } else {
            false
        }
    });

    let signature = node
        .child_by_field_name("parameters")
        .and_then(|p| p.utf8_text(source).ok())
        .map(|s| s.into());

    Some(RawSymbol {
        name,
        kind: if in_impl {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        },
        source_range: source_range_from_node(node),
        visibility: extract_visibility(node),
        signature,
        docstring: None,
        is_async,
    })
}

fn extract_named_item<'a>(
    node: &tree_sitter::Node<'a>,
    source: &'a [u8],
    kind: SymbolKind,
) -> Option<RawSymbol<'a>> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.into();

    Some(RawSymbol {
        name,
        kind,
        source_range: source_range_from_node(node),
        visibility: extract_visibility(node),
        signature: None,
        docstring: None,
        is_async: false,
    })
}

impl_language!(Rust, tree_sitter_rust::LANGUAGE.into(), extract, &["rs"]);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::{SymbolKind, Visibility};

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn rust_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
    }

    #[test]
    fn extract_function() {
        let src = b"fn hello() {}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_pub_function() {
        let src = b"pub fn hello() {}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].visibility, Some(Visibility::Public));
    }

    #[test]
    fn extract_async_function() {
        let src = b"async fn fetch() {}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_struct() {
        let src = b"struct Point { x: f64, y: f64 }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Point");
        assert!(matches!(symbols[0].kind, SymbolKind::Struct));
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
    fn extract_trait() {
        let src = b"trait Shape { fn area(&self); }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let shape = symbols.iter().find(|s| s.name == "Shape").unwrap();
        assert!(matches!(shape.kind, SymbolKind::Trait));
    }

    #[test]
    fn extract_impl_methods() {
        let src = b"impl Foo { fn bar(&self) {} }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert!(matches!(bar.kind, SymbolKind::Method));
    }

    #[test]
    fn rust_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/rust/structs_enums.rs"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract(&tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
