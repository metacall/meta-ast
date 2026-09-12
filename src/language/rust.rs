use crate::language::pack::define_language_pack;
use crate::language::{DefaultVisibility, DocCommentConfig};
use std::path::{Path, PathBuf};

/// Resolve a module path against a base directory.
///
/// The trailing segments may name items rather than modules, so the longest
/// path that yields a file wins: `crate::lib::compute` resolves to `lib.rs`.
fn resolve_rust_module_path(rest: &str, base: &Path) -> Option<PathBuf> {
    let segments: Vec<&str> = rest.split("::").filter(|s| !s.is_empty()).collect();
    (1..=segments.len())
        .rev()
        .find_map(|end| module_file(&segments[..end], base))
}

fn module_file(segments: &[&str], base: &Path) -> Option<PathBuf> {
    let (last, parents) = segments.split_last()?;

    let mut current = base.to_path_buf();
    for segment in parents {
        current = current.join(segment);
        if !current.is_dir() {
            return None;
        }
    }

    let direct_file = current.join(format!("{last}.rs"));
    if direct_file.is_file() {
        return Some(direct_file);
    }

    let dir_mod = current.join(last).join("mod.rs");
    dir_mod.is_file().then_some(dir_mod)
}

fn resolve_rust_import(raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if let Some(rest) = raw.strip_prefix("self::") {
        return resolve_rust_module_path(rest, source_dir);
    }
    if let Some(rest) = raw.strip_prefix("super::") {
        let parent = source_dir.parent()?;
        return resolve_rust_module_path(rest, parent);
    }
    if let Some(rest) = raw.strip_prefix("crate::") {
        let src_dir = project_root.join("src");
        let base = if src_dir.is_dir() {
            src_dir
        } else {
            project_root.to_path_buf()
        };
        return resolve_rust_module_path(rest, &base);
    }
    None
}

const RUST_IMPORT_QUERY_STR: &str = r#"
(use_declaration
  argument: (scoped_identifier) @import.path)
(use_declaration
  argument: (scoped_use_list
    path: (scoped_identifier) @import.path))
(use_as_clause
  path: (_) @import.path
  alias: (identifier) @import.alias)
"#;

const RUST_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (scoped_identifier
    name: (identifier) @reference.name))
(call_expression
  function: (field_expression
    field: (field_identifier) @reference.name))
(call_expression
  function: (field_expression
    value: (identifier) @reference.name))
(macro_invocation
  macro: (identifier) @reference.name)
"#;

