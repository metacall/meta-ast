//! Regression coverage for the command line exit policy.

use std::path::PathBuf;
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

/// A parse failure is a warning, so the default policy keeps the run green and
/// an explicit warning policy fails it.
#[test]
fn graph_exit_status_follows_the_diagnostic_policy() {
    let dir = scratch("broken");
    std::fs::write(dir.join("broken.py"), "def oops(:\n").unwrap();

    let default_run = run(&["graph", dir.to_str().unwrap()]);
    assert_eq!(
        default_run.status.code(),
        Some(0),
        "a warning must not fail the default policy, stderr: {}",
        String::from_utf8_lossy(&default_run.stderr)
    );

    let strict_run = run(&["graph", dir.to_str().unwrap(), "--fail-on", "warning"]);
    assert_eq!(
        strict_run.status.code(),
        Some(1),
        "the warning policy must fail the run, stderr: {}",
        String::from_utf8_lossy(&strict_run.stderr)
    );

    let permissive_run = run(&["graph", dir.to_str().unwrap(), "--fail-on", "never"]);
    assert_eq!(
        permissive_run.status.code(),
        Some(0),
        "the permissive policy must never fail the run"
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
}

/// The deploy command applies the same diagnostic policy as inspect and graph.
#[cfg(feature = "metacall-deploy")]
#[test]
fn deploy_exit_status_follows_the_diagnostic_policy() {
    let dir = scratch("deploy_policy");
    std::fs::write(dir.join("broken.py"), "def oops(:\n").unwrap();
    let permissive_out = scratch("deploy_policy_never");
    let strict_out = scratch("deploy_policy_warning");

    let permissive = run(&[
        "deploy",
        dir.to_str().unwrap(),
        "--out",
        permissive_out.to_str().unwrap(),
        "--fail-on",
        "never",
    ]);
    assert_eq!(
        permissive.status.code(),
        Some(0),
        "the permissive policy must never fail the run, stderr: {}",
        String::from_utf8_lossy(&permissive.stderr)
    );

    let strict = run(&[
        "deploy",
        dir.to_str().unwrap(),
        "--out",
        strict_out.to_str().unwrap(),
        "--fail-on",
        "warning",
    ]);
    assert_eq!(
        strict.status.code(),
        Some(1),
        "the warning policy must fail the run, stderr: {}",
        String::from_utf8_lossy(&strict.stderr)
    );
}

/// Splitting the deploy entry point must not change a single emitted byte.
#[cfg(feature = "metacall-deploy")]
#[test]
fn deploy_emits_byte_identical_documents() {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mixed/python_calls_js");
    let first = scratch("deploy_bytes_first");
    let second = scratch("deploy_bytes_second");

    for out in [&first, &second] {
        let output = run(&[
            "deploy",
            root.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert!(
            output.status.success(),
            "deploy must succeed, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for name in ["metacall.pods.json", "metacall.mesh.json"] {
        let left = std::fs::read(first.join(name)).unwrap();
        let right = std::fs::read(second.join(name)).unwrap();
        assert_eq!(left, right, "{name} must be byte identical across runs");
        assert!(!left.is_empty(), "{name} must not be empty");
    }
}

/// The entry point parses, installs the subscriber and dispatches. Command
/// bodies live in the interface layer.
#[test]
fn the_entry_point_stays_a_dispatcher() {
    const MAIN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"));

    let lines = MAIN.lines().count();
    assert!(
        lines <= 90,
        "src/main.rs has {lines} lines; move command bodies into src/interface"
    );
    for token in [
        "format!(",
        "emit_",
        "discover_files(",
        "extract_with_options(",
        "GraphBuilder",
        "run_deploy",
        "run_watch(",
    ] {
        assert!(
            !MAIN.contains(token),
            "src/main.rs must not contain {token}"
        );
    }
    assert!(MAIN.contains("Cli::parse"));
}
