use crate::language::pack::define_language_pack;
use crate::language::{DefaultVisibility, DocCommentConfig};
use crate::model::Visibility;
use std::path::{Path, PathBuf};
use crate::language::LangId;

fn resolve_go_import(raw: &str, _source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
    use crate::language::import_resolver::{find_go_module, strip_import_quotes};
    let raw = strip_import_quotes(raw);
    if raw.is_empty() || raw.starts_with('.') {
        // Go modules reject relative imports, so there is no path to build.
        return None;
    }

    let (dir, module_name) = find_go_module(project_root)?;
    let relative = raw
        .strip_prefix(module_name.as_str())
        .filter(|rest| rest.starts_with('/'))?
        .trim_start_matches('/');
    if relative.is_empty() {
        // A package is a directory. One file cannot represent it.
        return None;
    }
    Some(dir.join(relative).with_extension("go"))
}

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

define_language_pack!(
    spec: GO_SPEC,
    label: "Go",
    lang: LangId::Go,
    grammar: tree_sitter_go::LANGUAGE,
    extensions: ["go"],
    resolver: resolve_go_import,
    symbols: {
        static: GO_QUERY,
        accessor: go_query,
        query: r#"
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
    },
    imports_refs: {
        static: GO_IMPORT_REF_QUERY,
        accessor: go_import_ref_query,
        import: GO_IMPORT_QUERY_STR,
        reference: GO_REFERENCE_QUERY_STR,
    },
    import_statement_kinds: ["import_declaration"],
    class_like_parents: [],
    ancestors: [],
    visibility_from_name: Some(|name| {
        name.starts_with(|c: char| c.is_uppercase())
            .then_some(Visibility::Public)
    }),
    default_visibility: DefaultVisibility::PrivateByDefault,
    doc_comment: Some(DocCommentConfig {
        line_prefixes: &["//"],
        block_open: None,
        block_close: "",
        strip_continuation_marker: false,
    }),
    fixture: "tests/fixtures/go/methods.go",
    snapshot: go_insta_snapshot,
    tests {
            use crate::model::SymbolKind;


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
                let (imports, _, _) = extract_imports_and_references_for(
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
                assert_eq!(imports[0].import_specifier, "fmt");
                assert!(imports[0].alias.is_none());
            }

            #[test]
            fn extract_aliased_import_no_duplicates() {
                use crate::language::extract_imports_and_references_for;
                let src = b"package main\n\nimport alias \"fmt\"\n";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
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
                assert_eq!(imports[0].import_specifier, "fmt");
                assert_eq!(imports[0].alias.as_deref(), Some("alias"));
            }

            #[test]
            fn extract_multiple_named_imports_no_aliases() {
                use crate::language::extract_imports_and_references_for;
                let src = b"package main\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
                    LangId::Go,
                    &tree,
                    src,
                    &std::path::PathBuf::from("test.go"),
                );
                assert_eq!(imports.len(), 2, "expected 2 import records for fmt and os");
                assert_eq!(imports[0].import_specifier, "fmt");
                assert_eq!(imports[1].import_specifier, "os");
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
    },
);
