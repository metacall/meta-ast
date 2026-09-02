//! Stable naming and descriptor generation for shard endpoints.

use std::path::Path;

use petgraph::graph::NodeIndex;

use crate::graph::{CodeGraph, NodeData};
use crate::model::SymbolKind;
use crate::output::shard::error::ShardError;

pub(crate) fn node_belongs_to_file(graph: &CodeGraph, node_index: NodeIndex, path: &Path) -> bool {
    match graph.graph().node_weight(node_index) {
        Some(NodeData::File(file)) => file.path == path,
        Some(NodeData::Symbol(symbol)) => graph
            .file_node(symbol.file_id)
            .is_some_and(|file| file.path == path),
        Some(NodeData::External(_) | NodeData::Data(_)) | None => false,
    }
}

pub(crate) fn stable_node_name(
    graph: &CodeGraph,
    node_index: NodeIndex,
) -> Result<String, ShardError> {
    let node = graph
        .graph()
        .node_weight(node_index)
        .ok_or(ShardError::MissingNodeOwner {
            node_index: node_index.index(),
        })?;
    match node {
        NodeData::File(file) => Ok(format!(
            "{} file {}",
            file.language.as_ref(),
            escape_component(&normalized_path(&file.path)?)
        )),
        NodeData::Symbol(_) => stable_symbol_name(graph, node_index),
        NodeData::External(external) => Ok(format!(
            "{} external {}",
            external.language.as_ref(),
            escape_component(&external.raw_path)
        )),
        NodeData::Data(_) => Err(ShardError::MissingNodeOwner {
            node_index: node_index.index(),
        }),
    }
}

pub(crate) fn stable_symbol_name(
    graph: &CodeGraph,
    node_index: NodeIndex,
) -> Result<String, ShardError> {
    let NodeData::Symbol(symbol) = &graph.graph()[node_index] else {
        return Err(ShardError::MissingNodeOwner {
            node_index: node_index.index(),
        });
    };
    let file = graph
        .file_node(symbol.file_id)
        .ok_or(ShardError::MissingNodeOwner {
            node_index: node_index.index(),
        })?;
    let mut hierarchy = Vec::new();
    let mut current = Some(node_index);
    while let Some(index) = current {
        hierarchy.push(index);
        current = parent_symbol(graph, index);
    }
    hierarchy.reverse();

    let descriptors = hierarchy
        .into_iter()
        .map(|index| symbol_descriptor(graph, index))
        .collect::<Vec<_>>()
        .join(" . ");
    Ok(format!(
        "{} {} . {} .",
        file.language.as_ref(),
        escape_component(&normalized_path(&file.path)?),
        descriptors
    ))
}

pub(crate) fn symbol_descriptor(graph: &CodeGraph, node_index: NodeIndex) -> String {
    let NodeData::Symbol(symbol) = &graph.graph()[node_index] else {
        return String::new();
    };
    let parent = parent_symbol(graph, node_index);
    let ordinal = graph
        .graph()
        .node_indices()
        .filter(|candidate_index| {
            if *candidate_index == node_index {
                return false;
            }
            let NodeData::Symbol(candidate) = &graph.graph()[*candidate_index] else {
                return false;
            };
            if candidate.file_id != symbol.file_id
                || candidate.name != symbol.name
                || candidate.kind != symbol.kind
            {
                return false;
            }
            if parent_symbol(graph, *candidate_index) != parent {
                return false;
            }
            candidate.source_range.byte_start < symbol.source_range.byte_start
                || (candidate.source_range.byte_start == symbol.source_range.byte_start
                    && (candidate.source_range.byte_end < symbol.source_range.byte_end
                        || (candidate.source_range.byte_end == symbol.source_range.byte_end
                            && candidate.id < symbol.id)))
        })
        .count();
    format!(
        "{}#{}!{ordinal}",
        escape_component(&symbol.name),
        symbol_kind_name(symbol.kind)
    )
}

pub(crate) fn parent_symbol(graph: &CodeGraph, node_index: NodeIndex) -> Option<NodeIndex> {
    let NodeData::Symbol(symbol) = &graph.graph()[node_index] else {
        return None;
    };
    graph
        .graph()
        .node_indices()
        .filter(|candidate_index| *candidate_index != node_index)
        .filter_map(|candidate_index| {
            let NodeData::Symbol(candidate) = &graph.graph()[candidate_index] else {
                return None;
            };
            let contains = candidate.file_id == symbol.file_id
                && candidate.source_range.byte_start <= symbol.source_range.byte_start
                && candidate.source_range.byte_end >= symbol.source_range.byte_end
                && (candidate.source_range.byte_start < symbol.source_range.byte_start
                    || candidate.source_range.byte_end > symbol.source_range.byte_end);
            contains.then_some((
                candidate.source_range.byte_end - candidate.source_range.byte_start,
                candidate.source_range.byte_start,
                candidate.source_range.byte_end,
                candidate_index,
            ))
        })
        .min_by_key(|(span, start, end, _)| (*span, *start, *end))
        .map(|(_, _, _, index)| index)
}

pub(crate) fn normalized_path(path: &Path) -> Result<String, ShardError> {
    let value = path.to_str().ok_or_else(|| ShardError::NonUtf8Path {
        path: path.to_path_buf(),
    })?;
    #[cfg(windows)]
    let value = value.replace('\\', "/");
    #[cfg(not(windows))]
    let value = value.to_string();
    Ok(value)
}

pub(crate) fn escape_component(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            escaped.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(escaped, "%{byte:02X}");
        }
    }
    escaped
}

pub(crate) fn symbol_kind_name(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Interface => "interface",
        SymbolKind::Trait => "trait",
        SymbolKind::Enum => "enum",
        SymbolKind::Object => "object",
        SymbolKind::Constant => "constant",
        SymbolKind::Static => "static",
        SymbolKind::Module => "module",
        SymbolKind::Namespace => "namespace",
        SymbolKind::TypeAlias => "type_alias",
    }
}
