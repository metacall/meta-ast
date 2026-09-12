use crate::deploy::scanner::{CallSite, CallSiteVariant};
use crate::error::{Diagnostic, Severity};
use crate::graph::edge::EdgeKind;
use crate::output::OutputFormat;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub mod check;
pub mod client_call;
pub mod config;
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

/// MetaCall call sites collected from an analysis.
///
/// The scan reuses the extractions of the pipeline run, so no file is parsed
/// twice.
fn scan_call_sites(analysis: &crate::pipeline::GraphAnalysis) -> Vec<CallSite> {
    analysis
        .extractions
        .iter()
        .flat_map(|file| file.call_sites.clone())
        .collect()
}

/// Path to graph node index for every file node.
fn file_index(graph: &crate::graph::CodeGraph) -> HashMap<PathBuf, petgraph::graph::NodeIndex> {
    let mut index = HashMap::new();
    let raw = graph.graph();
    for node in raw.node_indices() {
        if let crate::graph::node::NodeData::File(file) = &raw[node] {
            index.insert(file.path.clone(), node);
        }
    }
    index
}

/// Inject the edges of every MetaCall load call site and recompute the SCC
/// analysis over the injected graph.
///
/// The scripts of a `metacall_load_from_configuration` call are read once here:
/// the configuration names a language and a script list that the graph cannot
/// see on its own.
fn inject_load_edges(
    analysis: &mut crate::pipeline::GraphAnalysis,
    call_sites: &[CallSite],
    config: &DeployConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let path_to_idx = file_index(&analysis.graph);

    for site in call_sites {
        if site.variant != CallSiteVariant::LoadFromConfiguration {
            continue;
        }
        let Some(config_script) = site.scripts.first() else {
            continue;
        };
        let config_file = config.root.join(config_script);
        let bytes = match std::fs::read(&config_file) {
            Ok(bytes) => bytes,
            Err(error) => {
                diagnostics.push(config::config_diagnostic(
                    &config_file,
                    site.source_range.as_ref(),
                    format!("unreadable MetaCall configuration: {error}"),
                ));
                continue;
            }
        };
        let parsed = match config::parse_load_configuration(&bytes) {
            Ok(parsed) => parsed,
            Err(message) => {
                diagnostics.push(config::config_diagnostic(
                    &config_file,
                    site.source_range.as_ref(),
                    message,
                ));
                continue;
            }
        };
        let Some(language_id) = parsed.language_id.as_deref() else {
            diagnostics.push(config::config_diagnostic(
                &config_file,
                site.source_range.as_ref(),
                "MetaCall configuration has no language_id".to_string(),
            ));
            continue;
        };
        let Some(target_lang) = tags::from_metacall_tag(language_id) else {
            diagnostics.push(config::config_diagnostic(
                &config_file,
                site.source_range.as_ref(),
                format!("unknown MetaCall language_id '{language_id}'"),
            ));
            continue;
        };
        if parsed.scripts.is_empty() {
            diagnostics.push(config::config_diagnostic(
                &config_file,
                site.source_range.as_ref(),
                "MetaCall configuration has no scripts".to_string(),
            ));
            continue;
        }
        let Some(&from_idx) = path_to_idx.get(&site.source_file) else {
            continue;
        };
        let base = config::script_base(&config_file, &parsed, &config.root);
        for script in &parsed.scripts {
            add_metacall_edge(
                &base,
                from_idx,
                CallSiteVariant::LoadFromConfiguration,
                target_lang,
                script,
                site.confidence,
                &path_to_idx,
                analysis,
            );
        }
    }

    for site in call_sites {
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
            diagnostics.push(Diagnostic {
                path: site.source_file.clone(),
                severity: Severity::Warning,
                message: format!("unknown MetaCall load tag '{target_lang_tag}'"),
                source_range: site.source_range.clone(),
            });
            continue;
        };
        if !crate::deploy::tags::has_loader(target_lang) {
            diagnostics.push(Diagnostic {
                path: site.source_file.clone(),
                severity: Severity::Warning,
                message: format!(
                    "no MetaCall loader for language '{}', the tag '{}' is a meta-ast tag",
                    target_lang.as_ref(),
                    target_lang_tag
                ),
                source_range: site.source_range.clone(),
            });
        }
        for script in &site.scripts {
            add_metacall_edge(
                &config.root,
                from_idx,
                site.variant.clone(),
                target_lang,
                script,
                site.confidence,
                &path_to_idx,
                analysis,
            );
        }
    }

    analysis.scc = crate::graph::scc::SccAnalysis::analyze(analysis.graph.graph());
}

