//! Client-call resolution: map `metacall('fn', ...)` invocations to symbol nodes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::deploy::scanner::{CallSite, CallSiteVariant};
use crate::error::{Diagnostic, Severity};
use crate::graph::edge::{
    CONFIDENCE_CLIENT_MULTI_GLOBAL, CONFIDENCE_CLIENT_MULTI_LOAD, CONFIDENCE_CLIENT_UNIQUE_GLOBAL,
    CONFIDENCE_CLIENT_UNIQUE_LOAD,
};
use crate::graph::{CodeGraph, NodeData};
use crate::language::LangId;
use crate::model::{FileExtraction, FileId, SymbolId};
use petgraph::graph::NodeIndex;

/// Result of mapping ClientCall sites to target symbol nodes.
pub(crate) struct ClientCallResolution {
    /// (source file node, target symbol node, confidence)
    pub edges: Vec<(NodeIndex, NodeIndex, f32)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Resolve a load script to a file node, trying the same strategies as
/// add_metacall_edge (root-relative, source-file-relative, filename match,
/// component-stripping). Returns None when no file node matches.
pub(crate) fn resolve_script_to_file(
    root: &Path,
    script: &str,
    source_file: &Path,
    path_to_idx: &HashMap<PathBuf, NodeIndex>,
) -> Option<NodeIndex> {
    // Strategy 1: root-relative.
    let candidate = root.join(script);
    if let Some(&idx) = path_to_idx.get(&candidate) {
        return Some(idx);
    }

    // Strategy 2: source-file-relative.
    let source_dir = source_file.parent().unwrap_or(Path::new("."));
    let candidate = source_dir.join(script);
    if let Some(&idx) = path_to_idx.get(&candidate) {
        return Some(idx);
    }

    // Strategy 3: strip leading path components until the filename matches.
    // Collect every match and pick the lexicographically smallest path so the
    // result is deterministic even with duplicate basenames.
    let script_path = Path::new(script);
    let target_filename = script_path.file_name().unwrap_or(std::ffi::OsStr::new(""));
    let mut filename_matches: Vec<(PathBuf, NodeIndex)> = path_to_idx
        .iter()
        .filter(|(path, _)| {
            path.file_name() == Some(target_filename) || path.ends_with(script_path)
        })
        .map(|(path, &idx)| (path.clone(), idx))
        .collect();
    filename_matches.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some((_, idx)) = filename_matches.first() {
        return Some(*idx);
    }

    // Strategy 4: pop path prefixes from the script until one matches.
    let mut components: Vec<_> = script_path.components().collect();
    while components.len() > 1 {
        components.remove(0);
        let stripped: PathBuf = components.iter().collect();
        if let Some(&idx) = path_to_idx.get(&stripped) {
            return Some(idx);
        }
    }

    None
}

/// Resolve ClientCall sites to target symbol nodes.
///
/// Phase A (load-aware): resolves against symbols loaded by the source file (unique = 1.0, multiple = 0.8).
/// Phase B (global fallback): searches all project symbols (unique = 0.6, multiple = 0.5).
/// Computed function names cap confidence at 0.4. Unresolved names emit a Warning diagnostic.
pub(crate) fn resolve_client_calls(
    graph: &CodeGraph,
    extractions: &[Arc<FileExtraction>],
    call_sites: &[CallSite],
    root: &Path,
) -> ClientCallResolution {
    // Path -> node index for file nodes (paths are project-root relative).
    let mut path_to_idx: HashMap<PathBuf, NodeIndex> = HashMap::new();
    // Path -> FileId, used to filter Phase A candidates by loaded file.
    let mut path_to_file_id: HashMap<PathBuf, FileId> = HashMap::new();
    for (&fid, &idx) in &graph.file_to_index {
        if let NodeData::File(f) = &graph.graph()[idx] {
            path_to_idx.insert(f.path.clone(), idx);
            path_to_file_id.insert(f.path.clone(), fid);
        }
    }

    // Name -> (symbol id, extraction path), pushed in extraction order.
    let mut name_index: HashMap<String, Vec<(SymbolId, PathBuf)>> = HashMap::new();
    for extraction in extractions {
        for symbol in &extraction.symbols {
            name_index
                .entry(symbol.name.clone())
                .or_default()
                .push((symbol.id, extraction.path.clone()));
        }
    }

    let mut edges: Vec<(NodeIndex, NodeIndex, f32)> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();

    // Files each source file loads, in load-site order (deduplicated). The
    // tag (when present) constrains Phase A candidates to that language.
    let mut loaded_by_source: HashMap<PathBuf, Vec<(FileId, Option<LangId>)>> = HashMap::new();
    for site in call_sites {
        let (scripts, tag): (Vec<String>, Option<LangId>) = match site.variant {
            CallSiteVariant::LoadFromFile => (
                site.scripts.clone(),
                site.target_lang
                    .as_deref()
                    .and_then(crate::deploy::tags::from_metacall_tag),
            ),
            CallSiteVariant::LoadFromConfiguration => {
                let Some(config_script) = site.scripts.first() else {
                    continue;
                };
                let config_file = root.join(config_script);
                let config_json = match std::fs::read_to_string(&config_file).and_then(|s| {
                    serde_json::from_str::<serde_json::Value>(&s)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                }) {
                    Ok(json) => json,
                    Err(_) => {
                        // A referenced config that cannot be read or parsed
                        // loads nothing: surface it instead of dropping it.
                        diagnostics.push(Diagnostic {
                            path: config_file,
                            severity: Severity::Warning,
                            message: format!(
                                "unreadable or unparseable MetaCall configuration referenced by {}",
                                site.source_file.display()
                            ),
                            source_range: site.source_range.clone(),
                        });
                        continue;
                    }
                };
                let Some(scripts_arr) = config_json.get("scripts").and_then(|v| v.as_array())
                else {
                    continue;
                };
                (
                    scripts_arr
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect(),
                    // The config declares the language per entry; no per-call
                    // tag constraint is known here.
                    None,
                )
            }
            _ => continue,
        };
        let loaded = loaded_by_source
            .entry(site.source_file.clone())
            .or_default();
        for script in scripts {
            let Some(file_idx) =
                resolve_script_to_file(root, &script, &site.source_file, &path_to_idx)
            else {
                continue;
            };
            if let NodeData::File(f) = &graph.graph()[file_idx]
                && !loaded.iter().any(|(fid, _)| *fid == f.id)
            {
                loaded.push((f.id, tag));
            }
        }
    }

    for site in call_sites {
        if site.variant != CallSiteVariant::ClientCall {
            continue;
        }
        let Some(fn_name) = site.function_name.as_deref() else {
            continue;
        };
        let Some(&caller_idx) = path_to_idx.get(&site.source_file) else {
            continue;
        };

        // Phase A: symbols in files this source file loads.
        let mut candidates: Vec<SymbolId> = Vec::new();
        if let Some(loaded) = loaded_by_source.get(&site.source_file)
            && let Some(entries) = name_index.get(fn_name)
        {
            candidates.extend(
                entries
                    .iter()
                    .filter(|(_, path)| {
                        let Some(&fid) = path_to_file_id.get(path) else {
                            return false;
                        };
                        loaded.iter().any(|&(loaded_fid, tag)| {
                            loaded_fid == fid
                                && match tag {
                                    // Untagged load: path membership is the signal.
                                    None => true,
                                    // Tagged load: the file must be loaded under its
                                    // own language, so a 'node' tag never matches a py
                                    // file with the same name.
                                    Some(tag_lang) => {
                                        graph.file_node(fid).is_some_and(|f| f.language == tag_lang)
                                    }
                                }
                        })
                    })
                    .map(|(sid, _)| *sid),
            );
        }

        // Phase B: global fallback when no loaded file defines the name.
        let global = candidates.is_empty();
        if global && let Some(entries) = name_index.get(fn_name) {
            candidates.extend(entries.iter().map(|(sid, _)| *sid));
        }

        if candidates.is_empty() {
            diagnostics.push(unresolved_diagnostic(site, fn_name));
            continue;
        }

        let base: f32 = if global {
            if candidates.len() == 1 {
                CONFIDENCE_CLIENT_UNIQUE_GLOBAL
            } else {
                CONFIDENCE_CLIENT_MULTI_GLOBAL
            }
        } else if candidates.len() == 1 {
            CONFIDENCE_CLIENT_UNIQUE_LOAD
        } else {
            CONFIDENCE_CLIENT_MULTI_LOAD
        };
        let confidence = base.min(site.confidence);

        let mut emitted = 0;
        for sid in candidates {
            if let Some(sym_idx) = graph.symbol_node_index(sid) {
                edges.push((caller_idx, sym_idx, confidence));
                emitted += 1;
            }
        }
        if emitted == 0 {
            diagnostics.push(unresolved_diagnostic(site, fn_name));
        }
    }

    ClientCallResolution { edges, diagnostics }
}

/// Build the Warning diagnostic for an invocation whose target could not be
/// resolved to any symbol.
fn unresolved_diagnostic(site: &CallSite, fn_name: &str) -> Diagnostic {
    Diagnostic {
        path: site.source_file.clone(),
        severity: Severity::Warning,
        message: format!("unresolved MetaCall invocation target: '{fn_name}'"),
        source_range: site.source_range.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::node::{FileNode, SymbolNode};
    use crate::language::LangId;
    use crate::model::{LineColumn, SnapshotId, SourceRange, Symbol, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn test_range() -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 10,
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn {
                line: 1,
                column: 10,
            },
        }
    }

    fn symbol(id: u32, name: &str, path: &str, lang: LangId) -> Symbol {
        Symbol {
            id: SymbolId::new(id).unwrap(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            language: lang,
            file_path: PathBuf::from(path),
            source_range: test_range(),
            visibility: Some(Visibility::Public),
            signature: None,
            docstring: None,
            is_async: false,
        }
    }

    fn extraction(path: &str, lang: LangId, symbols: Vec<Symbol>) -> Arc<FileExtraction> {
        let mut out = FileExtraction::empty(PathBuf::from(path), lang);
        out.symbols = symbols;
        Arc::new(out)
    }

    fn load_from_file(source: &str, scripts: Vec<&str>) -> CallSite {
        CallSite {
            source_file: PathBuf::from(source),
            caller_lang: LangId::Python,
            variant: CallSiteVariant::LoadFromFile,
            target_lang: Some("node".to_string()),
            scripts: scripts.into_iter().map(str::to_string).collect(),
            function_name: None,
            is_async: false,
            source_range: None,
            confidence: 1.0,
        }
    }

    fn client_call(source: &str, fn_name: &str, confidence: f32) -> CallSite {
        CallSite {
            source_file: PathBuf::from(source),
            caller_lang: LangId::Python,
            variant: CallSiteVariant::ClientCall,
            target_lang: None,
            scripts: vec![],
            function_name: Some(fn_name.to_string()),
            is_async: false,
            source_range: None,
            confidence,
        }
    }

    /// Synthetic graph plus extractions for resolution tests.
    struct Fixture {
        graph: CodeGraph,
        extractions: Vec<Arc<FileExtraction>>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                graph: CodeGraph::new(SnapshotId::new(1).unwrap()),
                extractions: Vec::new(),
            }
        }

        fn add_file(&mut self, path: &str, lang: LangId) -> (FileId, NodeIndex) {
            let id = FileId::new(self.graph.file_to_index.len() as u32 + 1).unwrap();
            let idx = self.graph.add_node(NodeData::File(FileNode::new(
                id,
                PathBuf::from(path),
                lang,
                SnapshotId::new(1).unwrap(),
            )));
            self.graph.file_to_index.insert(id, idx);
            (id, idx)
        }

        fn add_symbol(
            &mut self,
            sym: &Symbol,
            file_id: FileId,
            path: &str,
            lang: LangId,
        ) -> NodeIndex {
            let idx = self
                .graph
                .add_node(NodeData::Symbol(SymbolNode::from_symbol(sym, file_id)));
            self.graph.symbol_to_index.insert(sym.id, idx);
            self.extractions
                .push(extraction(path, lang, vec![sym.clone()]));
            idx
        }
    }

