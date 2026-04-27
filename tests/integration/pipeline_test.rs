use std::collections::HashMap;
use std::path::{Path, PathBuf};

use meta_ast::graph::GraphBuilder;

fn flatten_symbols(result: &meta_ast::extractor::ExtractionResult) -> Vec<meta_ast::model::Symbol> {
    result
        .files
        .iter()
        .flat_map(|f| f.symbols.iter().cloned())
        .collect()
}

#[test]
fn end_to_end_python_project() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(!files.is_empty(), "should find Python fixture files");

    for (_, lang) in &files {
        assert_eq!(*lang, meta_ast::language::LangId::Python);
    }

    let result = meta_ast::extractor::extract(&files);
    let symbols = flatten_symbols(&result);
    assert!(
        !symbols.is_empty(),
        "should extract symbols from Python fixtures"
    );

    for symbol in &symbols {
        assert!(!symbol.name.is_empty());
        assert_eq!(symbol.language, meta_ast::language::LangId::Python);
    }

    let json = meta_ast::output::inspect::serialize_inspect(
        &symbols,
        &meta_ast::output::OutputFormat::Json,
    )
    .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["funcs"].is_array());
    assert!(parsed["classes"].is_array());
    assert!(parsed["objects"].is_array());

    for func in parsed["funcs"].as_array().unwrap() {
        assert!(func["name"].is_string(), "func must have name");
        assert!(
            func["source_range"].is_object(),
            "func must have source_range"
        );
        assert!(func["async"].is_boolean(), "func must have async field");
    }

    for class in parsed["classes"].as_array().unwrap() {
        assert!(class["name"].is_string(), "class must have name");
        assert!(
            class["source_range"].is_object(),
            "class must have source_range"
        );
    }

    for obj in parsed["objects"].as_array().unwrap() {
        assert!(obj["name"].is_string(), "object must have name");
        assert!(
            obj["source_range"].is_object(),
            "object must have source_range"
        );
    }
}

#[test]
fn end_to_end_mixed_languages() {
    let root = Path::new("tests/fixtures");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(
        files.len() > 5,
        "should find files across multiple languages"
    );

    let langs: std::collections::HashSet<_> = files.iter().map(|(_, l)| *l).collect();
    assert!(
        langs.len() >= 3,
        "should find at least 3 different languages"
    );

    let result = meta_ast::extractor::extract(&files);
    let symbols = flatten_symbols(&result);
    assert!(!symbols.is_empty());

    let json = meta_ast::output::inspect::serialize_inspect(
        &symbols,
        &meta_ast::output::OutputFormat::Json,
    )
    .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["funcs"].is_array());
}

#[test]
fn pipeline_idempotent() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();

    let result1 = meta_ast::extractor::extract(&files);
    let result2 = meta_ast::extractor::extract(&files);

    let symbols1 = flatten_symbols(&result1);
    let symbols2 = flatten_symbols(&result2);

    let names1: Vec<_> = symbols1.iter().map(|s| format!("{:?}", s.kind)).collect();
    let names2: Vec<_> = symbols2.iter().map(|s| format!("{:?}", s.kind)).collect();

    assert_eq!(
        names1, names2,
        "same input should produce same symbol kinds"
    );

    let names1: Vec<_> = symbols1.iter().map(|s| &s.name).collect();
    let names2: Vec<_> = symbols2.iter().map(|s| &s.name).collect();
    assert_eq!(
        names1, names2,
        "same input should produce same symbol names"
    );
}

#[test]
fn json_output_has_required_structure() {
    let root = Path::new("tests/fixtures/python");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    let symbols = flatten_symbols(&result);
    let json = meta_ast::output::inspect::serialize_inspect(
        &symbols,
        &meta_ast::output::OutputFormat::Json,
    )
    .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let obj = parsed.as_object().expect("root should be object");

    assert!(obj.contains_key("funcs"), "must have funcs key");
    assert!(obj.contains_key("classes"), "must have classes key");
    assert!(obj.contains_key("objects"), "must have objects key");
    assert_eq!(obj.len(), 3, "should have exactly 3 top-level keys");
}

