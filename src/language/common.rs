//! Shared extraction engine for all language packs.
//!
//! Provides `extract_with_spec` (symbols), `extract_imports_with_spec`,
//! `extract_references_with_spec`, and the combined
//! `extract_imports_and_references_with_spec` that runs a single
//! tree-sitter query traversal for both imports and references.

use super::{LanguageSpec, RawSymbol};
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
// TODO! Handle multi-line docstrings and different quoting styles more robustly.
// we need to extract all utils to a one cetnralized place, and make them available for all languages. This is one of them.
fn clean_docstring(text: &str) -> &str {
    let s = text.trim_start();
    let prefix_len = if s.starts_with("\"\"\"") || s.starts_with("'''") {
        3
    } else if s.starts_with('\"') || s.starts_with('\'') {
        1
    } else {
        return text.trim();
    };

    let content_start = prefix_len;
    let trimmed_end = text.trim_end();
    let content_end = trimmed_end.len().saturating_sub(prefix_len);

    if content_end <= content_start {
        return "";
    }

    trimmed_end[content_start..content_end].trim()
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
    result.sort_by_key(|s| s.source_range.byte_start);
    result
}

pub(crate) fn extract_imports_with_spec<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    spec: &LanguageSpec,
    _file_path: &std::path::Path,
) -> Vec<crate::model::UnresolvedImport> {
    let query = (spec.import_query_fn)();
    let path_idx = query
        .capture_index_for_name("import.path")
        .expect("import.path capture must exist");
    let alias_idx = query.capture_index_for_name("import.alias");
    let symbol_idx = query.capture_index_for_name("import.symbol");
    let star_idx = query.capture_index_for_name("import.star");

    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut matches = query_cursor.matches(query, tree.root_node(), source);

    // Collect raw captures first - defer String allocation out of hot loop.
    struct RawImport {
        range: SourceRange,
        namespace: Option<(usize, usize)>,
        alias: Option<(usize, usize)>,
        symbol: Option<(usize, usize)>,
        star: bool,
    }
    let mut raw: Vec<RawImport> = Vec::new();

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
            } else if alias_idx.is_some() && idx == alias_idx.unwrap() {
                let rng = source_range_from_node(&capture.node);
                alias = Some((rng.byte_start, rng.byte_end));
            } else if symbol_idx.is_some() && idx == symbol_idx.unwrap() {
                let rng = source_range_from_node(&capture.node);
                symbol = Some((rng.byte_start, rng.byte_end));
                if node.is_none() {
                    node = Some(capture.node);
                }
            } else if star_idx.is_some() && idx == star_idx.unwrap() {
                star = true;
            }
        }

        if let Some(ns) = namespace {
            let range = source_range_from_node(&node.unwrap_or_else(|| tree.root_node()));
            raw.push(RawImport {
                range,
                namespace: Some(ns),
                alias,
                symbol,
                star,
            });
        }
    }

    // Allocate Strings outside the hot loop, in a single pass.
    let slice = |(s, e): (usize, usize)| -> String {
        let s = &source[s..e];
        std::str::from_utf8(s).unwrap_or("").to_string()
    };
    let mut imports: Vec<crate::model::UnresolvedImport> = Vec::with_capacity(raw.len());
    for r in raw {
        imports.push(crate::model::UnresolvedImport {
            namespace: slice(r.namespace.expect("namespace is required")),
            alias: r.alias.map(slice),
            symbol: r.symbol.map(slice),
            star: r.star,
            range: r.range,
        });
    }

    imports.sort_by_key(|i| i.range.byte_start);
    imports
}

// TODO(MVP): Handle multi-line import statements where the statement spans
//multiple lines (PEP 328, JS template literals).

pub(crate) fn extract_references_with_spec<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    spec: &LanguageSpec,
) -> Vec<crate::model::UnresolvedReference> {
    let query = (spec.reference_query_fn)();
    let ref_idx = query
        .capture_index_for_name("reference.name")
        .expect("reference.name capture must exist");

    let mut query_cursor = tree_sitter::QueryCursor::new();
    let mut matches = query_cursor.matches(query, tree.root_node(), source);

    // Collect byte ranges first - no String allocation in the hot loop.
    let mut ranges: Vec<SourceRange> = Vec::new();

    while let Some(m) = matches.next() {
        for capture in m.captures {
            if capture.index == ref_idx {
                ranges.push(source_range_from_node(&capture.node));
            }
        }
    }

    // Allocate Strings once, in a linear pass after the cursor is done.
    let mut references: Vec<crate::model::UnresolvedReference> = Vec::with_capacity(ranges.len());
    for range in ranges {
        let name = std::str::from_utf8(&source[range.byte_start..range.byte_end])
            .unwrap_or("")
            .to_string();
        references.push(crate::model::UnresolvedReference { name, range });
    }

    references.sort_by_key(|r| r.range.byte_start);
    references
}

fn resolve_import_path_from_symbol_node<'a>(
    node: tree_sitter::Node<'a>,
    source: &'a [u8],
) -> Option<(usize, usize)> {
    let mut ancestor = node;
    loop {
        if ancestor.kind() == "import_statement" {
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
    let path_idx = query
        .capture_index_for_name("import.path")
        .expect("import.path capture must exist");
    let alias_idx = query.capture_index_for_name("import.alias");
    let symbol_idx = query.capture_index_for_name("import.symbol");
    let star_idx = query.capture_index_for_name("import.star");
    let ref_idx = query
        .capture_index_for_name("reference.name")
        .expect("reference.name capture must exist");

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
            } else if alias_idx.is_some() && idx == alias_idx.unwrap() {
                let rng = source_range_from_node(&capture.node);
                alias = Some((rng.byte_start, rng.byte_end));
            } else if symbol_idx.is_some() && idx == symbol_idx.unwrap() {
                let rng = source_range_from_node(&capture.node);
                symbol = Some((rng.byte_start, rng.byte_end));
                if node.is_none() {
                    node = Some(capture.node);
                }
            } else if star_idx.is_some() && idx == star_idx.unwrap() {
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
            && let Some(ns) = resolve_import_path_from_symbol_node(capture_node, source)
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
        std::str::from_utf8(s).unwrap_or("").to_string()
    };
    let mut imports: Vec<crate::model::UnresolvedImport> = Vec::with_capacity(raw_imports.len());
    for r in raw_imports {
        imports.push(crate::model::UnresolvedImport {
            namespace: slice(r.namespace.expect("namespace is required")),
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
            .unwrap_or("")
            .to_string();
        references.push(crate::model::UnresolvedReference { name, range });
    }

    imports.sort_by_key(|i| i.range.byte_start);
    references.sort_by_key(|r| r.range.byte_start);

    (imports, references)
}

// TODO(MVP): Capture references inside attribute chains (obj.method().field)
//             and destructuring patterns.

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
