use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_rust_module_path(rest: &str, base: &Path) -> Option<PathBuf> {
    let segments: Vec<&str> = rest.split("::").collect();
    if segments.is_empty() {
        return None;
    }
    let module = segments[0];

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
