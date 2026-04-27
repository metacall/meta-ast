use crate::language::LanguageSpec;
use std::sync::LazyLock;

static GO_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_go::LANGUAGE.into(),
        r#"
(function_declaration
  name: (identifier) @name
  parameters: (parameter_list) @signature
) @kind.function

(method_declaration
  name: (field_identifier) @name
  parameters: (parameter_list) @signature
) @kind.method

(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (struct_type)
  )
) @kind.struct

(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: (interface_type)
  )
) @kind.interface

(type_declaration
  (type_spec
    name: (type_identifier) @name
    type: [
      (type_identifier)
      (pointer_type)
      (function_type)
      (array_type)
      (slice_type)
      (map_type)
      (channel_type)
    ]
  )
) @kind.type_alias

(const_spec
  name: (identifier) @name
) @kind.constant

(var_spec
  name: (identifier) @name
) @kind.object
"#,
    )
    .expect("Failed to parse Go query")
});

const GO_IMPORT_QUERY_STR: &str = r#"
(import_spec
  name: (_)? @import.alias
  path: (interpreted_string_literal) @import.path)
"#;

static GO_IMPORT_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(&tree_sitter_go::LANGUAGE.into(), GO_IMPORT_QUERY_STR)
        .expect("Failed to parse Go import query")
});

const GO_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (selector_expression
    field: (field_identifier) @reference.name))
"#;

static GO_REFERENCE_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(&tree_sitter_go::LANGUAGE.into(), GO_REFERENCE_QUERY_STR)
        .expect("Failed to parse Go reference query")
});

fn go_query() -> &'static tree_sitter::Query {
    &GO_QUERY
}

fn go_import_query() -> &'static tree_sitter::Query {
    &GO_IMPORT_QUERY
}

fn go_reference_query() -> &'static tree_sitter::Query {
    &GO_REFERENCE_QUERY
}

static GO_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_go::LANGUAGE.into(),
        &format!("{}\n{}", GO_IMPORT_QUERY_STR, GO_REFERENCE_QUERY_STR),
    )
    .expect("Failed to parse Go combined import+ref query")
});

fn go_import_ref_query() -> &'static tree_sitter::Query {
    &GO_IMPORT_REF_QUERY
}

pub const GO_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["go"],
    grammar_fn: || tree_sitter_go::LANGUAGE.into(),
    query_fn: go_query,
    import_query_fn: go_import_query,
    reference_query_fn: go_reference_query,
    import_ref_query_fn: go_import_ref_query,
    class_like_parents: &[],
    ancestor_visibility_rules: &[],
};

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Go)).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_function() {
        let src = b"package main\n\nfunc Hello() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Go, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_struct() {
        let src = b"package main\n\ntype Rect struct {\n\tWidth float64\n}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Go, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Rect");
        assert!(matches!(symbols[0].kind, SymbolKind::Struct));
    }

    #[test]
    fn extract_method_with_receiver() {
        let src = b"package main\n\nfunc (r *Rect) Area() float64 { return 0 }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Go, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Area");
        assert!(matches!(symbols[0].kind, SymbolKind::Method));
    }

    #[test]
    fn extract_import_no_alias() {
        use crate::language::extract_imports_and_references_for;
        let src = b"package main\n\nimport \"fmt\"\n";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::Go,
            &tree,
            src,
            &std::path::PathBuf::from("test.go"),
        );
        assert_eq!(
            imports.len(),
            1,
            "expected 1 import record for non-aliased import"
        );
        assert_eq!(imports[0].namespace, "\"fmt\"");
        assert!(imports[0].alias.is_none());
    }

    #[test]
    fn extract_aliased_import_no_duplicates() {
        use crate::language::extract_imports_and_references_for;
        let src = b"package main\n\nimport alias \"fmt\"\n";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::Go,
            &tree,
            src,
            &std::path::PathBuf::from("test.go"),
        );
        assert_eq!(
            imports.len(),
            1,
            "expected 1 import record, not 2 (CR-03 regression check)"
        );
        assert_eq!(imports[0].namespace, "\"fmt\"");
        assert_eq!(imports[0].alias.as_deref(), Some("alias"));
    }

    #[test]
    fn extract_multiple_named_imports_no_aliases() {
        use crate::language::extract_imports_and_references_for;
        let src = b"package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::Go,
            &tree,
            src,
            &std::path::PathBuf::from("test.go"),
        );
        assert_eq!(imports.len(), 2, "expected 2 import records for fmt and os");
        assert_eq!(imports[0].namespace, "\"fmt\"");
        assert_eq!(imports[1].namespace, "\"os\"");
    }

    #[test]
    fn go_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/go/methods.go"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Go, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
