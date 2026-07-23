//! Data/flow node extraction dispatcher.
//!
//! Extracts `DataNode` (value-bearing entities) and `FlowEdge`
//! (def-use transitions) from parse trees.
//!
//! Each language implements its own extraction in its respective module
//! (e.g. `rust::extract_rust_dataflow`). This module dispatches via
//! exhaustive `match` on `LangId` — no trait objects.
//!
//! ## Current status
//!
//! | Language   | Status       |
//! |------------|--------------|
//! | Rust       | Implemented  |
//! | Python     | Implemented  |
//! | JS/TS/TSX  | TODO         |
//! | Go         | TODO         |
//! | C/C++      | TODO         |
//!
//! ## Design
//!
//! Extraction follows a two-phase approach per file:
//! 1. **Definition capture**: tree-sitter queries identify `let` bindings,
//!    function parameters, and other binding constructs.
//! 2. **Usage matching**: identifier references in expression context are
//!    matched to the nearest preceding definition of the same name within
//!    the same function scope, creating `DefUse` flow edges.
//!
//! This is a conservative intra-procedural analysis. Inter-procedural
//! (cross-function) def-use and full type-based resolution are deferred.

use crate::language::LangId;
use crate::model::{DataNode, DataNodeId, FlowEdge, IdGenerator};

/// Extract data nodes and flow edges from a parse tree.
///
/// Language-specific dispatch. Returns empty vectors for languages
/// without dataflow extraction implemented yet.
pub fn extract_dataflow(
    lang: LangId,
    tree: &tree_sitter::Tree,
    source: &[u8],
    id_gen: &IdGenerator<DataNodeId>,
) -> (Vec<DataNode>, Vec<FlowEdge>) {
    match lang {
        LangId::Rust => crate::language::rust::extract_rust_dataflow(tree, source, id_gen),
        LangId::Python => crate::language::python::extract_python_dataflow(tree, source, id_gen),
        // TODO: Implement JavaScript/TypeScript dataflow extraction
        LangId::JavaScript | LangId::TypeScript | LangId::Tsx => (Vec::new(), Vec::new()),
        // TODO: Implement Go dataflow extraction
        LangId::Go => (Vec::new(), Vec::new()),
        // TODO: Implement C/C++ dataflow extraction
        LangId::C | LangId::Cpp => (Vec::new(), Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_lang_returns_empty() {
        let source = b"var x = 1;\n";
        let lang = LangId::JavaScript;
        let tree = crate::parser::parse_tree(lang, source).unwrap();
        let id_gen = IdGenerator::new();
        let (nodes, edges) = extract_dataflow(lang, &tree, source, &id_gen);
        assert!(nodes.is_empty());
        assert!(edges.is_empty());
    }
}