    impl Default for Fixture {
        fn default() -> Self {
            Self::new()
        }
    }

    #[test]
    fn load_aware_unique_match() {
        let mut fx = Fixture::new();
        let (py_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("math.js", LangId::JavaScript);
        let multiply = symbol(1, "multiply", "math.js", LangId::JavaScript);
        let sym_idx = fx.add_symbol(&multiply, js_id, "math.js", LangId::JavaScript);

        let call_sites = vec![
            load_from_file("orchestrator.py", vec!["math.js"]),
            client_call("orchestrator.py", "multiply", 1.0),
        ];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        assert_eq!(resolution.edges.len(), 1);
        let (from, to, confidence) = resolution.edges[0];
        let py_idx = *fx.graph.file_to_index.get(&py_id).unwrap();
        assert_eq!(from, py_idx);
        assert_eq!(to, sym_idx);
        assert_eq!(confidence, 1.0);
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn phase_a_language_tag_filter_excludes_mistagged_files() {
        // math.js is loaded under its own 'node' tag; weird.py defines the
        // same name but is loaded under a mismatched 'node' tag, so its
        // symbols must not become Phase A candidates.
        let mut fx = Fixture::new();
        let (_py_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("math.js", LangId::JavaScript);
        let (weird_id, _) = fx.add_file("weird.py", LangId::Python);
        let js_multiply = symbol(1, "multiply", "math.js", LangId::JavaScript);
        let py_multiply = symbol(2, "multiply", "weird.py", LangId::Python);
        let js_sym = fx.add_symbol(&js_multiply, js_id, "math.js", LangId::JavaScript);
        let _py_sym = fx.add_symbol(&py_multiply, weird_id, "weird.py", LangId::Python);

        let mistagged_load = CallSite {
            source_file: PathBuf::from("orchestrator.py"),
            caller_lang: LangId::Python,
            variant: CallSiteVariant::LoadFromFile,
            target_lang: Some("node".to_string()),
            scripts: vec!["weird.py".to_string()],
            function_name: None,
            is_async: false,
            source_range: None,
            confidence: 1.0,
        };
        let call_sites = vec![
            load_from_file("orchestrator.py", vec!["math.js"]),
            mistagged_load,
            client_call("orchestrator.py", "multiply", 1.0),
        ];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        assert_eq!(resolution.edges.len(), 1);
        assert_eq!(resolution.edges[0].1, js_sym);
        assert_eq!(resolution.edges[0].2, 1.0);
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn phase_a_ambiguous_across_languages_stays_ambiguous() {
        // Both math.js (node) and math.py (py) are loaded under their own
        // tags and both define 'multiply'. The call does not name a language,
        // so both candidates remain, at 0.8 each.
        let mut fx = Fixture::new();
        let (_orchestrator_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("math.js", LangId::JavaScript);
        let (py_math_id, _) = fx.add_file("math.py", LangId::Python);
        let js_multiply = symbol(1, "multiply", "math.js", LangId::JavaScript);
        let py_multiply = symbol(2, "multiply", "math.py", LangId::Python);
        let js_sym = fx.add_symbol(&js_multiply, js_id, "math.js", LangId::JavaScript);
        let py_sym = fx.add_symbol(&py_multiply, py_math_id, "math.py", LangId::Python);

        let py_load = CallSite {
            target_lang: Some("py".to_string()),
            scripts: vec!["math.py".to_string()],
            ..load_from_file("orchestrator.py", vec![])
        };
        let call_sites = vec![
            load_from_file("orchestrator.py", vec!["math.js"]),
            py_load,
            client_call("orchestrator.py", "multiply", 1.0),
        ];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        let mut targets: Vec<_> = resolution.edges.iter().map(|(_, to, _)| *to).collect();
        targets.sort();
        let mut expected = vec![js_sym, py_sym];
        expected.sort();
        assert_eq!(targets, expected);
        assert!(resolution.edges.iter().all(|(_, _, c)| *c == 0.8));
    }

    #[test]
    fn resolve_script_to_file_basename_collision_is_deterministic() {
        // Two files share the basename; the lexicographically smallest path
        // must win so the result is stable across runs.
        let mut path_to_idx: HashMap<PathBuf, NodeIndex> = HashMap::new();
        path_to_idx.insert(PathBuf::from("b/math.js"), NodeIndex::new(1));
        path_to_idx.insert(PathBuf::from("a/math.js"), NodeIndex::new(0));

        let resolved = resolve_script_to_file(
            Path::new("."),
            "math.js",
            Path::new("orchestrator.py"),
            &path_to_idx,
        );
        assert_eq!(resolved, Some(NodeIndex::new(0)));
    }

    #[test]
    fn config_parse_failure_emits_diagnostic() {
        let mut fx = Fixture::new();
        fx.add_file("orchestrator.py", LangId::Python);
        let config_site = CallSite {
            source_file: PathBuf::from("orchestrator.py"),
            caller_lang: LangId::Python,
            variant: CallSiteVariant::LoadFromConfiguration,
            target_lang: None,
            scripts: vec!["missing.conf.json".to_string()],
            function_name: None,
            is_async: false,
            source_range: None,
            confidence: 1.0,
        };
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &[config_site], Path::new("."));
        assert!(resolution.edges.is_empty());
        assert_eq!(resolution.diagnostics.len(), 1);
        assert_eq!(resolution.diagnostics[0].severity, Severity::Warning);
        assert_eq!(
            resolution.diagnostics[0].path,
            PathBuf::from(".").join("missing.conf.json")
        );
    }

    #[test]
    fn load_aware_multiple_matches() {
        let mut fx = Fixture::new();
        let (py_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("math.js", LangId::JavaScript);
        let (utils_id, _) = fx.add_file("utils.js", LangId::JavaScript);
        let m1 = symbol(1, "multiply", "math.js", LangId::JavaScript);
        let m2 = symbol(2, "multiply", "utils.js", LangId::JavaScript);
        let s1 = fx.add_symbol(&m1, js_id, "math.js", LangId::JavaScript);
        let s2 = fx.add_symbol(&m2, utils_id, "utils.js", LangId::JavaScript);

        let call_sites = vec![
            load_from_file("orchestrator.py", vec!["math.js", "utils.js"]),
            client_call("orchestrator.py", "multiply", 1.0),
        ];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        assert_eq!(resolution.edges.len(), 2);
        let py_idx = *fx.graph.file_to_index.get(&py_id).unwrap();
        let mut targets: Vec<NodeIndex> = resolution.edges.iter().map(|(_, to, _)| *to).collect();
        targets.sort();
        let mut expected = vec![s1, s2];
        expected.sort();
        assert_eq!(targets, expected);
        for (from, _, confidence) in &resolution.edges {
            assert_eq!(*from, py_idx);
            assert_eq!(*confidence, 0.8);
        }
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn global_fallback_unique() {
        let mut fx = Fixture::new();
        let (py_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("helpers.js", LangId::JavaScript);
        let helper = symbol(1, "helper", "helpers.js", LangId::JavaScript);
        let sym_idx = fx.add_symbol(&helper, js_id, "helpers.js", LangId::JavaScript);

        let call_sites = vec![client_call("orchestrator.py", "helper", 1.0)];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        assert_eq!(resolution.edges.len(), 1);
        let (from, to, confidence) = resolution.edges[0];
        let py_idx = *fx.graph.file_to_index.get(&py_id).unwrap();
        assert_eq!(from, py_idx);
        assert_eq!(to, sym_idx);
        assert_eq!(confidence, 0.6);
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn global_fallback_ambiguous() {
        let mut fx = Fixture::new();
        let (py_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (a_id, _) = fx.add_file("helpers.js", LangId::JavaScript);
        let (b_id, _) = fx.add_file("utils.js", LangId::JavaScript);
        let h1 = symbol(1, "helper", "helpers.js", LangId::JavaScript);
        let h2 = symbol(2, "helper", "utils.js", LangId::JavaScript);
        let s1 = fx.add_symbol(&h1, a_id, "helpers.js", LangId::JavaScript);
        let s2 = fx.add_symbol(&h2, b_id, "utils.js", LangId::JavaScript);

        let call_sites = vec![client_call("orchestrator.py", "helper", 1.0)];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        assert_eq!(resolution.edges.len(), 2);
        let py_idx = *fx.graph.file_to_index.get(&py_id).unwrap();
        let mut targets: Vec<NodeIndex> = resolution.edges.iter().map(|(_, to, _)| *to).collect();
        targets.sort();
        let mut expected = vec![s1, s2];
        expected.sort();
        assert_eq!(targets, expected);
        for (from, _, confidence) in &resolution.edges {
            assert_eq!(*from, py_idx);
            assert_eq!(*confidence, 0.5);
        }
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn computed_name_cap() {
        let mut fx = Fixture::new();
        let (py_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("lib.js", LangId::JavaScript);
        let target = symbol(1, "fn_var", "lib.js", LangId::JavaScript);
        let sym_idx = fx.add_symbol(&target, js_id, "lib.js", LangId::JavaScript);

        // A computed first argument keeps the source text as function_name and
        // drops the site confidence to 0.4 (scanner convention).
        let call_sites = vec![client_call("orchestrator.py", "fn_var", 0.4)];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        assert_eq!(resolution.edges.len(), 1);
        let (from, to, confidence) = resolution.edges[0];
        let py_idx = *fx.graph.file_to_index.get(&py_id).unwrap();
        assert_eq!(from, py_idx);
        assert_eq!(to, sym_idx);
        assert_eq!(confidence, 0.4);
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn unresolved_emits_diagnostic() {
        let mut fx = Fixture::new();
        let _ = fx.add_file("orchestrator.py", LangId::Python);

        let call_sites = vec![client_call("orchestrator.py", "no_such_fn", 1.0)];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));

        assert!(resolution.edges.is_empty());
        assert_eq!(resolution.diagnostics.len(), 1);
        let diag = &resolution.diagnostics[0];
        assert_eq!(diag.path, PathBuf::from("orchestrator.py"));
        assert_eq!(diag.severity, Severity::Warning);
        assert_eq!(
            diag.message,
            "unresolved MetaCall invocation target: 'no_such_fn'"
        );
        assert!(diag.source_range.is_none());
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

        let root = Path::new("proj");
        let source_file = Path::new("proj/src/main.py");

        // Strategy 1: script relative to the project root.
        assert_eq!(
            resolve_script_to_file(root, "lib/math.js", source_file, &path_to_idx),
            Some(root_relative)
        );
        // Strategy 2: script relative to the source file's directory.
        assert_eq!(
            resolve_script_to_file(root, "util.js", source_file, &path_to_idx),
            Some(source_relative)
        );
        // Strategy 3: filename match after stripping path components.
        assert_eq!(
            resolve_script_to_file(root, "deep/path/legacy.js", source_file, &path_to_idx),
            Some(filename_fallback)
        );
        // No strategy matches.
        assert_eq!(
            resolve_script_to_file(root, "missing.js", source_file, &path_to_idx),
            None
        );
    }

    /// Characterization: a file-to-symbol client-call edge can never be
    /// absorbed by a scope-resolved symbol-to-symbol reference (endpoint node
    /// types differ, so `(src, dst, kind)` triples never collide).
    #[test]
    fn computed_name_client_call_survives_alongside_scope_reference() {
        use crate::graph::edge::EdgeKind;

        let mut fx = Fixture::new();
        let (py_id, py_idx) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("math.js", LangId::JavaScript);
        let multiply = symbol(1, "multiply", "math.js", LangId::JavaScript);
        let sym_idx = fx.add_symbol(&multiply, js_id, "math.js", LangId::JavaScript);
        let caller = symbol(2, "caller", "orchestrator.py", LangId::Python);
        let caller_idx = fx.add_symbol(&caller, py_id, "orchestrator.py", LangId::Python);

        // Scope-resolved reference (what GraphBuilder produces): sym -> sym @ 1.0.
        fx.graph
            .add_edge_normalized(caller_idx, sym_idx, EdgeKind::Reference, 1.0);

        // Computed-name client call: file -> sym @ 0.4.
        let call_sites = vec![client_call("orchestrator.py", "multiply", 0.4)];
        let resolution =
            resolve_client_calls(&fx.graph, &fx.extractions, &call_sites, Path::new("."));
        assert_eq!(resolution.edges.len(), 1);
        let (from, to, confidence) = resolution.edges[0];
        assert_eq!((from, to, confidence), (py_idx, sym_idx, 0.4));
        fx.graph
            .add_edge_normalized(from, to, EdgeKind::Reference, confidence);

        // Both edges coexist with their own confidences.
        let client_edge = fx.graph.graph().find_edge(py_idx, sym_idx).unwrap();
        let scope_edge = fx.graph.graph().find_edge(caller_idx, sym_idx).unwrap();
        assert_ne!(client_edge, scope_edge);
        assert_eq!(
            fx.graph
                .graph()
                .edge_weight(client_edge)
                .unwrap()
                .confidence,
            0.4
        );
        assert_eq!(
            fx.graph.graph().edge_weight(scope_edge).unwrap().confidence,
            1.0
        );
        assert!(resolution.diagnostics.is_empty());
    }
}
