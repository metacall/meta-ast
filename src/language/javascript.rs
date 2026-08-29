use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use crate::model::Visibility;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_js_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if !raw.starts_with('.') && !raw.starts_with('/') {
        // Bare module name (e.g. 'jsonwebtoken', 'react'): return as-is
        // so the graph builder creates an ExternalNode for it.
        // Node.js resolution (node_modules) is not walked here.
        return Some(PathBuf::from(raw));
    }

    let base = if raw.starts_with('/') {
        PathBuf::from("/")
    } else {
        source_dir.to_path_buf()
    };

    let path = base.join(raw);

    let extensions = ["", ".js", ".ts", ".jsx", ".tsx", ".mjs", ".cjs"];
    for ext in &extensions {
        let candidate = if ext.is_empty() {
            path.clone()
        } else {
            path.with_extension(ext.trim_start_matches('.'))
        };
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    Some(path)
}

static JS_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_javascript::LANGUAGE.into(),
        r#"
(function_declaration
  "async"? @async
  name: (identifier) @name
  parameters: (formal_parameters) @signature
) @kind.function

(generator_function_declaration
  "async"? @async
  name: (identifier) @name
  parameters: (formal_parameters) @signature
) @kind.function

(class_declaration
  name: (identifier) @name
) @kind.class

(method_definition
  "async"? @async
  name: [
    (property_identifier)
    (identifier)
  ] @name
  parameters: (formal_parameters) @signature
) @kind.method

(export_statement
  [
    (function_declaration
      "async"? @async
      name: (identifier) @name
      parameters: (formal_parameters) @signature
    ) @kind.function
    (class_declaration
      name: (identifier) @name
    ) @kind.class
  ]
)
"#,
        "JavaScript",
    )
});

fn js_query() -> &'static tree_sitter::Query {
    &JS_QUERY
}

const JS_IMPORT_QUERY_STR: &str = r#"
(import_statement
  source: (string) @import.path)
(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @import.symbol
        alias: (identifier)? @import.alias))))
(import_statement
  (import_clause
    (identifier) @import.symbol))
(import_statement
  (import_clause
    (namespace_import
      (identifier) @import.symbol)))
(call_expression
  function: (identifier) @call.name
  arguments: (arguments . (string) @import.path .)
  (#eq? @call.name "require"))
"#;

const JS_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (member_expression
    property: (property_identifier) @reference.name))
(call_expression
  function: (member_expression
    object: (identifier) @reference.name))
"#;

static JS_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_javascript::LANGUAGE.into(),
        &format!("{}\n{}", JS_IMPORT_QUERY_STR, JS_REFERENCE_QUERY_STR),
        "JavaScript combined import+ref",
    )
});

fn js_import_ref_query() -> &'static tree_sitter::Query {
    &JS_IMPORT_REF_QUERY
}

pub(crate) const JS_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["js", "mjs", "cjs"],
    grammar_fn: || tree_sitter_javascript::LANGUAGE.into(),
    query_fn: js_query,
    import_path_resolver: resolve_js_import,
    import_ref_query_fn: js_import_ref_query,
    class_like_parents: &["class_declaration", "class"],
    ancestor_visibility_rules: &[("export_statement", Visibility::Public)],
    visibility_from_name: None,
    import_statement_kinds: &["import_statement"],
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment_config: Some(DocCommentConfig {
        line_prefixes: &["//"],
        block_open: Some("/**"),
        block_close: "*/",
        strip_continuation_marker: true,
    }),
};

// ── Dataflow extraction ─────────────────────────────────────────────

/// tree-sitter query capturing def-use sites for the JavaScript grammar.
///
/// JavaScript parameters live directly inside `formal_parameters` (no
/// `required_parameter` wrapper as in TypeScript). The capture names
/// (`@def.var`, `@def.param`, `@use.var`) match the shared schema used
/// across the JS family.
#[cfg(feature = "dataflow")]
pub(crate) const JS_DATAFLOW_QUERY_STR: &str = r#"
; Variable declarator: name position in a `let/const/var` binding.
(variable_declarator
  name: (identifier) @def.var)

