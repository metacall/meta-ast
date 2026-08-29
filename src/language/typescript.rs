use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use crate::model::Visibility;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_ts_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if !raw.starts_with('.') && !raw.starts_with('/') {
        // Bare module name -- returns as-is so graph builder creates ExternalNode.
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

pub(crate) const TS_FAMILY_QUERY: &str = r#"
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
  (type_identifier) @name
) @kind.class

(abstract_class_declaration
  (type_identifier) @name
) @kind.class

(interface_declaration
  (type_identifier) @name
) @kind.interface

(enum_declaration
  (identifier) @name
) @kind.enum

(type_alias_declaration
  (type_identifier) @name
) @kind.type_alias

(method_definition
  "async"? @async
  name: (_) @name
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
      (type_identifier) @name
    ) @kind.class
    (abstract_class_declaration
      (type_identifier) @name
    ) @kind.class
    (interface_declaration
      (type_identifier) @name
    ) @kind.interface
    (enum_declaration
      (identifier) @name
    ) @kind.enum
    (type_alias_declaration
      (type_identifier) @name
    ) @kind.type_alias
  ]
)
"#;

static TS_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        TS_FAMILY_QUERY,
        "TypeScript",
    )
});

pub(crate) const TS_FAMILY_IMPORT_QUERY: &str = r#"
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

pub(crate) const TS_FAMILY_REFERENCE_QUERY: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (member_expression
    property: (property_identifier) @reference.name))
(call_expression
  function: (member_expression
    object: (identifier) @reference.name))
"#;

static TS_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        &format!("{}\n{}", TS_FAMILY_IMPORT_QUERY, TS_FAMILY_REFERENCE_QUERY),
        "TypeScript combined import+ref",
    )
});

fn ts_import_ref_query() -> &'static tree_sitter::Query {
    &TS_IMPORT_REF_QUERY
}

fn ts_query() -> &'static tree_sitter::Query {
    &TS_QUERY
}

pub(crate) const TS_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["ts", "cts", "mts"],
    grammar_fn: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    query_fn: ts_query,
    import_path_resolver: resolve_ts_import,
    import_ref_query_fn: ts_import_ref_query,
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

#[cfg(feature = "dataflow")]
static TS_DATAFLOW_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        crate::language::javascript::TS_FAMILY_DATAFLOW_QUERY,
        "TypeScript dataflow",
    )
});

/// TypeScript AST node kinds that introduce a new intra-procedural scope.
#[cfg(feature = "dataflow")]
pub(crate) const TS_FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "generator_function",
    "arrow_function",
    "method_definition",
];