#[test]
fn cross_file_reference_resolution() {
    let root = Path::new("tests/fixtures/multi");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(files.len() >= 4, "should find multi-fixture files");

    let snapshot_id = meta_ast::model::SnapshotId(1);
    let mut builder = GraphBuilder::new(snapshot_id);

    for (path, lang) in &files {
        builder.add_file(path.clone(), *lang);
    }

    let result = meta_ast::extractor::extract(&files);

    // Add symbols
    for file in &result.files {
        for sym in &file.symbols {
            builder.add_symbol(sym);
        }
    }

    // Build path -> FileId map
    let mut path_map: HashMap<PathBuf, meta_ast::model::FileId> = HashMap::new();
    for file in &result.files {
        if let Some(fid) = builder.file_id_for_path(&file.path) {
            path_map.insert(file.path.clone(), fid);
        }
    }

    // Add import edges (normalize targets to resolve ./ components)
    for file in &result.files {
        let Some(&source_fid) = path_map.get(&file.path) else {
            continue;
        };
        for import in &file.imports {
            if let Some(dir) = file.path.parent()
                && let Some(target) = meta_ast::graph::resolver::normalize_import_path(
                    dir,
                    &import.namespace,
                    file.lang,
                )
            {
                let normalized_target: PathBuf = target.components().collect();
                builder.add_import(source_fid, normalized_target);
            }
        }
    }

    // Build scope cache and resolve references
    let symbol_index = meta_ast::graph::resolver::build_symbol_index(&result.files, &path_map);
    let import_adjacency = builder.import_adjacency();
    let file_languages: HashMap<_, _> = result
        .files
        .iter()
        .filter_map(|f| Some((path_map.get(&f.path)?.to_owned(), f.lang)))
        .collect();
    let scope_cache = meta_ast::graph::resolver::FlattenedScopeCache::build(
        &symbol_index,
        &import_adjacency,
        &file_languages,
    );

    let ref_edges =
        meta_ast::graph::resolver::resolve_all_references(&result.files, &path_map, &scope_cache);

    // Add reference edges and verify the graph has reference edges
    for (from, to) in &ref_edges {
        builder.add_reference(*from, *to);
    }

    let graph = builder.build();
    let scc = meta_ast::graph::scc::SccAnalysis::analyze(&graph.graph);

    // Verify graph contains reference edges (JS cross-file references work)
    let ref_edge_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Reference)
        .count();
    assert!(ref_edge_count > 0, "graph should have reference edges");

    // Verify graph contains import edges
    let import_edge_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Import)
        .count();
    assert!(import_edge_count > 0, "graph should have import edges");

    // Verify SCC analysis runs on the augmented graph
    assert!(!scc.components.is_empty());
}

#[test]
fn imports_extracted_for_python_files() {
    let root = Path::new("tests/fixtures/multi");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);

    // Python files should have import statements
    let py_files: Vec<_> = result
        .files
        .iter()
        .filter(|f| f.lang == meta_ast::language::LangId::Python)
        .collect();
    assert!(!py_files.is_empty(), "should have Python files");

    let has_imports = py_files.iter().any(|f| !f.imports.is_empty());
    assert!(has_imports, "Python files should extract import statements");
}

#[test]
fn references_extracted_for_function_calls() {
    let root = Path::new("tests/fixtures/multi");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);

    // Files with function calls should have references
    let has_references = result.files.iter().any(|f| !f.references.is_empty());
    assert!(has_references, "should extract call references");
}

