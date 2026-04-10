use crate::language::{RawSymbol, impl_language};
use crate::model::{LineColumn, SourceRange, SymbolKind};

fn extract(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawSymbol> {
    let mut symbols = Vec::new();
    let root = tree.root_node();

    fn visit(node: tree_sitter::Node, source: &[u8], symbols: &mut Vec<RawSymbol>) {
        if node.is_error() || node.is_missing() {
            return;
        }

        match node.kind() {
            "function_declaration" => {
                if let Some(sym) = extract_function(&node, source) {
                    symbols.push(sym);
                }
            }
            "method_declaration" => {
                if let Some(sym) = extract_method(&node, source) {
                    symbols.push(sym);
                }
            }
            "type_declaration" => {
                extract_type_declaration(&node, source, symbols);
            }
            "const_declaration" => {
                extract_const_declaration(&node, source, symbols);
            }
            "var_declaration" => {
                extract_var_declaration(&node, source, symbols);
            }
            _ => {
                for child in node.children(&mut node.walk()) {
                    visit(child, source, symbols);
                }
            }
        }
    }

    visit(root, source, &mut symbols);
    symbols
}

fn source_range_from_node(node: &tree_sitter::Node) -> SourceRange {
    SourceRange {
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        start: LineColumn {
            line: node.start_position().row,
            column: node.start_position().column,
        },
        end: LineColumn {
            line: node.end_position().row,
            column: node.end_position().column,
        },
    }
}

fn extract_function(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    let signature = node
        .child_by_field_name("parameters")
        .and_then(|p| p.utf8_text(source).ok())
        .map(|s| s.to_string());

    Some(RawSymbol {
        name,
        kind: SymbolKind::Function,
        source_range: source_range_from_node(node),
        visibility: None,
        signature,
        docstring: None,
        is_async: false,
    })
}

fn extract_method(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    let signature = node
        .child_by_field_name("parameters")
        .and_then(|p| p.utf8_text(source).ok())
        .map(|s| s.to_string());

    Some(RawSymbol {
        name,
        kind: SymbolKind::Method,
        source_range: source_range_from_node(node),
        visibility: None,
        signature,
        docstring: None,
        is_async: false,
    })
}

fn extract_type_declaration(node: &tree_sitter::Node, source: &[u8], symbols: &mut Vec<RawSymbol>) {
    for child in node.children(&mut node.walk()) {
        if child.kind() == "type_spec" {
            let name_node = child
                .children(&mut child.walk())
                .find(|c| c.kind() == "type_identifier");
            let Some(name_node) = name_node else {
                continue;
            };
            let name = match name_node.utf8_text(source) {
                Ok(n) => n.to_string(),
                Err(_) => continue,
            };

            let kind = child
                .child_by_field_name("type")
                .map(|t| t.kind())
                .map(|k| match k {
                    "struct_type" => SymbolKind::Struct,
                    "interface_type" => SymbolKind::Interface,
                    _ => SymbolKind::Struct,
                })
                .unwrap_or(SymbolKind::Struct);

            symbols.push(RawSymbol {
                name,
                kind,
                source_range: source_range_from_node(node),
                visibility: None,
                signature: None,
                docstring: None,
                is_async: false,
            });
        }
    }
}

fn extract_const_declaration(
    node: &tree_sitter::Node,
    source: &[u8],
    symbols: &mut Vec<RawSymbol>,
) {
    for child in node.children(&mut node.walk()) {
        if child.kind() == "const_spec" {
            let name_node = child.child_by_field_name("name");
            if let Some(name_node) = name_node
                && let Ok(name) = name_node.utf8_text(source)
            {
                symbols.push(RawSymbol {
                    name: name.to_string(),
                    kind: SymbolKind::Constant,
                    source_range: source_range_from_node(&child),
                    visibility: None,
                    signature: None,
                    docstring: None,
                    is_async: false,
                });
            }
        }
    }
}

fn extract_var_declaration(node: &tree_sitter::Node, source: &[u8], symbols: &mut Vec<RawSymbol>) {
    for child in node.children(&mut node.walk()) {
        if child.kind() == "var_spec" {
            let name_node = child.child_by_field_name("name");
            if let Some(name_node) = name_node
                && let Ok(name) = name_node.utf8_text(source)
            {
                symbols.push(RawSymbol {
                    name: name.to_string(),
                    kind: SymbolKind::Object,
                    source_range: source_range_from_node(&child),
                    visibility: None,
                    signature: None,
                    docstring: None,
                    is_async: false,
                });
            }
        }
    }
}

impl_language!(Go, tree_sitter_go::LANGUAGE.into(), extract, &["go"]);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn go_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
    }

    #[test]
    fn extract_function() {
        let src = b"package main\n\nfunc Hello() {}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_struct() {
        let src = b"package main\n\ntype Rect struct {\n\tWidth float64\n}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Rect");
        assert!(matches!(symbols[0].kind, SymbolKind::Struct));
    }

    #[test]
    fn extract_method_with_receiver() {
        let src = b"package main\n\nfunc (r *Rect) Area() float64 { return 0 }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Area");
        assert!(matches!(symbols[0].kind, SymbolKind::Method));
    }

    #[test]
    fn extract_interface() {
        let src = b"package main\n\ntype Writer interface {\n\tWrite([]byte) error\n}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Writer");
        assert!(matches!(symbols[0].kind, SymbolKind::Interface));
    }

    #[test]
    fn go_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/go/methods.go"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract(&tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
