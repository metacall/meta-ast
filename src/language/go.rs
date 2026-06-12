use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use crate::model::Visibility;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_go_import(raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if let Some(relative) = raw.strip_prefix('.') {
        let path = source_dir.join(relative);
        return Some(path.with_extension("go"));
    }

    let mut current = Some(project_root);
    while let Some(dir) = current {
        let go_mod = dir.join("go.mod");
        if go_mod.is_file() {
            if let Ok(content) = std::fs::read_to_string(&go_mod) {
                for line in content.lines() {
                    let line = line.trim();
                    if let Some(module) = line.strip_prefix("module ") {
                        let module_name = module.trim().to_string();
                        if raw.starts_with(&module_name) {
                            let relative = raw[module_name.len()..].trim_start_matches('/');
                            return Some(dir.join(relative).with_extension("go"));
                        }
                    }
                }
            }
            break;
        }
        current = dir.parent();
    }

    None
}

static GO_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
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
        "Go",
    )
});

const GO_IMPORT_QUERY_STR: &str = r#"
(import_spec
  name: (_)? @import.alias
  path:     (interpreted_string_literal) @import.path)
"#;

const GO_REFERENCE_QUERY_STR: &str = r#"
(call_expression
  function: (identifier) @reference.name)
(call_expression
  function: (selector_expression
    field: (field_identifier) @reference.name))
(call_expression
  function: (selector_expression
    operand: (identifier) @reference.name))
"#;

fn go_query() -> &'static tree_sitter::Query {
    &GO_QUERY
}

static GO_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_go::LANGUAGE.into(),
        &format!("{}\n{}", GO_IMPORT_QUERY_STR, GO_REFERENCE_QUERY_STR),
        "Go combined import+ref",
    )
});

fn go_import_ref_query() -> &'static tree_sitter::Query {
    &GO_IMPORT_REF_QUERY
}

pub(crate) const GO_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["go"],
    grammar_fn: || tree_sitter_go::LANGUAGE.into(),
    query_fn: go_query,
    import_path_resolver: resolve_go_import,
    import_ref_query_fn: go_import_ref_query,
    class_like_parents: &[],
    ancestor_visibility_rules: &[],
    visibility_from_name: Some(|name| {
        name.starts_with(|c: char| c.is_uppercase())
            .then_some(Visibility::Public)
    }),
    import_statement_kinds: &["import_declaration"],
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment_config: Some(DocCommentConfig {
        line_prefixes: &["//"],
        block_open: None,
        block_close: "",
        strip_continuation_marker: false,
    }),
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
        assert_eq!(imports[0].import_specifier, "\"fmt\"");
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
        assert_eq!(imports[0].import_specifier, "\"fmt\"");
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
        assert_eq!(imports[0].import_specifier, "\"fmt\"");
        assert_eq!(imports[1].import_specifier, "\"os\"");
    }

    #[test]
    fn go_docstring_extraction() {
        let src = b"package main\n\n// Godoc comment.\nfunc Documented() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Go, &tree, src);
        let func = symbols.iter().find(|s| s.name == "Documented").unwrap();
        assert!(func.docstring.is_some(), "Documented should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("Godoc comment"));
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
