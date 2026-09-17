//! Client-call resolution: map `metacall('fn', ...)` invocations to symbol nodes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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

/// Resolve ClientCall sites to target symbol nodes.
///
/// Phase A (load-aware): resolves against symbols loaded by the source file (unique = 1.0, multiple = 0.8).
/// Phase B (global fallback): searches all project symbols (unique = 0.6, multiple = 0.5).
/// Computed function names cap confidence at 0.4. Unresolved names emit a Warning diagnostic.
fn resolve_sites<F>(
    graph: &CodeGraph,
    extractions: &[F],
    call_sites: &[CallSite],
    root: &Path,
    index: &super::index::DeployIndex,
) -> (Vec<ResolvedClientCall>, Vec<Diagnostic>)
where
    F: std::borrow::Borrow<FileExtraction> + Sync,
{
    let path_to_idx = &index.path_to_idx;
    let path_to_file_id = &index.path_to_file_id;

    // Name -> (symbol id, extraction path), pushed in extraction order.
    let mut name_index: HashMap<String, Vec<(SymbolId, PathBuf)>> = HashMap::new();
    for extraction in extractions {
        let extraction = extraction.borrow();
        for symbol in &extraction.symbols {
            name_index
                .entry(symbol.name.clone())
                .or_default()
                .push((symbol.id, extraction.path.clone()));
        }
    }

    let mut resolved: Vec<ResolvedClientCall> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    let mut configs = super::config::ConfigCache::default();
    // One broken file reports once no matter how many sites name it.
    let mut reported: HashSet<PathBuf> = HashSet::new();

    // Files each source file loads, in load-site order (deduplicated). The
    // tag (when present) constrains Phase A candidates to that language.
    let mut loaded_by_source: HashMap<PathBuf, Vec<(FileId, Option<LangId>)>> = HashMap::new();
    for site in call_sites {
        let (scripts, tag, base): (Vec<String>, Option<LangId>, PathBuf) = match site.variant {
            CallSiteVariant::LoadFromFile => (
                site.scripts.clone(),
                site.target_lang
                    .as_deref()
                    .and_then(crate::deploy::tags::from_metacall_tag),
                root.to_path_buf(),
            ),
            CallSiteVariant::LoadFromConfiguration => {
                let Some(config_script) = site.scripts.first() else {
                    continue;
                };
                let Some(config_file) = super::config::join_contained(root, config_script) else {
                    diagnostics.push(super::config::config_diagnostic(
                        &site.source_file,
                        site.source_range.as_ref(),
                        format!("MetaCall configuration escapes the project root: {config_script}"),
                    ));
                    continue;
                };
                let parsed = match configs.get(&config_file) {
                    Ok(parsed) => parsed,
                    Err(message) => {
                        if reported.insert(config_file.clone()) {
                            diagnostics.push(super::config::config_diagnostic(
                                &config_file,
                                site.source_range.as_ref(),
                                message,
                            ));
                        }
                        continue;
                    }
                };
                let tag = parsed
                    .language_id
                    .as_deref()
                    .and_then(crate::deploy::tags::from_metacall_tag);
                let base = super::config::script_base(&config_file, &parsed, root);
                (parsed.scripts.clone(), tag, base)
            }
            _ => continue,
        };
        let loaded = loaded_by_source
            .entry(site.source_file.clone())
            .or_default();
        for script in scripts {
            let Some(file_idx) = index.resolve_script(&base, &script, &site.source_file) else {
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
        if !path_to_idx.contains_key(&site.source_file) {
            continue;
        }

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
            if graph.symbol_node_index(sid).is_some() {
                resolved.push(ResolvedClientCall {
                    source_file: site.source_file.clone(),
                    source_range: site.source_range.clone(),
                    target: sid,
                    confidence,
                });
                emitted += 1;
            }
        }
        if emitted == 0 {
            diagnostics.push(unresolved_diagnostic(site, fn_name));
        }
    }

    (resolved, diagnostics)
}

/// One resolved invocation target, with the call site that produced it.
///
/// The caller projections name the calling file and the enclosing symbol; this
/// record keeps the call-site range too, so a consumer can point at the
/// invocation instead of re-deriving it from the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedClientCall {
    /// File that contains the call site, as extracted.
    pub source_file: PathBuf,
    /// Range of the call site, when the source spelled one.
    pub source_range: Option<crate::model::SourceRange>,
    /// Symbol the invocation resolves to.
    pub target: SymbolId,
    /// Ladder confidence: 1.0 unique load-confirmed, 0.8 multiple
    /// load-confirmed, 0.6 unique global, 0.5 multiple global, 0.4 computed.
    pub confidence: f32,
}

