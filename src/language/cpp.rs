use crate::language::{RawSymbol, impl_language};
use crate::model::SymbolKind;

use super::common::source_range_from_node;

fn extract(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawSymbol> {
    let mut symbols = Vec::new();
    visit(tree.root_node(), source, &mut symbols, false);
    symbols
}

fn visit(node: tree_sitter::Node, source: &[u8], symbols: &mut Vec<RawSymbol>, in_class: bool) {
    if node.is_error() || node.is_missing() {
        return;
    }

    match node.kind() {
        "function_definition" => {
            if in_class {
                if let Some(sym) = extract_method(&node, source) {
                    symbols.push(sym);
                }
            } else if let Some(sym) = extract_function(&node, source) {
                symbols.push(sym);
            }
        }
        "declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "class_specifier" => {
                        if let Some(sym) = extract_class(&child, source) {
                            symbols.push(sym);
                        }
                        if let Some(body) = child.child_by_field_name("body") {
                            for member in body.children(&mut body.walk()) {
                                visit(member, source, symbols, true);
                            }
                        }
                    }
                    "struct_specifier" => {
                        if let Some(sym) = extract_struct(&child, source) {
                            symbols.push(sym);
                        }
                    }
                    "enum_specifier" => {
                        if let Some(sym) = extract_enum(&child, source) {
                            symbols.push(sym);
                        }
                    }
                    _ => {}
                }
            }
        }
        "namespace_definition" => {
            if let Some(sym) = extract_namespace(&node, source) {
                symbols.push(sym);
            }
            for child in node.children(&mut node.walk()) {
                visit(child, source, symbols, false);
            }
        }
        "template_declaration" => {
            for child in node.children(&mut node.walk()) {
                visit(child, source, symbols, in_class);
            }
        }
        "type_definition" => {
            if let Some(sym) = extract_typedef(&node, source) {
                symbols.push(sym);
            }
        }
        "class_specifier" => {
            if let Some(sym) = extract_class(&node, source) {
                symbols.push(sym);
            }
            if let Some(body) = node.child_by_field_name("body") {
                for member in body.children(&mut body.walk()) {
                    visit(member, source, symbols, true);
                }
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
        _ => {
            for child in node.children(&mut node.walk()) {
                visit(child, source, symbols, in_class);
            }
        }
    }
}

fn get_cpp_function_name(declarator: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    match declarator.kind() {
        "function_declarator" | "parenthesized_declarator" => {
            let inner = declarator.child_by_field_name("declarator")?;
            get_cpp_function_name(&inner, source)
        }
        "identifier" | "field_identifier" => declarator.utf8_text(source).ok().map(String::from),
        "qualified_identifier" => declarator
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(source).ok().map(String::from)),
        _ => None,
    }
}

fn get_cpp_function_params<'a>(
    declarator: &tree_sitter::Node<'a>,
) -> Option<tree_sitter::Node<'a>> {
    match declarator.kind() {
        "function_declarator" => declarator.child_by_field_name("parameters"),
        "parenthesized_declarator" => {
            let inner = declarator.child_by_field_name("declarator")?;
            get_cpp_function_params(&inner)
        }
        _ => None,
    }
}

fn extract_function(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = get_cpp_function_name(&declarator, source)?;

    let signature = get_cpp_function_params(&declarator)
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

fn extract_method(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let declarator = node.child_by_field_name("declarator")?;
    let name = get_cpp_function_name(&declarator, source)?;

    let signature = get_cpp_function_params(&declarator)
        .and_then(|p: tree_sitter::Node<'_>| p.utf8_text(source).ok().map(String::from));

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

fn extract_class(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    Some(RawSymbol {
        name,
        kind: SymbolKind::Class,
        source_range: source_range_from_node(node),
        visibility: None,
        signature: None,
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

fn extract_namespace(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    Some(RawSymbol {
        name,
        kind: SymbolKind::Namespace,
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
    fn cpp_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .unwrap();
    }

    #[test]
    fn extract_class() {
        let src = b"class Foo { public: void bar() {} };";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(foo.kind, SymbolKind::Class));
    }

    #[test]
    fn extract_method() {
        let src = b"class Foo { public: void bar() {} };";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert!(matches!(bar.kind, SymbolKind::Method));
    }

    #[test]
    fn extract_namespace() {
        let src = b"namespace math { int add(int a, int b) { return a + b; } }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
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
        let symbols = extract(&tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
