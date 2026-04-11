pub mod c;
pub(crate) mod common;
pub mod cpp;
pub mod go;
pub mod javascript;
pub mod python;
pub mod rust;
pub mod tsx;
pub mod typescript;

use std::fmt;

use serde::Serialize;
use tree_sitter::Tree;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
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

impl fmt::Display for LangId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string(self).unwrap_or_else(|_| "\"unknown\"".to_string());
        write!(f, "{}", s.trim_matches('"'))
    }
}

pub trait LanguagePack: Clone + Send + Sync + 'static {
    fn grammar(&self) -> tree_sitter::Language;
    fn id(&self) -> &'static str;
    fn file_extensions(&self) -> &'static [&'static str];
    fn extract_symbols(&self, tree: &Tree, source: &[u8]) -> Vec<RawSymbol>;
}

#[derive(Debug, Clone, Serialize)]
pub struct RawSymbol {
    pub name: String,
    pub kind: crate::model::SymbolKind,
    pub source_range: crate::model::SourceRange,
    pub visibility: Option<crate::model::Visibility>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub is_async: bool,
}

macro_rules! impl_language {
    ($lang:ident, $grammar:expr, $extract_fn:expr, $extensions:expr) => {
        #[derive(Clone, Copy, Debug)]
        pub struct $lang;

        impl $crate::language::LanguagePack for $lang {
            fn grammar(&self) -> tree_sitter::Language {
                $grammar
            }
            fn id(&self) -> &'static str {
                stringify!($lang)
            }
            fn file_extensions(&self) -> &'static [&'static str] {
                $extensions
            }
            fn extract_symbols(&self, tree: &tree_sitter::Tree, source: &[u8]) -> Vec<RawSymbol> {
                $extract_fn(tree, source)
            }
        }
    };
}

pub(crate) use impl_language;

pub fn grammar_for(id: LangId) -> tree_sitter::Language {
    match id {
        LangId::Python => python::Python.grammar(),
        LangId::JavaScript => javascript::JavaScript.grammar(),
        LangId::TypeScript => typescript::TypeScript.grammar(),
        LangId::Tsx => tsx::Tsx.grammar(),
        LangId::C => c::C.grammar(),
        LangId::Cpp => cpp::Cpp.grammar(),
        LangId::Rust => rust::Rust.grammar(),
        LangId::Go => go::Go.grammar(),
    }
}

pub fn extract_symbols_for(id: LangId, tree: &Tree, source: &[u8]) -> Vec<RawSymbol> {
    match id {
        LangId::Python => python::Python.extract_symbols(tree, source),
        LangId::JavaScript => javascript::JavaScript.extract_symbols(tree, source),
        LangId::TypeScript => typescript::TypeScript.extract_symbols(tree, source),
        LangId::Tsx => tsx::Tsx.extract_symbols(tree, source),
        LangId::C => c::C.extract_symbols(tree, source),
        LangId::Cpp => cpp::Cpp.extract_symbols(tree, source),
        LangId::Rust => rust::Rust.extract_symbols(tree, source),
        LangId::Go => go::Go.extract_symbols(tree, source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_id_all_variants_exist() {
        let variants = [
            LangId::Python,
            LangId::JavaScript,
            LangId::TypeScript,
            LangId::Tsx,
            LangId::C,
            LangId::Cpp,
            LangId::Rust,
            LangId::Go,
        ];
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
        for id in [
            LangId::Python,
            LangId::JavaScript,
            LangId::TypeScript,
            LangId::Tsx,
            LangId::C,
            LangId::Cpp,
            LangId::Rust,
            LangId::Go,
        ] {
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
}
