//! Language system: compile-time enum dispatch over 9 language packs.
//!
//! `LangId` enum selects a `LanguageSpec` via exhaustive `match`.
//! Each `LanguageSpec` bundles a grammar constructor, tree-sitter query
//! constructors (symbols, imports, references), and language-specific
//! extraction heuristics.

pub(crate) mod c;
pub(crate) mod common;
pub(crate) mod cpp;
#[cfg(feature = "dataflow")]
pub(crate) mod dataflow;
pub(crate) mod go;
pub mod import_resolver;
pub(crate) mod javascript;
pub(crate) mod pack;
pub(crate) mod python;
pub(crate) mod ruby;
pub(crate) mod rust;
pub(crate) mod tsx;
pub(crate) mod typescript;

use serde::{Deserialize, Serialize};
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

/// Shared JSDoc/C-style doc comment config.
///
/// Used by JS, TS, TSX, C, C++ specs. Only Rust, Go, Ruby, Python differ.
pub const C_LIKE_DOC_COMMENT: DocCommentConfig = DocCommentConfig {
    line_prefixes: &["//"],
    block_open: Some("/**"),
    block_close: "*/",
    strip_continuation_marker: true,
};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::Display,
    strum::AsRefStr,
    strum::EnumString,
)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
#[repr(usize)]
pub enum LangId {
    Python,
    #[strum(serialize = "javascript")]
    #[serde(rename = "javascript")]
    JavaScript,
    #[strum(serialize = "typescript")]
    #[serde(rename = "typescript")]
    TypeScript,
    Tsx,
    C,
    Cpp,
    Rust,
    Go,
    Ruby,
}

impl LangId {
    pub const COUNT: usize = 9;

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
            LangId::Ruby,
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
    pub query_fn: fn() -> Result<&'static Query, crate::error::Error>,
    pub import_path_resolver: fn(
        raw: &str,
        source_dir: &std::path::Path,
        project_root: &std::path::Path,
    ) -> Option<std::path::PathBuf>,
    pub import_ref_query_fn: fn() -> Result<&'static Query, crate::error::Error>,
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
        LangId::Ruby => &ruby::RUBY_SPEC,
    }
}

pub fn grammar_for(id: LangId) -> tree_sitter::Language {
    (spec_for(id).grammar_fn)()
}

/// Eagerly initialize all language query statics.
///
/// Call at startup so a broken query is reported before the first file. A
/// failure is logged, not fatal: the per-file path turns it into a diagnostic.
pub fn validate_queries() {
    for id in LangId::all() {
        let spec = spec_for(id);
        for (label, outcome) in [
            ("symbols", (spec.query_fn)()),
            ("imports and references", (spec.import_ref_query_fn)()),
        ] {
            if let Err(error) = outcome {
                tracing::error!(language = ?id, query = label, %error, "query failed to compile");
            }
        }
    }
}

/// Extract symbols, reporting a query failure as an empty result.
///
/// The analysis path uses [`extract_symbols_for_checked`] so the failure
/// reaches the file diagnostics; this wrapper serves callers that only need
/// the symbols.
pub fn extract_symbols_for<'a>(
    id: LangId,
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
) -> Vec<RawSymbol<'a>> {
    match extract_symbols_for_checked(id, tree, source) {
        Ok(symbols) => symbols,
        Err(error) => {
            tracing::error!(language = ?id, %error, "symbol extraction skipped");
            Vec::new()
        }
    }
}

pub fn extract_symbols_for_checked<'a>(
    id: LangId,
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
) -> Result<Vec<RawSymbol<'a>>, crate::error::Error> {
    common::extract_with_spec(tree, source, spec_for(id))
}

/// Extract imports and references, reporting a query failure as empty results.
pub fn extract_imports_and_references_for<'a>(
    id: LangId,
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    file_path: &std::path::Path,
) -> (
    Vec<crate::model::UnresolvedImport>,
    Vec<crate::model::UnresolvedReference>,
    Vec<crate::error::Diagnostic>,
) {
    match extract_imports_and_references_for_checked(id, tree, source, file_path) {
        Ok(extracted) => extracted,
        Err(error) => {
            tracing::error!(language = ?id, %error, "import and reference extraction skipped");
            (Vec::new(), Vec::new(), Vec::new())
        }
    }
}

