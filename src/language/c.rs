use crate::language::pack::define_language_pack;
use crate::language::{C_LIKE_DOC_COMMENT, DefaultVisibility};
use std::path::{Path, PathBuf};
use crate::language::LangId;

fn resolve_c_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    crate::language::import_resolver::resolve_c_family_import(raw, source_dir)
}

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

define_language_pack!(
    spec: C_SPEC,
    label: "C",
    lang: LangId::C,
    grammar: tree_sitter_c::LANGUAGE,
    extensions: ["c", "h"],
    resolver: resolve_c_import,
    symbols: {
        static: C_QUERY,
        accessor: c_query,
        query: r#"
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
    },
    imports_refs: {
        static: C_IMPORT_REF_QUERY,
        accessor: c_import_ref_query,
        import: C_FAMILY_IMPORT_QUERY_STR,
        reference: C_FAMILY_REFERENCE_QUERY_STR,
    },
    import_statement_kinds: ["preproc_include"],
    class_like_parents: [],
    ancestors: [],
    visibility_from_name: None,
    default_visibility: DefaultVisibility::PublicByDefault,
    doc_comment: Some(C_LIKE_DOC_COMMENT),
    fixture: "tests/fixtures/c/functions.c",
    snapshot: c_insta_snapshot,
    tests {
        use crate::model::SymbolKind;

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
    },
);