/// Extract data nodes and flow edges from a TypeScript parse tree.
#[cfg(feature = "dataflow")]
pub fn extract_typescript_dataflow(
    tree: &tree_sitter::Tree,
    source: &[u8],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    crate::language::javascript::extract_js_family_dataflow_with_query(
        tree,
        source,
        &TS_DATAFLOW_QUERY,
        TS_FUNCTION_KINDS,
        id_gen,
    )
}

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&grammar_for(LangId::TypeScript))
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_interface() {
        let src = b"interface Foo { bar(): void; }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::TypeScript, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Foo");
        assert!(matches!(symbols[0].kind, SymbolKind::Interface));
    }

    #[test]
    fn extract_type_alias() {
        let src = b"type Point = { x: number; };";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::TypeScript, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Point");
        assert!(matches!(symbols[0].kind, SymbolKind::TypeAlias));
    }

    #[test]
    fn extract_enum() {
        let src = b"enum Dir { A, B }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::TypeScript, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Dir");
        assert!(matches!(symbols[0].kind, SymbolKind::Enum));
    }

    #[test]
    fn extract_ts_named_imports() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import { Component, OnInit } from '@angular/core';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::TypeScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.ts"),
        );
        let named: Vec<_> = imports.iter().filter(|i| i.symbol.is_some()).collect();
        assert_eq!(named.len(), 2);
        for imp in &named {
            assert_eq!(imp.import_specifier, "'@angular/core'");
        }
        assert_eq!(named[0].symbol.as_deref(), Some("Component"));
        assert_eq!(named[1].symbol.as_deref(), Some("OnInit"));
    }

    #[test]
    fn extract_ts_default_import() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import React from 'react';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::TypeScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.ts"),
        );
        let named: Vec<_> = imports.iter().filter(|i| i.symbol.is_some()).collect();
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].import_specifier, "'react'");
        assert_eq!(named[0].symbol.as_deref(), Some("React"));
    }

    #[test]
    fn ts_docstring_extraction() {
        let src = b"/** TSDoc comment. */\nfunction documented() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::TypeScript, &tree, src);
        let func = symbols.iter().find(|s| s.name == "documented").unwrap();
        assert!(func.docstring.is_some(), "documented should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("TSDoc comment"));
    }

    #[test]
    fn ts_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/typescript/interfaces.ts"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::TypeScript, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }

    #[cfg(feature = "dataflow")]
    mod dataflow_tests {
        use super::*;
        use crate::language::typescript::extract_typescript_dataflow;
        use crate::model::{DataScope, FlowKind};

        fn extract(source: &[u8]) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
            let id_gen = crate::model::IdGenerator::new();
            extract_typescript_dataflow(&parse(source), source, &id_gen)
        }

        #[test]
        fn typed_let_binding_captured() {
            let src = b"function f(): number { let x: number = 42; return x; }";
            let (nodes, edges) = extract(src);
            assert!(
                nodes
                    .iter()
                    .any(|n| n.name.as_deref() == Some("x") && n.scope == DataScope::Local)
            );
            assert!(!edges.is_empty(), "x usage should yield an edge");
        }

        #[test]
        fn typed_parameter_captured_as_parameter() {
            let src = b"function add(a: number, b: number): number { return a + b; }";
            let (nodes, edges) = extract(src);
            let params: Vec<_> = nodes
                .iter()
                .filter(|n| n.scope == DataScope::Parameter)
                .collect();
            assert_eq!(params.len(), 2);
            let names: Vec<_> = params.iter().map(|n| n.name.as_deref()).collect();
            assert!(names.contains(&Some("a")));
            assert!(names.contains(&Some("b")));
            assert!(!edges.is_empty());
        }

        #[test]
        fn flow_edges_anchored_to_real_nodes() {
            let src = b"function f() { let x = 1; return x; }";
            let (nodes, edges) = extract(src);
            let ids: std::collections::HashSet<_> = nodes.iter().map(|n| n.id).collect();
            for edge in &edges {
                assert!(ids.contains(&edge.source), "edge source not in nodes");
                assert!(ids.contains(&edge.target), "edge target not in nodes");
                assert_eq!(edge.kind, FlowKind::DefUse);
                assert!((edge.confidence - 0.9).abs() < f32::EPSILON);
            }
        }

        #[test]
        fn no_duplicate_def_for_typed_let() {
            // Regression: the type annotation must not double-capture the var.
            let src = b"function f() { let x: number = 1; return x; }";
            let (nodes, _edges) = extract(src);
            let x_defs: Vec<_> = nodes
                .iter()
                .filter(|n| n.name.as_deref() == Some("x") && n.scope == DataScope::Local)
                .collect();
            // 1 definition + 1 usage-node.
            assert_eq!(
                x_defs.len(),
                2,
                "expected exactly one def + one use for `x` (no double-capture)"
            );
        }

        #[test]
        fn cross_function_scoping() {
            let src = b"function outer() { let x = 1; function inner() { let x = 2; return x; } return x; }";
            let (nodes, edges) = extract(src);
            // 2 def + 2 use nodes for `x`, all named "x".
            let defs: Vec<_> = nodes
                .iter()
                .filter(|n| n.name.as_deref() == Some("x") && n.scope == DataScope::Local)
                .collect();
            assert_eq!(defs.len(), 4);
            // Of those, the defs are the ones without an incoming edge.
            let true_defs: Vec<_> = nodes
                .iter()
                .filter(|n| {
                    n.name.as_deref() == Some("x")
                        && n.scope == DataScope::Local
                        && !edges.iter().any(|e| e.target == n.id)
                })
                .collect();
            assert_eq!(true_defs.len(), 2, "two distinct `x` defs expected");
            // 2 use edges must exist.
            assert!(edges.len() >= 2);
            // Each use must be anchored to a def in the same function scope.
            let inner_def = true_defs
                .iter()
                .max_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let outer_def = true_defs
                .iter()
                .min_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let uses: Vec<_> = nodes
                .iter()
                .filter(|n| {
                    n.name.as_deref() == Some("x")
                        && n.scope == DataScope::Local
                        && edges.iter().any(|e| e.target == n.id)
                })
                .collect();
            let inner_use = uses
                .iter()
                .min_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let outer_use = uses
                .iter()
                .max_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let inner_edge = edges.iter().find(|e| e.target == inner_use.id).unwrap();
            let outer_edge = edges.iter().find(|e| e.target == outer_use.id).unwrap();
            assert_eq!(inner_edge.source, inner_def.id);
            assert_eq!(outer_edge.source, outer_def.id);
            assert_ne!(inner_edge.source, outer_edge.source);
        }

        #[test]
        fn dataflow_against_fixture_file() {
            let src = std::fs::read_to_string(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/typescript/interfaces.ts"),
            )
            .unwrap();
            let (nodes, edges) = extract(src.as_bytes());
            // Interfaces / type aliases don't introduce data nodes,
            // but the class methods in the fixture should produce some.
            assert!(!nodes.is_empty(), "fixture must yield some data nodes");
            let ids: std::collections::HashSet<_> = nodes.iter().map(|n| n.id).collect();
            for edge in &edges {
                assert!(ids.contains(&edge.source));
                assert!(ids.contains(&edge.target));
            }
        }
    }
}
