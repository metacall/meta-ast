use crate::language::LanguageSpec;
use once_cell::sync::Lazy;

static GO_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
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

fn go_query() -> &'static tree_sitter::Query {
    &GO_QUERY
}

pub const GO_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["go"],
    grammar_fn: || tree_sitter_go::LANGUAGE.into(),
    query_fn: go_query,
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