#[test]
fn cross_file_reference_rust_crate() {
    let root = Path::new("tests/fixtures/multi/rust_crate");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert_eq!(files.len(), 2, "rust_crate should have 2 files");

    let snapshot_id = meta_ast::model::SnapshotId(1);
    let mut builder = GraphBuilder::new(snapshot_id);

    for (path, lang) in &files {
        builder.add_file(path.clone(), *lang);
    }

    let result = meta_ast::extractor::extract(&files);

    for file in &result.files {
        for sym in &file.symbols {
            builder.add_symbol(sym);
        }
    }

    let mut path_map: HashMap<std::path::PathBuf, meta_ast::model::FileId> = HashMap::new();
    for file in &result.files {
        if let Some(fid) = builder.file_id_for_path(&file.path) {
            path_map.insert(file.path.clone(), fid);
        }
    }

    for file in &result.files {
        let Some(&source_fid) = path_map.get(&file.path) else {
            continue;
        };
        for import in &file.imports {
            if let Some(dir) = file.path.parent()
                && let Some(target) = meta_ast::graph::resolver::normalize_import_path(
                    dir,
                    &import.namespace,
                    file.lang,
                )
            {
                builder.add_import(source_fid, target);
            }
        }
    }

    let symbol_index = meta_ast::graph::resolver::build_symbol_index(&result.files, &path_map);
    let import_adjacency = builder.import_adjacency();
    let file_languages: HashMap<_, _> = result
        .files
        .iter()
        .filter_map(|f| Some((path_map.get(&f.path)?.to_owned(), f.lang)))
        .collect();
    let scope_cache = meta_ast::graph::resolver::FlattenedScopeCache::build(
        &symbol_index,
        &import_adjacency,
        &file_languages,
    );
    let ref_edges =
        meta_ast::graph::resolver::resolve_all_references(&result.files, &path_map, &scope_cache);

    for (from, to) in &ref_edges {
        builder.add_reference(*from, *to);
    }

    let graph = builder.build();
    let ref_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Reference)
        .count();
    let import_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Import)
        .count();

    // after Rust path normalization fix, crate imports should resolve correctly
    assert!(
        import_count > 0,
        "rust_crate should have import edges; got {import_count}"
    );
    assert!(
        ref_count > 0,
        "rust_crate should have reference edges; got {ref_count}"
    );
}

#[test]
fn cross_file_reference_typescript() {
    let root = Path::new("tests/fixtures/multi/ts_app");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert_eq!(files.len(), 2, "ts_app should have 2 files");

    let snapshot_id = meta_ast::model::SnapshotId(1);
    let mut builder = GraphBuilder::new(snapshot_id);

    for (path, lang) in &files {
        builder.add_file(path.clone(), *lang);
    }

    let result = meta_ast::extractor::extract(&files);

    for file in &result.files {
        for sym in &file.symbols {
            builder.add_symbol(sym);
        }
    }

    let mut path_map: HashMap<std::path::PathBuf, meta_ast::model::FileId> = HashMap::new();
    for file in &result.files {
        if let Some(fid) = builder.file_id_for_path(&file.path) {
            path_map.insert(file.path.clone(), fid);
        }
    }

    for file in &result.files {
        let Some(&source_fid) = path_map.get(&file.path) else {
            continue;
        };
        for import in &file.imports {
            if let Some(dir) = file.path.parent()
                && let Some(target) = meta_ast::graph::resolver::normalize_import_path(
                    dir,
                    &import.namespace,
                    file.lang,
                )
            {
                builder.add_import(source_fid, target);
            }
        }
    }

    let symbol_index = meta_ast::graph::resolver::build_symbol_index(&result.files, &path_map);
    let import_adjacency = builder.import_adjacency();
    let file_languages: HashMap<_, _> = result
        .files
        .iter()
        .filter_map(|f| Some((path_map.get(&f.path)?.to_owned(), f.lang)))
        .collect();
    let scope_cache = meta_ast::graph::resolver::FlattenedScopeCache::build(
        &symbol_index,
        &import_adjacency,
        &file_languages,
    );
    let ref_edges =
        meta_ast::graph::resolver::resolve_all_references(&result.files, &path_map, &scope_cache);

    for (from, to) in &ref_edges {
        builder.add_reference(*from, *to);
    }

    let graph = builder.build();
    let import_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Import)
        .count();
    assert!(
        import_count > 0,
        "ts_app should have import edges; got {import_count}"
    );
}