pub fn extract_imports_and_references_for_checked<'a>(
    id: LangId,
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    file_path: &std::path::Path,
) -> Result<
    (
        Vec<crate::model::UnresolvedImport>,
        Vec<crate::model::UnresolvedReference>,
        Vec<crate::error::Diagnostic>,
    ),
    crate::error::Error,
> {
    common::extract_imports_and_references_with_spec(tree, source, spec_for(id), file_path)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

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
        assert_eq!(format!("{}", LangId::Ruby), "ruby");
        assert_eq!(format!("{}", LangId::JavaScript), "javascript");
        assert_eq!(format!("{}", LangId::TypeScript), "typescript");
    }

    #[test]
    fn lang_id_vocabulary_is_canonical() {
        let expected = [
            "python",
            "javascript",
            "typescript",
            "tsx",
            "c",
            "cpp",
            "rust",
            "go",
            "ruby",
        ];
        let all = LangId::all();
        let actual: Vec<&str> = all.iter().map(|l| l.as_ref()).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn lang_id_from_str_round_trip() {
        for id in LangId::all() {
            let name: &str = id.as_ref();
            let parsed = LangId::from_str(name)
                .unwrap_or_else(|e| panic!("from_str failed for documented name {name:?}: {e}"));
            assert_eq!(parsed, id);
        }
        assert_eq!(LangId::from_str("typescript"), Ok(LangId::TypeScript));
        assert_eq!(LangId::from_str("javascript"), Ok(LangId::JavaScript));
    }

    #[test]
    fn lang_id_serde_snake_case() {
        let json = serde_json::to_string(&LangId::Python).unwrap();
        assert_eq!(json, "\"python\"");

        let json = serde_json::to_string(&LangId::Ruby).unwrap();
        assert_eq!(json, "\"ruby\"");

        let json = serde_json::to_string(&LangId::JavaScript).unwrap();
        assert_eq!(json, "\"javascript\"");

        let json = serde_json::to_string(&LangId::TypeScript).unwrap();
        assert_eq!(json, "\"typescript\"");
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
        assert_eq!(LangId::COUNT, 9);
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
        assert_eq!(LangId::Cpp.metacall_tag(), "c");
        assert_eq!(LangId::Rust.metacall_tag(), "rs");
        assert_eq!(LangId::Go.metacall_tag(), "go");
        assert_eq!(LangId::Ruby.metacall_tag(), "rb");
    }

    /// The pack declarations keep this surface when a macro owns them:
    /// extensions, visibility defaults, doc comment rules and the node kinds
    /// each extraction step keys on.
    #[test]
    fn specs_keep_their_documented_surface() {
        struct Expected {
            extensions: &'static [&'static str],
            default_visibility: DefaultVisibility,
            doc_prefixes: Option<&'static [&'static str]>,
            doc_block_open: Option<&'static str>,
            class_like_parents: &'static [&'static str],
            ancestor_rules: usize,
            import_kinds: &'static [&'static str],
            visibility_from_name: bool,
        }

        let public = DefaultVisibility::PublicByDefault;
        let private = DefaultVisibility::PrivateByDefault;
        let cases: [(LangId, Expected); 9] = [
            (
                LangId::Python,
                Expected {
                    extensions: &["py", "pyi"],
                    default_visibility: public,
                    doc_prefixes: None,
                    doc_block_open: None,
                    class_like_parents: &["class_definition"],
                    ancestor_rules: 0,
                    import_kinds: &["import_statement", "import_from_statement"],
                    visibility_from_name: false,
                },
            ),
            (
                LangId::JavaScript,
                Expected {
                    extensions: &["js", "mjs", "cjs"],
                    default_visibility: private,
                    doc_prefixes: Some(&["//"]),
                    doc_block_open: Some("/**"),
                    class_like_parents: &["class_declaration", "class"],
                    ancestor_rules: 1,
                    import_kinds: &["import_statement"],
                    visibility_from_name: false,
                },
            ),
            (
                LangId::TypeScript,
                Expected {
                    extensions: &["ts", "cts", "mts"],
                    default_visibility: private,
                    doc_prefixes: Some(&["//"]),
                    doc_block_open: Some("/**"),
                    class_like_parents: &["class_declaration", "class"],
                    ancestor_rules: 1,
                    import_kinds: &["import_statement"],
                    visibility_from_name: false,
                },
            ),
            (
                LangId::Tsx,
                Expected {
                    extensions: &["tsx"],
                    default_visibility: private,
                    doc_prefixes: Some(&["//"]),
                    doc_block_open: Some("/**"),
                    class_like_parents: &["class_declaration", "class"],
                    ancestor_rules: 1,
                    import_kinds: &["import_statement"],
                    visibility_from_name: false,
                },
            ),
            (
                LangId::C,
                Expected {
                    extensions: &["c", "h"],
                    default_visibility: public,
                    doc_prefixes: Some(&["//"]),
                    doc_block_open: Some("/**"),
                    class_like_parents: &[],
                    ancestor_rules: 0,
                    import_kinds: &["preproc_include"],
                    visibility_from_name: false,
                },
            ),
            (
                LangId::Cpp,
                Expected {
                    extensions: &["cc", "cpp", "cxx", "hpp"],
                    default_visibility: private,
                    doc_prefixes: Some(&["//"]),
                    doc_block_open: Some("/**"),
                    class_like_parents: &["class_specifier", "struct_specifier"],
                    ancestor_rules: 0,
                    import_kinds: &["preproc_include"],
                    visibility_from_name: false,
                },
            ),
            (
                LangId::Rust,
                Expected {
                    extensions: &["rs"],
                    default_visibility: private,
                    doc_prefixes: Some(&["///", "//!"]),
                    doc_block_open: Some("/**"),
                    class_like_parents: &["impl_item"],
                    ancestor_rules: 0,
                    import_kinds: &["use_declaration"],
                    visibility_from_name: false,
                },
            ),
            (
                LangId::Go,
                Expected {
                    extensions: &["go"],
                    default_visibility: private,
                    doc_prefixes: Some(&["//"]),
                    doc_block_open: None,
                    class_like_parents: &[],
                    ancestor_rules: 0,
                    import_kinds: &["import_declaration"],
                    visibility_from_name: true,
                },
            ),
            (
                LangId::Ruby,
                Expected {
                    extensions: &["rb", "gemspec"],
                    default_visibility: public,
                    doc_prefixes: Some(&["#"]),
                    doc_block_open: None,
                    class_like_parents: &["class", "module"],
                    ancestor_rules: 0,
                    import_kinds: &[],
                    visibility_from_name: false,
                },
            ),
        ];

        for (lang, want) in cases {
            let spec = spec_for(lang);
            assert_eq!(spec.extensions, want.extensions, "{lang:?} extensions");
            assert_eq!(
                spec.default_visibility, want.default_visibility,
                "{lang:?} default visibility"
            );
            assert_eq!(
                spec.class_like_parents, want.class_like_parents,
                "{lang:?} class-like parents"
            );
            assert_eq!(
                spec.ancestor_visibility_rules.len(),
                want.ancestor_rules,
                "{lang:?} ancestor visibility rules"
            );
            assert_eq!(
                spec.import_statement_kinds, want.import_kinds,
                "{lang:?} import statement kinds"
            );
            assert_eq!(
                spec.visibility_from_name.is_some(),
                want.visibility_from_name,
                "{lang:?} name-based visibility"
            );
            match (spec.doc_comment_config.as_ref(), want.doc_prefixes) {
                (None, None) => {}
                (Some(config), Some(prefixes)) => {
                    assert_eq!(config.line_prefixes, prefixes, "{lang:?} doc prefixes");
                    assert_eq!(
                        config.block_open, want.doc_block_open,
                        "{lang:?} doc block opener"
                    );
                }
                (config, prefixes) => panic!(
                    "{lang:?} doc comment configuration mismatch: {config:?} against {prefixes:?}"
                ),
            }
        }

        let rule = spec_for(LangId::Go).visibility_from_name;
        assert!(rule.is_some(), "Go derives visibility from the name");
        let rule = rule.unwrap();
        assert_eq!(rule("Exported"), Some(Visibility::Public));
        assert_eq!(rule("hidden"), None);
    }
}
