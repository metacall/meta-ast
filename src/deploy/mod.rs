use crate::deploy::scanner::{CallSite, CallSiteVariant};
use crate::error::{Diagnostic, Severity};
use crate::graph::edge::EdgeKind;
use crate::output::OutputFormat;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub mod check;
pub mod client_call;
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
    pub max_pod_size: usize,
}

pub fn run_deploy(config: DeployConfig) -> anyhow::Result<()> {
    tracing::info!("Starting MetaCall deployment manifest generation");
    tracing::info!("Root path: {}", config.root.display());
    tracing::info!("Output path: {}", config.out.display());
    tracing::info!("Check mode: {}", config.check);
    if config.max_pod_size == 0 {
        anyhow::bail!("max_pod_size must be at least 1");
    }

    // 1. Run full pipeline graph analysis (covers extraction + SCC)
    let snapshot_id = crate::model::SnapshotId::new(1).unwrap();
    let (mut analysis, mut diagnostics) =
        crate::pipeline::analyze_graph(&config.root, snapshot_id, None)?;

    // 2. Collect MetaCall call sites from pipeline extractions (zero duplicate
    // I/O / parsing for the source scan; configuration JSONs referenced by
    // LoadFromConfiguration are re-read once for edge injection and once for
    // client-call Phase A resolution).
    let all_call_sites: Vec<CallSite> = analysis
        .extractions
        .iter()
        .flat_map(|f| f.call_sites.clone())
        .collect();

    // 4. Build path-to-node-index lookup once
    let mut path_to_idx: HashMap<PathBuf, petgraph::graph::NodeIndex> = HashMap::new();
    for idx in analysis.graph.graph().node_indices() {
        if let crate::graph::node::NodeData::File(f) = &analysis.graph.graph()[idx] {
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
    // 6b. Inject client-call edges (metacall('fn', ...) -> target symbol)
    // and collect unresolved-invocation diagnostics.
    let client_resolution = client_call::resolve_client_calls(
        &analysis.graph,
        &analysis.extractions,
        &all_call_sites,
        &config.root,
    );
    for (from_idx, to_idx, confidence) in client_resolution.edges {
        analysis
            .graph
            .add_edge_normalized(from_idx, to_idx, EdgeKind::Reference, confidence);
    }
    diagnostics.extend(client_resolution.diagnostics);

    // 6c. Report orphaned MetaCall configuration files and surface every
    // diagnostic collected during analysis and edge injection.
    diagnostics.extend(orphaned_config_diagnostics(&config.root, &all_call_sites));
    for diag in &diagnostics {
        tracing::warn!(
            path = %diag.path.display(),
            severity = ?diag.severity,
            "{}", diag.message
        );
    }

    analysis.scc = crate::graph::scc::SccAnalysis::analyze(analysis.graph.graph());

    // 7. Pod partitioning
    let partition = pod::partition_into_pods(&analysis.graph);
    let n_pods = partition.pods.len();
    let n_inter = partition.inter_pod_edges.len();

    // 8. Compute metrics from extractions (zero re-parsing)
    let file_metrics = metrics::compute_file_metrics(&analysis.extractions);
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
        if let Some(cut) = cut::find_oversized_pod_cut(pod, &analysis.graph, config.max_pod_size) {
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

/// Find `metacall.json` / `metacall-*.json` files that no
/// `LoadFromConfiguration` call site references. Such files are inert:
/// MetaCall only consumes a configuration when a call loads it.
fn orphaned_config_diagnostics(root: &Path, call_sites: &[CallSite]) -> Vec<Diagnostic> {
    let referenced: HashSet<PathBuf> = call_sites
        .iter()
        .filter(|s| s.variant == CallSiteVariant::LoadFromConfiguration)
        .filter_map(|s| s.scripts.first())
        .map(|script| root.join(script))
        .collect();

    let mut orphans = Vec::new();
    for entry in ignore::WalkBuilder::new(root)
        .build()
        .filter_map(|e| e.ok())
    {
        let path = dunce::simplified(&entry.into_path()).to_path_buf();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let is_config =
            name == "metacall.json" || (name.starts_with("metacall-") && name.ends_with(".json"));
        if is_config && !referenced.contains(&path) {
            orphans.push(Diagnostic {
                path,
                severity: Severity::Warning,
                message:
                    "orphaned MetaCall configuration file: not referenced by any metacall_load_from_configuration call"
                        .to_string(),
                source_range: None,
            });
        }
    }
    orphans.sort_by(|a, b| a.path.cmp(&b.path));
    orphans
}

/// Add a single MetaCall edge: from a source file node to either an
/// existing file node or a new ExternalNode.
///
/// Script resolution tries four strategies in order through
/// [`client_call::resolve_script_to_file`]:
/// 1. `root.join(script)` -- works when script is relative to project root
/// 2. `source_dir.join(script)` -- resolves relative to the source file's directory
/// 3. Filename match against any discovered file
/// 4. Strip path prefix components from script until a matching file is found
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

    let source_file = match &graph.graph()[from_idx] {
        crate::graph::node::NodeData::File(f) => f.path.clone(),
        _ => return,
    };
    if let Some(to_idx) =
        client_call::resolve_script_to_file(root, script, &source_file, path_to_idx)
    {
        graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
        return;
    }

    // No match: create or reuse ExternalNode (keeps external_index consistent).
    let to_idx = graph.get_or_create_external_node(script.to_string(), target_lang);
    graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
}