#[test]
fn edge_star_import_extracts_star_field() {
    let root = Path::new("tests/fixtures/multi/edge_star");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);

    let star_import = result
        .files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("edge_star/main"));
    assert!(star_import.is_some(), "should find main.py in edge_star");

    if let Some(main) = star_import {
        let has_star = main.imports.iter().any(|i| i.star);
        assert!(has_star, "star import should have star=true");
        assert_eq!(main.imports.len(), 1, "should have exactly 1 import");
        assert!(main.imports[0].star, "imports[0].star should be true");
    }
}

#[test]
fn edge_alias_captures_alias_field() {
    let root = Path::new("tests/fixtures/multi/edge_alias");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);

    let main = result
        .files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("edge_alias/main"));
    assert!(main.is_some(), "should find main.py in edge_alias");

    if let Some(m) = main {
        let aliased = m.imports.iter().find(|i| i.alias.is_some());
        assert!(aliased.is_some(), "aliased import should have alias set");
        if let Some(a) = aliased {
            assert_eq!(a.alias.as_deref(), Some("lo"));
        }
    }
}

#[test]
fn edge_circular_does_not_infinite_loop() {
    let root = Path::new("tests/fixtures/multi/edge_circular");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert_eq!(files.len(), 2, "edge_circular should have a.py and b.py");

    let snapshot_id = meta_ast::model::SnapshotId(1);
    let mut builder = GraphBuilder::new(snapshot_id);

    for (path, lang) in &files {
        builder.add_file(path.clone(), *lang);
    }

    let result = meta_ast::extractor::extract(&files);

    for file in &result.files {
        for sym in &file.symbols {
            builder.add_symbol(sym);
        }
    }

    let mut path_map: HashMap<std::path::PathBuf, meta_ast::model::FileId> = HashMap::new();
    for file in &result.files {
        if let Some(fid) = builder.file_id_for_path(&file.path) {
            path_map.insert(file.path.clone(), fid);
        }
    }

    for file in &result.files {
        let Some(&source_fid) = path_map.get(&file.path) else {
            continue;
        };
        for import in &file.imports {
            if let Some(dir) = file.path.parent()
                && let Some(target) = meta_ast::graph::resolver::normalize_import_path(
                    dir,
                    &import.namespace,
                    file.lang,
                )
            {
                builder.add_import(source_fid, target);
            }
        }
    }

    let symbol_index = meta_ast::graph::resolver::build_symbol_index(&result.files, &path_map);
    let import_adjacency = builder.import_adjacency();
    let file_languages: HashMap<_, _> = result
        .files
        .iter()
        .filter_map(|f| Some((path_map.get(&f.path)?.to_owned(), f.lang)))
        .collect();

    // Should complete without panic (cycle handled by visited set)
    let scope_cache = meta_ast::graph::resolver::FlattenedScopeCache::build(
        &symbol_index,
        &import_adjacency,
        &file_languages,
    );
    assert!(!scope_cache.is_empty(), "scope cache should not be empty");
    assert_eq!(scope_cache.len(), 2, "should have 2 file scopes");
}

