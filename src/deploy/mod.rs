use crate::deploy::scanner::{CallSite, CallSiteVariant, scan_file};
use crate::graph::edge::EdgeKind;
use crate::output::OutputFormat;
use std::collections::HashMap;
use std::path::PathBuf;

use rayon::prelude::*;

pub mod check;
pub mod cut;
pub mod dependency;
pub mod manifest;
pub mod mesh;
pub mod metrics;
pub mod pod;
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

    // 2. Run full pipeline graph analysis (covers extraction + SCC)
    let snapshot_id = crate::model::SnapshotId(1);
    let (mut analysis, _) = crate::pipeline::analyze_graph(&config.root, snapshot_id)?;

    // 3. Parallel scan for MetaCall call sites
    let all_call_sites: Vec<CallSite> = files
        .par_iter()
        .filter_map(|(path, lang)| {
            let source = std::fs::read(path).ok()?;
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&crate::language::grammar_for(*lang))
                .ok()?;
            let tree = parser.parse(&source, None)?;
            let sites = scan_file(*lang, &tree, &source, path);
            if sites.is_empty() { None } else { Some(sites) }
        })
        .flatten()
        .collect();

    // 4. Build path-to-node-index lookup once
    let mut path_to_idx: HashMap<PathBuf, petgraph::graph::NodeIndex> = HashMap::new();
    for idx in analysis.graph.graph.node_indices() {
        if let crate::graph::node::NodeData::File(f) = &analysis.graph.graph[idx] {
            path_to_idx.insert(f.path.clone(), idx);
        }
    }

    // 5. Inject LoadFromConfiguration call sites: read config JSON, expand to edges
    for site in &all_call_sites {
        if site.variant != CallSiteVariant::LoadFromConfiguration {
            continue;
        }
        let Some(config_script) = site.scripts.first() else {
            continue;
        };
        let config_file = config.root.join(config_script);
        let Ok(config_json) = std::fs::read_to_string(&config_file).and_then(|s| {
            serde_json::from_str::<serde_json::Value>(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        }) else {
            continue;
        };
        let Some(lang) = config_json.get("language_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(scripts_arr) = config_json.get("scripts").and_then(|v| v.as_array()) else {
            continue;
        };
        let Some(&from_idx) = path_to_idx.get(&site.source_file) else {
            continue;
        };
        let Some(target_lang) = crate::deploy::tags::from_metacall_tag(lang) else {
            continue;
        };
        for script_item in scripts_arr {
            let Some(script_str) = script_item.as_str() else {
                continue;
            };
            add_metacall_edge(
                &config.root,
                from_idx,
                target_lang,
                script_str,
                site.confidence as f32,
                &path_to_idx,
                &mut analysis,
            );
        }
    }

    // 6. Inject all other MetaCall load edges (file, memory, package)
    for site in &all_call_sites {
        if site.variant == CallSiteVariant::LoadFromConfiguration {
            continue;
        }
        let Some(target_lang_tag) = &site.target_lang else {
            continue;
        };
        let Some(&from_idx) = path_to_idx.get(&site.source_file) else {
            continue;
        };
        let Some(target_lang) = crate::deploy::tags::from_metacall_tag(target_lang_tag) else {
            continue;
        };
        for script in &site.scripts {
            add_metacall_edge(
                &config.root,
                from_idx,
                target_lang,
                script,
                site.confidence as f32,
                &path_to_idx,
                &mut analysis,
            );
        }
    }
    analysis.scc = crate::graph::scc::SccAnalysis::analyze(&analysis.graph.graph);

    // 7. Pod partitioning
    let partition = pod::partition_into_pods(&analysis.graph);
    let n_pods = partition.pods.len();
    let n_inter = partition.inter_pod_edges.len();

    // 8. Compute metrics from re-extraction
    let extraction = crate::extractor::extract(&files);
    let file_metrics = metrics::compute_file_metrics(&extraction.files);
    let pod_metrics = metrics::compute_pod_metrics(&partition, &file_metrics, &analysis.graph);

    // 9. Detect cross-language SCC cuts
    let lang_map: HashMap<_, _> = partition
        .file_languages
        .iter()
        .map(|(&fid, &lang)| (fid, lang))
        .collect();
    let mut all_cuts =
        cut::find_cross_language_cuts(&analysis.scc, &analysis.graph, &lang_map, &partition);

    // 10. Rebalance oversized pods
    for pod in &partition.pods {
        if let Some(cut) =
            cut::find_oversized_pod_cut(pod, &analysis.graph, cut::DEFAULT_MAX_POD_SIZE)
        {
            all_cuts.push(cut);
        }
    }

    // 11. Resolve external dependencies and scope per pod
    let dependencies = dependency::resolve_dependencies(&analysis.graph, &partition, &config.root);

    // 12. Generate pod manifest (includes dependency lists per pod)
    let pod_manifest = manifest::generate_pod_manifest(
        &partition,
        &pod_metrics,
        &all_cuts,
        &dependencies,
        &analysis.graph,
    );

    // 13. Generate mesh annotation
    let mesh = mesh::generate_mesh_annotation(&analysis, &all_call_sites);

    // 14. Write manifests or run checks
    if !config.check {
        std::fs::create_dir_all(&config.out)?;

        let manifest_json = serde_json::to_string_pretty(&pod_manifest)?;
        std::fs::write(config.out.join("metacall.pods.json"), manifest_json)?;

        let mesh_json = serde_json::to_string_pretty(&mesh)?;
        std::fs::write(config.out.join("metacall.mesh.json"), mesh_json)?;

        tracing::info!(
            "Generated pod manifest with {} deployments and {} inter-pod edges.",
            n_pods,
            n_inter
        );
    } else {
        let diagnostics = check::check_cut_fairness(&pod_manifest, &all_cuts);
        if diagnostics.is_empty() {
            println!("Check passed: no fairness issues in cut edges.");
        } else {
            println!("Check failed: found {} fairness issues.", diagnostics.len());
            for diag in &diagnostics {
                println!("  - {}", diag);
            }
            anyhow::bail!(
                "MetaCall deployment cut fairness check failed with {} issues",
                diagnostics.len()
            );
        }
    }

    Ok(())
}

