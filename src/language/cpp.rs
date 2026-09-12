use crate::language::DefaultVisibility;
use crate::language::c::{C_FAMILY_IMPORT_QUERY_STR, C_FAMILY_REFERENCE_QUERY_STR};
use crate::language::pack::define_language_pack;
use std::path::{Path, PathBuf};

fn resolve_cpp_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    crate::language::import_resolver::resolve_c_family_import(raw, source_dir)
}

define_language_pack!(
    spec: CPP_SPEC,
    label: "C++",
    lang: LangId::Cpp,
    grammar: tree_sitter_cpp::LANGUAGE,
    extensions: ["cc", "cpp", "cxx", "hpp"],
    resolver: resolve_cpp_import,
    symbols: {
        static: CPP_QUERY,
        accessor: cpp_query,
        query: r#"
        (function_definition
          declarator: (function_declarator
            declarator: [
              (identifier)
              (field_identifier)
              (operator_name)
              (destructor_name)
            ] @name
            parameters: (parameter_list) @signature
          )
        ) @kind.function

        (function_definition
          declarator: (function_declarator
            declarator: (qualified_identifier name: (identifier) @name)
            parameters: (parameter_list) @signature
          )
        ) @kind.function

        (function_definition
          declarator: (pointer_declarator
            declarator: (function_declarator
              declarator: (identifier) @name
              parameters: (parameter_list) @signature
            )
          )
        ) @kind.function

        (function_definition
          declarator: (reference_declarator
            (function_declarator
              declarator: (identifier) @name
              parameters: (parameter_list) @signature
            )
          )
        ) @kind.function

        (declaration
          declarator: (function_declarator
            declarator: (identifier) @name
            parameters: (parameter_list) @signature
          )
        ) @kind.declaration

        (declaration
          declarator: (function_declarator
            declarator: (qualified_identifier name: (identifier) @name)
            parameters: (parameter_list) @signature
          )
        ) @kind.declaration

        (declaration
          declarator: (pointer_declarator
            declarator: (function_declarator
              declarator: (identifier) @name
              parameters: (parameter_list) @signature
            )
          )
        ) @kind.declaration

        (field_declaration
          declarator: (function_declarator
            declarator: [
              (field_identifier)
              (operator_name)
              (destructor_name)
            ] @name
            parameters: (parameter_list) @signature
          )
        ) @kind.declaration

        (class_specifier
            name: (type_identifier) @name) @kind.class

        (struct_specifier
            name: (type_identifier) @name) @kind.struct

        (enum_specifier
            name: (type_identifier) @name) @kind.enum

        (namespace_definition
            name: (namespace_identifier) @name) @kind.namespace

        (type_definition
            declarator: (type_identifier) @name) @kind.type_alias
        "#,
    },
    imports_refs: {
        static: CPP_IMPORT_REF_QUERY,
        accessor: cpp_import_ref_query,
        import: C_FAMILY_IMPORT_QUERY_STR,
        reference: C_FAMILY_REFERENCE_QUERY_STR,
    },
    import_statement_kinds: ["preproc_include"],
    class_like_parents: ["class_specifier", "struct_specifier"],
    ancestors: [],
    visibility_from_name: None,
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment: Some(crate::language::C_LIKE_DOC_COMMENT),
    fixture: "tests/fixtures/cpp/classes.cpp",
    snapshot: cpp_insta_snapshot,
    tests {
            use crate::model::SymbolKind;


            #[test]
            fn extract_class() {
                let src = b"class Foo { public: void bar() {} };";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
                assert!(matches!(foo.kind, SymbolKind::Class));
            }

            #[test]
            fn extract_method() {
                let src = b"class Foo { public: void bar() {} };";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
                assert!(matches!(bar.kind, SymbolKind::Method));
            }

            #[test]
            fn extract_namespace() {
                let src = b"namespace math { int add(int a, int b) { return a + b; } }";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                let ns = symbols.iter().find(|s| s.name == "math").unwrap();
                assert!(matches!(ns.kind, SymbolKind::Namespace));
            }

            #[test]
            fn cpp_docstring_extraction() {
                let src = b"/** C++ doc comment. */\nint documented() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                let func = symbols.iter().find(|s| s.name == "documented").unwrap();
                assert!(func.docstring.is_some(), "documented should have docstring");
                assert!(func.docstring.as_ref().unwrap().contains("C++ doc comment"));
            }

            #[test]
            fn extract_class_member_function_declaration() {
                let src = b"class C { void m(); };";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                let found = symbols.iter().find(|s| s.name == "m");
                assert!(found.is_some(), "missing m: {symbols:?}");
                assert_eq!(format!("{:?}", found.unwrap().kind), "Declaration");
            }

            #[test]
            fn qualified_definition_uses_the_plain_name() {
                let src = b"void C::m() {}";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                let names: Vec<&str> = symbols.iter().map(|s| s.name.as_ref()).collect();
                assert!(names.contains(&"m"), "names: {names:?}");
                assert!(!names.contains(&"C::m"), "names: {names:?}");
            }

            #[test]
            fn pointer_returning_definition_is_captured() {
                let src = b"void *alloc(int n) { return 0; }";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                assert!(symbols.iter().any(|s| s.name == "alloc"), "{symbols:?}");
            }

            #[test]
            fn reference_returning_definition_is_captured() {
                let src = b"int &ref(int n) { return n; }";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                assert!(symbols.iter().any(|s| s.name == "ref"), "{symbols:?}");
            }

            #[test]
            fn data_member_is_not_a_symbol() {
                let src = b"class C { int x; };";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
                assert!(!symbols.iter().any(|s| s.name == "x"));
            }
    },
);
