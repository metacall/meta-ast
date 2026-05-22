use std::collections::HashMap;

use clap::Parser;
use meta_ast::graph::resolver;
use meta_ast::graph::{GraphBuilder, SccAnalysis};
use meta_ast::interface::args::Cli;
use meta_ast::model::SnapshotId;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli {
        Cli::Inspect(args) => {
            let files = meta_ast::input::discover_files(&args.path, None)?;

            let result = meta_ast::extractor::extract(&files);

            let symbols: Vec<_> = result
                .files
                .iter()
                .flat_map(|f| f.symbols.iter().cloned())
                .collect();

            for file in &result.files {
                for diag in &file.diagnostics {
                    eprintln!(
                        "[{:?}] {}: {}",
                        diag.severity,
                        diag.path.display(),
                        diag.message
                    );
                }
            }

            let content = meta_ast::output::inspect::serialize_inspect(&symbols, &args.format)?;

            match args.output {
                Some(path) => std::fs::write(&path, &content)?,
                None => println!("{content}"),
            }

            Ok(())
        }

        Cli::Graph(args) => {
            let files = meta_ast::input::discover_files(&args.path, None)?;
            let snapshot_id = SnapshotId(1);
            let mut builder = GraphBuilder::new(snapshot_id);

            for (path, lang) in &files {
                builder.add_file(path.clone(), *lang);
            }

            let result = meta_ast::extractor::extract(&files);

            for file in &result.files {
                for diag in &file.diagnostics {
                    eprintln!(
                        "[{:?}] {}: {}",
                        diag.severity,
                        diag.path.display(),
                        diag.message
                    );
                }
            }

            // Pass 2: add symbols and import edges
            for file in &result.files {
                for symbol in &file.symbols {
                    builder.add_symbol(symbol)?;
                }
            }

            // Resolve import edges: convert UnresolvedImport → builder.add_import
            let mut path_to_file_id: HashMap<std::path::PathBuf, meta_ast::model::FileId> =
                HashMap::new();
            for file in &result.files {
                if let Some(fid) = builder.file_id_for_path(&file.path) {
                    path_to_file_id.insert(file.path.clone(), fid);
                }
            }

            for file in &result.files {
                let Some(&source_fid) = path_to_file_id.get(&file.path) else {
                    continue;
                };
                for import in &file.imports {
                    if let Some(dir) = file.path.parent()
                        && let Some(target) =
                            resolver::normalize_import_path(dir, &import.namespace, file.lang)
                    {
                        builder.add_import(source_fid, target);
                    }
                }
            }

            // Pass 3: build scope cache and resolve references
            let symbol_index = resolver::build_symbol_index(&result.files, &path_to_file_id);
            let import_adjacency = builder.import_adjacency();
            let file_languages: HashMap<_, _> = result
                .files
                .iter()
                .filter_map(|f| Some((path_to_file_id.get(&f.path)?.to_owned(), f.lang)))
                .collect();
            let file_paths: HashMap<_, _> = path_to_file_id
                .iter()
                .map(|(path, &fid)| (fid, path.clone()))
                .collect();
            let mut diagnostics: Vec<meta_ast::error::Diagnostic> = Vec::new();
            let scope_cache = resolver::FlattenedScopeCache::build(
                &symbol_index,
                &import_adjacency,
                &file_languages,
                &file_paths,
                &mut diagnostics,
            );
            let ref_edges = resolver::resolve_all_references(
                &result.files,
                &path_to_file_id,
                &scope_cache,
                &mut diagnostics,
            );

            for diag in &diagnostics {
                eprintln!(
                    "[{:?}] {}: {}",
                    diag.severity,
                    diag.path.display(),
                    diag.message
                );
            }

            // Pass 4: add reference edges
            for (from, to) in ref_edges {
                builder.add_reference(from, to);
            }

            let graph = builder.build();
            let scc_analysis = SccAnalysis::analyze(&graph.graph);

            if args.html {
                let html = meta_ast::output::dashboard::to_graph_html(
                    &graph,
                    &scc_analysis,
                    snapshot_id.0 as u64,
                    args.self_contained,
                )?;
                let path = args.output.unwrap_or_else(|| {
                    let name = args
                        .path
                        .file_stem()
                        .map(|s: &std::ffi::OsStr| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "project".to_string());
                    std::path::PathBuf::from(format!("{}.metast", name))
                });
                let path_str = path.to_string_lossy().to_string();
                std::fs::write(&path, &html)?;
                if let Err(e) = webbrowser::open(&path_str) {
                    eprintln!("Warning: could not open browser: {e}");
                }
            } else {
                let content = meta_ast::output::graph::serialize_graph(
                    &graph,
                    &scc_analysis,
                    snapshot_id.0 as u64,
                    &args.format,
                )?;
                match args.output {
                    Some(path) => std::fs::write(&path, &content)?,
                    None => println!("{content}"),
                }
            }

            Ok(())
        }
    }
}