/// Add a single MetaCall edge: from a source file node to either an
/// existing file node or a new ExternalNode.
///
/// Script resolution tries three strategies in order:
/// 1. `root.join(script)` -- works when script is relative to project root
/// 2. `source_dir.join(script)` -- resolves relative to the source file's directory
/// 3. Strip path prefix components from script until a matching file is found
///
/// Edges and external nodes are added through `CodeGraph` helpers so injected
/// edges obey the same dedup/confidence invariant as builder-constructed ones
/// and `external_index` stays consistent across repeated loads.
fn add_metacall_edge(
    root: &std::path::Path,
    from_idx: petgraph::graph::NodeIndex,
    target_lang: crate::language::LangId,
    script: &str,
    confidence: f32,
    path_to_idx: &HashMap<PathBuf, petgraph::graph::NodeIndex>,
    analysis: &mut crate::pipeline::GraphAnalysis,
) {
    let graph = &mut analysis.graph;

    // Strategy 1: root-relative
    let candidate = root.join(script);
    if let Some(&to_idx) = path_to_idx.get(&candidate) {
        graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
        return;
    }

    // Strategy 2: source-file-relative (strip script from its parent dir)
    if let crate::graph::node::NodeData::File(f) = &graph.graph[from_idx] {
        let source_dir = f.path.parent().unwrap_or(std::path::Path::new("."));
        let candidate = source_dir.join(script);
        if let Some(&to_idx) = path_to_idx.get(&candidate) {
            graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
            return;
        }
    }

    // Strategy 3: strip leading path components until filename matches
    let script_path = std::path::Path::new(script);
    let target_filename = script_path.file_name().unwrap_or(std::ffi::OsStr::new(""));
    for (path, &idx) in path_to_idx {
        if path.file_name() == Some(target_filename) || path.ends_with(script_path) {
            graph.add_edge_normalized(from_idx, idx, EdgeKind::Import, confidence);
            return;
        }
    }

    // Strategy 4: try component-stripping (pop prefixes from script)
    {
        let mut components: Vec<_> = script_path.components().collect();
        while components.len() > 1 {
            components.remove(0);
            let stripped: std::path::PathBuf = components.iter().collect();
            if let Some(&to_idx) = path_to_idx.get(&stripped) {
                graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
                return;
            }
        }
    }

    // No match: create or reuse ExternalNode (keeps external_index consistent).
    let to_idx = graph.get_or_create_external_node(script.to_string(), target_lang);
    graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
}