/// Both call-edge projections of one resolution pass.
#[derive(Debug)]
pub struct ClientCallProjections {
    /// File node to symbol node. Deployment needs the calling file, because a
    /// top-level call has no enclosing symbol.
    pub file_edges: Vec<(NodeIndex, NodeIndex, f32)>,
    /// Symbol node to symbol node. Navigation keys on the caller symbol.
    pub symbol_edges: Vec<(SymbolId, SymbolId, f32)>,
    /// One entry per resolved target, in resolution order.
    pub resolved: Vec<ResolvedClientCall>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Resolve ClientCall sites once and emit both projections.
pub fn resolve_client_call_projections<F>(
    graph: &CodeGraph,
    extractions: &[F],
    call_sites: &[CallSite],
    root: &Path,
) -> ClientCallProjections
where
    F: std::borrow::Borrow<FileExtraction> + Sync,
{
    let index = super::index::DeployIndex::build(graph);
    let (resolved, diagnostics) = resolve_sites(graph, extractions, call_sites, root, &index);
    let path_to_idx = &index.path_to_idx;

    // One extraction lookup per path, instead of a scan per resolved site.
    let path_to_extraction: HashMap<&Path, &FileExtraction> = extractions
        .iter()
        .map(std::borrow::Borrow::borrow)
        .map(|extraction| (extraction.path.as_path(), extraction))
        .collect();

    let mut file_edges = Vec::with_capacity(resolved.len());
    let mut symbol_edges = Vec::with_capacity(resolved.len());
    for call in &resolved {
        if let (Some(&caller_idx), Some(target_idx)) = (
            path_to_idx.get(&call.source_file),
            graph.symbol_node_index(call.target),
        ) {
            file_edges.push((caller_idx, target_idx, call.confidence));
        }
        if let Some(caller) = enclosing_symbol(
            &path_to_extraction,
            &call.source_file,
            call.source_range.as_ref(),
        ) {
            symbol_edges.push((caller, call.target, call.confidence));
        }
    }

    ClientCallProjections {
        file_edges,
        symbol_edges,
        resolved,
        diagnostics,
    }
}

fn enclosing_symbol(
    path_to_extraction: &HashMap<&Path, &FileExtraction>,
    path: &Path,
    range: Option<&crate::model::SourceRange>,
) -> Option<SymbolId> {
    let range = range?;
    let file = path_to_extraction.get(path)?;
    file.symbols
        .iter()
        .filter(|symbol| {
            symbol.source_range.byte_start <= range.byte_start
                && range.byte_end <= symbol.source_range.byte_end
        })
        .min_by_key(|symbol| symbol.source_range.byte_end - symbol.source_range.byte_start)
        .map(|symbol| symbol.id)
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
    use std::sync::Arc;

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
            name_range: None,
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
            let id = FileId::new(self.graph.file_count() as u32 + 1).unwrap();
            let idx = self.graph.add_node(NodeData::File(FileNode::new(
                id,
                PathBuf::from(path),
                lang,
                SnapshotId::new(1).unwrap(),
            )));
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        assert_eq!(resolution.file_edges.len(), 1);
        let (from, to, confidence) = resolution.file_edges[0];
        let py_idx = fx.graph.file_node_index(py_id).unwrap();
        assert_eq!(from, py_idx);
        assert_eq!(to, sym_idx);
        assert_eq!(confidence, 1.0);
        assert!(resolution.diagnostics.is_empty());
    }

