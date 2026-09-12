use crate::language::{DefaultVisibility, LanguageSpec};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_c_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    crate::language::import_resolver::resolve_c_family_import(raw, source_dir)
}

static C_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
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

        (declaration
            declarator: [
                (function_declarator
                    declarator: (identifier) @name
                    parameters: (parameter_list) @signature)
                (pointer_declarator
                    declarator: (function_declarator
                        declarator: (identifier) @name
                        parameters: (parameter_list) @signature))
            ]) @kind.declaration
        "#,
        "C",
    )
});

pub(crate) const C_FAMILY_IMPORT_QUERY_STR: &str = r#"
(preproc_include
  path: (string_literal) @import.path)
(preproc_include
    path: (system_lib_string) @import.path)
"#;

pub(crate) const C_FAMILY_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (field_expression
    field: (field_identifier) @reference.name))
(call_expression
  function: (field_expression
    argument: (identifier) @reference.name))
"#;

fn c_query() -> &'static tree_sitter::Query {
    &C_QUERY
}

static C_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_c::LANGUAGE.into(),
        &format!(
            "{}\n{}",
            C_FAMILY_IMPORT_QUERY_STR, C_FAMILY_REFERENCE_QUERY_STR
        ),
        "C combined import+ref",
    )
});

fn c_import_ref_query() -> &'static tree_sitter::Query {
    &C_IMPORT_REF_QUERY
}

pub(crate) const C_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["c", "h"],
    grammar_fn: || tree_sitter_c::LANGUAGE.into(),
    query_fn: c_query,
    import_path_resolver: resolve_c_import,
    import_ref_query_fn: c_import_ref_query,
    class_like_parents: &[],
    ancestor_visibility_rules: &[],
    visibility_from_name: None,
    import_statement_kinds: &["preproc_include"],
    default_visibility: DefaultVisibility::PublicByDefault,
    doc_comment_config: Some(crate::language::C_LIKE_DOC_COMMENT),
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
    fn c_plain_line_comment_is_not_a_docstring() {
        let note = b"// note for readers\nint noted(void) { return 0; }";
        let tree = parse(note);
        let symbols = extract_symbols_for(LangId::C, &tree, note);
        let func = symbols.iter().find(|s| s.name == "noted").unwrap();
        assert!(
            func.docstring.is_none(),
            "a plain line comment must stay a comment"
        );

        let documented = b"/// line doc\nint lined(void) { return 0; }";
        let tree = parse(documented);
        let symbols = extract_symbols_for(LangId::C, &tree, documented);
        let func = symbols.iter().find(|s| s.name == "lined").unwrap();
        assert_eq!(func.docstring.as_deref(), Some("line doc"));
    }

    #[test]
    fn c_docstring_extraction() {
        let src = b"/** Doxygen comment. */\nint documented() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        let func = symbols.iter().find(|s| s.name == "documented").unwrap();
        assert!(func.docstring.is_some(), "documented should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("Doxygen comment"));
    }

    #[test]
    fn extract_function_declaration() {
        let src = b"int add(int a, int b);";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        let found = symbols.iter().find(|s| s.name == "add");
        assert!(found.is_some(), "missing add: {symbols:?}");
        let found = found.unwrap();
        assert_eq!(format!("{:?}", found.kind), "Declaration");
        assert_eq!(found.signature.as_deref(), Some("(int a, int b)"));
    }

    #[test]
    fn extract_pointer_returning_function_declaration() {
        let src = b"int *f(void);";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        let found = symbols.iter().find(|s| s.name == "f");
        assert!(found.is_some(), "missing f: {symbols:?}");
        assert_eq!(format!("{:?}", found.unwrap().kind), "Declaration");
    }

    #[test]
    fn extern_variable_declaration_is_not_a_symbol() {
        let src = b"extern int counter;";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        assert!(!symbols.iter().any(|s| s.name == "counter"));
    }

    #[test]
    fn typedef_function_pointer_is_not_a_declaration() {
        let src = b"typedef int (*cb)(int);";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        assert!(!symbols.iter().any(|s| s.name == "cb"));
    }

    #[test]
    fn definition_is_not_marked_as_a_declaration() {
        let src = b"int add(int a, int b) { return a + b; }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::C, &tree, src);
        assert_eq!(symbols.len(), 1, "{symbols:?}");
        assert_eq!(symbols[0].name, "add");
        assert_eq!(format!("{:?}", symbols[0].kind), "Function");
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
