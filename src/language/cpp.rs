use crate::language::LanguageSpec;
use once_cell::sync::Lazy;

static CPP_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_cpp::LANGUAGE.into(),
        r#"
        (function_definition
          declarator: (function_declarator
            declarator: (_) @name
            parameters: (parameter_list) @signature
          )
        ) @kind.function

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
    )
    .expect("Failed to parse C++ query")
});

fn cpp_query() -> &'static tree_sitter::Query {
    &CPP_QUERY
}

static CPP_IMPORT_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_cpp::LANGUAGE.into(),
        r#"
(preproc_include
  path: (string_literal) @import.path)
(preproc_include
  path: (system_lib_string) @import.path)
"#,
    )
    .expect("Failed to parse C++ import query")
});

static CPP_REFERENCE_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_cpp::LANGUAGE.into(),
        r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (field_expression
    field: (field_identifier) @reference.name))
"#,
    )
    .expect("Failed to parse C++ reference query")
});

fn cpp_import_query() -> &'static tree_sitter::Query {
    &CPP_IMPORT_QUERY
}
fn cpp_reference_query() -> &'static tree_sitter::Query {
    &CPP_REFERENCE_QUERY
}

pub const CPP_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["cc", "cpp", "cxx"],
    grammar_fn: || tree_sitter_cpp::LANGUAGE.into(),
    query_fn: cpp_query,
    import_query_fn: cpp_import_query,
    reference_query_fn: cpp_reference_query,
    class_like_parents: &["class_specifier"],
    ancestor_visibility_rules: &[],
};

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Cpp)).unwrap();
        parser.parse(source, None).unwrap()
    }

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
    fn cpp_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/cpp/classes.cpp"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Cpp, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
