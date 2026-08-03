//! Ruby language pack: symbol, import, and reference extraction.
//!
//! Resolves Ruby symbols from `method`, `singleton_method`, `class`,
//! `module`, and constant `assignment` nodes. Resolves `require` and
//! `require_relative` call imports and bare-call references.
//!
//! `require` names resolve against the project `lib/` and `app/`
//! directories, then the source directory. A gem-like name that resolves
//! nowhere is returned as-is so the graph builder creates an external node.
//! `require_relative` names resolve against the source directory only.
//!
//! Documented limitations:
//! - No dataflow extraction. The Ruby stub lives in `dataflow.rs`.
//! - `=begin...=end` block comments are not treated as docstrings.
//! - `attr_reader`, `alias`, `include`, and `extend` are not handled.
//! - Extension-less `Gemfile` and `Rakefile` are not detected.

use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_ruby_candidate(path: PathBuf) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path);
    }
    if path.with_extension("rb").is_file() {
        return Some(path.with_extension("rb"));
    }
    if path.with_extension("so").is_file() {
        return Some(path.with_extension("so"));
    }
    None
}

fn resolve_ruby_import(raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if raw.starts_with('.') {
        return resolve_ruby_candidate(source_dir.join(raw));
    }

    for base in [
        project_root.join("lib"),
        project_root.join("app"),
        source_dir.to_path_buf(),
    ] {
        if let Some(path) = resolve_ruby_candidate(base.join(raw)) {
            return Some(path);
        }
    }

    Some(PathBuf::from(raw))
}

static RUBY_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_ruby::LANGUAGE.into(),
        r#"
(method
  name: (identifier) @name
  parameters: (method_parameters)? @signature
) @kind.function

(singleton_method
  name: (identifier) @name
  parameters: (method_parameters)? @signature
) @kind.method

(class
  name: (constant) @name
) @kind.class

(module
  name: (constant) @name
) @kind.module

(assignment
  left: (constant) @name
) @kind.constant
"#,
        "Ruby",
    )
});

const RUBY_IMPORT_QUERY_STR: &str = r#"
(call
  method: (identifier) @import.method
  arguments: (argument_list . (string) @import.path .)
  (#match? @import.method "^(require|require_relative)$"))
"#;

const RUBY_REFERENCE_QUERY_STR: &str = r#"
(call
  method: (identifier) @reference.name
  (#not-match? @reference.name "^(require|require_relative)$"))
(call
  receiver: (constant) @reference.name)
(call
  receiver: (scope_resolution) @reference.name)
(scope_resolution
  name: (constant) @reference.name)
"#;

static RUBY_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_ruby::LANGUAGE.into(),
        &format!("{}\n{}", RUBY_IMPORT_QUERY_STR, RUBY_REFERENCE_QUERY_STR),
        "Ruby combined import+ref",
    )
});

fn ruby_query() -> &'static tree_sitter::Query {
    &RUBY_QUERY
}

fn ruby_import_ref_query() -> &'static tree_sitter::Query {
    &RUBY_IMPORT_REF_QUERY
}

pub(crate) const RUBY_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["rb", "gemspec"],
    grammar_fn: || tree_sitter_ruby::LANGUAGE.into(),
    query_fn: ruby_query,
    import_path_resolver: resolve_ruby_import,
    import_ref_query_fn: ruby_import_ref_query,
    class_like_parents: &["class", "module"],
    ancestor_visibility_rules: &[],
    visibility_from_name: None,
    import_statement_kinds: &[],
    default_visibility: DefaultVisibility::PublicByDefault,
    doc_comment_config: Some(DocCommentConfig {
        line_prefixes: &["#"],
        block_open: None,
        block_close: "",
        strip_continuation_marker: false,
    }),
};