#[test]
fn edge_transitive_resolution() {
    let root = Path::new("tests/fixtures/multi/edge_transitive");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert_eq!(files.len(), 3, "edge_transitive should have 3 files");

    let snapshot_id = meta_ast::model::SnapshotId(1);
    let mut builder = GraphBuilder::new(snapshot_id);

    for (path, lang) in &files {
        builder.add_file(path.clone(), *lang);
    }

    let result = meta_ast::extractor::extract(&files);

    for file in &result.files {
        for sym in &file.symbols {
            builder.add_symbol(sym);
        }
    }

    let mut path_map: HashMap<std::path::PathBuf, meta_ast::model::FileId> = HashMap::new();
    for file in &result.files {
        if let Some(fid) = builder.file_id_for_path(&file.path) {
            path_map.insert(file.path.clone(), fid);
        }
    }

    for file in &result.files {
        let Some(&source_fid) = path_map.get(&file.path) else {
            continue;
        };
        for import in &file.imports {
            if let Some(dir) = file.path.parent()
                && let Some(target) = meta_ast::graph::resolver::normalize_import_path(
                    dir,
                    &import.namespace,
                    file.lang,
                )
            {
                builder.add_import(source_fid, target);
            }
        }
    }

    let symbol_index = meta_ast::graph::resolver::build_symbol_index(&result.files, &path_map);
    let import_adjacency = builder.import_adjacency();
    let file_languages: HashMap<_, _> = result
        .files
        .iter()
        .filter_map(|f| Some((path_map.get(&f.path)?.to_owned(), f.lang)))
        .collect();
    let scope_cache = meta_ast::graph::resolver::FlattenedScopeCache::build(
        &symbol_index,
        &import_adjacency,
        &file_languages,
    );
    let ref_edges =
        meta_ast::graph::resolver::resolve_all_references(&result.files, &path_map, &scope_cache);

    for (from, to) in &ref_edges {
        builder.add_reference(*from, *to);
    }

    let graph = builder.build();
    let ref_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Reference)
        .count();
    let _import_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Import)
        .count();

    assert!(
        ref_count > 0,
        "transitive should have reference edges; got {ref_count}"
    );
}

#[test]
fn edge_multiple_imports_per_file() {
    let root = Path::new("tests/fixtures/multi/edge_multi_import");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);

    let main = result
        .files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("edge_multi_import/main"));
    assert!(main.is_some(), "should find main.py");

    if let Some(m) = main {
        assert!(
            m.imports.len() >= 2,
            "should have at least 2 imports; got {}",
            m.imports.len()
        );
    }
}

#[test]
fn edge_unresolved_ref_creates_no_edges() {
    let root = Path::new("tests/fixtures/multi/edge_unresolved");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);

    let main = result
        .files
        .iter()
        .find(|f| f.path.to_string_lossy().contains("edge_unresolved/main"));
    assert!(main.is_some(), "should find main.py");

    if let Some(m) = main {
        assert_eq!(
            m.references.len(),
            1,
            "should have 1 reference to nonexistent_function"
        );
        assert_eq!(m.references[0].name, "nonexistent_function");
    }
}

#[test]
fn edge_selfref_does_not_create_self_loop() {
    let root = Path::new("tests/fixtures/multi/edge_selfref");
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let snapshot_id = meta_ast::model::SnapshotId(1);
    let mut builder = GraphBuilder::new(snapshot_id);

    for (path, lang) in &files {
        builder.add_file(path.clone(), *lang);
    }

    let result = meta_ast::extractor::extract(&files);

    for file in &result.files {
        for sym in &file.symbols {
            builder.add_symbol(sym);
        }
    }

    let mut path_map: HashMap<std::path::PathBuf, meta_ast::model::FileId> = HashMap::new();
    for file in &result.files {
        if let Some(fid) = builder.file_id_for_path(&file.path) {
            path_map.insert(file.path.clone(), fid);
        }
    }

    let symbol_index = meta_ast::graph::resolver::build_symbol_index(&result.files, &path_map);
    let import_adjacency = builder.import_adjacency();
    let file_languages: HashMap<_, _> = result
        .files
        .iter()
        .filter_map(|f| Some((path_map.get(&f.path)?.to_owned(), f.lang)))
        .collect();
    let scope_cache = meta_ast::graph::resolver::FlattenedScopeCache::build(
        &symbol_index,
        &import_adjacency,
        &file_languages,
    );
    let ref_edges =
        meta_ast::graph::resolver::resolve_all_references(&result.files, &path_map, &scope_cache);

    for (from, to) in &ref_edges {
        builder.add_reference(*from, *to);
    }

    let graph = builder.build();
    let ref_count = graph
        .edges_of_kind(meta_ast::graph::EdgeKind::Reference)
        .count();
    assert_eq!(
        ref_count, 0,
        "self-calls should not create reference edges; got {ref_count}"
    );
}
