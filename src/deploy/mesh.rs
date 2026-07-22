use crate::deploy::scanner::CallSite;
use crate::graph::edge::EdgeKind;
use crate::graph::node::NodeData;
use crate::pipeline::GraphAnalysis;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;

#[derive(Serialize)]
pub struct MeshAnnotation {
    pub version: String,
    pub deployment_units: Vec<DeploymentUnit>,
    pub cross_language_edges: Vec<CrossLanguageEdge>,
    pub stats: MeshStats,
}

#[derive(Serialize)]
pub struct DeploymentUnit {
    pub id: usize,
    pub symbols: Vec<UnitSymbol>,
    pub is_cross_language: bool,
    pub is_mesh_candidate: bool,
    pub deployability: String,
}

#[derive(Serialize)]
pub struct UnitSymbol {
    pub name: String,
    pub file: String,
    pub language: String,
    pub kind: String,
}

#[derive(Serialize)]
pub struct CrossLanguageEdge {
    pub from_unit: usize,
    pub to_unit: usize,
    pub from_language: String,
    pub to_language: String,
    pub call_site: Option<String>,
    pub confidence: f64,
}

#[derive(Serialize, Default)]
pub struct MeshStats {
    pub total_units: usize,
    pub cross_language_units: usize,
    pub independent_candidates: usize,
    pub languages: Vec<String>,
}