#[cfg(test)]
mod tests {
    use crate::language::{
        LangId, extract_imports_and_references_for, extract_symbols_for, grammar_for,
    };
    use crate::model::SymbolKind;
    use std::path::PathBuf;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Ruby)).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn ruby_grammar_loads() {
        let _ = grammar_for(LangId::Ruby);
    }

    #[test]
    fn extract_method_with_signature() {
        let src = b"def greet(name)\n  \"hi #{name}\"\nend";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Ruby, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
        assert!(symbols[0].signature.as_deref().unwrap().contains("(name)"));
    }

    #[test]
    fn extract_singleton_method() {
        let src = b"class Calc\n  def self.square(x)\n    x * x\n  end\nend";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Ruby, &tree, src);
        let square = symbols.iter().find(|s| s.name == "square").unwrap();
        assert!(matches!(square.kind, SymbolKind::Method));
    }

    #[test]
    fn extract_class_and_module() {
        let src = b"module Util\n  class Helper\n  end\nend";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Ruby, &tree, src);
        let util = symbols.iter().find(|s| s.name == "Util").unwrap();
        assert!(matches!(util.kind, SymbolKind::Module));
        let helper = symbols.iter().find(|s| s.name == "Helper").unwrap();
        assert!(matches!(helper.kind, SymbolKind::Class));
    }

    #[test]
    fn extract_method_promoted_inside_class() {
        let src = b"class Foo\n  def bar\n  end\nend";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Ruby, &tree, src);
        let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
        assert!(matches!(bar.kind, SymbolKind::Method));
    }

    #[test]
    fn extract_constant_assignment() {
        let src = b"MAX_SIZE = 1024";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Ruby, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "MAX_SIZE");
        assert!(matches!(symbols[0].kind, SymbolKind::Constant));
    }

    #[test]
    fn extract_docstring_from_comment() {
        let src = b"# Adds two numbers.\ndef add(a, b)\n  a + b\nend";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Ruby, &tree, src);
        let add = symbols.iter().find(|s| s.name == "add").unwrap();
        assert!(
            add.docstring
                .as_deref()
                .unwrap()
                .contains("Adds two numbers.")
        );
    }

    #[test]
    fn extract_require_import() {
        let src = b"require 'json'";
        let tree = parse(src);
        let (imports, _) =
            extract_imports_and_references_for(LangId::Ruby, &tree, src, &PathBuf::from("test.rb"));
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].import_specifier, "'json'");
        assert!(imports[0].symbol.is_none());
    }

    #[test]
    fn extract_require_relative_import() {
        let src = b"require_relative './helper'";
        let tree = parse(src);
        let (imports, _) =
            extract_imports_and_references_for(LangId::Ruby, &tree, src, &PathBuf::from("test.rb"));
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].import_specifier, "'./helper'");
    }

    #[test]
    fn require_calls_do_not_leak_as_references() {
        let src = b"require 'json'\nputs 'hi'";
        let tree = parse(src);
        let (imports, references) =
            extract_imports_and_references_for(LangId::Ruby, &tree, src, &PathBuf::from("test.rb"));
        assert_eq!(imports.len(), 1);
        assert!(references.iter().any(|r| r.name == "puts"));
        assert!(!references.iter().any(|r| r.name == "require"));
    }

    #[test]
    fn extract_bare_call_reference() {
        let src = b"calc_total(items)";
        let tree = parse(src);
        let (_, references) =
            extract_imports_and_references_for(LangId::Ruby, &tree, src, &PathBuf::from("test.rb"));
        assert!(references.iter().any(|r| r.name == "calc_total"));
    }

    #[test]
    fn extract_constant_receiver_reference() {
        let src = b"Math.sqrt(4)";
        let tree = parse(src);
        let (_, references) =
            extract_imports_and_references_for(LangId::Ruby, &tree, src, &PathBuf::from("test.rb"));
        assert!(references.iter().any(|r| r.name == "Math"));
        assert!(references.iter().any(|r| r.name == "sqrt"));
    }

    #[test]
    fn resolve_ruby_import_bare_gem_name() {
        let source_dir = PathBuf::from("/tmp/src");
        let project_root = PathBuf::from("/tmp/project");
        let result = super::resolve_ruby_import("json", &source_dir, &project_root);
        assert_eq!(result, Some(PathBuf::from("json")));
    }

    #[test]
    fn resolve_ruby_import_relative_joins_source_dir() {
        let source_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ruby");
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let result =
            super::resolve_ruby_import("./classes_and_modules", &source_dir, &project_root);
        assert_eq!(
            result,
            Some(
                source_dir
                    .join("./classes_and_modules")
                    .with_extension("rb")
            )
        );
    }

    #[test]
    fn ruby_insta_snapshot() {
        let src = std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/ruby/classes_and_modules.rb"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Ruby, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
