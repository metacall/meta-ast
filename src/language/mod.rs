//! Language system: compile-time enum dispatch over 8 language packs.
//!
//! `LangId` enum selects a `LanguageSpec` via exhaustive `match`.
//! Each `LanguageSpec` bundles a grammar constructor, tree-sitter query
//! constructors (symbols, imports, references), and language-specific
//! extraction heuristics.

pub(crate) mod c;
pub(crate) mod common;
pub(crate) mod cpp;
pub(crate) mod go;
pub mod import_resolver;
pub(crate) mod javascript;
pub(crate) mod python;
pub(crate) mod rust;
pub(crate) mod tsx;
pub(crate) mod typescript;

use serde::Serialize;
use tree_sitter::Query;

use crate::model::Visibility;

/// Configuration for extracting doc comments from source code.
///
/// Used for languages where doc comments are tree-sitter extras
/// (siblings of declarations, not children) and cannot be captured
/// inline in queries.
#[derive(Debug, Clone)]
pub struct DocCommentConfig {
    /// Line comment prefixes that indicate doc comments (e.g., `["///", "//!"]` for Rust).
    pub line_prefixes: &'static [&'static str],
    /// Block comment opening (e.g., `Some("/**")` for doxygen/JSDoc).
    pub block_open: Option<&'static str>,
    /// Block comment closing (e.g., `"*/"`).
    pub block_close: &'static str,
    /// Whether to strip leading `*` continuation markers in block comments.
    pub strip_continuation_marker: bool,
}

/// Default visibility assumed when a symbol declares no explicit modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DefaultVisibility {
    /// When visibility is None, treat the symbol as public (Python, C functions).
    PublicByDefault,
    /// When visibility is None, treat the symbol as private (Rust, JS, TS, Go, C++).
    PrivateByDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, strum::Display, strum::AsRefStr)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[repr(usize)]
pub enum LangId {
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    C,
    Cpp,
    Rust,
    Go,
}

impl LangId {
    pub const COUNT: usize = 8;

    pub fn all() -> [LangId; Self::COUNT] {
        [
            LangId::Python,
            LangId::JavaScript,
            LangId::TypeScript,
            LangId::Tsx,
            LangId::C,
            LangId::Cpp,
            LangId::Rust,
            LangId::Go,
        ]
    }

    pub fn spec(self) -> &'static LanguageSpec {
        spec_for(self)
    }

    #[cfg(feature = "metacall-deploy")]
    pub fn metacall_tag(self) -> &'static str {
        crate::deploy::tags::metacall_tag(self)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RawSymbol<'a> {
    pub name: std::borrow::Cow<'a, str>,
    pub kind: crate::model::SymbolKind,
    pub source_range: crate::model::SourceRange,
    pub visibility: Option<crate::model::Visibility>,
    pub signature: Option<std::borrow::Cow<'a, str>>,
    pub docstring: Option<std::borrow::Cow<'a, str>>,
    pub is_async: bool,
}

pub struct LanguageSpec {
    pub extensions: &'static [&'static str],
    pub grammar_fn: fn() -> tree_sitter::Language,
    pub query_fn: fn() -> &'static Query,
    pub import_path_resolver: fn(
        raw: &str,
        source_dir: &std::path::Path,
        project_root: &std::path::Path,
    ) -> Option<std::path::PathBuf>,
    pub import_ref_query_fn: fn() -> &'static Query,
    pub class_like_parents: &'static [&'static str],
    pub ancestor_visibility_rules: &'static [(&'static str, Visibility)],
    pub visibility_from_name: Option<fn(&str) -> Option<Visibility>>,
    pub import_statement_kinds: &'static [&'static str],
    pub default_visibility: DefaultVisibility,
    pub doc_comment_config: Option<DocCommentConfig>,
}

pub fn spec_for(id: LangId) -> &'static LanguageSpec {
    match id {
        LangId::Python => &python::PYTHON_SPEC,
        LangId::JavaScript => &javascript::JS_SPEC,
        LangId::TypeScript => &typescript::TS_SPEC,
        LangId::Tsx => &tsx::TSX_SPEC,
        LangId::C => &c::C_SPEC,
        LangId::Cpp => &cpp::CPP_SPEC,
        LangId::Rust => &rust::RUST_SPEC,
        LangId::Go => &go::GO_SPEC,
    }
}