define_language_pack!(
    spec: RUST_SPEC,
    label: "Rust",
    lang: LangId::Rust,
    grammar: tree_sitter_rust::LANGUAGE,
    extensions: ["rs"],
    resolver: resolve_rust_import,
    symbols: {
        static: RUST_QUERY,
        accessor: rust_query,
        query: r#"
(function_item
  (visibility_modifier)? @visibility.public
  (function_modifiers "async"? @async)?
  name: (identifier) @name
  parameters: (parameters) @signature
) @kind.function

(struct_item
  (visibility_modifier)? @visibility.public
  name: (type_identifier) @name
) @kind.struct

(enum_item
  (visibility_modifier)? @visibility.public
  name: (type_identifier) @name
) @kind.enum

(trait_item
  (visibility_modifier)? @visibility.public
  name: (type_identifier) @name
) @kind.trait

(const_item
  (visibility_modifier)? @visibility.public
  name: (identifier) @name
) @kind.constant

(static_item
  (visibility_modifier)? @visibility.public
  name: (identifier) @name
) @kind.static

(type_item
  (visibility_modifier)? @visibility.public
  name: (type_identifier) @name
) @kind.type_alias

(mod_item
  (visibility_modifier)? @visibility.public
  name: (identifier) @name
) @kind.module
"#,
    },
    imports_refs: {
        static: RUST_IMPORT_REF_QUERY,
        accessor: rust_import_ref_query,
        import: RUST_IMPORT_QUERY_STR,
        reference: RUST_REFERENCE_QUERY_STR,
    },
    import_statement_kinds: ["use_declaration"],
    class_like_parents: ["impl_item"],
    ancestors: [],
    visibility_from_name: None,
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment: Some(DocCommentConfig {
        line_prefixes: &["///", "//!"],
        block_open: Some("/**"),
        block_close: "*/",
        strip_continuation_marker: true,
    }),
    fixture: "tests/fixtures/rust/structs_enums.rs",
    snapshot: rust_insta_snapshot,
    tests {
            use crate::model::{SymbolKind, Visibility};


            #[test]
            fn extract_function() {
                let src = b"fn hello() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].name, "hello");
                assert!(matches!(symbols[0].kind, SymbolKind::Function));
            }

            #[test]
            fn extract_pub_function() {
                let src = b"pub fn hello() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].visibility, Some(Visibility::Public));
            }

            #[test]
            fn extract_async_function() {
                let src = b"async fn fetch() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert!(symbols[0].is_async);
            }

            #[test]
            fn extract_struct() {
                let src = b"struct Point { x: f64, y: f64 }";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].name, "Point");
                assert!(matches!(symbols[0].kind, SymbolKind::Struct));
            }

            #[test]
            fn pub_crate_is_not_public() {
                let src = b"pub(crate) fn internal() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_ne!(symbols[0].visibility, Some(Visibility::Public));
            }

            #[test]
            fn pub_super_is_not_public() {
                let src = b"pub(super) fn internal() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_ne!(symbols[0].visibility, Some(Visibility::Public));
            }

            #[test]
            fn pub_in_path_is_not_public() {
                let src = b"pub(in crate::foo) fn internal() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_ne!(symbols[0].visibility, Some(Visibility::Public));
            }

            #[test]
            fn pub_crate_struct_is_not_public() {
                let src = b"pub(crate) struct Internal {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_ne!(symbols[0].visibility, Some(Visibility::Public));
            }

            #[test]
            fn bare_pub_function_is_public() {
                let src = b"pub fn hello() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].visibility, Some(Visibility::Public));
            }

            #[test]
            fn extract_impl_methods() {
                let src = b"impl Foo { fn bar(&self) {} }";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);
                let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
                assert!(matches!(bar.kind, SymbolKind::Method));
            }


            #[test]
            fn rust_docstring_extraction() {
                let src = br#"/// This is a doc comment.
        /// It has two lines.
        pub fn documented_func() {}

        //! Module-level doc comment.

        /// Single line doc.
        pub struct DocumentedStruct;
        "#;
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Rust, &tree, src);

                let func = symbols
                    .iter()
                    .find(|s| s.name == "documented_func")
                    .unwrap();
                assert!(
                    func.docstring.is_some(),
                    "documented_func should have docstring"
                );
                let ds = func.docstring.as_ref().unwrap();
                assert!(
                    ds.contains("This is a doc comment"),
                    "docstring should contain first line, got: {ds}"
                );
                assert!(
                    ds.contains("It has two lines"),
                    "docstring should contain second line, got: {ds}"
                );

                let st = symbols
                    .iter()
                    .find(|s| s.name == "DocumentedStruct")
                    .unwrap();
                assert!(
                    st.docstring.is_some(),
                    "DocumentedStruct should have docstring"
                );
                assert!(st.docstring.as_ref().unwrap().contains("Single line doc"));
            }
    },
);

// ── Dataflow extraction ─────────────────────────────────────────────

#[cfg(feature = "dataflow")]
static RUST_DATAFLOW_QUERY: std::sync::LazyLock<tree_sitter::Query> = std::sync::LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_rust::LANGUAGE.into(),
        r#"
; Let binding definitions: let x = expr;
(let_declaration
  pattern: (identifier) @def.var
)

; Identifier in let binding value (usage of a variable)
(let_declaration
  value: (identifier) @use.var
)

; Function parameters
(function_item
  parameters: (parameters
    (parameter
      pattern: (identifier) @def.param
    )
  )
)

