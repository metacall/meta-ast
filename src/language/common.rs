//! Shared extraction engine for all language packs.
//!
//! Provides `extract_with_spec` (symbols) and the combined
//! `extract_imports_and_references_with_spec` that runs a single
//! tree-sitter query traversal for both imports and references.

use super::{LanguageSpec, RawSymbol};
use crate::model::{LineColumn, SourceRange, SymbolKind, Visibility};
use tree_sitter::StreamingIterator;

/// Compile a query without panicking.
///
/// A broken query is a programming error, but the release profile aborts on
/// panic, so the failure travels as a value: the analysis turns it into a
/// per-file diagnostic and the process keeps running.
/// Imports, references and the diagnostics for text that cannot be decoded.
///
/// The diagnostic channel travels with the extraction so a file with an
/// undecodable specifier reports it without a second pass.
pub(crate) type ImportExtraction = (
    Vec<crate::model::UnresolvedImport>,
    Vec<crate::model::UnresolvedReference>,
    Vec<crate::error::Diagnostic>,
);

/// Scope that owns a def-use site.
///
/// A byte offset alone cannot name a scope: a function at the start of a
/// file shares offset zero with the module.
#[cfg(feature = "dataflow")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DefUseScope {
    Module,
    Function { start: usize },
}

/// Definitions for one enclosing scope, grouped by name in source order.
#[cfg(feature = "dataflow")]
type DefinitionsByScope<'a> = std::collections::HashMap<
    DefUseScope,
    std::collections::HashMap<&'a str, Vec<(usize, crate::model::DataNodeId)>>,
>;

pub(crate) fn compile_query_checked(
    lang: &tree_sitter::Language,
    src: &str,
    label: &str,
) -> Result<tree_sitter::Query, String> {
    tree_sitter::Query::new(lang, src)
        .map_err(|error| format!("query compilation failed for {label}: {error}"))
}

/// Borrow a compiled query from a lazily initialized slot.
pub(crate) fn query_from(
    cached: &Result<tree_sitter::Query, String>,
    language: crate::language::LangId,
) -> Result<&tree_sitter::Query, crate::error::Error> {
    cached
        .as_ref()
        .map_err(|message| crate::error::Error::Query {
            language,
            message: message.clone(),
        })
}