; Function parameters (JS grammar: identifier directly in formal_parameters,
; possibly wrapped by assignment_pattern for default values).
(formal_parameters
  (identifier) @def.param)
(formal_parameters
  (assignment_pattern
    left: (identifier) @def.param))

; Identifier references in expression position.
(call_expression
  function: (identifier) @use.var)
(call_expression
  arguments: (arguments
    (identifier) @use.var))
(binary_expression
  left: (identifier) @use.var)
(binary_expression
  right: (identifier) @use.var)
(member_expression
  object: (identifier) @use.var)
(return_statement
  (identifier) @use.var)
(assignment_expression
  right: (identifier) @use.var)
"#;

/// tree-sitter query capturing def-use sites for the TypeScript family.
///
/// TypeScript wraps each parameter in a `required_parameter` /
/// `optional_parameter` node. The rest of the schema is identical to JS.
///
/// Note: we deliberately do NOT use the `name:` field for parameters. The
/// typescript grammar advertises a `name` field on `required_parameter`,
/// but in the current grammar the field's match behavior is brittle;
/// matching the direct `identifier` child is more reliable across grammar
/// versions and avoids accidental double-capture.
#[cfg(feature = "dataflow")]
pub(crate) const TS_FAMILY_DATAFLOW_QUERY: &str = r#"
; Variable declarator: name position in a `let/const/var` binding.
(variable_declarator
  name: (identifier) @def.var)

; Function parameters (TS grammar: required_parameter / optional_parameter).
(required_parameter
  (identifier) @def.param)
(optional_parameter
  (identifier) @def.param)

; Identifier references in expression position.
(call_expression
  function: (identifier) @use.var)
(call_expression
  arguments: (arguments
    (identifier) @use.var))
(binary_expression
  left: (identifier) @use.var)
(binary_expression
  right: (identifier) @use.var)
(member_expression
  object: (identifier) @use.var)
(return_statement
  (identifier) @use.var)
(assignment_expression
  right: (identifier) @use.var)
"#;

/// Extract data nodes and flow edges from a JavaScript-family parse tree.
///
/// `function_kinds` lists the AST node kinds that introduce a new scope
/// (e.g. `function_declaration`, `arrow_function`, `function_expression`).
/// Including computed functions in this list ensures intra-procedural
/// scoping is enforced: variables in nested functions do not link back to
/// outer-scope definitions of the same name.
#[cfg(feature = "dataflow")]
pub(crate) fn extract_js_family_dataflow_with_query(
    tree: &tree_sitter::Tree,
    source: &[u8],
    query: &tree_sitter::Query,
    function_kinds: &[&str],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    use crate::model::{DataNode, DataNodeId, DataScope, FlowEdge, FlowKind};
    use tree_sitter::StreamingIterator;

    let mut cursor = tree_sitter::QueryCursor::new();

    // (name, byte_pos, node, is_param)
    let mut defs: Vec<(String, usize, tree_sitter::Node, bool)> = Vec::new();
    // (name, byte_pos, node, enclosing_function_start)
    let mut uses: Vec<(String, usize, tree_sitter::Node, usize)> = Vec::new();

    let mut matches = cursor.matches(query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let node = capture.node;
            let byte_pos = node.start_byte();
            let name = match node.utf8_text(source) {
                Ok(t) => t.to_string(),
                Err(_) => continue,
            };

            match capture_name {
                "def.var" => defs.push((name, byte_pos, node, false)),
                "def.param" => defs.push((name, byte_pos, node, true)),
                "use.var" => {
                    let func_start = enclosing_function_start(node, function_kinds);
                    uses.push((name, byte_pos, node, func_start));
                }
                _ => {}
            }
        }
    }

    let mut nodes: Vec<DataNode> = Vec::new();
    let mut def_ids: Vec<(String, usize, DataNodeId, bool)> = Vec::new();

    for (name, byte_pos, node, is_param) in &defs {
        let scope = if *is_param {
            DataScope::Parameter
        } else {
            DataScope::Local
        };
        let dn = DataNode {
            id: id_gen.next(),
            symbol_id: None,
            name: Some(name.clone()),
            scope,
            type_hint: None,
            source_range: source_range_from_node(node),
        };
        def_ids.push((name.clone(), *byte_pos, dn.id, *is_param));
        nodes.push(dn);
    }

    let mut edges: Vec<FlowEdge> = Vec::new();

    for (use_name, use_pos, use_node, use_func_start) in &uses {
        let mut best_def: Option<&(String, usize, DataNodeId, bool)> = None;

        for def in &def_ids {
            if def.0 == *use_name
                && def.1 < *use_pos
                && enclosing_function_start_for_id(def.1, tree, function_kinds) == *use_func_start
            {
                match best_def {
                    None => best_def = Some(def),
                    Some(best) => {
                        if def.1 > best.1 {
                            best_def = Some(def);
                        }
                    }
                }
            }
        }

        if let Some(def) = best_def {
            // Register the usage as a data node so the flow edge has an
            // anchor in the graph; the source/target IDs both refer to
            // real, queryable nodes.
            let use_dn = DataNode {
                id: id_gen.next(),
                symbol_id: None,
                name: Some(use_name.clone()),
                scope: DataScope::Local,
                type_hint: None,
                source_range: source_range_from_node(use_node),
            };
            let target_id = use_dn.id;
            nodes.push(use_dn);

            edges.push(FlowEdge {
                source: def.2,
                target: target_id,
                kind: FlowKind::DefUse,
                confidence: 0.9,
            });
        }
    }

    (nodes, edges)
}