; Identifier usages in expression context (calls, binary ops, returns, etc.)
(call_expression
  function: (identifier) @use.var)
(binary_expression
  (identifier) @use.var)
(return_expression
  (identifier) @use.var)
(assignment_expression
  right: (identifier) @use.var)
(field_expression
  value: (identifier) @use.var)
"#,
        "Rust dataflow",
    )
});

/// Extract data nodes (definitions) and flow edges (def-use) from a Rust parse tree.
///
/// Phase 3 MVP: intra-procedural def-use analysis.
/// Rust AST node kinds that introduce a new intra-procedural scope.
#[cfg(feature = "dataflow")]
pub(crate) const RUST_FUNCTION_KINDS: &[&str] = &["function_item"];

/// - Captures `let` binding targets as `DataScope::Local`
/// - Captures function parameters as `DataScope::Parameter`
/// - Creates `DefUse` flow edges from each definition to each subsequent
///   usage of the same name within the same function scope
#[cfg(feature = "dataflow")]
pub fn extract_rust_dataflow(
    tree: &tree_sitter::Tree,
    source: &[u8],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    crate::language::common::extract_def_use_dataflow(
        tree,
        source,
        &RUST_DATAFLOW_QUERY,
        RUST_FUNCTION_KINDS,
        id_gen,
    )
}

#[cfg(feature = "dataflow")]
#[cfg(test)]
mod dataflow_tests {
    use super::*;
    use crate::language::LangId;

    fn extract(source: &[u8]) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
        let tree = crate::parser::parse_tree(LangId::Rust, source).unwrap();
        let id_gen = crate::model::IdGenerator::new();
        extract_rust_dataflow(&tree, source, &id_gen)
    }

    #[test]
    fn let_binding_extracts_data_node() {
        let source = b"fn main() {\n    let x = 42;\n}\n";
        let (nodes, _edges) = extract(source);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name.as_deref(), Some("x"));
        assert_eq!(nodes[0].scope, crate::model::DataScope::Local);
    }

    #[test]
    fn fn_params_extract_as_parameters() {
        let source = b"fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let (nodes, _edges) = extract(source);
        assert_eq!(nodes.len(), 4); // 2 params + 2 usages
        assert_eq!(
            nodes
                .iter()
                .filter(|n| n.scope == crate::model::DataScope::Parameter)
                .count(),
            2
        );
    }

    #[test]
    fn def_use_edge_created_for_same_name() {
        let source = b"fn main() {\n    let x = 1;\n    let y = x;\n}\n";
        let (nodes, edges) = extract(source);
        assert_eq!(nodes.len(), 3); // def x, def y, use x
        assert!(!edges.is_empty(), "should have at least one def-use edge");
        assert_eq!(edges[0].kind, crate::model::FlowKind::DefUse);
    }

    #[test]
    fn no_edge_for_different_names() {
        let source = b"fn main() {\n    let x = 1;\n    let y = 2;\n}\n";
        let (_nodes, edges) = extract(source);
        assert!(edges.is_empty(), "no flow edges for different names");
    }

    #[test]
    fn param_to_usage_edge() {
        let source = b"fn add(a: i32) -> i32 {\n    a + 1\n}\n";
        let (nodes, edges) = extract(source);
        assert_eq!(nodes.len(), 2); // param 'a' def + usage
        assert!(
            !edges.is_empty(),
            "param 'a' should have a def-use edge to its usage"
        );
    }

    #[test]
    fn data_node_ids_unique() {
        let source = b"fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n";
        let (nodes, _edges) = extract(source);
        let mut ids: Vec<u32> = nodes.iter().map(|n| n.id.to_raw()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), nodes.len(), "all data node IDs must be unique");
    }

    #[test]
    fn empty_fn_no_data_nodes() {
        let source = b"fn main() {}\n";
        let (nodes, edges) = extract(source);
        assert!(nodes.is_empty());
        assert!(edges.is_empty());
    }
}
