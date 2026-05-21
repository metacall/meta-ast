use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_cpp_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '<' || c == '>' || c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    let path = source_dir.join(raw);
    if path.extension().is_none() {
        Some(path.with_extension("h"))
    } else {
        Some(path)
    }
}

static CPP_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
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
        "C++",
    )
});

fn cpp_query() -> &'static tree_sitter::Query {
    &CPP_QUERY
}

const CPP_IMPORT_QUERY_STR: &str = r#"
(preproc_include
  path: (string_literal) @import.path)
(preproc_include
  path: (system_lib_string) @import.path)
"#;

const CPP_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (field_expression
    field: (field_identifier) @reference.name))
(call_expression
  function: (field_expression
    argument: (identifier) @reference.name))
"#;

static CPP_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_cpp::LANGUAGE.into(),
        &format!("{}\n{}", CPP_IMPORT_QUERY_STR, CPP_REFERENCE_QUERY_STR),
        "C++ combined import+ref",
    )
});

fn cpp_import_ref_query() -> &'static tree_sitter::Query {
    &CPP_IMPORT_REF_QUERY
}

pub(crate) const CPP_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["cc", "cpp", "cxx"],
    grammar_fn: || tree_sitter_cpp::LANGUAGE.into(),
    query_fn: cpp_query,
    import_path_resolver: resolve_cpp_import,
    import_ref_query_fn: cpp_import_ref_query,
    class_like_parents: &["class_specifier", "struct_specifier"],
    ancestor_visibility_rules: &[],
    visibility_from_name: None,
    import_statement_kinds: &["preproc_include"],
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment_config: Some(DocCommentConfig {
        line_prefixes: &["//"],
        block_open: Some("/**"),
        block_close: "*/",
        strip_continuation_marker: true,
    }),
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
    fn cpp_docstring_extraction() {
        let src = b"/** C++ doc comment. */\nint documented() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Cpp, &tree, src);
        let func = symbols.iter().find(|s| s.name == "documented").unwrap();
        assert!(func.docstring.is_some(), "documented should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("C++ doc comment"));
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
