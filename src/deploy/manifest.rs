use crate::deploy::scanner::CallSite;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A single per-language deploy manifest (metacall.{tag}.json).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DeployManifest {
    pub language_id: String,
    pub path: String,
    pub scripts: Vec<String>,
}

/// The root manifest composing all per-language manifests.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct RootManifest {
    pub language_id: String,
    pub path: String,
    pub scripts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packages: Option<HashMap<String, Vec<String>>>,
}

pub fn generate_manifests(
    root: &Path,
    files: &[(PathBuf, crate::language::LangId)],
    call_sites: &[CallSite],
) -> (HashMap<String, DeployManifest>, RootManifest) {
    let mut language_groups: HashMap<String, Vec<String>> = HashMap::new();
    let mut primary_lang = "node".to_string();

    // 1. Map all discovered files to their MetaCall tags
    for (path, lang) in files {
        let tag = crate::deploy::tags::metacall_tag(*lang).to_string();
        let scripts = language_groups.entry(tag.clone()).or_default();

        // Use relative path for scripts
        let rel_path: &Path = path.strip_prefix(root).unwrap_or(path);
        let rel_str = rel_path.to_string_lossy().to_string();
        if !scripts.contains(&rel_str) {
            scripts.push(rel_str);
        }

        // Heuristic for primary language: the one with files in the root
        if path.parent() == Some(root) {
            primary_lang = tag;
        }
    }

    // 2. Ensure all target languages from call sites are represented and their scripts added
    for site in call_sites {
        if site.variant == crate::deploy::scanner::CallSiteVariant::LoadFromConfiguration {
            if let Some(config_path_raw) = &site.target_lang {
                let config_path = root.join(config_path_raw);
                if config_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&config_path) {
                        // Try to parse as RootManifest first (more general)
                        if let Ok(config_root) = serde_json::from_str::<RootManifest>(&content) {
                            if let Some(pkgs) = config_root.packages {
                                for (lang, pkg_scripts) in pkgs {
                                    let scripts = language_groups.entry(lang).or_default();
                                    for s in pkg_scripts {
                                        if !scripts.contains(&s) {
                                            scripts.push(s);
                                        }
                                    }
                                }
                            }
                            // Also handle the top-level scripts in the config if it's a single-lang config
                            let scripts =
                                language_groups.entry(config_root.language_id).or_default();
                            for s in config_root.scripts {
                                if !scripts.contains(&s) {
                                    scripts.push(s);
                                }
                            }
                        } else if let Ok(config_dep) =
                            serde_json::from_str::<DeployManifest>(&content)
                        {
                            let scripts =
                                language_groups.entry(config_dep.language_id).or_default();
                            for s in config_dep.scripts {
                                if !scripts.contains(&s) {
                                    scripts.push(s);
                                }
                            }
                        }
                    }
                } else {
                    tracing::warn!(
                        "metacall_load_from_configuration target missing: {}",
                        config_path.display()
                    );
                }
            }
            continue;
        }

        if let Some(target_lang) = &site.target_lang {
            let scripts = language_groups.entry(target_lang.clone()).or_default();
            for script in &site.scripts {
                if !scripts.contains(script) {
                    scripts.push(script.clone());
                }
            }

            // Refine primary lang: the one that calls others is often the entry point
            primary_lang = crate::deploy::tags::metacall_tag(site.caller_lang).to_string();
        }
    }

    let mut manifests = HashMap::new();
    for (lang, scripts) in &language_groups {
        manifests.insert(
            lang.clone(),
            DeployManifest {
                language_id: lang.clone(),
                path: ".".to_string(), // In real-world, we might need to resolve this
                scripts: scripts.clone(),
            },
        );
    }

    let mut packages = HashMap::new();
    for (lang, scripts) in &language_groups {
        packages.insert(lang.clone(), scripts.clone());
    }

    let root_manifest = RootManifest {
        language_id: primary_lang,
        path: ".".to_string(),
        scripts: Vec::new(), // Root script list might be empty if it only loads others
        packages: if packages.is_empty() {
            None
        } else {
            Some(packages)
        },
    };

    (manifests, root_manifest)
}
