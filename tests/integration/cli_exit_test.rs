//! Regression coverage for the command line exit policy.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_meta-ast")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meta_ast_cli_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(binary()).args(args).output().unwrap()
}

/// `--help` and `--version` must not print the startup banner.
#[test]
fn help_and_version_skip_the_banner() {
    for flag in ["--help", "--version"] {
        let output = run(&[flag]);
        assert!(output.status.success(), "{flag} must succeed");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("Polyglot static analyzer"),
            "{flag} must not print the banner, got: {stderr}"
        );
    }
}

/// A parse failure is an error, so the default policy makes the run fail.
#[test]
fn graph_exits_non_zero_on_an_error_diagnostic() {
    let dir = scratch("broken");
    std::fs::write(dir.join("broken.py"), "def oops(:\n").unwrap();

    let output = run(&["graph", dir.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "graph must fail on an error diagnostic, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn graph_exits_zero_on_a_clean_project() {
    let dir = scratch("clean");
    std::fs::write(dir.join("ok.py"), "def ok():\n    return 1\n").unwrap();

    let output = run(&["graph", dir.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a clean project must exit zero, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        Path::new(&dir).exists(),
        "the fixture directory must stay in place"
    );
}
