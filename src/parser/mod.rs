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

pub(crate) fn error_ratio(tree: &tree_sitter::Tree, source: &[u8]) -> f64 {
    if source.is_empty() {
        return 0.0;
    }
    let root = tree.root_node();
    if !root.has_error() {
        return 0.0;
    }
    let mut total = 0u32;
    let mut errors = 0u32;
    count_nodes(&root, &mut total, &mut errors);
    if total == 0 {
        return 0.0;
    }
    errors as f64 / total as f64
}

/// Count total tree-sitter nodes (named + anonymous) in a parse tree.
///
/// Walks the tree once, same O(n) pass as `error_ratio` but without the
/// error-tracking overhead. Useful as a proxy for computational surface
/// area in deployment metrics.
///
/// Iterates with a single reusable cursor instead of allocating a fresh
/// `TreeCursor` per node, which the recursive `node.children(&mut node.walk())`
/// form does for every node in the tree.
pub fn ast_node_count(tree: &tree_sitter::Tree) -> usize {
    let mut cursor = tree.walk();
    let mut total = 0u32;
    let mut reached_root = false;
    while !reached_root {
        if cursor.node().is_named() {
            total += 1;
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
    total as usize
}

fn count_nodes(node: &tree_sitter::Node, total: &mut u32, errors: &mut u32) {
    let mut cursor = node.walk();
    let mut reached_root = false;
    while !reached_root {
        let n = cursor.node();
        *total += 1;
        if n.is_error() || n.is_missing() {
            *errors += 1;
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
    fn error_ratio_valid_source() {
        let tree = parse_tree(LangId::Python, b"def hello(): pass").unwrap();
        let ratio = error_ratio(&tree, b"def hello(): pass");
        assert!(ratio < 0.1);
    }

    #[test]
    fn error_ratio_malformed() {
        let tree = parse_tree(LangId::Python, b"def broken(").unwrap();
        let ratio = error_ratio(&tree, b"def broken(");
        assert!(ratio > 0.0);
    }

    #[test]
    fn error_ratio_empty_source() {
        let tree = parse_tree(LangId::Python, b"").unwrap();
        let ratio = error_ratio(&tree, b"");
        assert_eq!(ratio, 0.0);
    }
}
