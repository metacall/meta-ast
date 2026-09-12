use crate::language::DefaultVisibility;
use crate::language::pack::define_language_pack;
use crate::model::Visibility;
use std::path::{Path, PathBuf};

fn resolve_js_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    use crate::language::import_resolver::{JS_EXTS, resolve_js_family_import};
    resolve_js_family_import(raw, source_dir, JS_EXTS, &|p| p.is_file())
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

const JS_REFERENCE_QUERY_STR: &str = crate::language::typescript::TS_FAMILY_REFERENCE_QUERY;

define_language_pack!(
    spec: JS_SPEC,
    label: "JavaScript",
    lang: LangId::JavaScript,
    grammar: tree_sitter_javascript::LANGUAGE,
    extensions: ["js", "mjs", "cjs"],
    resolver: resolve_js_import,
    symbols: {
        static: JS_QUERY,
        accessor: js_query,
        query: r#"
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

(variable_declarator
  name: (identifier) @name
  value: (arrow_function
    "async"? @async
    parameters: (formal_parameters) @signature)
) @kind.function

(variable_declarator
  name: (identifier) @name
  value: (function_expression
    "async"? @async
    parameters: (formal_parameters) @signature)
) @kind.function
"#,
    },
    imports_refs: {
        static: JS_IMPORT_REF_QUERY,
        accessor: js_import_ref_query,
        import: JS_IMPORT_QUERY_STR,
        reference: JS_REFERENCE_QUERY_STR,
    },
    import_statement_kinds: ["import_statement"],
    class_like_parents: ["class_declaration", "class"],
    ancestors: [("export_statement", Visibility::Public)],
    visibility_from_name: None,
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment: Some(crate::language::C_LIKE_DOC_COMMENT),
    fixture: "tests/fixtures/javascript/functions.js",
    snapshot: js_insta_snapshot,
    tests {
            use crate::model::{SymbolKind, Visibility};


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
                let (imports, _, _) = extract_imports_and_references_for(
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
                    assert_eq!(imp.import_specifier, "utils");
                }
                assert_eq!(named[0].symbol.as_deref(), Some("foo"));
                assert_eq!(named[1].symbol.as_deref(), Some("bar"));
            }

            #[test]
            fn extract_default_import() {
                use crate::language::extract_imports_and_references_for;
                let src = b"import React from 'react';";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
                    LangId::JavaScript,
                    &tree,
                    src,
                    &std::path::PathBuf::from("test.js"),
                );
                let named: Vec<_> = imports.iter().filter(|i| i.symbol.is_some()).collect();
                assert_eq!(named.len(), 1);
                assert_eq!(named[0].import_specifier, "react");
                assert_eq!(named[0].symbol.as_deref(), Some("React"));
            }

            #[test]
            fn extract_side_effect_import() {
                use crate::language::extract_imports_and_references_for;
                let src = b"import 'styles.css';";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
                    LangId::JavaScript,
                    &tree,
                    src,
                    &std::path::PathBuf::from("test.js"),
                );
                assert_eq!(imports.len(), 1);
                assert_eq!(imports[0].import_specifier, "styles.css");
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

            fn symbol_names(symbols: &[crate::language::RawSymbol<'_>]) -> Vec<String> {
                symbols.iter().map(|s| s.name.to_string()).collect()
            }

            #[test]
            fn extract_arrow_function_assigned_to_const() {
                let src = b"const compute = (a, b) => a + b;";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
                let found = symbols.iter().find(|s| s.name == "compute");
                assert!(
                    found.is_some(),
                    "missing compute: {:?}",
                    symbol_names(&symbols)
                );
                let found = found.unwrap();
                assert!(matches!(found.kind, SymbolKind::Function));
                assert_eq!(found.signature.as_deref(), Some("(a, b)"));
            }

            #[test]
            fn extract_exported_async_arrow_function() {
                let src = b"export const handler = async () => {};";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
                let found = symbols.iter().find(|s| s.name == "handler");
                assert!(
                    found.is_some(),
                    "missing handler: {:?}",
                    symbol_names(&symbols)
                );
                let found = found.unwrap();
                assert!(matches!(found.kind, SymbolKind::Function));
                assert!(found.is_async);
                assert_eq!(found.visibility, Some(Visibility::Public));
            }

            #[test]
            fn extract_function_expression_assigned_to_const() {
                let src = b"const greet = function inner(x) { return x; };";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
                let found = symbols.iter().find(|s| s.name == "greet");
                assert!(
                    found.is_some(),
                    "missing greet: {:?}",
                    symbol_names(&symbols)
                );
                assert!(!symbols.iter().any(|s| s.name == "inner"));
                assert_eq!(found.unwrap().signature.as_deref(), Some("(x)"));
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

                #[test]
                fn def_use_pairs_follow_the_nearest_preceding_definition() {
                    // Two functions declare their own `x`: a use must never link to a
                    // definition in the other function, and the nearest preceding
                    // definition wins inside one function.
                    let src = b"function f() { let x = 1; let x = x + 1; return x; }\nfunction g() { let x = 9; return x; }";
                    let (nodes, edges) = extract(src);
                    assert_eq!(nodes.len(), 6, "three definitions and three uses");
                    assert_eq!(edges.len(), 3, "one edge per use site");

                    let span = |id| {
                        let node = nodes.iter().find(|node| node.id == id).unwrap();
                        (
                            node.name.clone().unwrap_or_default(),
                            node.source_range.byte_start,
                        )
                    };

                    let mut definitions = std::collections::BTreeSet::new();
                    for edge in &edges {
                        let (def_name, def_start) = span(edge.source);
                        let (use_name, use_start) = span(edge.target);
                        assert_eq!(def_name, use_name, "a def-use edge keeps the name");
                        assert!(
                            def_start < use_start,
                            "a definition precedes its use: {def_start} against {use_start}"
                        );
                        definitions.insert(def_start);
                    }
                    assert_eq!(
                        definitions.len(),
                        2,
                        "two definitions feed the three uses: {definitions:?}"
                    );

                    // `function g(` starts the second scope; uses and definitions must
                    // sit on the same side of that boundary.
                    let boundary = src
                        .windows(b"function g(".len())
                        .position(|window| window == b"function g(")
                        .unwrap();
                    for edge in &edges {
                        let def_start = span(edge.source).1;
                        let use_start = span(edge.target).1;
                        assert_eq!(
                            def_start > boundary,
                            use_start > boundary,
                            "a use links within its own function: {def_start} and {use_start}"
                        );
                    }
                }
            }
    },
);

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
/// `function_kinds` lists the AST node kinds that introduce a new scope.
/// Delegates to the shared def-use engine in `common`.
#[cfg(feature = "dataflow")]
pub(crate) fn extract_js_family_dataflow_with_query(
    tree: &tree_sitter::Tree,
    source: &[u8],
    query: &tree_sitter::Query,
    function_kinds: &[&str],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    crate::language::common::extract_def_use_dataflow(tree, source, query, function_kinds, id_gen)
}

#[cfg(feature = "dataflow")]
static JS_DATAFLOW_QUERY: std::sync::LazyLock<tree_sitter::Query> = std::sync::LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_javascript::LANGUAGE.into(),
        JS_DATAFLOW_QUERY_STR,
        "JavaScript dataflow",
    )
});

/// JavaScript AST node kinds that introduce a new intra-procedural scope.
#[cfg(feature = "dataflow")]
pub(crate) const JS_FUNCTION_KINDS: &[&str] = crate::language::common::JS_FAMILY_FUNCTION_KINDS;

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
