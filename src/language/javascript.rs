use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use crate::model::Visibility;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_js_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if !raw.starts_with('.') && !raw.starts_with('/') {
        // Bare module name (e.g. 'jsonwebtoken', 'react'): return as-is
        // so the graph builder creates an ExternalNode for it.
        // Node.js resolution (node_modules) is not walked here.
        return Some(PathBuf::from(raw));
    }

    let base = if raw.starts_with('/') {
        PathBuf::from("/")
    } else {
        source_dir.to_path_buf()
    };

    let path = base.join(raw);

    let extensions = ["", ".js", ".ts", ".jsx", ".tsx", ".mjs", ".cjs"];
    for ext in &extensions {
        let candidate = if ext.is_empty() {
            path.clone()
        } else {
            path.with_extension(ext.trim_start_matches('.'))
        };
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    Some(path)
}

static JS_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
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
        "JavaScript",
    )
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
(import_statement
  (import_clause
    (namespace_import
      (identifier) @import.symbol)))
(call_expression
  function: (identifier) @call.name
  arguments: (arguments . (string) @import.path .)
  (#eq? @call.name "require"))
"#;

const JS_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (member_expression
    property: (property_identifier) @reference.name))
(call_expression
  function: (member_expression
    object: (identifier) @reference.name))
"#;

static JS_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_javascript::LANGUAGE.into(),
        &format!("{}\n{}", JS_IMPORT_QUERY_STR, JS_REFERENCE_QUERY_STR),
        "JavaScript combined import+ref",
    )
});

fn js_import_ref_query() -> &'static tree_sitter::Query {
    &JS_IMPORT_REF_QUERY
}

pub(crate) const JS_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["js", "mjs", "cjs"],
    grammar_fn: || tree_sitter_javascript::LANGUAGE.into(),
    query_fn: js_query,
    import_path_resolver: resolve_js_import,
    import_ref_query_fn: js_import_ref_query,
    class_like_parents: &["class_declaration", "class"],
    ancestor_visibility_rules: &[("export_statement", Visibility::Public)],
    visibility_from_name: None,
    import_statement_kinds: &["import_statement"],
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
    fn extract_named_imports() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import { foo, bar } from 'utils';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::JavaScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.js"),
        );
        let named: Vec<_> = imports.iter().filter(|i| i.symbol.is_some()).collect();
        assert_eq!(
            named.len(),
            2,
            "expected 2 named import records for foo and bar"
        );
        for imp in &named {
            assert_eq!(imp.import_specifier, "'utils'");
        }
        assert_eq!(named[0].symbol.as_deref(), Some("foo"));
        assert_eq!(named[1].symbol.as_deref(), Some("bar"));
    }

    #[test]
    fn extract_default_import() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import React from 'react';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::JavaScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.js"),
        );
        let named: Vec<_> = imports.iter().filter(|i| i.symbol.is_some()).collect();
        assert_eq!(named.len(), 1);
        assert_eq!(named[0].import_specifier, "'react'");
        assert_eq!(named[0].symbol.as_deref(), Some("React"));
    }

    #[test]
    fn extract_side_effect_import() {
        use crate::language::extract_imports_and_references_for;
        let src = b"import 'styles.css';";
        let tree = parse(src);
        let (imports, _) = extract_imports_and_references_for(
            LangId::JavaScript,
            &tree,
            src,
            &std::path::PathBuf::from("test.js"),
        );
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].import_specifier, "'styles.css'");
        assert!(imports[0].symbol.is_none());
    }

    #[test]
    fn js_docstring_extraction() {
        let src = b"/** JSDoc comment. */\nfunction documented() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::JavaScript, &tree, src);
        let func = symbols.iter().find(|s| s.name == "documented").unwrap();
        assert!(func.docstring.is_some(), "documented should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("JSDoc comment"));
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