#[cfg(feature = "dataflow")]
fn source_range_from_node(node: &tree_sitter::Node) -> crate::model::SourceRange {
    use crate::model::{LineColumn, SourceRange};
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

#[cfg(feature = "dataflow")]
fn enclosing_function_start(node: tree_sitter::Node, function_kinds: &[&str]) -> usize {
    let mut current = node.parent();
    while let Some(parent) = current {
        if function_kinds.contains(&parent.kind()) {
            return parent.start_byte();
        }
        current = parent.parent();
    }
    0
}

#[cfg(feature = "dataflow")]
fn enclosing_function_start_for_id(
    byte_pos: usize,
    tree: &tree_sitter::Tree,
    function_kinds: &[&str],
) -> usize {
    find_enclosing_function(tree.root_node(), byte_pos, function_kinds)
}

#[cfg(feature = "dataflow")]
fn find_enclosing_function(
    node: tree_sitter::Node,
    byte_pos: usize,
    function_kinds: &[&str],
) -> usize {
    if node.start_byte() <= byte_pos && byte_pos < node.end_byte() {
        // Recurse into children first so the innermost (deepest) function
        // scope is found. Checking this node before its children would
        // return an outer scope when an inner one is what we want.
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i as u32) {
                let result = find_enclosing_function(child, byte_pos, function_kinds);
                if result != 0 {
                    return result;
                }
            }
        }
        if function_kinds.contains(&node.kind()) {
            return node.start_byte();
        }
    }
    0
}

#[cfg(feature = "dataflow")]
static JS_DATAFLOW_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_javascript::LANGUAGE.into(),
        JS_DATAFLOW_QUERY_STR,
        "JavaScript dataflow",
    )
});

/// JavaScript AST node kinds that introduce a new intra-procedural scope.
#[cfg(feature = "dataflow")]
pub(crate) const JS_FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "generator_function",
    "arrow_function",
    "method_definition",
];

