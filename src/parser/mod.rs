use tree_sitter::Parser;

use crate::error::Error;
use crate::language::LangId;

pub struct ParsedFile {
    pub tree: tree_sitter::Tree,
    pub source: Vec<u8>,
}

pub fn parse(lang: LangId, source: &[u8]) -> Result<ParsedFile, Error> {
    let mut parser = Parser::new();
    let grammar = crate::language::grammar_for(lang);
    parser
        .set_language(&grammar)
        .map_err(|e| Error::Config(format!("failed to set language: {e}")))?;
    let tree = parser.parse(source, None).ok_or_else(|| Error::Parse {
        path: Default::default(),
        message: "parser returned no tree".into(),
    })?;
    Ok(ParsedFile {
        tree,
        source: source.to_vec(),
    })
}

pub fn error_ratio(tree: &tree_sitter::Tree, source: &[u8]) -> f64 {
    if source.is_empty() {
        return 0.0;
    }
    let root = tree.root_node();
    let mut total = 0u32;
    let mut errors = 0u32;
    count_nodes(&root, &mut total, &mut errors);
    if total == 0 {
        return 0.0;
    }
    errors as f64 / total as f64
}

fn count_nodes(node: &tree_sitter::Node, total: &mut u32, errors: &mut u32) {
    *total += 1;
    if node.is_error() || node.is_missing() {
        *errors += 1;
    }
    for child in node.children(&mut node.walk()) {
        count_nodes(&child, total, errors);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LangId;

    fn has_error_node(node: &tree_sitter::Node) -> bool {
        if node.is_error() {
            return true;
        }
        for child in node.children(&mut node.walk()) {
            if has_error_node(&child) {
                return true;
            }
        }
        false
    }

    #[test]
    fn parse_valid_python() {
        let result = parse(LangId::Python, b"def hello(): pass");
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn parse_malformed_python() {
        let result = parse(LangId::Python, b"def broken(");
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(has_error_node(&parsed.tree.root_node()));
    }

    #[test]
    fn parse_empty_source() {
        let result = parse(LangId::Python, b"");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_javascript() {
        let result = parse(LangId::JavaScript, b"function hello() {}");
        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(!parsed.tree.root_node().has_error());
    }

    #[test]
    fn error_ratio_valid_source() {
        let parsed = parse(LangId::Python, b"def hello(): pass").unwrap();
        let ratio = error_ratio(&parsed.tree, &parsed.source);
        assert!(ratio < 0.1);
    }

    #[test]
    fn error_ratio_malformed() {
        let parsed = parse(LangId::Python, b"def broken(").unwrap();
        let ratio = error_ratio(&parsed.tree, &parsed.source);
        assert!(ratio > 0.0);
    }

    #[test]
    fn error_ratio_empty_source() {
        let parsed = parse(LangId::Python, b"").unwrap();
        let ratio = error_ratio(&parsed.tree, &parsed.source);
        assert_eq!(ratio, 0.0);
    }

    #[test]
    fn tree_root_node_kind() {
        let parsed = parse(LangId::Python, b"def hello(): pass").unwrap();
        assert_eq!(parsed.tree.root_node().kind(), "module");
    }
}
