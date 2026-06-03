//! Shared extraction engine for all language packs.
//!
//! Provides `extract_with_spec` (symbols) and the combined
//! `extract_imports_and_references_with_spec` that runs a single
//! tree-sitter query traversal for both imports and references.

use super::{LanguageSpec, RawSymbol};
use crate::model::{LineColumn, SourceRange, SymbolKind, Visibility};
use tree_sitter::StreamingIterator;

pub(crate) fn compile_query(
    lang: &tree_sitter::Language,
    src: &str,
    label: &str,
) -> tree_sitter::Query {
    match tree_sitter::Query::new(lang, src) {
        Ok(q) => q,
        Err(e) => {
            panic!("query compilation failed for {label}: {e}");
        }
    }
}

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
fn clean_docstring(text: &str) -> &str {
    let s = text.trim();
    if s.is_empty() {
        return "";
    }
    for delim in ["\"\"\"", "'''"] {
        if let Some(inner) = s.strip_prefix(delim)
            && let Some(inner) = inner.strip_suffix(delim)
        {
            return inner.trim();
        }
    }
    for delim in ["\"", "'"] {
        if let Some(inner) = s.strip_prefix(delim)
            && let Some(inner) = inner.strip_suffix(delim)
        {
            return inner.trim();
        }
    }
    s
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

pub(crate) fn extract_with_spec<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    spec: &LanguageSpec,
) -> Vec<RawSymbol<'a>> {
    use std::collections::HashMap;
    let mut symbols_map: HashMap<usize, (RawSymbol<'a>, usize)> = HashMap::new();
    let mut query_cursor = tree_sitter::QueryCursor::new();
    let query = (spec.query_fn)();
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
                        let cleaned = clean_docstring(text);

                        docstring = Some(std::borrow::Cow::Borrowed(cleaned));
                    }
                }
                "async" => is_async = true,
                "visibility.public" => {
                    if capture.node.named_child_count() == 0 {
                        visibility = Some(Visibility::Public);
                    }
                }
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
                        "static" => SymbolKind::Static,
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

            if kind == SymbolKind::Function {
                let mut parent = node.parent();
                while let Some(p) = parent {
                    if spec.class_like_parents.contains(&p.kind()) {
                        kind = SymbolKind::Method;
                        break;
                    }
                    parent = p.parent();
                }
            }

            if visibility.is_none() && !spec.ancestor_visibility_rules.is_empty() {
                let mut parent = node.parent();
                while let Some(p) = parent {
                    for (ancestor_kind, vis) in spec.ancestor_visibility_rules {
                        if p.kind() == *ancestor_kind {
                            visibility = Some(*vis);
                            break;
                        }
                    }
                    if visibility.is_some() {
                        break;
                    }
                    parent = p.parent();
                }
            }

            if visibility.is_none()
                && let Some(f) = spec.visibility_from_name
            {
                visibility = f(&name);
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
    associate_docstrings(&mut result, source, tree, spec);
    result.sort_by_key(|s| s.source_range.byte_start);
    result
}

/// Associate doc comments with symbols via post-processing.
///
/// For languages where doc comments are tree-sitter extras (Rust, JS/TS, etc.),
/// this function scans the source for comment nodes and associates them with
/// the nearest following symbol based on proximity.
pub(crate) fn associate_docstrings<'a>(
    symbols: &mut Vec<RawSymbol<'a>>,
    source: &'a [u8],
    tree: &'a tree_sitter::Tree,
    spec: &LanguageSpec,
) {
    let Some(config) = &spec.doc_comment_config else {
        return;
    };

    let comments = collect_comment_nodes(tree, source, config);

    for sym in symbols.iter_mut() {
        if sym.docstring.is_some() {
            continue;
        }
        let sym_start = sym.source_range.byte_start;
        let doc = find_preceding_docstring(sym_start, &comments, source, config);
        if let Some(text) = doc {
            sym.docstring = Some(std::borrow::Cow::Owned(text));
        }
    }
}

