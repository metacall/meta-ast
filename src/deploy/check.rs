use crate::deploy::manifest::{DeployManifest, RootManifest};
use std::collections::HashMap;
use std::path::Path;

pub fn check_manifests(
    root: &Path,
    generated_manifests: &HashMap<String, DeployManifest>,
    generated_root: &RootManifest,
) -> anyhow::Result<Vec<String>> {
    let mut diagnostics = Vec::new();

    // 1. Check Root Manifest
    let root_path = root.join("metacall.json");
    if root_path.exists() {
        let content = std::fs::read_to_string(&root_path)?;
        match serde_json::from_str::<RootManifest>(&content) {
            Ok(existing_root) => {
                // Check primary language
                if existing_root.language_id != generated_root.language_id {
                    diagnostics.push(format!(
                        "Root manifest language mismatch: expected '{}', found '{}'",
                        generated_root.language_id, existing_root.language_id
                    ));
                }

                // Check packages/scripts
                if let (Some(gen_pkgs), Some(ext_pkgs)) =
                    (&generated_root.packages, &existing_root.packages)
                {
                    for (lang, gen_scripts) in gen_pkgs {
                        if let Some(ext_scripts) = ext_pkgs.get(lang) {
                            for script in gen_scripts {
                                if !ext_scripts.contains(script) {
                                    diagnostics.push(format!(
                                        "Missing script in root manifest for '{}': {}",
                                        lang, script
                                    ));
                                }
                            }
                            for script in ext_scripts {
                                if !gen_scripts.contains(script) {
                                    diagnostics.push(format!(
                                        "Extra script in root manifest for '{}': {}",
                                        lang, script
                                    ));
                                }
                            }
                        } else {
                            diagnostics.push(format!(
                                "Missing language package in root manifest: '{}'",
                                lang
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                diagnostics.push(format!("Failed to parse existing metacall.json: {}", e));
            }
        }
    } else {
        diagnostics.push("Root manifest (metacall.json) is missing".to_string());
    }

    // 2. Check Per-Language Manifests
    for (lang, gen_manifest) in generated_manifests {
        let manifest_name = format!("metacall.{}.json", lang);
        let manifest_path = root.join(&manifest_name);

        if manifest_path.exists() {
            let content = std::fs::read_to_string(&manifest_path)?;
            match serde_json::from_str::<DeployManifest>(&content) {
                Ok(existing_manifest) => {
                    for script in &gen_manifest.scripts {
                        if !existing_manifest.scripts.contains(script) {
                            diagnostics
                                .push(format!("Missing script in {}: {}", manifest_name, script));
                        }
                    }
                    for script in &existing_manifest.scripts {
                        if !gen_manifest.scripts.contains(script) {
                            diagnostics
                                .push(format!("Extra script in {}: {}", manifest_name, script));
                        }
                    }
                }
                Err(e) => {
                    diagnostics.push(format!("Failed to parse {}: {}", manifest_name, e));
                }
            }
        } else {
            // Not strictly required by MetaCall but we can flag it
            diagnostics.push(format!("Missing per-language manifest: {}", manifest_name));
        }
    }

    Ok(diagnostics)
}