pub fn grammar_for(id: LangId) -> tree_sitter::Language {
    (spec_for(id).grammar_fn)()
}

/// Eagerly initialize all language query statics.
/// Call at startup to fail fast on query compilation bugs.
pub fn validate_queries() {
    for id in LangId::all() {
        let _ = (spec_for(id).query_fn)();
        let _ = (spec_for(id).import_ref_query_fn)();
    }
}

pub fn extract_symbols_for<'a>(
    id: LangId,
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
) -> Vec<RawSymbol<'a>> {
    common::extract_with_spec(tree, source, spec_for(id))
}

pub fn extract_imports_and_references_for<'a>(
    id: LangId,
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    file_path: &std::path::Path,
) -> (
    Vec<crate::model::UnresolvedImport>,
    Vec<crate::model::UnresolvedReference>,
) {
    common::extract_imports_and_references_with_spec(tree, source, spec_for(id), file_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_id_all_variants_exist() {
        let variants = LangId::all();
        for i in 0..variants.len() {
            for j in (i + 1)..variants.len() {
                assert_ne!(variants[i], variants[j]);
            }
        }
    }

    #[test]
    fn lang_id_display() {
        assert_eq!(format!("{}", LangId::Python), "python");
    }

    #[test]
    fn lang_id_serde_snake_case() {
        let json = serde_json::to_string(&LangId::Python).unwrap();
        assert_eq!(json, "\"python\"");
    }

    #[test]
    fn grammar_for_all_variants() {
        for id in LangId::all() {
            let _lang = grammar_for(id);
        }
    }

    #[test]
    fn extract_symbols_for_python() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Python)).unwrap();
        let tree = parser.parse(b"def hello(): pass", None).unwrap();
        let symbols = extract_symbols_for(LangId::Python, &tree, b"def hello(): pass");
        assert!(!symbols.is_empty());
    }

    #[test]
    fn spec_for_returns_spec_with_matching_extensions() {
        let python_spec = spec_for(LangId::Python);
        assert!(python_spec.extensions.contains(&"py"));
        assert!(python_spec.extensions.contains(&"pyi"));

        let js_spec = spec_for(LangId::JavaScript);
        assert!(js_spec.extensions.contains(&"js"));
    }

    #[test]
    fn lang_id_count_matches_variant_count() {
        assert_eq!(LangId::COUNT, 8);
        assert_eq!(LangId::all().len(), LangId::COUNT);
    }

    #[test]
    fn all_specs_have_non_empty_extensions() {
        for id in LangId::all() {
            let spec = spec_for(id);
            assert!(
                !spec.extensions.is_empty(),
                "{id:?} spec has empty extensions"
            );
        }
    }

    #[test]
    fn no_duplicate_extensions_across_specs() {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        for id in LangId::all() {
            let spec = spec_for(id);
            for &ext in spec.extensions {
                assert!(
                    seen.insert(ext),
                    "extension {ext:?} appears in more than one language spec"
                );
            }
        }
    }

    #[test]
    fn grammar_fn_smoke_test_all_variants() {
        for id in LangId::all() {
            let spec = spec_for(id);
            let grammar = (spec.grammar_fn)();
            let mut parser = tree_sitter::Parser::new();
            assert!(
                parser.set_language(&grammar).is_ok(),
                "grammar_fn failed for {id:?}"
            );
        }
    }

    #[test]
    fn query_fn_smoke_test_all_variants() {
        for id in LangId::all() {
            let spec = spec_for(id);
            let _query = (spec.query_fn)();
        }
    }

    #[test]
    #[cfg(feature = "metacall-deploy")]
    fn test_lang_id_metacall_tag() {
        assert_eq!(LangId::Python.metacall_tag(), "py");
        assert_eq!(LangId::JavaScript.metacall_tag(), "node");
        assert_eq!(LangId::TypeScript.metacall_tag(), "ts");
        assert_eq!(LangId::Tsx.metacall_tag(), "ts");
        assert_eq!(LangId::C.metacall_tag(), "c");
        assert_eq!(LangId::Cpp.metacall_tag(), "cpp");
        assert_eq!(LangId::Rust.metacall_tag(), "rs");
        assert_eq!(LangId::Go.metacall_tag(), "go");
    }
}
