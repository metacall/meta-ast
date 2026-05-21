use crate::language::typescript::{
    TS_FAMILY_IMPORT_QUERY, TS_FAMILY_QUERY, TS_FAMILY_REFERENCE_QUERY,
};
use crate::language::{DefaultVisibility, DocCommentConfig, LanguageSpec};
use crate::model::Visibility;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_tsx_import(raw: &str, source_dir: &Path, _project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if !raw.starts_with('.') && !raw.starts_with('/') {
        return None;
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

static TSX_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        TS_FAMILY_QUERY,
        "TSX",
    )
});

static TSX_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        &format!("{}\n{}", TS_FAMILY_IMPORT_QUERY, TS_FAMILY_REFERENCE_QUERY),
        "TSX combined import+ref",
    )
});

fn tsx_import_ref_query() -> &'static tree_sitter::Query {
    &TSX_IMPORT_REF_QUERY
}

fn tsx_query() -> &'static tree_sitter::Query {
    &TSX_QUERY
}

pub(crate) const TSX_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["tsx"],
    grammar_fn: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    query_fn: tsx_query,
    import_path_resolver: resolve_tsx_import,
    import_ref_query_fn: tsx_import_ref_query,
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
        parser.set_language(&grammar_for(LangId::Tsx)).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn extract_tsx_function() {
        let src = b"function App(): JSX.Element { return <div/>; }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "App");
        assert!(matches!(symbols[0].kind, SymbolKind::Function));
    }

    #[test]
    fn extract_tsx_exported_class() {
        let src = b"export class Foo extends React.Component { render() { return <div/>; } }";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src);
        let class = symbols.iter().find(|s| s.name == "Foo").unwrap();
        assert!(matches!(class.kind, SymbolKind::Class));
        assert_eq!(class.visibility, Some(Visibility::Public));
    }

    #[test]
    fn tsx_docstring_extraction() {
        let src = b"/** Component doc. */\nfunction App() {}";
        let tree = parse(src);
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src);
        let func = symbols.iter().find(|s| s.name == "App").unwrap();
        assert!(func.docstring.is_some(), "App should have docstring");
        assert!(func.docstring.as_ref().unwrap().contains("Component doc"));
    }

    #[test]
    fn tsx_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tsx/components.tsx"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Tsx, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
    }
}
