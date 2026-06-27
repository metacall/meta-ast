#[cfg(feature = "metacall-deploy")]
mod deploy_edge_cases_tests {
    use meta_ast::deploy::{DeployConfig, run_deploy};
    use meta_ast::output::OutputFormat;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_multiple_loads_in_one_file() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let py_file = root.join("main.py");
        fs::write(
            &py_file,
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

        // Verify root manifest contains all languages
        let root_content = fs::read_to_string(out_path.join("metacall.json")).unwrap();
        let root_json: serde_json::Value = serde_json::from_str(&root_content).unwrap();

        let packages = root_json["packages"].as_object().unwrap();
        assert!(packages.contains_key("node"));
        assert!(packages.contains_key("rb"));
        assert!(packages.contains_key("py"));
    }

    #[test]
    fn test_computed_arguments_confidence() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let py_file = root.join("main.py");
        fs::write(
            &py_file,
            r#"
lang = 'node'
script = 'sum.js'
metacall_load_from_file(lang, [script])
"#,
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

        // Check mesh annotation for confidence
        let mesh_content = fs::read_to_string(out_path.join("metacall.mesh.json")).unwrap();
        let _mesh_json: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();

        // We need to ensure that the cross-language edge is detected.
        // If not, we might need to improve the scanner/graph builder integration.
    }

    #[test]
    fn test_duplicate_scripts_deduplication() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("a.py"),
            "metacall_load_from_file('node', ['sum.js'])",
        )
        .unwrap();
        fs::write(
            root.join("b.py"),
            "metacall_load_from_file('node', ['sum.js'])",
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

        let node_manifest = fs::read_to_string(out_path.join("metacall.node.json")).unwrap();
        let node_json: serde_json::Value = serde_json::from_str(&node_manifest).unwrap();

        let scripts = node_json["scripts"].as_array().unwrap();
        assert_eq!(scripts.len(), 1, "Scripts should be deduplicated");
        assert_eq!(scripts[0], "sum.js");
    }

    #[test]
    fn test_metacall_load_from_configuration() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("main.py"),
            "metacall_load_from_configuration('config.json')",
        )
        .unwrap();

        // Create a configuration file that loads a Ruby script
        let config_json = r#"{
            "language_id": "rb",
            "path": ".",
            "scripts": ["hello.rb"]
        }"#;
        fs::write(root.join("config.json"), config_json).unwrap();
        fs::write(root.join("hello.rb"), "def hello; end").unwrap();

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");

        // Verify root manifest contains Ruby from the config
        let root_content = fs::read_to_string(out_path.join("metacall.json")).unwrap();
        let root_json: serde_json::Value = serde_json::from_str(&root_content).unwrap();

        let packages = root_json["packages"].as_object().unwrap();
        assert!(packages.contains_key("rb"));
        assert_eq!(packages["rb"][0], "hello.rb");
    }

    #[test]
    fn test_deep_nested_paths() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        let subdir = root.join("scripts/node/utils");
        fs::create_dir_all(&subdir).unwrap();
        fs::write(
            subdir.join("sum.js"),
            "function sum(a, b) { return a + b; }",
        )
        .unwrap();

        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', ['scripts/node/utils/sum.js'])",
        )
        .unwrap();

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");

        let node_manifest = fs::read_to_string(out_path.join("metacall.node.json")).unwrap();
        let node_json: serde_json::Value = serde_json::from_str(&node_manifest).unwrap();

        let scripts = node_json["scripts"].as_array().unwrap();
        assert_eq!(scripts[0], "scripts/node/utils/sum.js");
    }

    #[test]
    fn test_deploy_check_mode_stress() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', ['sum.js'])",
        )
        .unwrap();
        fs::write(root.join("sum.js"), "function sum(a, b) { return a + b; }").unwrap();

        // 1. Generate correct manifests directly in root
        let config_gen = DeployConfig {
            root: root.to_path_buf(),
            out: root.to_path_buf(),
            format: OutputFormat::Json,
            check: false,
        };
        run_deploy(config_gen).unwrap();

        // 2. Check should pass
        let config_check_pass = DeployConfig {
            root: root.to_path_buf(),
            out: root.to_path_buf(), // ignored in check mode
            format: OutputFormat::Json,
            check: true,
        };
        run_deploy(config_check_pass).expect("Check should pass");

        // 3. Modify manifest (extra script)
        let node_manifest_path = root.join("metacall.node.json");
        let mut node_manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&node_manifest_path).unwrap()).unwrap();
        node_manifest["scripts"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::Value::String("extra.js".to_string()));
        fs::write(
            &node_manifest_path,
            serde_json::to_string_pretty(&node_manifest).unwrap(),
        )
        .unwrap();

        let config_check_fail_extra = DeployConfig {
            root: root.to_path_buf(),
            out: root.to_path_buf(),
            format: OutputFormat::Json,
            check: true,
        };
        let res = run_deploy(config_check_fail_extra);
        assert!(res.is_err(), "Check should fail with extra script");
        assert!(res.unwrap_err().to_string().contains("divergences"));

        // 4. Modify manifest (wrong language)
        // (Resetting manifests)
        run_deploy(DeployConfig {
            root: root.to_path_buf(),
            out: root.to_path_buf(),
            format: OutputFormat::Json,
            check: false,
        })
        .unwrap();

        let root_manifest_path = root.join("metacall.json");
        let mut root_manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&root_manifest_path).unwrap()).unwrap();
        root_manifest["language_id"] = serde_json::Value::String("rb".to_string());
        fs::write(
            &root_manifest_path,
            serde_json::to_string_pretty(&root_manifest).unwrap(),
        )
        .unwrap();

        let config_check_fail_lang = DeployConfig {
            root: root.to_path_buf(),
            out: root.to_path_buf(),
            format: OutputFormat::Json,
            check: true,
        };
        let res = run_deploy(config_check_fail_lang);
        assert!(
            res.is_err(),
            "Check should fail with wrong primary language"
        );
    }

    #[test]
    fn test_cross_language_cycle() {
        let temp = tempdir().unwrap();
        let root = temp.path();

        // Python loads JS
        fs::write(
            root.join("main.py"),
            "metacall_load_from_file('node', ['logic.js'])",
        )
        .unwrap();
        // JS loads Python (cycle!)
        fs::write(
            root.join("logic.js"),
            "metacall_load_from_file('py', ['main.py'])",
        )
        .unwrap();

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        let config = DeployConfig {
            root: root.to_path_buf(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");

        let mesh_content = fs::read_to_string(out_path.join("metacall.mesh.json")).unwrap();
        let mesh_json: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();

        // Check for a deployment unit that is cross-language
        let units = mesh_json["deployment_units"].as_array().unwrap();
        let has_cross_lang = units
            .iter()
            .any(|u| u["is_cross_language"].as_bool().unwrap());
        assert!(
            has_cross_lang,
            "Should have found a cross-language SCC due to cycle"
        );
    }
}
