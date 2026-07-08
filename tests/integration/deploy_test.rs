#[cfg(feature = "metacall-deploy")]
mod deploy_tests {
    use meta_ast::deploy::{DeployConfig, run_deploy};
    use meta_ast::output::OutputFormat;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_deploy_auth_function_mesh() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/examples/auth-function-mesh");
        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        let config = DeployConfig {
            root: root.clone(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: false,
        };

        run_deploy(config).expect("Deploy failed");

        // Verify files were generated
        assert!(out_path.join("metacall.json").exists());
        assert!(out_path.join("metacall.py.json").exists());
        assert!(out_path.join("metacall.node.json").exists());
        assert!(out_path.join("metacall.mesh.json").exists());

        // Verify root manifest content
        let root_content = fs::read_to_string(out_path.join("metacall.json")).unwrap();
        let root_json: serde_json::Value = serde_json::from_str(&root_content).unwrap();
        assert_eq!(root_json["language_id"], "py");
        assert!(root_json["packages"]["node"].is_array());

        // Verify mesh annotation
        let mesh_content = fs::read_to_string(out_path.join("metacall.mesh.json")).unwrap();
        let mesh_json: serde_json::Value = serde_json::from_str(&mesh_content).unwrap();
        assert_eq!(mesh_json["version"], "1.0");
        assert!(mesh_json["deployment_units"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn test_deploy_check_mode_fail() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/examples/auth-function-mesh");

        let out_dir = tempdir().unwrap();
        let out_path = out_dir.path().to_path_buf();

        // Copy everything to temp dir so we can modify it
        copy_dir_recursive(&root, &out_path).unwrap();

        // Modify the manifest to cause a failure
        let manifest_path = out_path.join("metacall.json");
        let mut content = fs::read_to_string(&manifest_path).unwrap();
        content = content.replace("\"language_id\": \"py\"", "\"language_id\": \"node\"");
        fs::write(&manifest_path, content).unwrap();

        let config = DeployConfig {
            root: out_path.clone(),
            out: out_path.clone(),
            format: OutputFormat::Json,
            check: true,
        };

        let result = run_deploy(config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("check failed"));
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
