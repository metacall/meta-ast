use crate::language::LanguageSpec;
use crate::language::typescript::{
    TS_FAMILY_IMPORT_QUERY, TS_FAMILY_QUERY, TS_FAMILY_REFERENCE_QUERY,
};
use crate::model::Visibility;
use std::sync::LazyLock;

static TSX_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        TS_FAMILY_QUERY,
    )
    .expect("Failed to parse TSX query")
});

static TSX_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    tree_sitter::Query::new(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        &format!("{}\n{}", TS_FAMILY_IMPORT_QUERY, TS_FAMILY_REFERENCE_QUERY),
    )
    .expect("Failed to parse TSX combined import+ref query")
});

fn tsx_import_ref_query() -> &'static tree_sitter::Query {
    &TSX_IMPORT_REF_QUERY
}

fn tsx_query() -> &'static tree_sitter::Query {
    &TSX_QUERY
}

pub const TSX_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["tsx"],
    grammar_fn: || tree_sitter_typescript::LANGUAGE_TSX.into(),
    query_fn: tsx_query,
    import_ref_query_fn: tsx_import_ref_query,
    class_like_parents: &["class_declaration", "class"],
    ancestor_visibility_rules: &[("export_statement", Visibility::Public)],
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