    #[test]
    fn resolved_targets_keep_the_call_site_range() {
        let mut fx = Fixture::new();
        let (py_id, _) = fx.add_file("orchestrator.py", LangId::Python);
        let (js_id, _) = fx.add_file("math.js", LangId::JavaScript);
        let multiply = symbol(1, "multiply", "math.js", LangId::JavaScript);
        fx.add_symbol(&multiply, js_id, "math.js", LangId::JavaScript);
        let run = symbol(2, "run", "orchestrator.py", LangId::Python);
        fx.add_symbol(&run, py_id, "orchestrator.py", LangId::Python);
        let mut call = client_call("orchestrator.py", "multiply", 1.0);
        call.source_range = Some(test_range());

        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &[load_from_file("orchestrator.py", vec!["math.js"]), call],
            Path::new("."),
        );

        assert_eq!(resolution.resolved.len(), 1);
        let record = &resolution.resolved[0];
        assert_eq!(record.target, SymbolId::new(1).unwrap());
        assert_eq!(record.source_range, Some(test_range()));
        assert_eq!(record.confidence, 1.0);
        assert_eq!(
            resolution.symbol_edges,
            vec![(SymbolId::new(2).unwrap(), SymbolId::new(1).unwrap(), 1.0)],
            "the symbol projection keys on the enclosing caller"
        );
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        assert_eq!(resolution.file_edges.len(), 1);
        assert_eq!(resolution.file_edges[0].1, js_sym);
        assert_eq!(resolution.file_edges[0].2, 1.0);
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        let mut targets: Vec<_> = resolution.file_edges.iter().map(|(_, to, _)| *to).collect();
        targets.sort();
        let mut expected = vec![js_sym, py_sym];
        expected.sort();
        assert_eq!(targets, expected);
        assert!(resolution.file_edges.iter().all(|(_, _, c)| *c == 0.8));
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &[config_site],
            Path::new("."),
        );
        assert!(resolution.file_edges.is_empty());
        assert_eq!(resolution.diagnostics.len(), 1);
        assert_eq!(resolution.diagnostics[0].severity, Severity::Warning);
        assert_eq!(
            resolution.diagnostics[0].path,
            PathBuf::from(".").join("missing.conf.json")
        );
    }

    #[test]
    fn shared_broken_config_reports_once() {
        let mut fx = Fixture::new();
        fx.add_file("orchestrator.py", LangId::Python);
        let site = |source: &str| CallSite {
            source_file: PathBuf::from(source),
            caller_lang: LangId::Python,
            variant: CallSiteVariant::LoadFromConfiguration,
            target_lang: None,
            scripts: vec!["missing.conf.json".to_string()],
            function_name: None,
            is_async: false,
            source_range: None,
            confidence: 1.0,
        };
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &[site("orchestrator.py"), site("other.py")],
            Path::new("."),
        );
        assert_eq!(
            resolution.diagnostics.len(),
            1,
            "one file, one failure, one diagnostic: {:?}",
            resolution.diagnostics
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        assert_eq!(resolution.file_edges.len(), 2);
        let py_idx = fx.graph.file_node_index(py_id).unwrap();
        let mut targets: Vec<NodeIndex> =
            resolution.file_edges.iter().map(|(_, to, _)| *to).collect();
        targets.sort();
        let mut expected = vec![s1, s2];
        expected.sort();
        assert_eq!(targets, expected);
        for (from, _, confidence) in &resolution.file_edges {
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        assert_eq!(resolution.file_edges.len(), 1);
        let (from, to, confidence) = resolution.file_edges[0];
        let py_idx = fx.graph.file_node_index(py_id).unwrap();
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        assert_eq!(resolution.file_edges.len(), 2);
        let py_idx = fx.graph.file_node_index(py_id).unwrap();
        let mut targets: Vec<NodeIndex> =
            resolution.file_edges.iter().map(|(_, to, _)| *to).collect();
        targets.sort();
        let mut expected = vec![s1, s2];
        expected.sort();
        assert_eq!(targets, expected);
        for (from, _, confidence) in &resolution.file_edges {
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        assert_eq!(resolution.file_edges.len(), 1);
        let (from, to, confidence) = resolution.file_edges[0];
        let py_idx = fx.graph.file_node_index(py_id).unwrap();
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );

        assert!(resolution.file_edges.is_empty());
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
        let resolution = resolve_client_call_projections(
            &fx.graph,
            &fx.extractions,
            &call_sites,
            Path::new("."),
        );
        assert_eq!(resolution.file_edges.len(), 1);
        let (from, to, confidence) = resolution.file_edges[0];
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
