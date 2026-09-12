use crate::language::DefaultVisibility;
use crate::language::pack::define_language_pack;
use crate::language::typescript::{
    TS_FAMILY_IMPORT_QUERY, TS_FAMILY_QUERY, TS_FAMILY_REFERENCE_QUERY,
};
use crate::model::Visibility;
use std::path::{Path, PathBuf};

fn resolve_tsx_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    use crate::language::import_resolver::{TS_EXTS, resolve_js_family_import};
    // Bare specifiers (react, @angular/core) resolve to external nodes,
    // matching the JS/TS behavior.
    resolve_js_family_import(raw, source_dir, TS_EXTS, &|p| p.is_file())
}

define_language_pack!(
    spec: TSX_SPEC,
    label: "TSX",
    lang: LangId::Tsx,
    grammar: tree_sitter_typescript::LANGUAGE_TSX,
    extensions: ["tsx"],
    resolver: resolve_tsx_import,
    symbols: {
        static: TSX_QUERY,
        accessor: tsx_query,
        query: TS_FAMILY_QUERY,
    },
    imports_refs: {
        static: TSX_IMPORT_REF_QUERY,
        accessor: tsx_import_ref_query,
        import: TS_FAMILY_IMPORT_QUERY,
        reference: TS_FAMILY_REFERENCE_QUERY,
    },
    import_statement_kinds: ["import_statement"],
    class_like_parents: ["class_declaration", "class"],
    ancestors: [("export_statement", Visibility::Public)],
    visibility_from_name: None,
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment: Some(crate::language::C_LIKE_DOC_COMMENT),
    fixture: "tests/fixtures/tsx/components.tsx",
    snapshot: tsx_insta_snapshot,
    tests {
            use crate::model::{SymbolKind, Visibility};


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
            fn tsx_bare_import_resolves_to_external() {
                use crate::language::LangId;
                let spec = crate::language::spec_for(LangId::Tsx);
                let out = (spec.import_path_resolver)(
                    "react",
                    std::path::Path::new("/proj/src"),
                    std::path::Path::new("/proj"),
                );
                assert_eq!(out, Some(std::path::PathBuf::from("react")));
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
    },
);

// ── Dataflow extraction ─────────────────────────────────────────────

#[cfg(feature = "dataflow")]
static TSX_DATAFLOW_QUERY: std::sync::LazyLock<tree_sitter::Query> =
    std::sync::LazyLock::new(|| {
        crate::language::common::compile_query(
            &tree_sitter_typescript::LANGUAGE_TSX.into(),
            crate::language::javascript::TS_FAMILY_DATAFLOW_QUERY,
            "TSX dataflow",
        )
    });

/// TSX AST node kinds that introduce a new intra-procedural scope.
#[cfg(feature = "dataflow")]
pub(crate) const TSX_FUNCTION_KINDS: &[&str] = crate::language::common::JS_FAMILY_FUNCTION_KINDS;

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
