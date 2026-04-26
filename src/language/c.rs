use crate::language::LanguageSpec;
use once_cell::sync::Lazy;

static C_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_c::LANGUAGE.into(),
        r#"
        (function_definition
            declarator: [
                (function_declarator
                    declarator: (identifier) @name
                    parameters: (parameter_list) @signature)
                (function_declarator
                    declarator: (parenthesized_declarator
                        (identifier) @name)
                    parameters: (parameter_list) @signature)
                (pointer_declarator
                    declarator: (function_declarator
                        declarator: (identifier) @name
                        parameters: (parameter_list) @signature))
            ]) @kind.function

        (struct_specifier
            name: (type_identifier) @name) @kind.struct

        (enum_specifier
            name: (type_identifier) @name) @kind.enum

        (type_definition
            declarator: (type_identifier) @name) @kind.type_alias
        "#,
    )
    .expect("Failed to parse C query")
});

static C_IMPORT_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_c::LANGUAGE.into(),
        r#"
(preproc_include
  path: (string_literal) @import.path)
(preproc_include
  path: (system_lib_string) @import.path)
"#,
    )
    .expect("Failed to parse C import query")
});

static C_REFERENCE_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_c::LANGUAGE.into(),
        r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (field_expression
    field: (field_identifier) @reference.name))
"#,
    )
    .expect("Failed to parse C reference query")
});

fn c_query() -> &'static tree_sitter::Query {
    &C_QUERY
}

fn c_import_query() -> &'static tree_sitter::Query {
    &C_IMPORT_QUERY
}

fn c_reference_query() -> &'static tree_sitter::Query {
    &C_REFERENCE_QUERY
}

pub const C_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["c"],
    grammar_fn: || tree_sitter_c::LANGUAGE.into(),
    query_fn: c_query,
    import_query_fn: c_import_query,
    reference_query_fn: c_reference_query,
    class_like_parents: &[],
    ancestor_visibility_rules: &[],
};

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::C)).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn c_grammar_loads() {
        let _ = grammar_for(LangId::C);
    }

    #[test]
    fn extract_function() {
        let src = b"int add(int a, int b) { return a + b; }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "add");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
        assert_eq!(symbols[0].signature.as_deref(), Some("(int a, int b)"));
    }

    #[test]
    fn extract_struct() {
        let src = b"struct Point { int x; int y; };";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        let s = symbols.iter().find(|s| s.name == "Point").unwrap();
        assert!(matches!(s.kind, SymbolKind::Struct));
    }

    #[test]
    fn extract_enum() {
        let src = b"enum Color { RED, GREEN };";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        let s = symbols.iter().find(|s| s.name == "Color").unwrap();
        assert!(matches!(s.kind, SymbolKind::Enum));
    }

    #[test]
    fn c_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/c/functions.c"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::C, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
