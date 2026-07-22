use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_rust_module_path(rest: &str, base: &Path) -> Option<PathBuf> {
    let segments: Vec<&str> = rest.split("::").collect();
    let module = segments.first()?;

    // Try direct file: base/module.rs
    let direct_file = base.join(format!("{module}.rs"));
    if direct_file.exists() {
        return Some(direct_file);
    }

    // Try directory module: base/module/mod.rs
    let dir_mod = base.join(module).join("mod.rs");
    if dir_mod.exists() {
        return Some(dir_mod);
    }

    None
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
        return resolve_rust_module_path(rest, project_root);
    }
    None
}

static RUST_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_rust::LANGUAGE.into(),
        r#"
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
        "Rust",
    )
});

fn rust_query() -> &'static tree_sitter::Query {
    &RUST_QUERY
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

static RUST_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_rust::LANGUAGE.into(),
        &format!("{}\n{}", RUST_IMPORT_QUERY_STR, RUST_REFERENCE_QUERY_STR),
        "Rust combined import+ref",
    )
});

fn rust_import_ref_query() -> &'static tree_sitter::Query {
    &RUST_IMPORT_REF_QUERY
}

pub(crate) const RUST_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["rs"],
    grammar_fn: || tree_sitter_rust::LANGUAGE.into(),
    query_fn: rust_query,
    import_path_resolver: resolve_rust_import,
    import_ref_query_fn: rust_import_ref_query,
    class_like_parents: &["impl_item"],
    ancestor_visibility_rules: &[],
    visibility_from_name: None,
    import_statement_kinds: &["use_declaration"],
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment_config: Some(DocCommentConfig {
        line_prefixes: &["///", "//!"],
        block_open: Some("/**"),
        block_close: "*/",
        strip_continuation_marker: true,
    }),
};

// ── Dataflow extraction ─────────────────────────────────────────────

#[cfg(feature = "dataflow")]
static RUST_DATAFLOW_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
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
    use crate::model::{DataNode, DataNodeId, DataScope, FlowEdge, FlowKind};
    use tree_sitter::StreamingIterator;

    let query = &*RUST_DATAFLOW_QUERY;
    let mut cursor = tree_sitter::QueryCursor::new();

    // Collect definitions and usages with their byte positions and function scope
    // Each entry: (name, byte_pos, node, is_param)
    let mut defs: Vec<(String, usize, tree_sitter::Node, bool)> = Vec::new();
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
                "def.var" => {
                    defs.push((name, byte_pos, node, false));
                }
                "def.param" => {
                    defs.push((name, byte_pos, node, true));
                }
                "use.var" => {
                    let func_start = enclosing_function_start(node);
                    uses.push((name, byte_pos, node, func_start));
                }
                _ => {}
            }
        }
    }

    // Build DataNodes from definitions
    let mut nodes = Vec::new();
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

    // Build FlowEdges: for each usage, find the nearest preceding definition
    // of the same name within the same function scope. Register a usage DataNode
    // so both source and target exist in the graph.
    let mut edges = Vec::new();

    for (use_name, use_pos, use_node, use_func_start) in &uses {
        let mut best_def: Option<&(String, usize, DataNodeId, bool)> = None;

        for def in &def_ids {
            if def.0 == *use_name && def.1 < *use_pos {
                let def_func_start = enclosing_function_start_for_id(def.1, tree);
                if def_func_start == *use_func_start {
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
        }

        if let Some(def) = best_def {
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

/// Find the start byte of the enclosing function_item for a node.
#[cfg(feature = "dataflow")]
fn enclosing_function_start(node: tree_sitter::Node) -> usize {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_item" {
            return parent.start_byte();
        }
        current = parent.parent();
    }
    // Top-level code (not in any function)
    0
}

/// Find the enclosing function start for a given byte position.
#[cfg(feature = "dataflow")]
fn enclosing_function_start_for_id(byte_pos: usize, tree: &tree_sitter::Tree) -> usize {
    let node = tree.root_node();
    find_enclosing_function(node, byte_pos)
}

#[cfg(feature = "dataflow")]
fn find_enclosing_function(node: tree_sitter::Node, byte_pos: usize) -> usize {
    if node.start_byte() <= byte_pos && byte_pos < node.end_byte() {
        if node.kind() == "function_item" {
            return node.start_byte();
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i as u32) {
                let result = find_enclosing_function(child, byte_pos);
                if result != 0 {
                    return result;
                }
            }
        }
    }
    0
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
        let mut ids: Vec<u32> = nodes.iter().map(|n| n.id.0).collect();
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

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::{SymbolKind, Visibility};

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Rust)).unwrap();
        parser.parse(source, None).unwrap()
    }

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
    fn rust_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/rust/structs_enums.rs"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Rust, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
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
}