#[inline]
pub(crate) fn source_range_from_node(node: &tree_sitter::Node) -> SourceRange {
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
) -> Result<Vec<RawSymbol<'a>>, crate::error::Error> {
    use std::collections::HashMap;
    let mut symbols_map: HashMap<usize, (RawSymbol<'a>, usize)> = HashMap::new();
    let mut query_cursor = tree_sitter::QueryCursor::new();
    let query = (spec.query_fn)()?;
    let mut matches = query_cursor.matches(query, tree.root_node(), source);

    while let Some(m) = matches.next() {
        let mut name: Option<std::borrow::Cow<'a, str>> = None;
        let mut kind: Option<SymbolKind> = None;
        let mut signature: Option<std::borrow::Cow<'a, str>> = None;
        let mut docstring: Option<std::borrow::Cow<'a, str>> = None;
        let mut is_async = false;
        let mut visibility: Option<Visibility> = None;
        let mut primary_node: Option<tree_sitter::Node<'a>> = None;
        let mut name_range: Option<SourceRange> = None;

        let capture_count = m.captures().len();

        for capture in m.captures() {
            let capture_name = query.capture_names()[capture.index as usize];
            match capture_name {
                "name" => {
                    if let Ok(text) = capture.node.utf8_text(source) {
                        name = Some(std::borrow::Cow::Borrowed(text));
                        name_range = Some(source_range_from_node(&capture.node));
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
                        "declaration" => SymbolKind::Declaration,
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
                name_range,
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
    Ok(result)
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

/// Byte span of the specifier inside an import node.
///
/// Import statements wrap the specifier in a string literal or, for C and C++
/// system headers, in angle brackets. Consumers want the bare form, so the wrap
/// characters are trimmed here, at the single place the value is produced. A
/// leading quote implies a string literal and always closes the span; angle
/// brackets only count when they wrap the whole specifier, because a qualified
/// Rust path can start with `<`.
fn bare_span(source: &[u8], (start, end): (usize, usize)) -> (usize, usize) {
    let Some(&first) = source.get(start) else {
        return (start, end);
    };
    match first {
        b'\'' | b'"' => {
            let end = if end > start && source[end - 1] == first {
                end - 1
            } else {
                end
            };
            (start + 1, end)
        }
        b'<' if end > start + 1 && source[end - 1] == b'>' => (start + 1, end - 1),
        _ => (start, end),
    }
}

/// One warning for a text range that is not valid UTF-8.
///
/// The range travels as a diagnostic instead of a placeholder string, so a
/// consumer can point at the defect instead of parsing a sentinel.
fn undecodable(path: &std::path::Path, range: SourceRange, what: &str) -> crate::error::Diagnostic {
    crate::error::Diagnostic {
        path: path.to_path_buf(),
        severity: crate::error::Severity::Warning,
        message: format!("{what} is not valid UTF-8"),
        source_range: Some(range),
    }
}

pub(crate) fn extract_imports_and_references_with_spec<'a>(
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    spec: &LanguageSpec,
    file_path: &std::path::Path,
) -> Result<ImportExtraction, crate::error::Error> {
    let query = (spec.import_ref_query_fn)()?;
    let Some(path_idx) = query.capture_index_for_name("import.path") else {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
    };
    let alias_idx = query.capture_index_for_name("import.alias");
    let symbol_idx = query.capture_index_for_name("import.symbol");
    let star_idx = query.capture_index_for_name("import.star");
    let Some(ref_idx) = query.capture_index_for_name("reference.name") else {
        return Ok((Vec::new(), Vec::new(), Vec::new()));
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

        for capture in m.captures() {
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

    let text =
        |(s, e): (usize, usize)| -> Option<&'a str> { std::str::from_utf8(&source[s..e]).ok() };
    let mut diagnostics: Vec<crate::error::Diagnostic> = Vec::new();
    let mut imports: Vec<crate::model::UnresolvedImport> = Vec::with_capacity(raw_imports.len());
    for r in raw_imports {
        let Some(ns) = r.namespace else {
            continue;
        };
        let Some(specifier) = text(bare_span(source, ns)) else {
            diagnostics.push(undecodable(file_path, r.range, "the import specifier"));
            continue;
        };
        imports.push(crate::model::UnresolvedImport {
            import_specifier: specifier.to_string(),
            alias: r.alias.and_then(text).map(str::to_string),
            symbol: r.symbol.and_then(text).map(str::to_string),
            star: r.star,
            range: r.range,
        });
    }

    let mut references: Vec<crate::model::UnresolvedReference> =
        Vec::with_capacity(ref_ranges.len());
    for range in ref_ranges {
        match std::str::from_utf8(&source[range.byte_start..range.byte_end]) {
            Ok(name) => references.push(crate::model::UnresolvedReference {
                name: name.to_string(),
                range,
            }),
            Err(_) => diagnostics.push(undecodable(file_path, range, "the reference name")),
        }
    }

    imports.sort_by_key(|i| i.range.byte_start);
    references.sort_by_key(|r| r.range.byte_start);

    Ok((imports, references, diagnostics))
}

/// JS-family AST node kinds that introduce a new intra-procedural scope.
///
/// Shared by JavaScript, TypeScript, and TSX dataflow extraction.
#[cfg(feature = "dataflow")]
pub(crate) const JS_FAMILY_FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "generator_function",
    "arrow_function",
    "method_definition",
];

/// Scope owning a syntax node: the nearest enclosing function, else the module.
#[cfg(feature = "dataflow")]
pub(crate) fn enclosing_scope_key(node: tree_sitter::Node, function_kinds: &[&str]) -> DefUseScope {
    let mut current = node.parent();
    while let Some(parent) = current {
        if function_kinds.contains(&parent.kind()) {
            return DefUseScope::Function {
                start: parent.start_byte(),
            };
        }
        current = parent.parent();
    }
    DefUseScope::Module
}

/// Shared def-use extraction engine.
///
/// Captures `@def.var`, `@def.param`, and `@use.var` sites with `query`,
/// assigns data nodes via `id_gen`, and links each use to the nearest
/// preceding def of the same name in the same scope from `function_kinds`.
#[cfg(feature = "dataflow")]
pub(crate) fn extract_def_use_dataflow(
    tree: &tree_sitter::Tree,
    source: &[u8],
    query: &tree_sitter::Query,
    function_kinds: &[&str],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    use crate::graph::edge::CONFIDENCE_DEF_USE;
    use crate::model::{DataNode, DataScope, FlowEdge, FlowKind};
    use tree_sitter::StreamingIterator;

    let mut cursor = tree_sitter::QueryCursor::new();
    // Names borrow the source: the text is only materialized for a node that
    // is actually emitted, so an unmatched use costs no allocation.
    let mut defs: Vec<(&str, usize, tree_sitter::Node, bool)> = Vec::new();
    let mut uses: Vec<(&str, usize, tree_sitter::Node, DefUseScope)> = Vec::new();

    let mut matches = cursor.matches(query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        for capture in m.captures() {
            let capture_name = query.capture_names()[capture.index as usize];
            let node = capture.node;
            let byte_pos = node.start_byte();
            let name = match node.utf8_text(source) {
                Ok(text) => text,
                Err(_) => continue,
            };
            match capture_name {
                "def.var" => defs.push((name, byte_pos, node, false)),
                "def.param" => defs.push((name, byte_pos, node, true)),
                "use.var" => {
                    let scope = enclosing_scope_key(node, function_kinds);
                    uses.push((name, byte_pos, node, scope));
                }
                _ => {}
            }
        }
    }

    let mut nodes: Vec<DataNode> = Vec::with_capacity(defs.len() + uses.len());
    // Definitions grouped by their enclosing scope and name, in source order:
    // a use looks up its own bucket instead of scanning every definition, and
    // the nearest preceding definition is the last entry before the use. The
    // name key borrows the source, so grouping allocates no text.
    let mut defs_by_scope: DefinitionsByScope<'_> = std::collections::HashMap::new();
    for (name, byte_pos, node, is_param) in defs {
        let scope = if is_param {
            DataScope::Parameter
        } else {
            DataScope::Local
        };
        let dn = DataNode {
            id: id_gen.next(),
            symbol_id: None,
            name: Some(name.to_string()),
            scope,
            type_hint: None,
            source_range: source_range_from_node(&node),
        };
        let scope_start = enclosing_scope_key(node, function_kinds);
        defs_by_scope
            .entry(scope_start)
            .or_default()
            .entry(name)
            .or_default()
            .push((byte_pos, dn.id));
        nodes.push(dn);
    }
    for by_name in defs_by_scope.values_mut() {
        for candidates in by_name.values_mut() {
            candidates.sort_by_key(|(byte_pos, _)| *byte_pos);
        }
    }

    let mut edges: Vec<FlowEdge> = Vec::with_capacity(uses.len());
    for (use_name, use_pos, use_node, use_scope) in uses {
        let best_def = defs_by_scope
            .get(&use_scope)
            .and_then(|by_name| by_name.get(use_name))
            .and_then(|candidates| {
                let preceding = candidates.partition_point(|(byte_pos, _)| *byte_pos < use_pos);
                preceding
                    .checked_sub(1)
                    .and_then(|index| candidates.get(index))
                    .map(|(_, def_id)| *def_id)
            });

        if let Some(def_id) = best_def {
            let use_dn = DataNode {
                id: id_gen.next(),
                symbol_id: None,
                name: Some(use_name.to_string()),
                scope: DataScope::Local,
                type_hint: None,
                source_range: source_range_from_node(&use_node),
            };
            let target_id = use_dn.id;
            nodes.push(use_dn);
            edges.push(FlowEdge {
                source: def_id,
                target: target_id,
                kind: FlowKind::DefUse,
                confidence: CONFIDENCE_DEF_USE,
            });
        }
    }

    (nodes, edges)
}

#[cfg(test)]
mod tests {
    use tree_sitter::Parser;

    use crate::language::LangId;

    #[cfg(feature = "dataflow")]
    use super::{DefUseScope, enclosing_scope_key};
    use super::{bare_span, clean_docstring, field_text, source_range_from_node};

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
    fn name_range_is_the_identifier_inside_the_symbol() {
        for (lang, source, name) in [
            (
                LangId::Python,
                "def greet(value):\n    return value\n",
                "greet",
            ),
            (
                LangId::Rust,
                "fn greet(value: usize) -> usize { value }\n",
                "greet",
            ),
            (
                LangId::C,
                "int greet(int value) { return value; }\n",
                "greet",
            ),
        ] {
            let mut parser = Parser::new();
            parser
                .set_language(&crate::language::grammar_for(lang))
                .unwrap();
            let tree = parser.parse(source, None).unwrap();
            let symbols =
                super::extract_with_spec(&tree, source.as_bytes(), crate::language::spec_for(lang))
                    .unwrap();
            let symbol = symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap_or_else(|| panic!("{lang:?} did not extract {name}"));
            let name_range = symbol.name_range.clone().expect("name range");
            assert!(name_range.byte_start >= symbol.source_range.byte_start);
            assert!(name_range.byte_end <= symbol.source_range.byte_end);
            assert_eq!(&source[name_range.byte_start..name_range.byte_end], name);
        }
    }

    #[test]
    fn bare_span_strips_only_the_wrapping_delimiters() {
        let quoted = b"'react'";
        assert_eq!(bare_span(quoted, (0, quoted.len())), (1, 6));

        let system_header = b"<stdio.h>";
        assert_eq!(bare_span(system_header, (0, system_header.len())), (1, 8));

        let bare = b"json";
        assert_eq!(bare_span(bare, (0, bare.len())), (0, 4));

        // An unterminated literal still loses its opening quote.
        let unterminated = b"'foo";
        assert_eq!(bare_span(unterminated, (0, unterminated.len())), (1, 4));

        // A qualified path that starts with an angle bracket is not a wrap.
        let qualified = b"<T as Trait>::x";
        assert_eq!(
            bare_span(qualified, (0, qualified.len())),
            (0, qualified.len())
        );
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

    #[cfg(feature = "dataflow")]
    fn parse_python(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::language::grammar_for(LangId::Python))
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[cfg(feature = "dataflow")]
    fn find_identifier<'a>(
        node: tree_sitter::Node<'a>,
        source: &[u8],
        target: &str,
    ) -> Option<tree_sitter::Node<'a>> {
        if node.kind() == "identifier" && node.utf8_text(source).ok() == Some(target) {
            return Some(node);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if let Some(found) = find_identifier(child, source, target) {
                return Some(found);
            }
        }
        None
    }

    /// Module scope stays distinct from a function that starts at byte zero.
    #[cfg(feature = "dataflow")]
    #[test]
    fn module_scope_differs_from_function_at_byte_zero() {
        use crate::language::python::PYTHON_FUNCTION_KINDS;

        let source = b"def f():\n    x = 1\n";
        let tree = parse_python(source);
        let root = tree.root_node();
        assert_eq!(root.start_byte(), 0);

        let function = root.named_child(0).unwrap();
        assert_eq!(function.kind(), "function_definition");
        assert_eq!(function.start_byte(), 0);

        let x = find_identifier(root, source, "x").unwrap();
        assert_eq!(
            enclosing_scope_key(x, PYTHON_FUNCTION_KINDS),
            DefUseScope::Function { start: 0 }
        );
        assert_eq!(
            enclosing_scope_key(root, PYTHON_FUNCTION_KINDS),
            DefUseScope::Module
        );
        assert_ne!(
            DefUseScope::Module,
            DefUseScope::Function { start: 0 },
            "module scope must not collide with a function at byte zero"
        );
    }

    /// A module-level use must not link to a function-local def.
    ///
    /// `print(x)` at module level has no module def, so it emits no node
    /// and no edge, even though `x = 1` inside `f` precedes it in the file.
    #[cfg(feature = "dataflow")]
    #[test]
    fn function_local_def_does_not_leak_to_module_use() {
        let source = b"def f():\n    x = 1\n    return x\nprint(x)\n";
        let tree = parse_python(source);
        let id_gen = crate::model::IdGenerator::new();
        let (nodes, edges) =
            crate::language::python::extract_python_dataflow(&tree, source, &id_gen);

        assert_eq!(edges.len(), 1, "only the in-function use links: {edges:?}");
        assert_eq!(nodes.len(), 2, "one def and one matched use: {nodes:?}");

        let module_x = source
            .iter()
            .rposition(|&b| b == b'x')
            .expect("module-level x");
        assert!(
            nodes.iter().all(|n| n.source_range.byte_start != module_x),
            "module-level x must emit no data node: {nodes:?}"
        );
    }

    /// Nested functions keep separate scopes under the same rule.
    ///
    /// `x = 1` belongs to `outer`, so the `print(x)` in `outer` links
    /// while the one in `inner` stays unmatched.
    #[cfg(feature = "dataflow")]
    #[test]
    fn nested_functions_keep_separate_scopes() {
        let source = b"def outer():\n    x = 1\n    def inner():\n        print(x)\n    print(x)\n";
        let tree = parse_python(source);
        let id_gen = crate::model::IdGenerator::new();
        let (nodes, edges) =
            crate::language::python::extract_python_dataflow(&tree, source, &id_gen);

        assert_eq!(edges.len(), 1, "only the outer use links: {edges:?}");
        assert_eq!(nodes.len(), 2, "one def and one matched use: {nodes:?}");

        let outer_x = source
            .iter()
            .rposition(|&b| b == b'x')
            .expect("outer use of x");
        assert!(
            nodes.iter().any(|n| n.source_range.byte_start == outer_x),
            "the matched use is the outer print(x): {nodes:?}"
        );
    }
}
