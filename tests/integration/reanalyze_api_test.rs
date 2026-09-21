//! External-crate tests for the extraction cache and re-analysis seams.
use std::io::Write;
use std::path::{Path, PathBuf};

use meta_ast::model::{SnapshotId, SymbolId};
use meta_ast::{
    ExtractionCache, FileExtraction, GraphBuilder, LangId, Overlay, WatchState, fingerprint,
    reanalyze_extractions,
};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meta_ast_lsp_seams_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(root: &Path, name: &str, content: &str) -> PathBuf {
    let path = root.join(name);
    let mut file = std::fs::File::create(&path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
    path
}

fn overlay(path: &Path, content: &str) -> Overlay {
    Overlay {
        uri: format!("file://{}", path.display()),
        path: path.to_path_buf(),
        text: content.to_string(),
        version: 2,
        lang: LangId::Python,
    }
}

#[test]
fn cache_public_surface() {
    let root = temp_dir("cache");
    let a = write_file(&root, "a.py", "def greet(): pass\n");
    let bytes = std::fs::read(&a).unwrap();
    let fp = fingerprint(&bytes);

    let mut cache = ExtractionCache::new();
    assert!(cache.is_empty());
    let extraction = FileExtraction::empty(a.clone(), LangId::Python);
    cache.update(a.clone(), fp, std::sync::Arc::new(extraction));

    assert_eq!(cache.len(), 1);
    assert_eq!(cache.fingerprint_of(&a), Some(fp));
    assert!(cache.get(&a).is_some());
    assert_eq!(cache.max_symbol_id(), 0);

    cache.remove(&a);
    assert!(cache.is_empty());
}

#[test]
fn reanalyze_extractions_reuses_and_overrides() {
    let root = temp_dir("reanalyze");
    let a = write_file(&root, "a.py", "def greet(): pass\n");
    write_file(&root, "b.py", "def caller(): pass\n");

    let mut state = WatchState::new();
    let (first, cs, _) = reanalyze_extractions(&root, None, &[], &mut state).unwrap();
    assert_eq!(cs.files_added, 2);
    assert_eq!(first.len(), 2);

    let (second, cs2, _) = reanalyze_extractions(&root, None, &[], &mut state).unwrap();
    assert_eq!(cs2.files_unchanged, 2);
    assert_eq!(second.len(), 2);

    let edited = overlay(&a, "def greet_v2(): pass\n");
    let (third, cs3, _) =
        reanalyze_extractions(&root, None, std::slice::from_ref(&edited), &mut state).unwrap();
    assert_eq!(cs3.files_modified, 1);
    let names: Vec<&str> = third
        .iter()
        .flat_map(|file| file.symbols.iter().map(|symbol| symbol.name.as_str()))
        .collect();
    assert!(names.contains(&"greet_v2"));
}

#[test]
fn graph_and_scope_cache_are_public() {
    let root = temp_dir("scope");
    write_file(&root, "a.py", "def greet(): pass\n");

    let mut state = WatchState::new();
    let (extractions, _, _) = reanalyze_extractions(&root, None, &[], &mut state).unwrap();

    let mut diagnostics = Vec::new();
    let (graph, _scc, scope) = GraphBuilder::from_extractions_with_scope(
        &extractions,
        &root,
        SnapshotId::new(1).unwrap(),
        &mut diagnostics,
    );

    let (file_id, _file) = graph.files().next().unwrap();
    let resolved = scope.resolve(file_id, "greet").expect("greet is visible");
    assert!(!resolved.is_empty());
    assert!(scope.scope(file_id).is_some());
    assert_eq!(scope.iter_scopes().count(), 1);
}

#[cfg(feature = "metacall-deploy")]
#[test]
fn graph_includes_metacall_client_call_edges() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed/client_call_mesh");
    let mut state = WatchState::new();
    let (extractions, _, _) = reanalyze_extractions(&root, None, &[], &mut state).unwrap();

    let mut diagnostics = Vec::new();
    let (graph, _scc, _scope) = GraphBuilder::from_extractions_with_scope(
        &extractions,
        &root,
        SnapshotId::new(1).unwrap(),
        &mut diagnostics,
    );

    // The fixture has no imports or references. Every Reference edge comes
    // from the `metacall("multiply", ...)` client call.
    let references = graph.edges_of_kind(meta_ast::EdgeKind::Reference).count();
    assert!(references > 0, "expected a metacall client-call edge");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("no_such_function")),
        "expected an unresolved invocation diagnostic"
    );
}

/// A call through a file alias of the metacall module resolves like a bare call.
#[cfg(feature = "metacall-deploy")]
#[test]
fn an_aliased_member_call_keeps_the_load_aware_confidence() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed/aliased_client_call");
    let mut state = WatchState::new();
    let (extractions, _, _) = reanalyze_extractions(&root, None, &[], &mut state).unwrap();

    let sites: Vec<_> = extractions
        .iter()
        .flat_map(|extraction| extraction.call_sites.iter())
        .collect();
    assert_eq!(sites.len(), 3, "one load and two aliased calls: {sites:?}");
    assert!(
        sites
            .iter()
            .any(|site| site.variant == meta_ast::deploy::scanner::CallSiteVariant::LoadFromFile),
        "the aliased load is found"
    );

    let mut diagnostics = Vec::new();
    let (graph, _scc, _scope) = GraphBuilder::from_extractions_with_scope(
        &extractions,
        &root,
        SnapshotId::new(1).unwrap(),
        &mut diagnostics,
    );

    let caller = symbol_id(&extractions, "orchestrator.js", "compute_total");
    let target = symbol_id(&extractions, "math.py", "multiply");
    let edge = graph
        .reference_edges()
        .find(|(source, target_id, _)| *source == caller && *target_id == target);
    assert_eq!(
        edge.map(|(_, _, confidence)| confidence),
        Some(meta_ast::graph::edge::CONFIDENCE_CLIENT_UNIQUE_LOAD),
        "a unique load-aware target keeps the ladder value"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("no_such_function")),
        "an unresolved aliased target still reports"
    );
}

#[cfg(feature = "metacall-deploy")]
fn symbol_id(extractions: &[std::sync::Arc<FileExtraction>], file: &str, name: &str) -> SymbolId {
    extractions
        .iter()
        .filter(|extraction| extraction.path.ends_with(file))
        .flat_map(|extraction| extraction.symbols.iter())
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("{name} in {file}"))
        .id
}