pub fn generate_mesh_annotation(
    analysis: &GraphAnalysis,
    call_sites: &[CallSite],
) -> MeshAnnotation {
    let mut deployment_units = Vec::new();
    let mut cross_language_edges = Vec::new();
    let mut languages = HashSet::new();
    let mut cross_language_units_count = 0;
    let mut independent_candidates_count = 0;
    // Maps SCC component index -> emitted deployment unit id. Only components
    // with deployable (symbol/file/external) nodes are emitted, so this is a
    // strict subset of all component indices. Cross-language edges use raw SCC
    // indices; without this map they would reference skipped components.
    let mut component_to_unit: HashMap<usize, usize> = HashMap::new();
    // Maps a file path -> the emitted unit id that owns its symbols. Used to
    // anchor cross-language edges whose endpoints land on file-only SCCs
    // (e.g. a metacall_load_from_file call site) so they resolve to a real
    // unit instead of a skipped component index.
    let mut file_to_unit: HashMap<String, usize> = HashMap::new();

    for (idx, scc) in analysis.scc.components.iter().enumerate() {
        let mut symbols = Vec::new();
        let mut unit_languages = HashSet::new();
        let mut has_real_node = false;

        for &node_idx in &scc.nodes {
            let node_data = &analysis.graph.graph[node_idx];
            match node_data {
                NodeData::Symbol(sym) => {
                    has_real_node = true;
                    let file_node =
                        analysis
                            .graph
                            .file_to_index
                            .get(&sym.file_id)
                            .and_then(|&f_idx| match &analysis.graph.graph[f_idx] {
                                NodeData::File(f) => Some(f),
                                _ => None,
                            });

                    let lang_tag = file_node
                        .map(|f| crate::deploy::tags::metacall_tag(f.language))
                        .unwrap_or("unknown");

                    languages.insert(lang_tag.to_string());
                    unit_languages.insert(lang_tag.to_string());

                    symbols.push(UnitSymbol {
                        name: sym.name.clone(),
                        file: file_node
                            .map(|f| f.path.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default(),
                        language: lang_tag.to_string(),
                        kind: format!("{:?}", sym.kind).to_lowercase(),
                    });
                }
                NodeData::File(f) => {
                    has_real_node = true;
                    let lang_tag = crate::deploy::tags::metacall_tag(f.language);
                    languages.insert(lang_tag.to_string());
                    unit_languages.insert(lang_tag.to_string());

                    symbols.push(UnitSymbol {
                        name: f.path.to_string_lossy().replace('\\', "/"),
                        file: f.path.to_string_lossy().replace('\\', "/"),
                        language: lang_tag.to_string(),
                        kind: "file".to_string(),
                    });
                }
                NodeData::External(ext) => {
                    let lang_tag = crate::deploy::tags::metacall_tag(ext.language);
                    languages.insert(lang_tag.to_string());
                    unit_languages.insert(lang_tag.to_string());

                    symbols.push(UnitSymbol {
                        name: ext.raw_path.clone(),
                        file: "external".to_string(),
                        language: lang_tag.to_string(),
                        kind: "external".to_string(),
                    });
                }
                NodeData::Data(_) => {}
            }
        }

        let is_cross_language = unit_languages.len() > 1;
        if is_cross_language {
            cross_language_units_count += 1;
        }

        // Skip SCCs composed entirely of ExternalNodes -- they have
        // no file/symbol content to deploy.
        if !has_real_node {
            continue;
        }

        let is_mesh_candidate = scc.hint == crate::graph::scc::DeployabilityHint::Independent;
        if is_mesh_candidate {
            independent_candidates_count += 1;
        }

        let unit_id = deployment_units.len();
        component_to_unit.insert(idx, unit_id);
        // Anchor every real file in this unit so file-only SCCs (call sites,
        // loaded targets) resolve to this unit via file ownership.
        for s in &symbols {
            if s.kind != "external" {
                file_to_unit.insert(s.file.clone(), unit_id);
            }
        }

        deployment_units.push(DeploymentUnit {
            id: unit_id,
            symbols,
            is_cross_language,
            is_mesh_candidate,
            deployability: scc.hint.to_string(),
        });
    }

    // Build path -> component lookup for cross-language edge annotation.
    let mut file_to_component: HashMap<String, usize> = HashMap::new();
    for (idx, scc) in analysis.scc.components.iter().enumerate() {
        for &node_idx in &scc.nodes {
            if let NodeData::File(f) = &analysis.graph.graph[node_idx] {
                let path_str = f.path.to_string_lossy().replace('\\', "/");
                file_to_component.insert(path_str, idx);
            }
        }
    }

    // Resolves a component index to an emitted unit id. Skipped
    // (file-only / external-only) components have no direct mapping, so we
    // fall back to file ownership: the endpoint's file nodes anchor to the
    // unit that owns their symbols. Returns None only when no anchor exists.
    let resolve_unit = |comp: usize| -> Option<usize> {
        if let Some(&uid) = component_to_unit.get(&comp) {
            return Some(uid);
        }
        for &node_idx in &analysis.scc.components[comp].nodes {
            if let NodeData::File(f) = &analysis.graph.graph[node_idx] {
                let path_str = f.path.to_string_lossy().replace('\\', "/");
                if let Some(&uid) = file_to_unit.get(&path_str) {
                    return Some(uid);
                }
            }
        }
        None
    };

    // Detect cross-language edges from graph, annotated with call-site info.
    let mut seen_edges: HashSet<(usize, usize, &str, &str)> = HashSet::new();
    for edge_idx in analysis.graph.graph.edge_indices() {
        let weight = &analysis.graph.graph[edge_idx];
        if weight.kind == EdgeKind::Ownership {
            continue;
        }

        let Some((u, v)) = analysis.graph.graph.edge_endpoints(edge_idx) else {
            continue;
        };
        let (Some(u_comp), Some(v_comp)) =
            (analysis.scc.component_of(u), analysis.scc.component_of(v))
        else {
            continue;
        };

        if u_comp != v_comp {
            let u_lang = get_node_language(&analysis.graph.graph[u], &analysis.graph);
            let v_lang = get_node_language(&analysis.graph.graph[v], &analysis.graph);

            // Remap raw SCC indices to emitted unit ids. Edges whose
            // endpoints are skipped components anchor to the unit owning their
            // file's symbols; edges with no anchor are not part of the mesh.
            let (Some(from_unit), Some(to_unit)) = (resolve_unit(u_comp), resolve_unit(v_comp))
            else {
                continue;
            };

            if u_lang != v_lang && u_lang != "unknown" && v_lang != "unknown" {
                // Parallel graph edges (e.g. a duplicated load call) must
                // not produce duplicate mesh edges.
                let key = (from_unit, to_unit, u_lang, v_lang);
                if !seen_edges.insert(key) {
                    continue;
                }

                // Find the call site whose source_file is in u_comp and
                // whose scripts target v_comp (or match the file name).
                let call_site_file = call_sites.iter().find_map(|site| {
                    let site_path = site.source_file.to_string_lossy().replace('\\', "/");
                    if file_to_component.get(&site_path) != Some(&u_comp) {
                        return None;
                    }
                    let site_target_lang = site.target_lang.as_ref().map(|t| {
                        crate::deploy::tags::metacall_tag(
                            crate::deploy::tags::from_metacall_tag(t).unwrap_or(site.caller_lang),
                        )
                    });
                    if site_target_lang == Some(v_lang) {
                        Some(site_path)
                    } else if let NodeData::File(dst_file) = &analysis.graph.graph[v] {
                        let dst_name = dst_file
                            .path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("");
                        if site
                            .scripts
                            .iter()
                            .any(|s| s.contains(dst_name) || dst_name.contains(s.as_str()))
                        {
                            Some(site_path)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                });

                cross_language_edges.push(CrossLanguageEdge {
                    from_unit,
                    to_unit,
                    from_language: u_lang.to_string(),
                    to_language: v_lang.to_string(),
                    call_site: call_site_file,
                    confidence: weight.confidence as f64,
                });
            }
        }
    }

    let total_units = deployment_units.len();
    MeshAnnotation {
        version: "1.0".to_string(),
        deployment_units,
        cross_language_edges,
        stats: MeshStats {
            total_units,
            cross_language_units: cross_language_units_count,
            independent_candidates: independent_candidates_count,
            languages: languages.into_iter().collect(),
        },
    }
}

fn get_node_language<'a>(node: &'a NodeData, graph: &'a crate::graph::CodeGraph) -> &'a str {
    match node {
        NodeData::Symbol(s) => {
            if let Some(&f_idx) = graph.file_to_index.get(&s.file_id)
                && let NodeData::File(f) = &graph.graph[f_idx]
            {
                return crate::deploy::tags::metacall_tag(f.language);
            }
            "unknown"
        }
        NodeData::File(f) => crate::deploy::tags::metacall_tag(f.language),
        NodeData::External(e) => crate::deploy::tags::metacall_tag(e.language),
        NodeData::Data(_) => "unknown",
    }
}
