use crate::language::LanguageSpec;
use crate::model::Visibility;
use once_cell::sync::Lazy;

static JS_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_javascript::LANGUAGE.into(),
        r#"
(function_declaration
  "async"? @async
  name: (identifier) @name
  parameters: (formal_parameters) @signature
) @kind.function

(generator_function_declaration
  "async"? @async
  name: (identifier) @name
  parameters: (formal_parameters) @signature
) @kind.function

(class_declaration
  name: (identifier) @name
) @kind.class

(method_definition
  "async"? @async
  name: [
    (property_identifier)
    (identifier)
  ] @name
  parameters: (formal_parameters) @signature
) @kind.method

(export_statement
  [
    (function_declaration
      "async"? @async
      name: (identifier) @name
      parameters: (formal_parameters) @signature
    ) @kind.function
    (class_declaration
      name: (identifier) @name
    ) @kind.class
  ]
)
"#,
    )
    .expect("Failed to parse JavaScript query")
});

fn js_query() -> &'static tree_sitter::Query {
    &JS_QUERY
}

const JS_IMPORT_QUERY_STR: &str = r#"
(import_statement
  source: (string) @import.path)
(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @import.symbol
        alias: (identifier)? @import.alias))))
(import_statement
  (import_clause
    (identifier) @import.symbol))
"#;

static JS_IMPORT_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_javascript::LANGUAGE.into(),
        JS_IMPORT_QUERY_STR,
    )
    .expect("Failed to parse JavaScript import query")
});

const JS_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (member_expression
    property: (property_identifier) @reference.name))
"#;

static JS_REFERENCE_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_javascript::LANGUAGE.into(),
        JS_REFERENCE_QUERY_STR,
    )
    .expect("Failed to parse JavaScript reference query")
});

fn js_import_query() -> &'static tree_sitter::Query {
    &JS_IMPORT_QUERY
}
fn js_reference_query() -> &'static tree_sitter::Query {
    &JS_REFERENCE_QUERY
}

static JS_IMPORT_REF_QUERY: Lazy<tree_sitter::Query> = Lazy::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_javascript::LANGUAGE.into(),
        &format!("{}\n{}", JS_IMPORT_QUERY_STR, JS_REFERENCE_QUERY_STR),
    )
    .expect("Failed to parse JavaScript combined import+ref query")
});

fn js_import_ref_query() -> &'static tree_sitter::Query {
    &JS_IMPORT_REF_QUERY
}

pub const JS_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["js", "mjs", "cjs"],
    grammar_fn: || tree_sitter_javascript::LANGUAGE.into(),
    query_fn: js_query,
    import_query_fn: js_import_query,
    reference_query_fn: js_reference_query,
    import_ref_query_fn: js_import_ref_query,
    class_like_parents: &["class_declaration", "class"],
    ancestor_visibility_rules: &[("export_statement", Visibility::Public)],
};

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::{SymbolKind, Visibility};

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&grammar_for(LangId::JavaScript))
            .unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_function_declaration() {
        let src = b"function hello() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_async_function() {
        let src = b"async function fetch() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].is_async);
    }

    #[test]
    fn extract_class_and_methods() {
        let src = b"class Foo {\n  constructor() {}\n  bar() {}\n}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        let methods: Vec<_> = symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Method))
            .collect();
        assert_eq!(methods.len(), 2);
    }

    #[test]
    fn extract_exported_class() {
        let src = b"export class Foo { bar() {} }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(class.visibility, Some(Visibility::Public));
    }

    #[test]
    fn js_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/javascript/functions.js"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
