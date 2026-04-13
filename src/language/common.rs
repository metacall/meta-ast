use super::RawSymbol;
use crate::model::{LineColumn, SourceRange, SymbolKind, Visibility};
use tree_sitter::StreamingIterator;

#[inline]
pub(super) fn source_range_from_node(node: &tree_sitter::Node) -> SourceRange {
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

#[cfg(test)]
#[inline]
pub(super) fn field_text<'a>(
    node: &tree_sitter::Node<'a>,
    field_name: &str,
    source: &'a [u8],
) -> Option<std::borrow::Cow<'a, str>> {
    let field = node.child_by_field_name(field_name)?;
    field.utf8_text(source).ok().map(|s| s.into())
}

pub(super) fn extract_with_query<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    query: &tree_sitter::Query,
) -> Vec<RawSymbol<'a>> {
    use std::collections::HashMap;
    let mut symbols_map: HashMap<usize, (RawSymbol<'a>, usize)> = HashMap::new();
    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut matches = query_cursor.matches(query, tree.root_node(), source);

    while let Some(m) = matches.next() {
        let mut name: Option<std::borrow::Cow<'a, str>> = None;
        let mut kind: Option<SymbolKind> = None;
        let mut signature: Option<std::borrow::Cow<'a, str>> = None;
        let mut docstring: Option<std::borrow::Cow<'a, str>> = None;
        let mut is_async = false;
        let mut visibility: Option<Visibility> = None;
        let mut primary_node: Option<tree_sitter::Node<'a>> = None;

        let capture_count = m.captures.len();

        for capture in m.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            match capture_name {
                "name" => {
                    if let Ok(text) = capture.node.utf8_text(source) {
                        name = Some(std::borrow::Cow::Borrowed(text));
                    }
                }
                "signature" => {
                    if let Ok(text) = capture.node.utf8_text(source) {
                        signature = Some(std::borrow::Cow::Borrowed(text));
                    }
                }
                "docstring" => {
                    if let Ok(text) = capture.node.utf8_text(source) {
                        let cleaned = text.trim_matches(|c: char| {
                            c == '\'' || c == '"' || c == ' ' || c == '\n' || c == '\r'
                        });
                        docstring = Some(std::borrow::Cow::Borrowed(cleaned));
                    }
                }
                "async" => is_async = true,
                "visibility.public" => visibility = Some(Visibility::Public),
                "visibility.private" => visibility = Some(Visibility::Private),
                c if c.starts_with("kind.") => {
                    let k = match &c[5..] {
                        "function" => SymbolKind::Function,
                        "method" => SymbolKind::Method,
                        "class" => SymbolKind::Class,
                        "struct" => SymbolKind::Struct,
                        "interface" => SymbolKind::Interface,
                        "trait" => SymbolKind::Trait,
                        "enum" => SymbolKind::Enum,
                        "constant" => SymbolKind::Constant,
                        "module" => SymbolKind::Module,
                        "namespace" => SymbolKind::Namespace,
                        "type_alias" => SymbolKind::TypeAlias,
                        "object" => SymbolKind::Object,
                        _ => continue,
                    };
                    kind = Some(k);
                    primary_node = Some(capture.node);
                }
                _ => {}
            }
        }

        if let (Some(name), Some(mut kind), Some(node)) = (name, kind, primary_node) {
            let node_id = node.id();

            // Context-sensitive kind refinement: Function -> Method if inside a class-like node
            if kind == SymbolKind::Function {
                let mut parent = node.parent();
                while let Some(p) = parent {
                    match p.kind() {
                        "class_definition"
                        | "class_declaration"
                        | "class_specifier"
                        | "impl_item"
                        | "class"
                        | "interface_declaration"
                        | "struct_item" => {
                            kind = SymbolKind::Method;
                            break;
                        }
                        _ => parent = p.parent(),
                    }
                }
            }

            // Automatic visibility detection if not explicitly captured
            // This is primarily for JavaScript/TypeScript where an export_statement
            // ancestor implies public visibility for its children.
            // but i think there's more proper way to do this by having language-specific queries capture visibility more accurately,
            // so this is just a fallback that works very well for now to refactor later
            // TODO!: Refactor language-specific queries to capture visibility more accurately, reducing reliance on this heuristic
            if visibility.is_none() {
                let lang_ptr = node.language().clone().into_raw();
                if lang_ptr
                    == crate::language::grammar_for(crate::language::LangId::JavaScript).into_raw()
                    || lang_ptr
                        == crate::language::grammar_for(crate::language::LangId::TypeScript)
                            .into_raw()
                    || lang_ptr
                        == crate::language::grammar_for(crate::language::LangId::Tsx).into_raw()
                {
                    let mut parent = node.parent();
                    while let Some(p) = parent {
                        if p.kind() == "export_statement" {
                            visibility = Some(Visibility::Public);
                            break;
                        }
                        parent = p.parent();
                    }
                }
            }

            let symbol = RawSymbol {
                name,
                kind,
                source_range: source_range_from_node(&node),
                visibility,
                signature,
                docstring,
                is_async,
            };

            // If we already have this node, only replace if the new match has more captures
            // (which usually means it's a more specific match like a decorated definition)
            if let Some((_, existing_count)) = symbols_map.get(&node_id) {
                if capture_count > *existing_count {
                    symbols_map.insert(node_id, (symbol, capture_count));
                }
            } else {
                symbols_map.insert(node_id, (symbol, capture_count));
            }
        }
    }

    let mut result: Vec<_> = symbols_map.into_values().map(|(s, _)| s).collect();
    // Sort by position to maintain order
    result.sort_by_key(|s| s.source_range.byte_start);
    result
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;

    use crate::language::LangId;

    use super::{field_text, source_range_from_node};

    #[test]
    fn source_range_tracks_node_positions() {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::language::grammar_for(LangId::JavaScript))
            .unwrap();

        let source = b"function hello(a, b) {}";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let function = root.named_child(0).unwrap();

        let range = source_range_from_node(&function);
        assert_eq!(range.byte_start, function.start_byte());
        assert_eq!(range.byte_end, function.end_byte());
    }

    #[test]
    fn field_text_extracts_name_field() {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::language::grammar_for(LangId::JavaScript))
            .unwrap();

        let source = b"function hello(a, b) {}";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let function = root.named_child(0).unwrap();

        let name = field_text(&function, "name", source).unwrap();
        assert_eq!(name, "hello");
    }
}
