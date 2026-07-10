use crate::deploy::scanner::CallSiteVariant;
use crate::output::OutputFormat;
use std::collections::HashMap;
use std::path::PathBuf;

pub mod check;
pub mod manifest;
pub mod mesh;
pub mod scanner;
pub mod tags;

pub struct DeployConfig {
    pub root: PathBuf,
    pub out: PathBuf,
    pub format: OutputFormat,
    pub check: bool,
}

pub fn run_deploy(config: DeployConfig) -> anyhow::Result<()> {
    tracing::info!("Starting MetaCall deployment manifest generation");
    tracing::info!("Root path: {}", config.root.display());
    tracing::info!("Output path: {}", config.out.display());
    tracing::info!("Check mode: {}", config.check);

    // 1. Discover files
    let files = crate::input::discover_files(&config.root, None)?;
    let mut all_call_sites = Vec::new();

    // 2. Scan for call sites
    for (path, lang) in &files {
        let source = std::fs::read(path)?;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&crate::language::grammar_for(*lang))
            .map_err(|e| anyhow::anyhow!("failed to load grammar for {lang:?}: {e}"))?;

        if let Some(tree) = parser.parse(&source, None) {
            let sites = scanner::scan_file(*lang, &tree, &source, path);
            if !sites.is_empty() {
                all_call_sites.extend(sites);
            }
        }
    }

    // 3. Run full graph analysis for SCCs
    let snapshot_id = crate::model::SnapshotId(1);
    let (mut analysis, _) = crate::pipeline::analyze_graph(&config.root, snapshot_id)?;

    // 4. Inject MetaCall loads into the graph and recompute SCC
    {
        let mut path_to_idx = HashMap::new();
        for idx in analysis.graph.graph.node_indices() {
            if let crate::graph::node::NodeData::File(f) = &analysis.graph.graph[idx] {
                path_to_idx.insert(f.path.clone(), idx);
            }
        }

        for site in &all_call_sites {
            if site.variant == CallSiteVariant::LoadFromConfiguration {
                continue;
            }

            if let Some(target_lang_tag) = &site.target_lang
                && let (Some(&from_idx), Some(target_lang)) = (
                    path_to_idx.get(&site.source_file),
                    crate::deploy::tags::from_metacall_tag(target_lang_tag),
                )
            {
                for script in &site.scripts {
                    let target_path = config.root.join(script);
                    let to_idx = if let Some(&to_idx) = path_to_idx.get(&target_path) {
                        to_idx
                    } else {
                        // Add external node if not found
                        let node = crate::graph::node::NodeData::External(
                            crate::graph::node::ExternalNode {
                                raw_path: script.clone(),
                                language: target_lang,
                            },
                        );
                        analysis.graph.graph.add_node(node)
                    };

                    analysis.graph.graph.add_edge(
                        from_idx,
                        to_idx,
                        crate::graph::edge::EdgeData {
                            kind: crate::graph::edge::EdgeKind::Import,
                            confidence: site.confidence as f32,
                        },
                    );
                }
            }
        }
        // Recompute SCC
        analysis.scc = crate::graph::scc::SccAnalysis::analyze(&analysis.graph.graph);
    }

    // 5. Generate manifests
    let (manifests, root_manifest) =
        manifest::generate_manifests(&config.root, &files, &all_call_sites);

    // 5. Generate mesh annotation
    let mesh = mesh::generate_mesh_annotation(&analysis, &all_call_sites);

    // 6. Write manifests to output directory
    if !config.check {
        std::fs::create_dir_all(&config.out)?;

        // Write root manifest
        let root_json = serde_json::to_string_pretty(&root_manifest)?;
        std::fs::write(config.out.join("metacall.json"), root_json)?;

        // Write per-language manifests
        for (lang, manifest) in &manifests {
            let json = serde_json::to_string_pretty(manifest)?;
            std::fs::write(config.out.join(format!("metacall.{}.json", lang)), json)?;
        }

        // Write mesh annotation
        let mesh_json = serde_json::to_string_pretty(&mesh)?;
        std::fs::write(config.out.join("metacall.mesh.json"), mesh_json)?;

        println!(
            "Generated {} manifests and mesh annotation.",
            manifests.len() + 1
        );
    } else {
        // 7. Check mode
        let diagnostics = check::check_manifests(&config.root, &manifests, &root_manifest)?;
        if diagnostics.is_empty() {
            println!("Check passed: existing manifests match analysis.");
        } else {
            println!("Check failed: found {} divergences.", diagnostics.len());
            for diag in &diagnostics {
                println!("  - {}", diag);
            }
            anyhow::bail!(
                "MetaCall deployment check failed with {} divergences",
                diagnostics.len()
            );
        }
    }

    Ok(())
}
