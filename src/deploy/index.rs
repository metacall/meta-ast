//! One file lookup index per deployment phase.
//!
//! Load-edge injection and client-call resolution each map script references
//! to file nodes. Both built their own path maps from the graph, and every
//! script reference re-scanned and re-sorted the whole map for filename
//! matches. One `DeployIndex` per phase replaces all of that: a single
//! linear build, then map lookups plus one path-ordered scan without
//! per-call allocation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::graph::NodeIndex;

use crate::graph::{CodeGraph, NodeData};
use crate::model::FileId;

/// File lookup maps for one deployment phase, built once from the graph.
#[derive(Debug, Default)]
pub(crate) struct DeployIndex {
    /// Project path to node index for every file node.
    pub path_to_idx: HashMap<PathBuf, NodeIndex>,
    /// Project path to file id, for load-aware candidate filtering.
    pub path_to_file_id: HashMap<PathBuf, FileId>,
    /// File id to project path.
    pub fid_to_path: HashMap<FileId, PathBuf>,
    /// File paths in path order, for the deterministic suffix scan.
    ordered: Vec<(PathBuf, NodeIndex)>,
}

impl DeployIndex {
    /// Build every map in one pass over the graph's file nodes.
    pub(crate) fn build(graph: &CodeGraph) -> Self {
        let mut index = Self::default();
        for (fid, idx) in graph.file_indices() {
            if let NodeData::File(file) = &graph.graph()[idx] {
                index.path_to_idx.insert(file.path.clone(), idx);
                index.path_to_file_id.insert(file.path.clone(), fid);
                index.fid_to_path.insert(fid, file.path.clone());
                index.ordered.push((file.path.clone(), idx));
            }
        }
        index.ordered.sort_by(|a, b| a.0.cmp(&b.0));
        index
    }

    /// Build from a plain path map. Test-only: strategies need paths, not ids.
    #[cfg(test)]
    pub(crate) fn from_index(path_to_idx: HashMap<PathBuf, NodeIndex>) -> Self {
        let mut ordered: Vec<(PathBuf, NodeIndex)> = path_to_idx
            .iter()
            .map(|(path, &idx)| (path.clone(), idx))
            .collect();
        ordered.sort_by(|a, b| a.0.cmp(&b.0));
        Self {
            path_to_idx,
            path_to_file_id: HashMap::new(),
            fid_to_path: HashMap::new(),
            ordered,
        }
    }

    /// Resolve a load script to a file node, trying the same strategies as
    /// before: base-relative, source-file-relative, filename or suffix
    /// match in path order, component-stripping. Returns None when no file
    /// node matches.
    pub(crate) fn resolve_script(
        &self,
        base: &Path,
        script: &str,
        source_file: &Path,
    ) -> Option<NodeIndex> {
        if let Some(idx) = self.candidate(base, script, source_file) {
            return Some(idx);
        }
        // A script written on Windows uses backslashes, which are not separators on
        // Unix, so the normalized form is a second candidate.
        let normalized = script.replace('\\', "/");
        if normalized != script {
            return self.candidate(base, &normalized, source_file);
        }
        None
    }

    fn candidate(&self, base: &Path, script: &str, source_file: &Path) -> Option<NodeIndex> {
        // Strategy 1: base-relative.
        if let Some(&idx) = self.path_to_idx.get(&base.join(script)) {
            return Some(idx);
        }

        // Strategy 2: source-file-relative.
        let source_dir = source_file.parent().unwrap_or(Path::new("."));
        if let Some(&idx) = self.path_to_idx.get(&source_dir.join(script)) {
            return Some(idx);
        }

        // Strategy 3: filename or suffix match. The entries come
        // path-ordered, so the first hit is the deterministic pick without
        // collecting and sorting per script.
        let script_path = Path::new(script);
        let target_filename = script_path.file_name().unwrap_or_default();
        if let Some((_, idx)) = self.ordered.iter().find(|(path, _)| {
            path.file_name() == Some(target_filename) || path.ends_with(script_path)
        }) {
            return Some(*idx);
        }

        // Strategy 4: pop path prefixes from the script until one matches.
        let mut components: Vec<_> = script_path.components().collect();
        while components.len() > 1 {
            components.remove(0);
            let stripped: PathBuf = components.iter().collect();
            if let Some(&idx) = self.path_to_idx.get(&stripped) {
                return Some(idx);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_script_to_file_basename_collision_is_deterministic() {
        // Two files share the basename; the lexicographically smallest path
        // must win so the result is stable across runs.
        let mut path_to_idx: HashMap<PathBuf, NodeIndex> = HashMap::new();
        path_to_idx.insert(PathBuf::from("b/math.js"), NodeIndex::new(1));
        path_to_idx.insert(PathBuf::from("a/math.js"), NodeIndex::new(0));
        let index = DeployIndex::from_index(path_to_idx);

        let resolved =
            index.resolve_script(Path::new("."), "math.js", Path::new("orchestrator.py"));
        assert_eq!(resolved, Some(NodeIndex::new(0)));
    }

    #[test]
    fn resolve_script_to_file_strategies() {
        let mut path_to_idx: HashMap<PathBuf, NodeIndex> = HashMap::new();
        let root_relative = NodeIndex::new(1);
        let source_relative = NodeIndex::new(2);
        let filename_fallback = NodeIndex::new(3);
        path_to_idx.insert(PathBuf::from("proj/lib/math.js"), root_relative);
        path_to_idx.insert(PathBuf::from("proj/src/util.js"), source_relative);
        path_to_idx.insert(
            PathBuf::from("vendor/third_party/legacy.js"),
            filename_fallback,
        );
        let index = DeployIndex::from_index(path_to_idx);

        let root = Path::new("proj");
        let source_file = Path::new("proj/src/main.py");

        // Strategy 1: script relative to the project root.
        assert_eq!(
            index.resolve_script(root, "lib/math.js", source_file),
            Some(root_relative)
        );
        // Strategy 2: script relative to the source file's directory.
        assert_eq!(
            index.resolve_script(root, "util.js", source_file),
            Some(source_relative)
        );
        // Strategy 3: filename match after stripping path components.
        assert_eq!(
            index.resolve_script(root, "deep/path/legacy.js", source_file),
            Some(filename_fallback)
        );
        // No strategy matches.
        assert_eq!(index.resolve_script(root, "missing.js", source_file), None);
    }

    #[test]
    fn windows_script_path_resolves_on_unix() {
        let dir = std::env::temp_dir().join("meta_ast_index_windows_path");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let util = dir.join("sub").join("util.js");
        std::fs::write(&util, "export const x = 1;\n").unwrap();

        let path_to_idx =
            std::collections::HashMap::from([(util.clone(), petgraph::graph::NodeIndex::new(0))]);
        let index = DeployIndex::from_index(path_to_idx);
        let source = dir.join("app.py");

        let resolved = index.resolve_script(&dir, "sub\\util.js", &source);
        assert!(
            resolved.is_some(),
            "a Windows-authored script path must resolve on Unix"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
