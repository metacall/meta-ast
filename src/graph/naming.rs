//! Canonical names of graph nodes.
//!
//! One authority for the writer side: a node kind or display name is spelled
//! here, so the serialized graph output cannot drift from the node model by
//! holding its own copy of the strings. Stable shard names are a separate
//! concern and keep their own module.

use crate::graph::node::NodeData;
use crate::language::LangId;

/// Kind name of a node: `file`, `symbol`, `external` or `data`.
pub fn node_kind_name(node: &NodeData) -> &'static str {
    node.kind_str()
}

/// Name a reader shows for a node, when the node has one.
///
/// Symbols carry their name and data nodes an optional name. Files and
/// external dependencies are addressed by path instead.
pub fn node_display_name(node: &NodeData) -> Option<&str> {
    match node {
        NodeData::Symbol(symbol) => Some(symbol.name.as_str()),
        NodeData::Data(data) => data.name.as_deref(),
        _ => None,
    }
}

/// Language of a node, when the node has one.
pub fn node_language(node: &NodeData) -> Option<LangId> {
    match node {
        NodeData::File(file) => Some(file.language),
        NodeData::External(external) => Some(external.language),
        _ => None,
    }
}
