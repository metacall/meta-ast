use crate::model::{LineColumn, SourceRange};

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

#[inline]
pub(super) fn field_text(
    node: &tree_sitter::Node,
    field_name: &str,
    source: &[u8],
) -> Option<String> {
    let field = node.child_by_field_name(field_name)?;
    field.utf8_text(source).ok().map(str::to_owned)
}

#[inline]
pub(super) fn has_child_kind(node: &tree_sitter::Node, kind: &str) -> bool {
    node.children(&mut node.walk()).any(|c| c.kind() == kind)
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;

    use crate::language::LangId;

    use super::{field_text, has_child_kind, source_range_from_node};

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

    #[test]
    fn has_child_kind_detects_async_marker() {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::language::grammar_for(LangId::JavaScript))
            .unwrap();

        let source = b"async function hello() {}";
        let tree = parser.parse(source, None).unwrap();
        let root = tree.root_node();
        let function = root.named_child(0).unwrap();

        assert!(has_child_kind(&function, "async"));
    }
}
