use crate::deploy::scanner::CallSite;
use crate::graph::edge::EdgeKind;
use crate::graph::node::NodeData;
use crate::pipeline::GraphAnalysis;
use serde::Serialize;
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

    for (idx, scc) in analysis.scc.components.iter().enumerate() {
        let mut symbols = Vec::new();
        let mut unit_languages = HashSet::new();

        for &node_idx in &scc.nodes {
            let node_data = &analysis.graph.graph[node_idx];
            match node_data {
                NodeData::Symbol(sym) => {
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
            }
        }

        let is_cross_language = unit_languages.len() > 1;
        if is_cross_language {
            cross_language_units_count += 1;
        }

        let is_mesh_candidate = scc.hint == crate::graph::scc::DeployabilityHint::Independent;
        if is_mesh_candidate {
            independent_candidates_count += 1;
        }

        deployment_units.push(DeploymentUnit {
            id: idx,
            symbols,
            is_cross_language,
            is_mesh_candidate,
            deployability: format!("{:?}", scc.hint).to_lowercase(),
        });
    }

    // Detect cross-language edges from graph
    for edge_idx in analysis.graph.graph.edge_indices() {
        let weight = &analysis.graph.graph[edge_idx];
        if weight.kind == EdgeKind::Ownership {
            continue;
        }

        // Both endpoints and their component membership are graph invariants
        // produced by SCC analysis; skip edges that violate them rather than panic.
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

            if u_lang != v_lang && u_lang != "unknown" && v_lang != "unknown" {
                cross_language_edges.push(CrossLanguageEdge {
                    from_unit: u_comp,
                    to_unit: v_comp,
                    from_language: u_lang.to_string(),
                    to_language: v_lang.to_string(),
                    call_site: None, // We don't easily have the source file here without more work
                    confidence: weight.confidence as f64,
                });
            }
        }
    }

    // Also add edges from call sites if they represent cross-language dependencies
    // (This might overlap with graph edges if the resolver caught them)
    for site in call_sites {
        if let Some(target_lang) = &site.target_lang {
            let caller_lang = crate::deploy::tags::metacall_tag(site.caller_lang);
            if caller_lang != target_lang {
                // Find unit containing the call site
                // This is harder because we need to find the node for site.source_file
                // For now, we skip adding them here if they are already in graph.
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
    }
}
