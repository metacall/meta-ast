use crate::language::{RawSymbol, impl_language};
use crate::model::SymbolKind;

use super::common::source_range_from_node;

fn extract(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawSymbol> {
    let mut symbols = Vec::new();
    visit(tree.root_node(), source, &mut symbols);
    symbols
}

fn visit(node: tree_sitter::Node, source: &[u8], symbols: &mut Vec<RawSymbol>) {
    if node.is_error() || node.is_missing() {
        return;
    }

    match node.kind() {
        "function_definition" => {
            if let Some(sym) = extract_function(&node, source) {
                symbols.push(sym);
            }
        }
        "struct_specifier" => {
            if let Some(sym) = extract_struct(&node, source) {
                symbols.push(sym);
            }
        }
        "enum_specifier" => {
            if let Some(sym) = extract_enum(&node, source) {
                symbols.push(sym);
            }
        }
        "type_definition" => {
            if let Some(sym) = extract_typedef(&node, source) {
                symbols.push(sym);
            }
        }
        _ => {
            for child in node.children(&mut node.walk()) {
                visit(child, source, symbols);
            }
        }
    }
}

fn get_c_function_name(declarator: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    match declarator.kind() {
        "function_declarator" | "parenthesized_declarator" => {
            let inner = declarator.child_by_field_name("declarator")?;
            get_c_function_name(&inner, source)
        }
        "identifier" => declarator.utf8_text(source).ok().map(String::from),
        _ => None,
    }
}

fn get_c_function_params<'a>(declarator: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    match declarator.kind() {
        "function_declarator" => declarator.child_by_field_name("parameters"),
        "parenthesized_declarator" => {
            let inner = declarator.child_by_field_name("declarator")?;
            get_c_function_params(&inner)
        }
        _ => None,
    }
}

fn extract_function(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = get_c_function_name(&declarator, source)?;

    let signature = get_c_function_params(&declarator)
        .and_then(|p: tree_sitter::Node<'_>| p.utf8_text(source).ok().map(String::from));

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

fn extract_struct(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    Some(RawSymbol {
        name,
        kind: SymbolKind::Struct,
        source_range: source_range_from_node(node),
        visibility: None,
        signature: None,
        docstring: None,
        is_async: false,
    })
}

fn extract_enum(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    Some(RawSymbol {
        name,
        kind: SymbolKind::Enum,
        source_range: source_range_from_node(node),
        visibility: None,
        signature: None,
        docstring: None,
        is_async: false,
    })
}

fn extract_typedef(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = declarator.utf8_text(source).ok()?.to_string();

    Some(RawSymbol {
        name,
        kind: SymbolKind::TypeAlias,
        source_range: source_range_from_node(node),
        visibility: None,
        signature: None,
        docstring: None,
        is_async: false,
    })
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
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "add");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
        assert_eq!(symbols[0].signature.as_deref(), Some("(int a, int b)"));
    }

    #[test]
    fn extract_struct() {
        let src = b"struct Point { int x; int y; };";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let s = symbols.iter().find(|s| s.name == "Point").unwrap();
        assert!(matches!(s.kind, SymbolKind::Struct));
    }

    #[test]
    fn extract_enum() {
        let src = b"enum Color { RED, GREEN };";
        let tree = parse(src);
        let symbols = extract(&tree, src);
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
        let symbols = extract(&tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
