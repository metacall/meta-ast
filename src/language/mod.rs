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
use tree_sitter::Tree;

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

pub trait LanguagePack: Clone + Send + Sync + 'static {
    fn grammar(&self) -> tree_sitter::Language;
    fn id(&self) -> &'static str;
    fn file_extensions(&self) -> &'static [&'static str];
    fn extract_symbols<'a>(
        &self,
        tree: &'a Tree,
        source: &'a [u8],
        cursor: &mut tree_sitter::TreeCursor<'a>,
    ) -> Vec<RawSymbol<'a>>;
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
            fn extract_symbols<'a>(
                &self,
                tree: &'a tree_sitter::Tree,
                source: &'a [u8],
                cursor: &mut tree_sitter::TreeCursor<'a>,
            ) -> Vec<RawSymbol<'a>> {
                $extract_fn(tree, source, cursor)
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

pub fn extract_symbols_for<'a>(
    id: LangId,
    tree: &'a Tree,
    source: &'a [u8],
    cursor: &mut tree_sitter::TreeCursor<'a>,
) -> Vec<RawSymbol<'a>> {
    match id {
        LangId::Python => python::Python.extract_symbols(tree, source, cursor),
        LangId::JavaScript => javascript::JavaScript.extract_symbols(tree, source, cursor),
        LangId::TypeScript => typescript::TypeScript.extract_symbols(tree, source, cursor),
        LangId::Tsx => tsx::Tsx.extract_symbols(tree, source, cursor),
        LangId::C => c::C.extract_symbols(tree, source, cursor),
        LangId::Cpp => cpp::Cpp.extract_symbols(tree, source, cursor),
        LangId::Rust => rust::Rust.extract_symbols(tree, source, cursor),
        LangId::Go => go::Go.extract_symbols(tree, source, cursor),
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
