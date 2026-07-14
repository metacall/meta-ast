#[cfg(feature = "metacall-deploy")]
mod deploy_mixed_tests {
    use meta_ast::deploy::{DeployConfig, run_deploy};
    use meta_ast::output::OutputFormat;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn run_deploy_on_fixture(fixture_name: &str) -> (tempfile::TempDir, serde_json::Value) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mixed")
            .join(fixture_name);
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();
        let config = DeployConfig {
            root,
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };
        run_deploy(config).expect("Deploy failed");

        let content = std::fs::read_to_string(out_path.join("metacall.pods.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();
        (out_dir, manifest)
    }

    #[test]
    fn test_three_lang_math_deploy() {
        let (_out_dir, manifest) = run_deploy_on_fixture("three_lang_math");

        let deployments = manifest["deployments"].as_array().unwrap();
        // Expect at least Python + JS + Rust = 3 pods
        assert!(
            deployments.len() >= 3,
            "expected >= 3 pods, got {}",
            deployments.len()
        );

        let languages: Vec<&str> = deployments
            .iter()
            .filter_map(|d| d["language"].as_str())
            .collect();
        assert!(languages.contains(&"py"), "missing Python pod");
        assert!(languages.contains(&"node"), "missing Node.js pod");
        assert!(languages.contains(&"rs"), "missing Rust pod");

        // Verify cross-language edges exist
        let cross_lang_count = manifest["metrics"]["cross_language_edges"]
            .as_u64()
            .unwrap_or(0);
        assert!(cross_lang_count > 0, "expected cross-language edges");

        // Each pod should have at least one file
        for d in deployments {
            let files = d["files"].as_array().unwrap();
            assert!(!files.is_empty(), "pod {} has no files", d["id"]);
        }

        // Mesh should reference each language
        let mesh_content =
            std::fs::read_to_string(_out_dir.path().join("metacall.mesh.json")).unwrap();
        let mesh: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        assert!(mesh["deployment_units"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_auth_microservice_deploy() {
        let (_out_dir, manifest) = run_deploy_on_fixture("auth_microservice");

        let deployments = manifest["deployments"].as_array().unwrap();
        // Expect Python + JS + Go + TS = 4 pods (or at least 3 with Go parsed separately)
        assert!(
            deployments.len() >= 3,
            "expected >= 3 pods, got {}",
            deployments.len()
        );

        let languages: Vec<&str> = deployments
            .iter()
            .filter_map(|d| d["language"].as_str())
            .collect();
        assert!(languages.contains(&"py"), "missing Python pod");
        assert!(languages.contains(&"node"), "missing Node.js pod");

        // Verify total pod count in metrics
        let total_pods = manifest["metrics"]["total_pods"].as_u64().unwrap_or(0);
        assert_eq!(total_pods as usize, deployments.len());

        // Verify global AST node count is non-trivial
        let ast_nodes = manifest["metrics"]["total_ast_nodes"].as_u64().unwrap_or(0);
        assert!(ast_nodes > 0, "expected positive AST node count");

        // Mesh should have deployment units
        let mesh_content =
            std::fs::read_to_string(_out_dir.path().join("metacall.mesh.json")).unwrap();
        let mesh: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        assert!(mesh["deployment_units"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_auth_microservice_check_mode() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mixed")
            .join("auth_microservice");
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        let config = DeployConfig {
            root,
            out: out_path,
            format: OutputFormat::Json,
            check: true,
        };

        let result = run_deploy(config);
        assert!(
            result.is_ok() || result.unwrap_err().to_string().contains("fairness"),
            "check mode should pass or report fairness"
        );
    }

    #[test]
    fn test_three_lang_inter_pod_edges() {
        let (_out_dir, manifest) = run_deploy_on_fixture("three_lang_math");

        let edges = manifest["edges"].as_array().unwrap();
        assert!(!edges.is_empty(), "expected inter-pod edges");

        // Each edge should have from_pod, to_pod, kind, confidence
        for edge in edges {
            assert!(edge["from_pod"].is_number(), "edge missing from_pod");
            assert!(edge["to_pod"].is_number(), "edge missing to_pod");
            assert!(edge["kind"].is_string(), "edge missing kind");
            assert!(edge["confidence"].is_number(), "edge missing confidence");
            assert!(
                edge["confidence"].as_f64().unwrap() >= 0.0
                    && edge["confidence"].as_f64().unwrap() <= 1.0,
                "confidence out of range"
            );
        }
    }

    #[test]
    fn test_cross_language_manifest_version() {
        let (_out_dir, manifest) = run_deploy_on_fixture("three_lang_math");
        assert_eq!(manifest["version"].as_str().unwrap(), "1.0");
    }

    #[test]
    fn test_mesh_cross_language_edges_reference_real_units() {
        // Every cross-language edge endpoint must resolve to an emitted
        // deployment unit id. Otherwise the mesh is un-consumable: edges
        // point at SCC indices that were skipped during unit emission.
        for fixture in [
            "python_calls_js",
            "three_lang_math",
            "auth_microservice",
            "auth_microservice_level2",
        ] {
            let (_out_dir, _manifest) = run_deploy_on_fixture(fixture);

            let mesh_content =
                std::fs::read_to_string(_out_dir.path().join("metacall.mesh.json")).unwrap();
            let mesh: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();

            let unit_ids: std::collections::HashSet<u64> = mesh["deployment_units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| u["id"].as_u64().unwrap())
                .collect();

            let edges = mesh["cross_language_edges"].as_array().unwrap();
            for edge in edges {
                let from = edge["from_unit"].as_u64().expect("from_unit missing");
                let to = edge["to_unit"].as_u64().expect("to_unit missing");
                assert!(
                    unit_ids.contains(&from),
                    "fixture {fixture}: from_unit {from} not in emitted units {unit_ids:?}"
                );
                assert!(
                    unit_ids.contains(&to),
                    "fixture {fixture}: to_unit {to} not in emitted units {unit_ids:?}"
                );
            }

            // No duplicate mesh edges: parallel graph edges must collapse.
            let mut seen = std::collections::HashSet::new();
            for edge in edges {
                let from = edge["from_unit"].as_u64().unwrap();
                let to = edge["to_unit"].as_u64().unwrap();
                let fl = edge["from_language"].as_str().unwrap();
                let tl = edge["to_language"].as_str().unwrap();
                assert!(
                    seen.insert((from, to, fl, tl)),
                    "fixture {fixture}: duplicate cross-language mesh edge ({from},{to},{fl},{tl})"
                );
            }
        }
    }

    #[test]
    fn test_auth_microservice_level2_cyclic_and_dynamic() {
        let (_out_dir, manifest) = run_deploy_on_fixture("auth_microservice_level2");

        let deployments = manifest["deployments"].as_array().unwrap();

        // Intra-language cycle (orchestrator.py <-> callback.py) must collapse
        // into a single same-language pod, not two separate pods.
        let py_pods: Vec<_> = deployments
            .iter()
            .filter(|d| d["language"].as_str() == Some("py"))
            .collect();
        assert_eq!(py_pods.len(), 1, "py cycle should form exactly one pod");
        let py_files = py_pods[0]["files"].as_array().unwrap();
        assert!(
            py_files.len() >= 2,
            "py pod should contain both cycle members, got {py_files:?}"
        );

        // Config-driven load (LoadFromConfiguration -> deploy.conf.json -> extra.js)
        // and the static validate.js both resolve to node pods.
        let node_pods: Vec<&str> = deployments
            .iter()
            .filter_map(|d| d["language"].as_str())
            .filter(|l| *l == "node")
            .collect();
        assert!(
            node_pods.len() >= 2,
            "expected static + config-driven node pods, got {node_pods:?}"
        );
        let all_files: Vec<String> = deployments
            .iter()
            .flat_map(|d| d["files"].as_array().unwrap().iter())
            .map(|f| f.as_str().unwrap().to_string())
            .collect();
        assert!(
            all_files.iter().any(|f| f.ends_with("extra.js")),
            "config-driven extra.js must be deployed, got {all_files:?}"
        );

        // The cross-language cut must fire for the py<->go metacall cycle.
        let edges = manifest["edges"].as_array().unwrap();
        let cross_lang = edges
            .iter()
            .filter(|e| e["is_cross_language"].as_bool() == Some(true))
            .count();
        assert!(
            cross_lang >= 4,
            "expected >= 4 cross-language edges (loads + cycle), got {cross_lang}"
        );
        let has_scc_cut = edges
            .iter()
            .any(|e| e["cut_annotation"]["cut_reason"].as_str() == Some("CrossLanguageScc"));
        assert!(
            has_scc_cut,
            "expected a CrossLanguageScc cut, edges: {edges:?}"
        );

        // Mesh edges must all resolve to emitted units (no skipped-SCC drift).
        let mesh_content =
            std::fs::read_to_string(_out_dir.path().join("metacall.mesh.json")).unwrap();
        let mesh: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        let unit_ids: std::collections::HashSet<u64> = mesh["deployment_units"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["id"].as_u64().unwrap())
            .collect();
        for edge in mesh["cross_language_edges"].as_array().unwrap() {
            let from = edge["from_unit"].as_u64().unwrap();
            let to = edge["to_unit"].as_u64().unwrap();
            assert!(
                unit_ids.contains(&from) && unit_ids.contains(&to),
                "mesh edge {from}->{to} references un-emitted unit"
            );
        }
    }

    #[test]
    fn test_auth_microservice_level2_check_mode() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mixed")
            .join("auth_microservice_level2");
        let out_dir = tempdir().unwrap();

        let config = DeployConfig {
            root,
            out: out_dir.path().to_path_buf(),
            format: OutputFormat::Json,
            check: true,
        };

        let result = run_deploy(config);
        assert!(
            result.is_ok() || result.unwrap_err().to_string().contains("fairness"),
            "check mode should pass or report fairness"
        );
    }

    #[test]
    fn test_empty_root_produces_empty_manifest() {
        let root = tempdir().unwrap();
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        let config = DeployConfig {
            root: root.path().to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("empty root should produce empty manifest");
        let content = std::fs::read_to_string(out_path.join("metacall.pods.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(manifest["metrics"]["total_pods"].as_u64().unwrap(), 0);
        assert!(manifest["deployments"].as_array().unwrap().is_empty());
    }

    /// Level 3: full-module stress fixture. Asserts every deploy subsystem:
    /// - static cross-language file loads (py/go/ts/node)
    /// - intra-language cycle collapse (orchestrator<->cache<->queue -> 1 py pod)
    /// - cross-language SCC cut (py<->go metacall round-trip)
    /// - dynamic/package/config load variants resolve without dropping units
    /// - mesh units all carry real files and edges resolve to emitted units
    #[test]
    fn test_auth_microservice_level3_full_module() {
        let (_out_dir, manifest) = run_deploy_on_fixture("auth_microservice_level3");

        let deployments = manifest["deployments"].as_array().unwrap();

        // Intra-language cycle must collapse into a single pod with >= 3 files.
        let py_pods: Vec<&serde_json::Value> = deployments
            .iter()
            .filter(|d| d["language"].as_str() == Some("py"))
            .collect();
        assert_eq!(py_pods.len(), 1, "py cycle should form exactly one pod");
        let py_files: Vec<String> = py_pods[0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap().to_string())
            .collect();
        assert!(
            py_files.len() >= 3,
            "py pod should contain the full cycle, got {py_files:?}"
        );
        for member in ["orchestrator.py", "cache.py", "queue.py"] {
            assert!(
                py_files.iter().any(|f| f.ends_with(member)),
                "py pod missing {member}: {py_files:?}"
            );
        }

        // Every loaded language must have at least one pod.
        let languages: Vec<&str> = deployments
            .iter()
            .filter_map(|d| d["language"].as_str())
            .collect();
        for lang in ["py", "go", "ts", "node"] {
            assert!(
                languages.contains(&lang),
                "missing {lang} pod in {languages:?}"
            );
        }

        // Config-driven load (LoadFromConfiguration -> deploy.conf.json -> extra.js)
        // must materialize as its own node pod.
        let all_files: Vec<String> = deployments
            .iter()
            .flat_map(|d| d["files"].as_array().unwrap().iter())
            .map(|f| f.as_str().unwrap().to_string())
            .collect();
        assert!(
            all_files.iter().any(|f| f.ends_with("extra.js")),
            "config-driven extra.js must be deployed, got {all_files:?}"
        );

        // The cross-language cut must fire for the py<->go metacall cycle.
        let edges = manifest["edges"].as_array().unwrap();
        let has_scc_cut = edges
            .iter()
            .any(|e| e["cut_annotation"]["cut_reason"].as_str() == Some("CrossLanguageScc"));
        assert!(
            has_scc_cut,
            "expected a CrossLanguageScc cut, edges: {edges:?}"
        );

        // Every cut edge must carry an rpc_stub-style cut_annotation and a
        // counterpart pod pair (fairness invariant from ADR 0003).
        for e in edges {
            if let Some(cut) = e["cut_annotation"].as_object() {
                assert!(
                    cut.get("cut_reason").is_some(),
                    "cut edge missing cut_reason: {e:?}"
                );
                assert_ne!(
                    e["from_pod"], e["to_pod"],
                    "cross-language cut must span distinct pods: {e:?}"
                );
            }
        }

        // Metrics block must be present and consistent.
        let metrics = &manifest["metrics"];
        assert!(
            metrics["total_pods"].as_u64().unwrap() >= 4,
            "expected >= 4 pods, got {metrics:?}"
        );
        assert!(
            metrics["cross_language_edges"].as_u64().unwrap() >= 4,
            "expected >= 4 cross-language edges, got {metrics:?}"
        );

        // Mesh units must all carry real files and cross-language edges must
        // resolve to emitted units (no skipped-SCC drift).
        let mesh_content =
            std::fs::read_to_string(_out_dir.path().join("metacall.mesh.json")).unwrap();
        let mesh: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        let unit_ids: std::collections::HashSet<u64> = mesh["deployment_units"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["id"].as_u64().unwrap())
            .collect();
        for unit in mesh["deployment_units"].as_array().unwrap() {
            assert!(
                !unit["symbols"].as_array().unwrap().is_empty(),
                "mesh unit {} has no symbols",
                unit["id"]
            );
        }
        for edge in mesh["cross_language_edges"].as_array().unwrap() {
            let from = edge["from_unit"].as_u64().unwrap();
            let to = edge["to_unit"].as_u64().unwrap();
            assert!(
                unit_ids.contains(&from) && unit_ids.contains(&to),
                "mesh edge {from}->{to} references un-emitted unit"
            );
        }
    }

    /// Level 3 must exercise external dependency resolution: the orchestrator
    /// loads "express" via LoadFromPackage and an inline string via
    /// LoadFromMemory, both of which become External nodes that the
    /// dependency classifier must resolve and attach (never silently drop).
    #[test]
    fn test_auth_microservice_level3_dependency_classification() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mixed")
            .join("auth_microservice_level3");
        let out_dir = tempdir().unwrap();

        let config = DeployConfig {
            root,
            out: out_dir.path().to_path_buf(),
            format: OutputFormat::Json,
            check: false,
        };
        run_deploy(config).expect("deploy failed");

        let content = std::fs::read_to_string(out_dir.path().join("metacall.pods.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();

        let deployments = manifest["deployments"].as_array().unwrap();

        // The python pod hosts the orchestrator, which declares both a packaged
        // ("express") and an inline-memory ("export const INLINE = 1;") load.
        // Both must surface as resolved dependencies on that pod.
        let py_pod = deployments
            .iter()
            .find(|d| {
                d["language"].as_str() == Some("py")
                    && d["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|f| f.as_str().unwrap().ends_with("orchestrator.py"))
            })
            .expect("orchestrator py pod present");
        let dep_names: Vec<String> = py_pod["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap().to_string())
            .collect();

        assert!(
            dep_names.iter().any(|n| n == "express"),
            "express (LoadFromPackage) must be classified, got {dep_names:?}"
        );
        assert!(
            dep_names.iter().any(|n| n.contains("INLINE")),
            "inline memory load must be classified as external, got {dep_names:?}"
        );

        // The Node validation pod imports the builtin "crypto"; the classifier
        // must attach it rather than leaving the pod dependency-free.
        let node_pod = deployments
            .iter()
            .find(|d| {
                d["language"].as_str() == Some("node")
                    && d["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|f| f.as_str().unwrap().ends_with("validate.js"))
            })
            .expect("validate.js node pod present");
        let node_deps: Vec<String> = node_pod["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap().to_string())
            .collect();
        assert!(
            node_deps.iter().any(|n| n == "crypto"),
            "crypto import must be classified, got {node_deps:?}"
        );
    }

    #[test]
    fn test_auth_microservice_level3_check_mode() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mixed")
            .join("auth_microservice_level3");
        let out_dir = tempdir().unwrap();

        let config = DeployConfig {
            root,
            out: out_dir.path().to_path_buf(),
            format: OutputFormat::Json,
            check: true,
        };

        let result = run_deploy(config);
        assert!(
            result.is_ok() || result.unwrap_err().to_string().contains("fairness"),
            "check mode should pass or report fairness"
        );
    }
}
