//! Language system: compile-time enum dispatch over 8 language packs.
//!
//! `LangId` enum selects a `LanguageSpec` via exhaustive `match`.
//! Each `LanguageSpec` bundles a grammar constructor, tree-sitter query
//! constructors (symbols, imports, references), and language-specific
//! extraction heuristics.

pub mod c;
pub(crate) mod common;
pub mod cpp;
pub mod go;
pub mod javascript;
pub mod python;
pub mod rust;
pub mod tsx;
pub mod typescript;

use serde::Serialize;
use tree_sitter::Query;

use crate::model::Visibility;

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
    // TODO(MVP): Add import_path_resolver: fn(&str, &Path) -> Option<PathBuf>
    //             to LanguageSpec for per-language path normalization logic.
    pub import_ref_query_fn: fn() -> &'static Query,
    pub class_like_parents: &'static [&'static str],
    pub ancestor_visibility_rules: &'static [(&'static str, Visibility)],
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
}