/// Partition the graph into same-language deployment units and report the
/// languages that have no loader yet.
fn partition_graph(analysis: &crate::pipeline::GraphAnalysis) -> pod::PodPartition {
    let partition = pod::partition_into_pods(&analysis.graph);

    for pod in &partition.pods {
        if !tags::has_loader(pod.language) {
            tracing::warn!(
                pod = pod.id,
                language = %pod.language,
                "MetaCall has no loader for this language yet; the pod tag is informational",
            );
        }
    }

    partition
}

/// Cross-language SCC cuts, plus the oversized pods that can be rebalanced.
fn detect_cuts(
    analysis: &crate::pipeline::GraphAnalysis,
    partition: &pod::PodPartition,
    config: &DeployConfig,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<cut::CutEdge> {
    let lang_map: HashMap<_, _> = partition
        .file_languages
        .iter()
        .map(|(&fid, &lang)| (fid, lang))
        .collect();
    let mut cuts =
        cut::find_cross_language_cuts(&analysis.scc, &analysis.graph, &lang_map, partition);

    rebalance_oversized_pods(
        partition,
        &analysis.graph,
        config.max_pod_size,
        &config.root,
        &mut cuts,
        diagnostics,
    );

    cuts
}

/// Pod manifest and mesh annotation for the current partition.
///
/// Both documents are pure functions of the analysis, the partition and the
/// cuts, so a repeated run over the same tree writes the same bytes.
fn build_documents(
    analysis: &crate::pipeline::GraphAnalysis,
    partition: &pod::PodPartition,
    cuts: &[cut::CutEdge],
    call_sites: &[CallSite],
    config: &DeployConfig,
) -> (manifest::PodManifest, mesh::MeshAnnotation) {
    let file_metrics = metrics::compute_file_metrics(&analysis.extractions);
    let pod_metrics = metrics::compute_pod_metrics(partition, &file_metrics, &analysis.graph);
    let dependencies = dependency::resolve_dependencies(&analysis.graph, partition, &config.root);
    let pod_manifest = manifest::generate_pod_manifest(
        partition,
        &pod_metrics,
        cuts,
        &dependencies,
        &analysis.graph,
    );
    let mesh = mesh::generate_mesh_annotation(analysis, call_sites);

    (pod_manifest, mesh)
}

/// Write both manifests, or check the cut fairness and report it.
fn write_documents(
    config: &DeployConfig,
    partition: &pod::PodPartition,
    cuts: &[cut::CutEdge],
    pod_manifest: &manifest::PodManifest,
    mesh: &mesh::MeshAnnotation,
) -> anyhow::Result<()> {
    if config.check {
        let diagnostics = check::check_cut_fairness(pod_manifest, cuts);
        if diagnostics.is_empty() {
            println!("Check passed: no fairness issues in cut edges.");
        } else {
            println!("Check failed: found {} fairness issues.", diagnostics.len());
            for diagnostic in &diagnostics {
                println!("  - {diagnostic}");
            }
            anyhow::bail!(
                "MetaCall deployment cut fairness check failed with {} issues",
                diagnostics.len()
            );
        }
        return Ok(());
    }

    std::fs::create_dir_all(&config.out)?;

    let extension = config.format.extension();
    let manifest_text = config.format.serialize(pod_manifest)?;
    crate::output::write_atomic(
        &config.out.join(format!("metacall.pods.{extension}")),
        manifest_text.as_bytes(),
    )?;

    let mesh_text = config.format.serialize(mesh)?;
    crate::output::write_atomic(
        &config.out.join(format!("metacall.mesh.{extension}")),
        mesh_text.as_bytes(),
    )?;

    tracing::info!(
        "Generated pod manifest with {} deployments and {} inter-pod edges.",
        partition.pods.len(),
        partition.inter_pod_edges.len()
    );

    Ok(())
}

/// Scan a project, partition it into deployment units, and write the MetaCall
/// manifests.
///
/// Returns every diagnostic collected on the way, in canonical order. The
/// caller decides the diagnostic policy and the exit status.
pub fn run_deploy(config: DeployConfig) -> anyhow::Result<Vec<Diagnostic>> {
    tracing::info!("Starting MetaCall deployment manifest generation");
    tracing::info!("Root path: {}", config.root.display());
    tracing::info!("Output path: {}", config.out.display());
    tracing::info!("Check mode: {}", config.check);
    if config.max_pod_size == 0 {
        anyhow::bail!("max_pod_size must be at least 1");
    }

    let snapshot_id = crate::model::SnapshotId::from(std::num::NonZeroU32::MIN);
    let (mut analysis, mut diagnostics) =
        crate::pipeline::analyze_graph(&config.root, snapshot_id, None)?;

    let call_sites = scan_call_sites(&analysis);
    inject_load_edges(&mut analysis, &call_sites, &config, &mut diagnostics);

    diagnostics.extend(orphaned_config_diagnostics(&config.root, &call_sites));
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    diagnostics.dedup_by(|a, b| a.path == b.path && a.message == b.message);

    let partition = partition_graph(&analysis);
    let cuts = detect_cuts(&analysis, &partition, &config, &mut diagnostics);
    let (pod_manifest, mesh) = build_documents(&analysis, &partition, &cuts, &call_sites, &config);
    write_documents(&config, &partition, &cuts, &pod_manifest, &mesh)?;

    Ok(diagnostics)
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
        let path = crate::input::simplified_path(&entry.into_path());
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
/// 1. `base.join(script)` -- base is the project root or the configuration `path`
/// 2. `source_dir.join(script)` -- resolves relative to the source file's directory
/// 3. Filename match against any discovered file
/// 4. Strip path prefix components from script until a matching file is found
///
/// Edges and external nodes are added through `CodeGraph` helpers so injected
/// edges obey the same dedup/confidence invariant as builder-constructed ones
/// and `external_index` stays consistent across repeated loads.
/// Cut every oversized pod, and report the pods that cannot be cut.
///
/// A pod can exceed the limit with no internal edge to cut, for example when a
/// hand-built partition disagrees with the graph. Staying silent would hide a
/// deployment unit that the limit was meant to bound.
fn rebalance_oversized_pods(
    partition: &crate::deploy::pod::PodPartition,
    graph: &crate::graph::CodeGraph,
    max_pod_size: usize,
    root: &std::path::Path,
    cuts: &mut Vec<cut::CutEdge>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for pod in &partition.pods {
        match cut::find_oversized_pod_cut(pod, graph, max_pod_size) {
            Some(cut) => cuts.push(cut),
            None if pod.files.len() > max_pod_size => {
                let path = pod
                    .files
                    .first()
                    .and_then(|fid| graph.file_node(*fid))
                    .map(|file| file.path.clone())
                    .unwrap_or_else(|| root.to_path_buf());
                diagnostics.push(Diagnostic {
                    path,
                    severity: Severity::Warning,
                    message: format!(
                        "pod {} holds {} files above the limit {} and has no internal edge to cut",
                        pod.id,
                        pod.files.len(),
                        max_pod_size
                    ),
                    source_range: None,
                });
            }
            None => {}
        }
    }
}

fn add_metacall_edge(
    base: &std::path::Path,
    from_idx: petgraph::graph::NodeIndex,
    variant: CallSiteVariant,
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

    // A memory load carries inline code and a package load carries a package
    // name, so neither may resolve to a project file.
    let resolved = match variant {
        CallSiteVariant::LoadFromMemory | CallSiteVariant::LoadFromPackage => None,
        _ => client_call::resolve_script_to_file(base, script, &source_file, path_to_idx),
    };
    if let Some(to_idx) = resolved {
        graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
        return;
    }

    // No match: create or reuse ExternalNode (keeps external_index consistent).
    // The code text of a memory load is not a name.
    let name = match variant {
        CallSiteVariant::LoadFromMemory => {
            format!(
                "<memory:{}>",
                crate::deploy::tags::metacall_tag(target_lang)
            )
        }
        _ => script.to_string(),
    };
    let to_idx = graph.get_or_create_external_node(name, target_lang);
    graph.add_edge_normalized(from_idx, to_idx, EdgeKind::Import, confidence);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::pod::{Pod, PodPartition};
    use crate::graph::node::{FileNode, NodeData};
    use crate::language::LangId;
    use crate::model::{FileId, SnapshotId};

    #[test]
    fn unsplittable_oversized_pod_is_reported() {
        let mut graph = crate::graph::CodeGraph::new(SnapshotId::new(1).unwrap());
        let mut files = Vec::new();
        for id in 1..=4 {
            let fid = FileId::new(id).unwrap();
            let idx = graph.add_node(NodeData::File(FileNode::new(
                fid,
                std::path::PathBuf::from(format!("f{id}.py")),
                LangId::Python,
                SnapshotId::new(1).unwrap(),
            )));
            graph.file_to_index.insert(fid, idx);
            files.push(fid);
        }
        // One pod over the limit, with no dependency edge inside it.
        let partition = PodPartition {
            pods: vec![Pod {
                id: 0,
                files,
                language: LangId::Python,
            }],
            inter_pod_edges: Vec::new(),
            file_languages: HashMap::new(),
        };

        let mut cuts = Vec::new();
        let mut diagnostics = Vec::new();
        rebalance_oversized_pods(
            &partition,
            &graph,
            2,
            std::path::Path::new("."),
            &mut cuts,
            &mut diagnostics,
        );

        assert!(cuts.is_empty());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].severity, Severity::Warning);
        assert!(
            diagnostics[0].message.contains("no internal edge to cut"),
            "{}",
            diagnostics[0].message
        );
    }

    #[test]
    fn oversized_pod_with_an_internal_edge_is_cut() {
        let mut graph = crate::graph::CodeGraph::new(SnapshotId::new(1).unwrap());
        let mut files = Vec::new();
        let mut indices = Vec::new();
        for id in 1..=3 {
            let fid = FileId::new(id).unwrap();
            let idx = graph.add_node(NodeData::File(FileNode::new(
                fid,
                std::path::PathBuf::from(format!("f{id}.py")),
                LangId::Python,
                SnapshotId::new(1).unwrap(),
            )));
            graph.file_to_index.insert(fid, idx);
            files.push(fid);
            indices.push(idx);
        }
        graph.add_edge_normalized(indices[0], indices[1], EdgeKind::Import, 0.5);

        let partition = PodPartition {
            pods: vec![Pod {
                id: 0,
                files,
                language: LangId::Python,
            }],
            inter_pod_edges: Vec::new(),
            file_languages: HashMap::new(),
        };

        let mut cuts = Vec::new();
        let mut diagnostics = Vec::new();
        rebalance_oversized_pods(
            &partition,
            &graph,
            2,
            std::path::Path::new("."),
            &mut cuts,
            &mut diagnostics,
        );

        assert_eq!(cuts.len(), 1, "the internal edge must be cut");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// Analysis of a fixture, with the deploy feature enabled.
    fn fixture_analysis(name: &str) -> crate::pipeline::GraphAnalysis {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let snapshot_id = SnapshotId::new(1).unwrap();
        let (analysis, diagnostics) =
            crate::pipeline::analyze_graph(&root, snapshot_id, None).unwrap();
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error),
            "the fixture must analyse without errors: {diagnostics:?}"
        );
        analysis
    }

    fn fixture_config(name: &str, out: &std::path::Path) -> DeployConfig {
        DeployConfig {
            root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
            out: out.to_path_buf(),
            format: OutputFormat::Json,
            check: false,
            max_pod_size: 20,
        }
    }

    #[test]
    fn scan_call_sites_finds_the_metacall_calls() {
        let analysis = fixture_analysis("mixed/python_calls_js");
        let call_sites = scan_call_sites(&analysis);

        assert!(
            !call_sites.is_empty(),
            "the fixture loads a script through MetaCall"
        );
        assert!(
            call_sites.iter().any(|site| !site.scripts.is_empty()),
            "a load call site names its script"
        );
    }

    #[test]
    fn inject_load_edges_connects_the_loaded_script() {
        let mut analysis = fixture_analysis("mixed/python_calls_js");
        let call_sites = scan_call_sites(&analysis);
        let before = analysis.graph.edge_count();
        let mut diagnostics = Vec::new();
        let out = std::env::temp_dir().join("meta_ast_deploy_stage_inject");
        let config = fixture_config("mixed/python_calls_js", &out);

        inject_load_edges(&mut analysis, &call_sites, &config, &mut diagnostics);

        assert!(
            analysis.graph.edge_count() > before,
            "the loaded script must gain an edge: {before} -> {}",
            analysis.graph.edge_count()
        );
        assert!(
            analysis
                .graph
                .files()
                .any(|(_, file)| file.path.to_string_lossy().ends_with("add.js")),
            "the loaded script joins the graph as a file node"
        );
    }

    #[test]
    fn partition_graph_groups_one_pod_per_language() {
        let analysis = fixture_analysis("mixed/python_calls_js");
        let partition = partition_graph(&analysis);

        assert_eq!(partition.pods.len(), 2, "python and javascript");
        let languages: HashSet<LangId> = partition.pods.iter().map(|pod| pod.language).collect();
        assert!(languages.contains(&LangId::Python));
        assert!(languages.contains(&LangId::JavaScript));
    }

    #[test]
    fn detect_cuts_bounds_an_oversized_pod() {
        let snapshot_id = SnapshotId::new(1).unwrap();
        let mut graph = crate::graph::CodeGraph::new(snapshot_id);
        let mut files = Vec::new();
        for id in 1..=4 {
            let file_id = FileId::new(id).unwrap();
            let index = graph.add_node(NodeData::File(FileNode::new(
                file_id,
                PathBuf::from(format!("f{id}.py")),
                LangId::Python,
                snapshot_id,
            )));
            graph.file_to_index.insert(file_id, index);
            files.push(file_id);
        }
        // One pod over the limit, with no dependency edge inside it, so the
        // rebalance cannot cut it and must report it instead.
        let partition = PodPartition {
            pods: vec![Pod {
                id: 0,
                files,
                language: LangId::Python,
            }],
            inter_pod_edges: Vec::new(),
            file_languages: HashMap::new(),
        };
        let scc = crate::graph::scc::SccAnalysis::analyze(graph.graph());
        let analysis = crate::pipeline::GraphAnalysis {
            graph,
            scc,
            snapshot_id,
            extractions: Vec::new(),
        };
        let out = std::env::temp_dir().join("meta_ast_deploy_stage_cuts");
        let mut config = fixture_config("mixed/python_calls_js", &out);
        config.max_pod_size = 2;
        let mut diagnostics = Vec::new();

        let cuts = detect_cuts(&analysis, &partition, &config, &mut diagnostics);

        assert!(
            cuts.is_empty(),
            "a pod without an internal edge cannot be cut"
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "the unsplittable pod must be reported"
        );
    }

    #[test]
    fn build_documents_follows_the_partition() {
        let analysis = fixture_analysis("mixed/python_calls_js");
        let partition = partition_graph(&analysis);
        let out = std::env::temp_dir().join("meta_ast_deploy_stage_documents");
        let config = fixture_config("mixed/python_calls_js", &out);
        let call_sites = scan_call_sites(&analysis);

        let (pod_manifest, mesh) =
            build_documents(&analysis, &partition, &[], &call_sites, &config);

        assert_eq!(pod_manifest.deployments.len(), partition.pods.len());
        assert!(
            !mesh.deployment_units.is_empty(),
            "the mesh annotation lists the deployment units"
        );
    }

    #[test]
    fn write_documents_writes_both_manifests() {
        let analysis = fixture_analysis("mixed/python_calls_js");
        let partition = partition_graph(&analysis);
        let out = std::env::temp_dir().join("meta_ast_deploy_stage_write");
        let _ = std::fs::remove_dir_all(&out);
        let config = fixture_config("mixed/python_calls_js", &out);
        let call_sites = scan_call_sites(&analysis);
        let (pod_manifest, mesh) =
            build_documents(&analysis, &partition, &[], &call_sites, &config);

        write_documents(&config, &partition, &[], &pod_manifest, &mesh).unwrap();

        for name in ["metacall.pods.json", "metacall.mesh.json"] {
            let path = out.join(name);
            let bytes = std::fs::read(&path).unwrap();
            assert!(!bytes.is_empty(), "{name} must not be empty");
        }
        let _ = std::fs::remove_dir_all(&out);
    }
}
