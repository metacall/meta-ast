//! Stable naming and descriptor generation for shard endpoints.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use petgraph::graph::NodeIndex;

use crate::error::{Diagnostic, Severity};
use crate::graph::{CodeGraph, NodeData};
use crate::model::FileId;
use crate::output::shard::error::ShardError;

/// Suffix of every shard file.
const SHARD_SUFFIX: &str = ".jsonl";
/// Longest escaped component kept verbatim in a name. The suffix and a
/// collision marker must still fit the common 255 byte file name limit.
const MAX_COMPONENT_BYTES: usize = 200;
/// Digest length used when an escaped path is too long to name a file.
const LONG_COMPONENT_DIGEST: usize = 32;

/// Path of the file that owns a node: the file itself, or the file that holds
/// a symbol. `None` for an external node, a data node, or an unknown index.
pub(crate) fn node_owner_path(graph: &CodeGraph, node_index: NodeIndex) -> Option<&Path> {
    match graph.graph().node_weight(node_index)? {
        NodeData::File(file) => Some(file.path.as_path()),
        NodeData::Symbol(symbol) => graph
            .file_node(symbol.file_id)
            .map(|file| file.path.as_path()),
        NodeData::External(_) | NodeData::Data(_) => None,
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

/// Key that two written names share when one file system would treat them as
/// one file: case folding, and the trailing dot or space that Windows strips.
///
/// A shard writer percent-encodes before a name reaches the file system, which
/// removes every other ambiguity, so Unicode normalization is not part of the
/// key.
pub fn collision_key(name: &str) -> String {
    name.trim_end_matches(['.', ' ']).to_lowercase()
}

/// Windows reserves these names whatever extension follows them.
pub(crate) fn is_device_stem(stem: &str) -> bool {
    const NAMES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];
    if NAMES.iter().any(|name| stem.eq_ignore_ascii_case(name)) {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && bytes[3].is_ascii_digit()
        && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
}

/// One shard file name per source path, in input order.
#[derive(Debug, Clone)]
pub struct ShardNamePlan {
    pub names: Vec<String>,
    /// One warning per name that had to change, naming the paths involved.
    pub warnings: Vec<Diagnostic>,
}

/// Escaped shard component and the two reasons it can differ from the path.
struct ShardComponent {
    name: String,
    /// The escaped path was too long, so the name is a digest.
    hashed: bool,
    /// A reserved device stem was escaped.
    device: bool,
}

/// Choose a shard file name for every source path.
///
/// Paths are expected relative to the index root; an absolute path still
/// produces a valid name. Two paths that one file system folds into one name
/// get a deterministic suffix instead of sharing a file, so the index also
/// loads after it moves to a platform with different file name rules.
pub fn plan_shard_file_names(paths: &[PathBuf]) -> Result<ShardNamePlan, ShardError> {
    let mut names = Vec::with_capacity(paths.len());
    let mut warnings = Vec::new();
    let mut taken: BTreeMap<String, PathBuf> = BTreeMap::new();

    for (position, path) in paths.iter().enumerate() {
        let component = shard_component(path)?;
        let name = format!("shards/{}{SHARD_SUFFIX}", component.name);
        if component.device {
            warnings.push(Diagnostic {
                path: path.clone(),
                severity: Severity::Warning,
                message: format!(
                    "{} names a Windows device, so the shard is written as {name}",
                    path.display()
                ),
                source_range: None,
            });
        }
        if component.hashed {
            taken.insert(collision_key(&name), path.clone());
            names.push(name);
            continue;
        }

        let Some(first) = taken.get(&collision_key(&name)).cloned() else {
            taken.insert(collision_key(&name), path.clone());
            names.push(name);
            continue;
        };

        let resolved = disambiguated(&name, &first, position, &taken)?;
        taken.insert(collision_key(&resolved), path.clone());
        warnings.push(Diagnostic {
            path: path.clone(),
            severity: Severity::Warning,
            message: format!(
                "{} and {} differ only by case or a trailing mark, so the shard is written as {resolved}",
                first.display(),
                path.display()
            ),
            source_range: None,
        });
        names.push(resolved);
    }

    Ok(ShardNamePlan { names, warnings })
}

/// Escaped form of the last path component, portable on every file system.
fn shard_component(path: &Path) -> Result<ShardComponent, ShardError> {
    let normalized = normalized_path(path)?;
    let (directory, file) = match normalized.rsplit_once('/') {
        Some((directory, file)) => (Some(directory), file),
        None => (None, normalized.as_str()),
    };
    let (escaped_file, device) = escaped_file_component(file);
    let component = match directory {
        Some(directory) => format!("{}%2F{escaped_file}", escape_component(directory)),
        None => escaped_file,
    };
    if component.len() <= MAX_COMPONENT_BYTES {
        return Ok(ShardComponent {
            name: component,
            hashed: false,
            device,
        });
    }
    let digest = blake3::hash(normalized.as_bytes()).to_hex().to_string();
    Ok(ShardComponent {
        name: digest[..LONG_COMPONENT_DIGEST].to_string(),
        hashed: true,
        device,
    })
}

/// Escape a file name so it can be created on Windows, macOS and Linux: a
/// trailing dot is escaped because Windows strips it, and a device stem is
/// escaped because Windows reserves it whatever the extension is. The second
/// value reports the device rewrite.
fn escaped_file_component(file: &str) -> (String, bool) {
    let mut escaped = escape_component(file);
    if escaped.ends_with('.') {
        escaped.truncate(escaped.len() - 1);
        escaped.push_str("%2E");
    }
    let stem_end = escaped.find('.').unwrap_or(escaped.len());
    if !is_device_stem(&escaped[..stem_end]) {
        return (escaped, false);
    }
    let first = escaped.as_bytes()[0];
    (format!("%{first:02X}{}", &escaped[1..]), true)
}

/// Deterministic marker that separates two names the file system folds.
fn disambiguated(
    name: &str,
    first: &Path,
    position: usize,
    taken: &BTreeMap<String, PathBuf>,
) -> Result<String, ShardError> {
    let stem = name.strip_suffix(SHARD_SUFFIX).unwrap_or(name);
    let seed = format!("{}\u{0}{name}\u{0}{position}", first.display());
    let digest = blake3::hash(seed.as_bytes()).to_hex().to_string();
    for width in [8usize, 16, 32, 64] {
        let candidate = format!("{stem}.{}{SHARD_SUFFIX}", &digest[..width]);
        if !taken.contains_key(&collision_key(&candidate)) {
            return Ok(candidate);
        }
    }
    Err(ShardError::UnwritableShardName {
        name: name.to_string(),
        reason: "no digest separates this name from the names already planned",
    })
}