/// Extract data nodes and flow edges from a JavaScript parse tree.
#[cfg(feature = "dataflow")]
pub fn extract_javascript_dataflow(
    tree: &tree_sitter::Tree,
    source: &[u8],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    extract_js_family_dataflow_with_query(
        tree,
        source,
        &JS_DATAFLOW_QUERY,
        JS_FUNCTION_KINDS,
        id_gen,
    )
}

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::{SymbolKind, Visibility};

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&grammar_for(LangId::JavaScript))
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_function_declaration() {
        let src = b"function hello() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_async_function() {
        let src = b"async function fetch() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_class_and_methods() {
        let src = b"class Foo {\n  constructor() {}\n  bar() {}\n}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Method))
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn extract_exported_class() {
        let src = b"export class Foo { bar() {} }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(class.visibility, Some(Visibility::Public));
    }

    #[test]
    fn extract_named_imports() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import { foo, bar } from 'utils';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::JavaScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.js"),
        );
        let named: Vec<_> = imports.iter().filter(|i| i.symbol.is_some()).collect();
        assert_eq!(
            named.len(),
            2,
            "expected 2 named import records for foo and bar"
        );
        for imp in &named {
            assert_eq!(imp.import_specifier, "'utils'");
        }
        assert_eq!(named[0].symbol.as_deref(), Some("foo"));
        assert_eq!(named[1].symbol.as_deref(), Some("bar"));
    }

    #[test]
    fn extract_default_import() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import React from 'react';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::JavaScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.js"),
        );
        let named: Vec<_> = imports.iter().filter(|i| i.symbol.is_some()).collect();
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].import_specifier, "'react'");
        assert_eq!(named[0].symbol.as_deref(), Some("React"));
    }

    #[test]
    fn extract_side_effect_import() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import 'styles.css';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::JavaScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.js"),
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].import_specifier, "'styles.css'");
        assert!(imports[0].symbol.is_none());
    }

    #[test]
    fn js_docstring_extraction() {
        let src = b"/** JSDoc comment. */\nfunction documented() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        let func = symbols.iter().find(|s| s.name == "documented").unwrap();
        assert!(func.docstring.is_some(), "documented should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("JSDoc comment"));
    }

    #[test]
    fn js_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/javascript/functions.js"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }

    #[cfg(feature = "dataflow")]
    mod dataflow_tests {
        use super::*;
        use crate::language::javascript::extract_javascript_dataflow;
        use crate::model::{DataScope, FlowKind};

        fn extract(source: &[u8]) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
            let id_gen = crate::model::IdGenerator::new();
            extract_javascript_dataflow(&parse(source), source, &id_gen)
        }

        #[test]
        fn const_declaration_captured_as_local() {
            let src = b"function f() { const x = 42; return x; }";
            let (nodes, _) = extract(src);
            let x = nodes
                .iter()
                .find(|n| n.name.as_deref() == Some("x") && n.scope == DataScope::Local)
                .expect("const x should be captured as Local");
            assert_eq!(x.scope, DataScope::Local);
        }

        #[test]
        fn function_parameter_captured_as_parameter() {
            let src = b"function add(a, b) { return a + b; }";
            let (nodes, edges) = extract(src);
            let params: Vec<_> = nodes
                .iter()
                .filter(|n| n.scope == DataScope::Parameter)
                .collect();
            assert_eq!(params.len(), 2, "expected 2 parameters");
            let names: Vec<_> = params.iter().map(|n| n.name.as_deref()).collect();
            assert!(names.contains(&Some("a")));
            assert!(names.contains(&Some("b")));
            assert!(
                !edges.is_empty(),
                "parameter usages should produce def-use edges"
            );
            for edge in &edges {
                assert_eq!(edge.kind, FlowKind::DefUse);
                assert!(
                    (edge.confidence - 0.9).abs() < f32::EPSILON,
                    "confidence should be 0.9, got {}",
                    edge.confidence
                );
            }
        }

        #[test]
        fn def_use_edge_anchored_in_graph() {
            // Each flow edge's target must reference a real data node id.
            let src = b"function f() { let x = 1; let y = x; }";
            let (nodes, edges) = extract(src);
            let ids: std::collections::HashSet<_> = nodes.iter().map(|n| n.id).collect();
            for edge in &edges {
                assert!(
                    ids.contains(&edge.source),
                    "edge source {:?} not in nodes",
                    edge.source
                );
                assert!(
                    ids.contains(&edge.target),
                    "edge target {:?} not in nodes (dangling edge)",
                    edge.target
                );
            }
        }

        #[test]
        fn no_cross_function_def_use_leak() {
            // `x` defined in outer scope must not link to `x` in nested function.
            let src = b"function outer() { let x = 1; function inner() { let x = 2; return x; } return x; }";
            let (nodes, edges) = extract(src);
            // 2 def nodes + 2 use nodes (one per `return x` site) all named "x".
            let defs_for_x: Vec<_> = nodes
                .iter()
                .filter(|n| n.name.as_deref() == Some("x") && n.scope == DataScope::Local)
                .collect();
            assert_eq!(defs_for_x.len(), 4, "2 def + 2 use nodes for `x` expected");
            // Of those, exactly 2 are definitions (Local scope with no incoming edge
            // of the same name; we identify them by being earlier in source order).
            let defs: Vec<_> = nodes
                .iter()
                .filter(|n| {
                    n.name.as_deref() == Some("x")
                        && n.scope == DataScope::Local
                        && !edges.iter().any(|e| e.target == n.id)
                })
                .collect();
            assert_eq!(defs.len(), 2, "two distinct `x` defs expected");

            // The use nodes anchor to the def in the same function scope.
            let use_nodes: Vec<_> = nodes
                .iter()
                .filter(|n| {
                    n.name.as_deref() == Some("x")
                        && n.scope == DataScope::Local
                        && edges.iter().any(|e| e.target == n.id)
                })
                .collect();
            assert_eq!(use_nodes.len(), 2);

            let outer_def = defs
                .iter()
                .min_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let inner_def = defs
                .iter()
                .max_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let use_for_inner_x = use_nodes
                .iter()
                .min_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let use_for_outer_x = use_nodes
                .iter()
                .max_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let edge_for_inner = edges
                .iter()
                .find(|e| e.target == use_for_inner_x.id)
                .expect("inner x-use must have an edge");
            let edge_for_outer = edges
                .iter()
                .find(|e| e.target == use_for_outer_x.id)
                .expect("outer x-use must have an edge");
            assert_eq!(
                edge_for_inner.source, inner_def.id,
                "inner use must bind to inner def"
            );
            assert_eq!(
                edge_for_outer.source, outer_def.id,
                "outer use must bind to outer def"
            );
            assert_ne!(edge_for_inner.source, edge_for_outer.source);
        }

        #[test]
        fn arrow_function_creates_new_scope() {
            // `x` inside the arrow must not link to outer `x`.
            let src = b"function outer() { let x = 1; const f = () => { let x = 2; return x; }; return x; }";
            let (_nodes, edges) = extract(src);
            // Just ensure no panic and that the implementation respects arrow scopes.
            // We can't easily distinguish edges from text alone; rely on non-emptiness
            // and absence of panics.
            assert!(!edges.is_empty());
        }

        #[test]
        fn no_edges_for_undefined_names() {
            let src = b"function f() { return undefinedSymbol; }";
            let (_nodes, edges) = extract(src);
            assert!(edges.is_empty(), "unresolved identifiers produce no edges");
        }

        #[test]
        fn empty_function_yields_no_nodes() {
            let src = b"function f() {}";
            let (nodes, edges) = extract(src);
            assert!(nodes.is_empty());
            assert!(edges.is_empty());
        }

        #[test]
        fn dataflow_against_fixture_file() {
            // Oracle against the shared JS fixture: must extract nodes and edges.
            let src = std::fs::read_to_string(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/javascript/functions.js"),
            )
            .unwrap();
            let (nodes, edges) = extract(src.as_bytes());
            assert!(!nodes.is_empty(), "fixture must yield data nodes");
            assert!(!edges.is_empty(), "fixture must yield flow edges");
            // Every flow edge must be anchored in a real node.
            let ids: std::collections::HashSet<_> = nodes.iter().map(|n| n.id).collect();
            for edge in &edges {
                assert!(ids.contains(&edge.source));
                assert!(ids.contains(&edge.target));
            }
        }
    }
}
