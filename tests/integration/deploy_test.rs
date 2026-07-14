#[cfg(feature = "metacall-deploy")]
mod deploy_tests {
    use meta_ast::deploy::{DeployConfig, run_deploy};
    use meta_ast::output::OutputFormat;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_deploy_python_calls_js() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed/python_calls_js");
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        let config = DeployConfig {
            root: root.clone(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");

        // Verify pod manifest was generated
        assert!(out_path.join("metacall.pods.json").exists());
        assert!(out_path.join("metacall.mesh.json").exists());

        // Verify pod manifest content
        let pod_content = fs::read_to_string(out_path.join("metacall.pods.json")).unwrap();
        let pod_json: serde_json::Value = serde_json::from_str(&pod_content).unwrap();
        assert_eq!(pod_json["version"], "1.0");
        assert!(!pod_json["deployments"].as_array().unwrap().is_empty());
        // python_calls_js has both Python and JS files
        let languages: Vec<&str> = pod_json["deployments"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["language"].as_str())
            .collect();
        assert!(languages.contains(&"py"), "expected a Python pod");
        assert!(languages.contains(&"node"), "expected a Node pod");
        // Mesh annotation still uses the old filename
        let mesh_content = fs::read_to_string(out_path.join("metacall.mesh.json")).unwrap();
        let mesh_json: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        assert_eq!(mesh_json["version"], "1.0");
        assert!(mesh_json["deployment_units"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_deploy_check_mode_pass() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed/python_calls_js");

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        // Copy everything to temp dir
        copy_dir_recursive(&root, &out_path).unwrap();

        let config = DeployConfig {
            root: out_path.clone(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: true,
        };

        // Without pre-existing cuts, the fairness check should pass
        let result = run_deploy(config);
        assert!(
            result.is_ok(),
            "check mode should pass without pre-existing metadata"
        );
    }

    fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                copy_dir_recursive(&entry.path(), &dst.join(entry.file_name()))?;
            } else {
                fs::copy(entry.path(), dst.join(entry.file_name()))?;
            }
        }
        Ok(())
    }
}
