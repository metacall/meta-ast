//! Tree-sitter parser lifecycle and parse quality metrics.
//!
//! Maintains a thread-local pool of `Parser` instances (one per
//! language) to avoid re-initializing grammars. Provides `parse_tree`
//! for single-file parsing and `error_ratio` for parse quality estimation.

use std::cell::RefCell;

use tree_sitter::Parser;

use crate::error::Error;
use crate::language::LangId;

thread_local! {
    static PARSERS: RefCell<[Option<Parser>; LangId::COUNT]> = const { RefCell::new([const { None }; LangId::COUNT]) };
}

fn get_or_init_parser(
    parsers: &mut [Option<Parser>; LangId::COUNT],
    lang: LangId,
) -> Result<&mut Parser, Error> {
    let idx = lang as usize;
    if parsers[idx].is_none() {
        let mut parser = Parser::new();
        let grammar = crate::language::grammar_for(lang);
        parser
            .set_language(&grammar)
            .map_err(|e| Error::Config(format!("failed to set language: {e}")))?;
        parsers[idx] = Some(parser);
    }
    parsers[idx]
        .as_mut()
        .ok_or_else(|| Error::Config("parser slot was not initialized".into()))
}

pub(crate) fn parse_tree(lang: LangId, source: &[u8]) -> Result<tree_sitter::Tree, Error> {
    PARSERS.with(|cache| {
        let parsers = &mut *cache.borrow_mut();
        let parser = get_or_init_parser(parsers, lang)?;
        parser.parse(source, None).ok_or_else(|| Error::Parse {
            path: Default::default(),
            message: "parser returned no tree".into(),
        })
    })
}

/// Unified metrics extracted in a single AST traversal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeMetrics {
    pub error_ratio: f64,
    pub node_count: usize,
}

/// Calculate parse metrics (error ratio and named node count) in a single AST pass.
pub fn tree_metrics(tree: &tree_sitter::Tree, source: &[u8]) -> TreeMetrics {
    if source.is_empty() {
        return TreeMetrics {
            error_ratio: 0.0,
            node_count: 0,
        };
    }
    let mut cursor = tree.walk();
    let mut total = 0u32;
    let mut errors = 0u32;
    let mut named = 0u32;
    let mut reached_root = false;

    while !reached_root {
        let n = cursor.node();
        total += 1;
        if n.is_named() {
            named += 1;
        }
        if n.is_error() || n.is_missing() {
            errors += 1;
        }
        if cursor.goto_first_child() {
            continue;
        }
        if cursor.goto_next_sibling() {
            continue;
        }
        let mut retracing = true;
        while retracing {
            if !cursor.goto_parent() {
                reached_root = true;
                break;
            }
            if cursor.goto_next_sibling() {
                retracing = false;
            }
        }
    }

    let error_ratio = if total == 0 {
        0.0
    } else {
        errors as f64 / total as f64
    };

    TreeMetrics {
        error_ratio,
        node_count: named as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LangId;

    #[test]
    fn parse_tree_valid_python() {
        let tree = parse_tree(LangId::Python, b"def hello(): pass").unwrap();
        assert!(!tree.root_node().has_error());
        assert_eq!(tree.root_node().kind(), "module");
    }

    #[test]
    fn parse_tree_switches_languages() {
        let python = parse_tree(LangId::Python, b"def hello(): pass").unwrap();
        assert_eq!(python.root_node().kind(), "module");

        let javascript = parse_tree(LangId::JavaScript, b"function hello() {}").unwrap();
        assert_eq!(javascript.root_node().kind(), "program");
    }

    #[test]
    fn tree_metrics_valid_source() {
        let tree = parse_tree(LangId::Python, b"def hello(): pass").unwrap();
        let metrics = tree_metrics(&tree, b"def hello(): pass");
        assert!(metrics.error_ratio < 0.1);
        assert!(metrics.node_count > 0);
    }

    #[test]
    fn tree_metrics_malformed() {
        let tree = parse_tree(LangId::Python, b"def broken(").unwrap();
        let metrics = tree_metrics(&tree, b"def broken(");
        assert!(metrics.error_ratio > 0.0);
    }

    #[test]
    fn tree_metrics_empty_source() {
        let tree = parse_tree(LangId::Python, b"").unwrap();
        let metrics = tree_metrics(&tree, b"");
        assert_eq!(metrics.error_ratio, 0.0);
        assert_eq!(metrics.node_count, 0);
    }
}