fn collect_comment_nodes<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    config: &super::DocCommentConfig,
) -> Vec<(usize, usize, bool)> {
    let mut result = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();

    loop {
        let node = cursor.node();
        let kind = node.kind();
        if (kind == "comment" || kind == "line_comment" || kind == "block_comment")
            && let Ok(text) = node.utf8_text(source)
        {
            let is_doc = config.line_prefixes.iter().any(|p| text.starts_with(p))
                || config.block_open.is_some_and(|o| text.starts_with(o));
            result.push((node.start_byte(), node.end_byte(), is_doc));
        }

        if cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            continue;
        }
        loop {
            if !cursor.goto_parent() {
                return result;
            }
            if cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn find_preceding_docstring(
    symbol_start: usize,
    comments: &[(usize, usize, bool)],
    source: &[u8],
    config: &super::DocCommentConfig,
) -> Option<String> {
    let mut doc_comments: Vec<&str> = Vec::new();
    let mut reference_point = symbol_start;

    for &(start, end, is_doc) in comments.iter().rev() {
        if end > symbol_start {
            continue;
        }
        if end < reference_point {
            let gap = &source[end..reference_point.min(source.len())];
            let newline_count = gap.iter().filter(|&&b| b == b'\n').count();
            if newline_count > 1 {
                break;
            }
        }
        if !is_doc {
            break;
        }
        if let Ok(text) = std::str::from_utf8(&source[start..end]) {
            doc_comments.push(text);
        }
        reference_point = start;
    }

    if doc_comments.is_empty() {
        return None;
    }

    doc_comments.reverse();
    let raw = doc_comments.join("\n");
    Some(clean_comment_docstring(&raw, config))
}

fn clean_comment_docstring(text: &str, config: &super::DocCommentConfig) -> String {
    let mut result = String::new();
    for line in text.lines() {
        let line = line.trim();
        let mut stripped = line;

        for prefix in config.line_prefixes {
            if let Some(rest) = stripped.strip_prefix(prefix) {
                stripped = rest.trim_start();
                break;
            }
        }

        if let Some(o) = config.block_open
            && let Some(rest) = stripped.strip_prefix(o)
        {
            stripped = rest.trim_start();
        }
        if !config.block_close.is_empty() && stripped.ends_with(config.block_close) {
            let end = stripped.len() - config.block_close.len();
            stripped = stripped[..end].trim_end();
        }

        if config.strip_continuation_marker
            && stripped.starts_with('*')
            && !stripped.starts_with("**")
        {
            stripped = stripped[1..].trim_start();
        }

        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(stripped);
    }
    result
}

fn resolve_import_path_from_symbol_node<'a>(
    node: tree_sitter::Node<'a>,
    source: &'a [u8],
    spec: &LanguageSpec,
) -> Option<(usize, usize)> {
    let mut ancestor = node;
    loop {
        if spec.import_statement_kinds.contains(&ancestor.kind()) {
            if let Some(source_node) = ancestor.child_by_field_name("source")
                && source_node.utf8_text(source).is_ok()
            {
                let rng = source_range_from_node(&source_node);
                return Some((rng.byte_start, rng.byte_end));
            }
            return None;
        }
        ancestor = ancestor.parent()?;
    }
}

pub(crate) fn extract_imports_and_references_with_spec<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    spec: &LanguageSpec,
    _file_path: &std::path::Path,
) -> (
    Vec<crate::model::UnresolvedImport>,
    Vec<crate::model::UnresolvedReference>,
) {
    let query = (spec.import_ref_query_fn)();
    let Some(path_idx) = query.capture_index_for_name("import.path") else {
        return (Vec::new(), Vec::new());
    };
    let alias_idx = query.capture_index_for_name("import.alias");
    let symbol_idx = query.capture_index_for_name("import.symbol");
    let star_idx = query.capture_index_for_name("import.star");
    let Some(ref_idx) = query.capture_index_for_name("reference.name") else {
        return (Vec::new(), Vec::new());
    };

    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut matches = query_cursor.matches(query, tree.root_node(), source);

    struct RawImport {
        range: crate::model::SourceRange,
        namespace: Option<(usize, usize)>,
        alias: Option<(usize, usize)>,
        symbol: Option<(usize, usize)>,
        star: bool,
    }
    let mut raw_imports: Vec<RawImport> = Vec::new();
    let mut ref_ranges: Vec<crate::model::SourceRange> = Vec::new();

    while let Some(m) = matches.next() {
        let mut namespace: Option<(usize, usize)> = None;
        let mut alias: Option<(usize, usize)> = None;
        let mut symbol: Option<(usize, usize)> = None;
        let mut star = false;
        let mut node: Option<tree_sitter::Node<'a>> = None;

        for capture in m.captures {
            let idx = capture.index;
            if idx == path_idx {
                let rng = source_range_from_node(&capture.node);
                namespace = Some((rng.byte_start, rng.byte_end));
                node = Some(capture.node);
            } else if let Some(alias_idx) = alias_idx
                && idx == alias_idx
            {
                let rng = source_range_from_node(&capture.node);
                alias = Some((rng.byte_start, rng.byte_end));
            } else if let Some(symbol_idx) = symbol_idx
                && idx == symbol_idx
            {
                let rng = source_range_from_node(&capture.node);
                symbol = Some((rng.byte_start, rng.byte_end));
                if node.is_none() {
                    node = Some(capture.node);
                }
            } else if let Some(star_idx) = star_idx
                && idx == star_idx
            {
                star = true;
            } else if idx == ref_idx {
                ref_ranges.push(source_range_from_node(&capture.node));
            }
        }

        if let Some(ns) = namespace {
            let range = source_range_from_node(&node.unwrap_or_else(|| tree.root_node()));
            raw_imports.push(RawImport {
                range,
                namespace: Some(ns),
                alias,
                symbol,
                star,
            });
        } else if symbol_idx.is_some()
            && (symbol.is_some() || star)
            && let Some(capture_node) = node
            && let Some(ns) = resolve_import_path_from_symbol_node(capture_node, source, spec)
        {
            let range = source_range_from_node(&capture_node);
            raw_imports.push(RawImport {
                range,
                namespace: Some(ns),
                alias,
                symbol,
                star,
            });
        }
    }

    let slice = |(s, e): (usize, usize)| -> String {
        let s = &source[s..e];
        std::str::from_utf8(s)
            .unwrap_or("<invalid-utf8>")
            .to_string()
    };
    let mut imports: Vec<crate::model::UnresolvedImport> = Vec::with_capacity(raw_imports.len());
    for r in raw_imports {
        imports.push(crate::model::UnresolvedImport {
            import_specifier: match r.namespace {
                Some(ns) => slice(ns),
                None => continue,
            },
            alias: r.alias.map(slice),
            symbol: r.symbol.map(slice),
            star: r.star,
            range: r.range,
        });
    }

    let mut references: Vec<crate::model::UnresolvedReference> =
        Vec::with_capacity(ref_ranges.len());
    for range in ref_ranges {
        let name = std::str::from_utf8(&source[range.byte_start..range.byte_end])
            .unwrap_or("<invalid-utf8>")
            .to_string();
        references.push(crate::model::UnresolvedReference { name, range });
    }

    imports.sort_by_key(|i| i.range.byte_start);
    references.sort_by_key(|r| r.range.byte_start);

    (imports, references)
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;

    use crate::language::LangId;

    use super::{clean_docstring, field_text, source_range_from_node};

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
    fn clean_docstring_triple_double_quote() {
        assert_eq!(clean_docstring(r#""""hello world""""#), "hello world");
    }

    #[test]
    fn clean_docstring_triple_single_quote() {
        assert_eq!(clean_docstring("'''hello world'''"), "hello world");
    }

    #[test]
    fn clean_docstring_single_double_quote() {
        assert_eq!(clean_docstring(r#""hello world""#), "hello world");
    }

    #[test]
    fn clean_docstring_single_single_quote() {
        assert_eq!(clean_docstring("'hello world'"), "hello world");
    }

    #[test]
    fn clean_docstring_no_quotes() {
        assert_eq!(clean_docstring("some bare text"), "some bare text");
    }

    #[test]
    fn clean_docstring_empty() {
        assert_eq!(clean_docstring(""), "");
    }

    #[test]
    fn clean_docstring_empty_triple() {
        assert_eq!(clean_docstring(r#""""""""#), "");
    }

    #[test]
    fn clean_docstring_leading_whitespace() {
        assert_eq!(clean_docstring(r#"  """  hello  """  "#), "hello");
    }

    #[test]
    fn clean_docstring_multiline_content() {
        assert_eq!(clean_docstring("line1\nline2"), "line1\nline2");
    }

    #[test]
    fn clean_docstring_only_whitespace() {
        let result = clean_docstring("   ");
        assert_eq!(result, "");
    }

    #[test]
    fn clean_docstring_mismatched_quotes() {
        let result = clean_docstring(r#""""hello'"#);
        assert_eq!(result, r#""""hello'"#);
    }
}
