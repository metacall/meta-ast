use crate::language::typescript::{
    TS_FAMILY_IMPORT_QUERY, TS_FAMILY_QUERY, TS_FAMILY_REFERENCE_QUERY,
};
use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use crate::model::Visibility;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_tsx_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if !raw.starts_with('.') && !raw.starts_with('/') {
        return None;
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

static TSX_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        TS_FAMILY_QUERY,
        "TSX",
    )
});

static TSX_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        &format!("{}\n{}", TS_FAMILY_IMPORT_QUERY, TS_FAMILY_REFERENCE_QUERY),
        "TSX combined import+ref",
    )
});

fn tsx_import_ref_query() -> &'static tree_sitter::Query {
    &TSX_IMPORT_REF_QUERY
}

fn tsx_query() -> &'static tree_sitter::Query {
    &TSX_QUERY
}

pub(crate) const TSX_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["tsx"],
    grammar_fn: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    query_fn: tsx_query,
    import_path_resolver: resolve_tsx_import,
    import_ref_query_fn: tsx_import_ref_query,
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
static TSX_DATAFLOW_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        crate::language::javascript::TS_FAMILY_DATAFLOW_QUERY,
        "TSX dataflow",
    )
});

/// TSX AST node kinds that introduce a new intra-procedural scope.
#[cfg(feature = "dataflow")]
pub(crate) const TSX_FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "generator_function",
    "arrow_function",
    "method_definition",
];

/// Extract data nodes and flow edges from a TSX parse tree.
#[cfg(feature = "dataflow")]
pub fn extract_tsx_dataflow(
    tree: &tree_sitter::Tree,
    source: &[u8],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    crate::language::javascript::extract_js_family_dataflow_with_query(
        tree,
        source,
        &TSX_DATAFLOW_QUERY,
        TSX_FUNCTION_KINDS,
        id_gen,
    )
}

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::{SymbolKind, Visibility};

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Tsx)).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_tsx_function() {
        let src = b"function App(): JSX.Element { return <div/>; }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "App");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_tsx_exported_class() {
        let src = b"export class Foo extends React.Component { render() { return <div/>; } }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        assert_eq!(class.visibility, Some(Visibility::Public));
    }

    #[test]
    fn tsx_docstring_extraction() {
        let src = b"/** Component doc. */\nfunction App() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src);
        let func = symbols.iter().find(|s| s.name == "App").unwrap();
        assert!(func.docstring.is_some(), "App should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("Component doc"));
    }

    #[test]
    fn tsx_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tsx/components.tsx"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }

    #[cfg(feature = "dataflow")]
    mod dataflow_tests {
        use super::*;
        use crate::language::tsx::extract_tsx_dataflow;
        use crate::model::{DataScope, FlowKind};

        fn extract(source: &[u8]) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
            let id_gen = crate::model::IdGenerator::new();
            extract_tsx_dataflow(&parse(source), source, &id_gen)
        }

        #[test]
        fn typed_parameter_captured() {
            let src = b"function App(props: { name: string }): JSX.Element { return <div>{props.name}</div>; }";
            let (nodes, edges) = extract(src);
            let params: Vec<_> = nodes
                .iter()
                .filter(|n| n.scope == DataScope::Parameter)
                .collect();
            assert!(
                params.iter().any(|n| n.name.as_deref() == Some("props")),
                "props parameter must be captured"
            );
            assert!(!edges.is_empty());
        }

        #[test]
        fn flow_edges_anchored_to_real_nodes() {
            let src = b"function App(props: { name: string }): JSX.Element { let local = 1; return <div>{local}</div>; }";
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
        fn cross_function_scoping() {
            let src = b"function outer() { let x = 1; const Inner = () => { let x = 2; return x; }; return x; }";
            let (nodes, edges) = extract(src);
            // 2 distinct `x` defs (one per scope).
            let defs: Vec<_> = nodes
                .iter()
                .filter(|n| {
                    n.name.as_deref() == Some("x")
                        && n.scope == DataScope::Local
                        && !edges.iter().any(|e| e.target == n.id)
                })
                .collect();
            assert_eq!(defs.len(), 2);
            // 2 use edges must exist.
            assert!(edges.len() >= 2);
            // Edges must connect each use to the def in the same scope.
            let inner_def = defs
                .iter()
                .max_by_key(|n| n.source_range.byte_start)
                .unwrap();
            let outer_def = defs
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
        }

        #[test]
        fn dataflow_against_fixture_file() {
            let src = std::fs::read_to_string(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/tsx/components.tsx"),
            )
            .unwrap();
            let (nodes, edges) = extract(src.as_bytes());
            assert!(!nodes.is_empty(), "fixture must yield data nodes");
            let ids: std::collections::HashSet<_> = nodes.iter().map(|n| n.id).collect();
            for edge in &edges {
                assert!(ids.contains(&edge.source));
                assert!(ids.contains(&edge.target));
            }
        }
    }
}
