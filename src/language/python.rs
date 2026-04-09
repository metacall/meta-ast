use crate::language::{RawSymbol, impl_language};

fn extract(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawSymbol> {
    let mut symbols = Vec::new();
    let root = tree.root_node();

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
            "class_definition" => {
                if let Some(sym) = extract_class(&node, source) {
                    symbols.push(sym);
                }
                for child in node.children(&mut node.walk()) {
                    if (child.kind() == "function_definition"
                        || child.kind() == "decorated_definition")
                        && let Some(sym) = extract_method(&child, source)
                    {
                        symbols.push(sym);
                    }
                }
            }
            "decorated_definition" => {
                for child in node.children(&mut node.walk()) {
                    if child.kind() == "function_definition" {
                        if let Some(sym) = extract_function(&child, source) {
                            symbols.push(sym);
                        }
                    } else if child.kind() == "class_definition" {
                        if let Some(sym) = extract_class(&child, source) {
                            symbols.push(sym);
                        }
                        for inner in child.children(&mut child.walk()) {
                            if (inner.kind() == "function_definition"
                                || inner.kind() == "decorated_definition")
                                && let Some(sym) = extract_method(&inner, source)
                            {
                                symbols.push(sym);
                            }
                        }
                    }
                }
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

fn extract_function(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    let is_async = node.children(&mut node.walk()).any(|c| c.kind() == "async");

    let signature = extract_signature(node, source);

    Some(RawSymbol {
        name,
        kind: crate::model::SymbolKind::Function,
        source_range: source_range_from_node(node),
        visibility: None,
        signature,
        docstring: extract_docstring(node, source),
        is_async,
    })
}

fn extract_class(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    let signature = extract_signature(node, source);

    Some(RawSymbol {
        name,
        kind: crate::model::SymbolKind::Class,
        source_range: source_range_from_node(node),
        visibility: None,
        signature,
        docstring: extract_docstring(node, source),
        is_async: false,
    })
}

fn extract_method(node: &tree_sitter::Node, source: &[u8]) -> Option<RawSymbol> {
    let target = if node.kind() == "decorated_definition" {
        node.children(&mut node.walk())
            .find(|c| c.kind() == "function_definition")?
    } else {
        *node
    };

    let name_node = target.child_by_field_name("name")?;
    let name = name_node.utf8_text(source).ok()?.to_string();

    let is_async = target
        .children(&mut target.walk())
        .any(|c| c.kind() == "async");

    let signature = extract_signature(&target, source);

    Some(RawSymbol {
        name,
        kind: crate::model::SymbolKind::Method,
        source_range: source_range_from_node(node),
        visibility: None,
        signature,
        docstring: extract_docstring(&target, source),
        is_async,
    })
}

fn source_range_from_node(node: &tree_sitter::Node) -> crate::model::SourceRange {
    crate::model::SourceRange {
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
    Some(params.utf8_text(source).ok()?.to_string())
}

fn extract_docstring(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let body = node.child_by_field_name("body")?;
    let first_stmt = body.child(0)?;
    if first_stmt.kind() == "expression_statement"
        && let Some(expr) = first_stmt.child(0)
        && expr.kind() == "string"
    {
        return expr
            .utf8_text(source)
            .ok()
            .map(|s| s.trim_matches(|c: char| c == '\'' || c == '"').to_string());
    }
    None
}

impl_language!(
    Python,
    tree_sitter_python::LANGUAGE.into(),
    extract,
    &["py", "pyi"]
);

#[cfg(test)]
mod tests {
    use super::extract;
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn python_grammar_loads() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
    }

    #[test]
    fn extract_simple_function() {
        let tree = parse(b"def hello(): pass");
        let symbols = extract(&tree, b"def hello(): pass");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_async_function() {
        let tree = parse(b"async def fetch(): pass");
        let symbols = extract(&tree, b"async def fetch(): pass");
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_class_and_methods() {
        let src =
            "class Foo:\n    def __init__(self):\n        pass\n    def bar(self):\n        pass\n";
        let tree = parse(src.as_bytes());
        let symbols = extract(&tree, src.as_bytes());
        let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(foo.kind, SymbolKind::Class));
    }

    #[test]
    fn extract_decorated_function() {
        let src = "@decorator\ndef decorated_func(x):\n    return x * 2\n";
        let tree = parse(src.as_bytes());
        let symbols = extract(&tree, src.as_bytes());
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "decorated_func");
    }

    #[test]
    fn skip_error_nodes() {
        let src = b"def valid_function(): pass\ndef broken_function(\n";
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"valid_function"));
    }

    #[test]
    fn extract_function_with_docstring() {
        let src = br#"def greet():
    """Say hello."""
    pass
"#;
        let tree = parse(src);
        let symbols = extract(&tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].docstring.as_deref(), Some("Say hello."));
    }

    #[test]
    fn extract_class_with_docstring() {
        let src = br#"class Foo:
    """A foo class."""
    pass
"#;
        let tree = parse(src);
        let symbols = extract(&tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(class.docstring.as_deref(), Some("A foo class."));
    }

    #[test]
    fn python_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/python/simple_functions.py"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract(&tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
