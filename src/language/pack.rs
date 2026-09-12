//! One macro that declares a language pack.
//!
//! Nine packs repeat the same wiring: a compiled symbol query, a compiled
//! import and reference query, an accessor for each, the `LanguageSpec`
//! literal and a test module that parses one fixture and snapshots the
//! extracted symbols. The macro owns that wiring, so a pack only carries what
//! differs: its grammar, its query text, its resolver and its extraction rules.
//!
//! Invariant: the generated test module keeps the module path and the test
//! name it is given. The `insta` snapshot file that pins a pack's extraction
//! output is keyed by both, so renaming either moves the snapshot.
//!
//! The generated registry is fallible. A query is compiled once into a
//! `Result`, the accessors hand out a `Result`, and a pack that declares a
//! dataflow query gets an accessor that logs a compile failure once and
//! returns `None`, because dataflow extraction has no diagnostic channel.

/// Declares the queries, the spec and the test module of one language pack.
///
/// `symbols.query` holds the symbol query text and `imports_refs` names the
/// import and reference query texts that are joined into one compiled query.
/// `fixture` and `snapshot` name the pack's snapshot test; the optional
/// `tests` group carries the pack's own tests, which land inside the generated
/// test module next to the snapshot test.
macro_rules! define_language_pack {
    (
        $(#[$spec_doc:meta])*
        spec: $spec:ident,
        label: $label:literal,
        lang: $lang:expr,
        grammar: $grammar:path,
        extensions: [$($extension:literal),* $(,)?],
        resolver: $resolver:path,
        symbols: {
            static: $symbols_static:ident,
            accessor: $symbols_accessor:ident,
            query: $symbols_query:expr $(,)?
        },
        imports_refs: {
            static: $imports_refs_static:ident,
            accessor: $imports_refs_accessor:ident,
            import: $import_query:expr,
            reference: $reference_query:expr $(,)?
        },
        $(dataflow: {
            static: $dataflow_static:ident,
            accessor: $dataflow_accessor:ident,
            query: $dataflow_query:expr $(,)?
        },)?
        import_statement_kinds: [$($import_kind:literal),* $(,)?],
        class_like_parents: [$($class_like_parent:literal),* $(,)?],
        ancestors: [$($ancestor:expr),* $(,)?],
        visibility_from_name: $visibility_from_name:expr,
        default_visibility: $default_visibility:expr,
        doc_comment: $doc_comment:expr,
        fixture: $fixture:literal,
        snapshot: $snapshot:ident
        $(, tests { $($pack_tests:tt)* })?
        $(,)?
    ) => {
        static $symbols_static: ::std::sync::LazyLock<
            ::std::result::Result<::tree_sitter::Query, ::std::string::String>,
        > = ::std::sync::LazyLock::new(|| {
            $crate::language::common::compile_query_checked(&$grammar.into(), $symbols_query, $label)
        });

        fn $symbols_accessor() -> ::std::result::Result<
            &'static ::tree_sitter::Query,
            $crate::error::Error,
        > {
            $crate::language::common::query_from(&$symbols_static, $lang)
        }

        static $imports_refs_static: ::std::sync::LazyLock<
            ::std::result::Result<::tree_sitter::Query, ::std::string::String>,
        > = ::std::sync::LazyLock::new(|| {
            $crate::language::common::compile_query_checked(
                &$grammar.into(),
                &::std::format!("{}\n{}", $import_query, $reference_query),
                ::std::concat!($label, " combined import+ref"),
            )
        });

        fn $imports_refs_accessor() -> ::std::result::Result<
            &'static ::tree_sitter::Query,
            $crate::error::Error,
        > {
            $crate::language::common::query_from(&$imports_refs_static, $lang)
        }

        $(
            #[cfg(feature = "dataflow")]
            static $dataflow_static: ::std::sync::LazyLock<
                ::std::result::Result<::tree_sitter::Query, ::std::string::String>,
            > = ::std::sync::LazyLock::new(|| {
                $crate::language::common::compile_query_checked(
                    &$grammar.into(),
                    $dataflow_query,
                    ::std::concat!($label, " dataflow"),
                )
            });

            /// Resolve the dataflow query, reporting a compile failure once.
            ///
            /// Dataflow extraction has no diagnostic channel, so a failed query
            /// yields no dataflow result for the file and an error log.
            #[cfg(feature = "dataflow")]
            fn $dataflow_accessor() -> ::std::option::Option<&'static ::tree_sitter::Query> {
                static CACHED: ::std::sync::OnceLock<
                    ::std::result::Result<&'static ::tree_sitter::Query, ()>,
                > = ::std::sync::OnceLock::new();
                let resolved = CACHED.get_or_init(|| {
                    $crate::language::common::query_from(&$dataflow_static, $lang).map_err(|error| {
                        ::tracing::error!(language = ?$lang, %error, "dataflow query failed to compile");
                    })
                });
                (*resolved).ok()
            }
        )?

        $(#[$spec_doc])*
        pub(crate) const $spec: $crate::language::LanguageSpec = $crate::language::LanguageSpec {
            extensions: &[$($extension),*],
            grammar_fn: || $grammar.into(),
            query_fn: $symbols_accessor,
            import_path_resolver: $resolver,
            import_ref_query_fn: $imports_refs_accessor,
            class_like_parents: &[$($class_like_parent),*],
            ancestor_visibility_rules: &[$($ancestor),*],
            visibility_from_name: $visibility_from_name,
            import_statement_kinds: &[$($import_kind),*],
            default_visibility: $default_visibility,
            doc_comment_config: $doc_comment,
        };

        #[cfg(test)]
        mod tests {
            use $crate::language::{LangId, extract_symbols_for, grammar_for};

            fn parse(source: &[u8]) -> ::tree_sitter::Tree {
                let mut parser = ::tree_sitter::Parser::new();
                parser.set_language(&grammar_for($lang)).unwrap();
                parser.parse(source, None).unwrap()
            }

            #[test]
            fn $snapshot() {
                let source = ::std::fs::read_to_string(
                    ::std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join($fixture),
                )
                .unwrap();
                let tree = parse(source.as_bytes());
                let symbols = extract_symbols_for($lang, &tree, source.as_bytes());
                insta::assert_json_snapshot!(symbols);
            }

            $($($pack_tests)*)?
        }
    };
}

pub(crate) use define_language_pack;
