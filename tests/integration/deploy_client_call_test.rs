#[cfg(feature = "metacall-deploy")]
mod deploy_client_call_tests {
    use meta_ast::deploy::{DeployConfig, run_deploy};
    use meta_ast::output::OutputFormat;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/mixed")
            .join("client_call_mesh")
    }

    fn run_deploy_on_fixture() -> (tempfile::TempDir, serde_json::Value) {
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();
        let config = DeployConfig {
            root: fixture_root(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
            max_pod_size: 20,
        };
        run_deploy(config).expect("Deploy failed");

        let content = std::fs::read_to_string(out_path.join("metacall.pods.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();
        (out_dir, manifest)
    }

    fn read_mesh(out_dir: &tempfile::TempDir) -> serde_json::Value {
        let content = std::fs::read_to_string(out_dir.path().join("metacall.mesh.json")).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    /// Pod ids for a language tag, read from the deployments array. Pod ids
    /// are path-derived, so resolve them via language instead of hardcoding.
    fn pod_ids_by_language(manifest: &serde_json::Value, language: &str) -> Vec<u64> {
        manifest["deployments"]
            .as_array()
            .map(|ds| {
                ds.iter()
                    .filter(|d| d["language"].as_str() == Some(language))
                    .filter_map(|d| d["id"].as_u64())
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn test_client_call_mesh_pod_manifest() {
        let (_out_dir, manifest) = run_deploy_on_fixture();

        assert_eq!(manifest["version"].as_str().unwrap(), "1.0");

        let deployments = manifest["deployments"].as_array().unwrap();
        assert_eq!(
            deployments.len(),
            2,
            "expected one py pod and one node pod, got {}",
            deployments.len()
        );

        let languages: HashSet<&str> = deployments
            .iter()
            .filter_map(|d| d["language"].as_str())
            .collect();
        assert_eq!(languages, HashSet::from(["py", "node"]));

        let total_pods = manifest["metrics"]["total_pods"].as_u64().unwrap_or(0);
        assert_eq!(total_pods, 2, "expected 2 pods in metrics");
        let ast_nodes = manifest["metrics"]["total_ast_nodes"].as_u64().unwrap_or(0);
        assert!(ast_nodes > 0, "expected positive AST node count");
    }

    #[test]
    fn test_client_call_mesh_load_and_reference_edges() {
        let (_out_dir, manifest) = run_deploy_on_fixture();

        let py_pods = pod_ids_by_language(&manifest, "py");
        let node_pods = pod_ids_by_language(&manifest, "node");
        assert_eq!(py_pods.len(), 1, "expected exactly one py pod");
        assert_eq!(node_pods.len(), 1, "expected exactly one node pod");
        let py_pod = py_pods[0];
        let node_pod = node_pods[0];

        let edges = manifest["edges"].as_array().unwrap();

        // The load edge: orchestrator.py loads math.js via load_from_file.
        let load_edges: Vec<&serde_json::Value> = edges
            .iter()
            .filter(|e| {
                e["kind"].as_str() == Some("import")
                    && e["from_pod"].as_u64() == Some(py_pod)
                    && e["to_pod"].as_u64() == Some(node_pod)
            })
            .collect();
        assert!(
            !load_edges.is_empty(),
            "expected an import edge from py pod {py_pod} to node pod {node_pod}, edges: {edges:?}"
        );

        // The resolved client call: exactly one reference edge py -> node at
        // full confidence (string-literal function name).
        let reference_edges: Vec<&serde_json::Value> = edges
            .iter()
            .filter(|e| {
                e["kind"].as_str() == Some("reference")
                    && e["from_pod"].as_u64() == Some(py_pod)
                    && e["to_pod"].as_u64() == Some(node_pod)
            })
            .collect();
        assert_eq!(
            reference_edges.len(),
            1,
            "expected exactly one reference edge py -> node, got {reference_edges:?}"
        );
        assert_eq!(
            reference_edges[0]["confidence"].as_f64().unwrap(),
            1.0,
            "a literal client call target must resolve at full confidence"
        );
    }

    #[test]
    fn test_client_call_mesh_mesh_targets_and_call_sites() {
        let (out_dir, _manifest) = run_deploy_on_fixture();
        let mesh = read_mesh(&out_dir);

        // The node unit must expose the multiply symbol, the resolved target.
        let units = mesh["deployment_units"].as_array().unwrap();
        let has_multiply = units.iter().any(|u| {
            u["symbols"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"].as_str() == Some("multiply"))
        });
        assert!(
            has_multiply,
            "expected a deployment unit exporting multiply"
        );

        // Every cross-language edge endpoint must resolve to an emitted unit
        // id (no skipped-SCC drift).
        let unit_ids: HashSet<u64> = units.iter().map(|u| u["id"].as_u64().unwrap()).collect();
        let edges = mesh["cross_language_edges"].as_array().unwrap();
        assert!(!edges.is_empty(), "expected cross-language mesh edges");
        for edge in edges {
            let from = edge["from_unit"].as_u64().expect("from_unit missing");
            let to = edge["to_unit"].as_u64().expect("to_unit missing");
            assert!(
                unit_ids.contains(&from),
                "from_unit {from} not in emitted units {unit_ids:?}"
            );
            assert!(
                unit_ids.contains(&to),
                "to_unit {to} not in emitted units {unit_ids:?}"
            );
        }

        // The py -> node edges must carry call-site attribution from the
        // orchestrator file that issued the load and the client calls.
        assert!(
            edges.iter().any(|e| e["call_site"]
                .as_str()
                .is_some_and(|cs| cs.ends_with("orchestrator.py"))),
            "expected a call site in orchestrator.py, edges: {edges:?}"
        );
    }

    #[test]
    fn test_client_call_mesh_no_unresolved_reference_edges() {
        let (_out_dir, manifest) = run_deploy_on_fixture();

        // metacall('no_such_function', value) must not emit an edge: the only
        // reference edge is the resolved multiply call at full confidence.
        let reference_edges: Vec<&serde_json::Value> = manifest["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"].as_str() == Some("reference"))
            .collect();
        assert_eq!(
            reference_edges.len(),
            1,
            "expected exactly one reference edge, got {reference_edges:?}"
        );
        assert!(
            reference_edges
                .iter()
                .all(|e| e["confidence"].as_f64().unwrap_or(0.0) >= 1.0),
            "no reference edge may carry degraded confidence"
        );
    }

    #[test]
    fn test_client_call_mesh_deterministic_output() {
        let run = |out: &std::path::Path| {
            let config = DeployConfig {
                root: fixture_root(),
                out: out.to_path_buf(),
                format: OutputFormat::Json,
                check: false,
                max_pod_size: 20,
            };
            run_deploy(config).expect("Deploy failed");
        };

        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        run(dir_a.path());
        run(dir_b.path());

        for name in ["metacall.pods.json", "metacall.mesh.json"] {
            let a = std::fs::read(dir_a.path().join(name)).unwrap();
            let b = std::fs::read(dir_b.path().join(name)).unwrap();
            assert_eq!(a, b, "{name} must be byte-identical across two deploy runs");
        }
    }
}
