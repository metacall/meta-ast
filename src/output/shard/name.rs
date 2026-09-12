//! Stable naming and descriptor generation for shard endpoints.

use std::collections::HashMap;
use std::path::Path;

use petgraph::graph::NodeIndex;

use crate::graph::{CodeGraph, NodeData};
use crate::model::FileId;
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

/// Stable names for every node of one graph.
///
/// A shard export needs one name per edge endpoint, and a shard restore needs
/// one per node. Answering a single lookup rescans the graph (parent and
/// ordinal search), so the scan happens once here: parents come from a stack
/// over range-sorted symbols, and ordinals from one sort per
/// (parent, name, kind) group.
pub(crate) struct StableNameIndex {
    names: HashMap<NodeIndex, String>,
}

impl StableNameIndex {
    pub(crate) fn new(graph: &CodeGraph) -> Result<Self, ShardError> {
        let missing = |index: NodeIndex| ShardError::MissingNodeOwner {
            node_index: index.index(),
        };
        let g = graph.graph();

        // Range and identity of every symbol, and the symbols of each file.
        let mut ranges: HashMap<NodeIndex, (usize, usize, u32)> = HashMap::new();
        let mut by_file: HashMap<FileId, Vec<NodeIndex>> = HashMap::new();
        let mut names: HashMap<NodeIndex, String> = HashMap::new();
        for index in g.node_indices() {
            match &g[index] {
                NodeData::File(file) => {
                    names.insert(
                        index,
                        format!(
                            "{} file {}",
                            file.language.as_ref(),
                            escape_component(&normalized_path(&file.path)?)
                        ),
                    );
                }
                NodeData::External(external) => {
                    names.insert(
                        index,
                        format!(
                            "{} external {}",
                            external.language.as_ref(),
                            escape_component(&external.raw_path)
                        ),
                    );
                }
                NodeData::Symbol(symbol) => {
                    ranges.insert(
                        index,
                        (
                            symbol.source_range.byte_start,
                            symbol.source_range.byte_end,
                            symbol.id.to_raw(),
                        ),
                    );
                    by_file.entry(symbol.file_id).or_default().push(index);
                }
                NodeData::Data(_) => {}
            }
        }

        let mut parents: HashMap<NodeIndex, Option<NodeIndex>> = HashMap::new();
        let mut ordinals: HashMap<NodeIndex, usize> = HashMap::new();
        for group in by_file.values_mut() {
            // Outermost first: by start, then by the wider range.
            group.sort_by_key(|index| {
                let (start, end, id) = ranges.get(index).copied().unwrap_or_default();
                (start, std::cmp::Reverse(end), id)
            });

            // A symbol's parent is the nearest enclosing symbol. The stack
            // holds the enclosing chain of the previous symbol.
            let mut stack: Vec<NodeIndex> = Vec::new();
            for &index in group.iter() {
                let (start, end, _) = ranges.get(&index).copied().unwrap_or_default();
                while let Some(&candidate) = stack.last() {
                    let (candidate_start, candidate_end, _) =
                        ranges.get(&candidate).copied().unwrap_or_default();
                    let contains = candidate_start <= start
                        && candidate_end >= end
                        && (candidate_start < start || candidate_end > end);
                    if contains {
                        break;
                    }
                    stack.pop();
                }
                parents.insert(index, stack.last().copied());
                stack.push(index);
            }

            // Ordinal: position among the symbols that share a parent, a name
            // and a kind, ordered by range and then by identifier.
            let mut same_name: HashMap<(Option<NodeIndex>, String, &'static str), Vec<NodeIndex>> =
                HashMap::new();
            for &index in group.iter() {
                let NodeData::Symbol(symbol) = &g[index] else {
                    return Err(missing(index));
                };
                same_name
                    .entry((
                        parents.get(&index).copied().flatten(),
                        symbol.name.clone(),
                        symbol.kind.as_str(),
                    ))
                    .or_default()
                    .push(index);
            }
            for members in same_name.values_mut() {
                members.sort_by_key(|index| ranges.get(index).copied().unwrap_or_default());
                for (ordinal, &index) in members.iter().enumerate() {
                    ordinals.insert(index, ordinal);
                }
            }
        }

        for &index in ranges.keys() {
            let NodeData::Symbol(symbol) = &g[index] else {
                return Err(missing(index));
            };
            let file = graph
                .file_node(symbol.file_id)
                .ok_or_else(|| missing(index))?;

            let mut hierarchy = vec![index];
            let mut ancestor = parents.get(&index).copied().flatten();
            while let Some(current) = ancestor {
                hierarchy.push(current);
                ancestor = parents.get(&current).copied().flatten();
            }
            hierarchy.reverse();

            let descriptors = hierarchy
                .iter()
                .map(|&current| {
                    let NodeData::Symbol(ancestor_symbol) = &g[current] else {
                        return String::new();
                    };
                    format!(
                        "{}#{}!{}",
                        escape_component(&ancestor_symbol.name),
                        ancestor_symbol.kind.as_str(),
                        ordinals.get(&current).copied().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(" . ");
            names.insert(
                index,
                format!(
                    "{} {} . {} .",
                    file.language.as_ref(),
                    escape_component(&normalized_path(&file.path)?),
                    descriptors
                ),
            );
        }

        Ok(Self { names })
    }

    /// Stable name of a node. `None` for a data node or an unknown index.
    pub(crate) fn name_of(&self, node_index: NodeIndex) -> Option<&str> {
        self.names.get(&node_index).map(String::as_str)
    }
}

pub(crate) fn normalized_path(path: &Path) -> Result<String, ShardError> {
    let simplified = dunce::simplified(path);
    let value = simplified.to_str().ok_or_else(|| ShardError::NonUtf8Path {
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
