use crate::language::{RawSymbol, impl_language};
use crate::model::{SourceRange, SymbolKind, Visibility};

fn source_range_from_node(node: &tree_sitter::Node) -> SourceRange {
    SourceRange {
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        start: crate::model::LineColumn {
            line: node.start_position().row,
            column: node.start_position().column,
        },
        end: crate::model::LineColumn {
            line: node.end_position().row,
            column: node.end_position().column,
        },
    }
}

fn extract_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let params = node.child_by_field_name("parameters")?;
    params.utf8_text(source).ok().map(|s| s.to_string())
}

fn has_async_child(node: &tree_sitter::Node) -> bool {
    node.children(&mut node.walk()).any(|c| c.kind() == "async")
}

fn extract_named_symbol(
    node: &tree_sitter::Node,
    source: &[u8],
    kind: SymbolKind,
    visibility: Option<Visibility>,
) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();
    let is_async = has_async_child(node);
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

fn extract_class_with_methods(
    node: &tree_sitter::Node,
    source: &[u8],
    visibility: Option<Visibility>,
) -> Vec<RawSymbol> {
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

fn extract(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawSymbol> {
    let mut symbols = Vec::new();
    let root = tree.root_node();

    fn visit(node: tree_sitter::Node, source: &[u8], symbols: &mut Vec<RawSymbol>, exported: bool) {
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
    fn js_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
    }

    #[test]
    fn extract_function_declaration() {
        let src = b"function hello() {}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_async_function() {
        let src = b"async function fetch() {}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_class_and_methods() {
        let src = b"class Foo {\n  constructor() {}\n  bar() {}\n}";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Method))
            .collect();
        assert_eq!(methods.len(), 2);
        let method_names: Vec<_> = methods.iter().map(|m| m.name.as_str()).collect();
        assert!(method_names.contains(&"constructor"));
        assert!(method_names.contains(&"bar"));
    }

    #[test]
    fn extract_exported_class() {
        let src = b"export class Foo { bar() {} }";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(class.visibility, Some(Visibility::Public));
        let method = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(method.visibility, Some(Visibility::Public));
    }

    #[test]
    fn js_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/javascript/functions.js"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract(&tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
