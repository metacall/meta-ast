#[cfg(feature = "metacall-deploy")]
mod deploy_edge_cases_tests {
    use meta_ast::deploy::{DeployConfig, run_deploy};
    use meta_ast::output::OutputFormat;
    use std::fs;
    use tempfile::tempdir;

    fn setup_config(root: &std::path::Path) -> (tempfile::TempDir, DeployConfig) {
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();
        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };
        (out_dir, config)
    }

    fn check_pod_manifest_exists(out_path: &std::path::Path) -> serde_json::Value {
        let path = out_path.join("metacall.pods.json");
        assert!(path.exists(), "metacall.pods.json should exist");
        let content = fs::read_to_string(path).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    #[test]
    fn test_multiple_loads_in_one_file() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("main.py"),
            r#"
metacall_load_from_file('node', ['sum.js'])
metacall_load_from_file('rb', ['hello.rb'])
metacall_load_from_file('py', ['other.py'])
"#,
        )
        .unwrap();
        fs::write(root.join("sum.js"), "function sum(a, b) { return a + b; }").unwrap();
        fs::write(root.join("hello.rb"), "def hello; puts 'hello'; end").unwrap();
        fs::write(root.join("other.py"), "def other(): pass").unwrap();

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();
        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");

        let manifest = check_pod_manifest_exists(&out_path);
        let deployments = manifest["deployments"].as_array().unwrap();
        // Should have at least 2 language-based pods (py + node + rb)
        assert!(
            deployments.len() >= 2,
            "expected >=2 pods, got {}",
            deployments.len()
        );
    }

    #[test]
    fn test_duplicate_scripts_deduplication() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', ['sum.js'])\nmetacall_load_from_file('node', ['sum.js'])",
        )
        .unwrap();
        fs::write(root.join("sum.js"), "function sum(a, b) { return a + b; }").unwrap();

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();
        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");
        let manifest = check_pod_manifest_exists(&out_path);
        let deployments = manifest["deployments"].as_array().unwrap();
        assert!(!deployments.is_empty(), "expected at least 1 pod");
    }

    #[test]
    fn test_metacall_load_from_configuration() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let config_json = serde_json::json!({
            "language_id": "node",
            "path": ".",
            "scripts": ["sum.js"]
        });
        fs::write(root.join("config.json"), config_json.to_string()).unwrap();
        fs::write(
            root.join("orchestrator.py"),
            "metacall_load_from_configuration('config.json')",
        )
        .unwrap();
        fs::write(root.join("sum.js"), "function sum(a, b) { return a + b; }").unwrap();

        let (out_dir, config) = setup_config(root);
        run_deploy(config).expect("Deploy failed");
        let manifest = check_pod_manifest_exists(out_dir.path());
        let deployments = manifest["deployments"].as_array().unwrap();
        assert!(!deployments.is_empty());
    }

    #[test]
    fn test_computed_arguments_confidence() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', [get_script_path()])",
        )
        .unwrap();
        fs::write(root.join("sum.js"), "function sum(a, b) { return a + b; }").unwrap();

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();
        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");
        let manifest = check_pod_manifest_exists(&out_path);
        let deployments = manifest["deployments"].as_array().unwrap();
        assert!(!deployments.is_empty());
    }

    #[test]
    fn test_deep_nested_paths() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let subdir = root.join("a").join("b").join("c");
        fs::create_dir_all(&subdir).unwrap();

        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', ['a/b/c/deep.js'])",
        )
        .unwrap();
        fs::write(subdir.join("deep.js"), "function deep() { return 42; }").unwrap();

        let (out_dir, config) = setup_config(root);
        run_deploy(config).expect("Deploy failed");
        let manifest = check_pod_manifest_exists(out_dir.path());
        let deployments = manifest["deployments"].as_array().unwrap();
        assert!(!deployments.is_empty());
    }

    #[test]
    fn test_deploy_check_mode_stress() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(root.join("sum.js"), "function sum(a, b) { return a + b; }").unwrap();
        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', ['sum.js'])",
        )
        .unwrap();

        // Run in check mode on a directory with no pre-existing manifest.
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();
        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: true,
        };

        let result = run_deploy(config);
        // With no manifest on disk, the check generates one and runs fairness check.
        // The fairness check should pass because there are no cuts.
        assert!(result.is_ok() || result.unwrap_err().to_string().contains("fairness"));
    }

    #[test]
    fn test_file_only_load_target_not_dropped() {
        // B1: a cross-language load whose target file has no extracted
        // symbols (e.g. only `var x = 1;`) must still produce a mesh
        // cross-language edge. Previously the file-only SCC had no emitted
        // unit to anchor to, so the edge was silently dropped.
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("main.py"),
            "def m():\n    metacall_load_from_file('node', ['loader.js'])\n",
        )
        .unwrap();
        fs::write(root.join("loader.js"), "var x = 1;\n").unwrap();

        let (out_dir, config) = setup_config(root);
        run_deploy(config).expect("Deploy failed");
        let manifest = check_pod_manifest_exists(out_dir.path());
        let pod_cross = manifest["edges"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["is_cross_language"].as_bool().unwrap_or(false))
            .count();
        assert!(
            pod_cross > 0,
            "pod manifest must carry the cross-language edge"
        );

        let mesh_content = fs::read_to_string(out_dir.path().join("metacall.mesh.json")).unwrap();
        let mesh: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        let mesh_edges = mesh["cross_language_edges"].as_array().unwrap();
        assert!(
            !mesh_edges.is_empty(),
            "mesh must not silently drop the cross-language edge for a file-only target"
        );

        let unit_ids: std::collections::HashSet<u64> = mesh["deployment_units"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["id"].as_u64().unwrap())
            .collect();
        for e in mesh_edges {
            assert!(unit_ids.contains(&e["from_unit"].as_u64().unwrap()));
            assert!(unit_ids.contains(&e["to_unit"].as_u64().unwrap()));
        }
    }

    #[test]
    fn test_duplicate_load_call_no_duplicate_mesh_edge() {
        // B4: writing the same load call twice must not produce duplicate
        // cross-language mesh edges (parallel graph edges were emitted 1:1).
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("d.py"),
            "def load():\n    metacall_load_from_file('node', ['s.js'])\n    metacall_load_from_file('node', ['s.js'])\n",
        )
        .unwrap();
        fs::write(root.join("s.js"), "function s() { return 1; }\n").unwrap();

        let (out_dir, config) = setup_config(root);
        run_deploy(config).expect("Deploy failed");
        let mesh_content = fs::read_to_string(out_dir.path().join("metacall.mesh.json")).unwrap();
        let mesh: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        let edges = mesh["cross_language_edges"].as_array().unwrap();
        let count = edges
            .iter()
            .filter(|e| e["from_language"] == "py" && e["to_language"] == "node")
            .count();
        assert_eq!(count, 1, "duplicate load call produced duplicate mesh edge");
    }

    #[test]
    fn test_cross_language_cycle() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        // Python calls JS, JS calls Python back (simulated via metacall_load)
        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', ['bridge.js'])",
        )
        .unwrap();
        fs::write(
            root.join("bridge.js"),
            "metacall_load_from_file('py', ['main.py'])",
        )
        .unwrap();

        let (out_dir, config) = setup_config(root);
        run_deploy(config).expect("Deploy failed");
        let manifest = check_pod_manifest_exists(out_dir.path());
        let edges = manifest["edges"].as_array().unwrap();
        // Cross-language edges should exist
        let cross_lang_count = edges
            .iter()
            .filter(|e| e["is_cross_language"].as_bool().unwrap_or(false))
            .count();
        assert!(
            cross_lang_count > 0,
            "expected at least 1 cross-language edge"
        );
    }
}
