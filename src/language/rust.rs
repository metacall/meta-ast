use crate::language::LanguageSpec;
use std::sync::LazyLock;

static RUST_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(
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
) @kind.constant

(type_item
  (visibility_modifier)? @visibility.public
  name: (type_identifier) @name
) @kind.type_alias

(mod_item
  (visibility_modifier)? @visibility.public
  name: (identifier) @name
) @kind.module
"#,
    )
    .expect("Failed to parse Rust query")
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
(macro_invocation
  macro: (identifier) @reference.name)
"#;

static RUST_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_rust::LANGUAGE.into(),
        &format!("{}\n{}", RUST_IMPORT_QUERY_STR, RUST_REFERENCE_QUERY_STR),
    )
    .expect("Failed to parse Rust combined import+ref query")
});

fn rust_import_ref_query() -> &'static tree_sitter::Query {
    &RUST_IMPORT_REF_QUERY
}

pub const RUST_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["rs"],
    grammar_fn: || tree_sitter_rust::LANGUAGE.into(),
    query_fn: rust_query,
    import_ref_query_fn: rust_import_ref_query,
    class_like_parents: &["impl_item"],
    ancestor_visibility_rules: &[],
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
}
