//! Per-language external dependency resolution.
//!
//! Classifies `ExternalNode` entries (created during graph builder import
//! resolution) into resolved dependencies with package name and version.
//! Lockfiles are preferred over manifests for exact pinning.
//! C/C++ relies on best-effort classification only.

use std::path::Path;

use crate::graph::node::{DependencySource, ExternalClassification, ExternalNode};
use crate::language::LangId;

/// A resolved dependency entry for the pod manifest.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DependencyEntry {
    pub name: String,
    pub version: Option<String>,
    pub language: LangId,
    pub source: DependencySource,
}

/// Classify a single external dependency using language-specific strategies.
///
/// Dispatches by `external.language` via exhaustive match, following the
/// repo's enum-static-dispatch convention. Lockfiles are tried first;
/// if missing or unparseable, falls back to the manifest file. If that
/// also fails, returns `Unresolved` (never blocks).
pub fn classify_external(external: &ExternalNode, project_root: &Path) -> ExternalClassification {
    match external.language {
        LangId::Python => classify_python(external, project_root),
        LangId::JavaScript | LangId::TypeScript | LangId::Tsx => {
            classify_node_ecosystem(external, project_root)
        }
        LangId::Rust => classify_rust(external, project_root),
        LangId::Go => classify_go(external, project_root),
        LangId::Ruby => classify_ruby(external, project_root),
        LangId::C | LangId::Cpp => classify_c_cpp_best_effort(external, project_root),
    }
}

/// Resolve all external nodes in a graph and return per-pod dependency lists.
///
/// Walks Import edges from each pod's files to ExternalNode targets,
/// classifies each external, and groups results by pod ID.
pub fn resolve_dependencies(
    graph: &crate::graph::CodeGraph,
    partition: &crate::deploy::pod::PodPartition,
    project_root: &Path,
) -> std::collections::HashMap<usize, Vec<DependencyEntry>> {
    let mut deps: std::collections::HashMap<usize, Vec<DependencyEntry>> =
        std::collections::HashMap::new();

    // Build FileId -> pod_id lookup.
    let mut file_to_pod: std::collections::HashMap<crate::model::FileId, usize> =
        std::collections::HashMap::new();
    for pod in &partition.pods {
        for &fid in &pod.files {
            file_to_pod.insert(fid, pod.id);
        }
    }

    let g = graph.graph();

    for edge_idx in g.edge_indices() {
        let weight = &g[edge_idx];
        if weight.kind != crate::graph::EdgeKind::Import {
            continue;
        }
        let Some((src, dst)) = g.edge_endpoints(edge_idx) else {
            continue;
        };

        // Source must be a file in a known pod.
        let src_fid = match &g[src] {
            crate::graph::NodeData::File(f) => f.id,
            crate::graph::NodeData::Symbol(s) => s.file_id,
            _ => continue,
        };
        let Some(&pod_id) = file_to_pod.get(&src_fid) else {
            continue;
        };

        // Target must be an ExternalNode.
        let ext = match &g[dst] {
            crate::graph::NodeData::External(e) => e,
            _ => continue,
        };

        let classification = classify_external(ext, project_root);
        let entry = match &classification {
            ExternalClassification::Classified {
                package_name,
                version,
                language,
                source,
            } => DependencyEntry {
                name: package_name.clone(),
                version: version.clone(),
                language: *language,
                source: *source,
            },
            ExternalClassification::Unresolved { .. } => continue,
        };

        let pod_deps = deps.entry(pod_id).or_default();
        if !pod_deps.iter().any(|d| d.name == entry.name) {
            pod_deps.push(entry);
        }
    }

    deps
}

// ── Per-language resolvers ─────────────────────────────────────────

fn classify_python(external: &ExternalNode, root: &Path) -> ExternalClassification {
    let lockfiles = [
        root.join("uv.lock"),
        root.join("poetry.lock"),
        root.join("Pipfile.lock"),
    ];
    for lf in &lockfiles {
        if !lf.exists() {
            continue;
        }
        if let Some(version) = parse_version_from_lockfile(lf, &external.raw_path) {
            return ExternalClassification::Classified {
                package_name: external.raw_path.clone(),
                version: Some(version),
                language: LangId::Python,
                source: DependencySource::Lockfile,
            };
        }
    }

    let manifests = [root.join("pyproject.toml"), root.join("requirements.txt")];
    for mf in &manifests {
        if mf.exists() {
            return ExternalClassification::Classified {
                package_name: external.raw_path.clone(),
                version: None,
                language: LangId::Python,
                source: DependencySource::Manifest,
            };
        }
    }

    // Check immediate subdirectories (monorepo layout).
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }
            for mf in &manifests {
                let p = subdir.join(mf.file_name().unwrap_or_default());
                if p.exists() {
                    return ExternalClassification::Classified {
                        package_name: external.raw_path.clone(),
                        version: None,
                        language: LangId::Python,
                        source: DependencySource::Manifest,
                    };
                }
            }
        }
    }

    ExternalClassification::Unresolved {
        raw_path: external.raw_path.clone(),
        reason: "no Python lockfile or manifest found".into(),
    }
}

fn classify_node_ecosystem(external: &ExternalNode, root: &Path) -> ExternalClassification {
    // Check root-level lockfiles and manifests first.
    let lockfiles = [
        root.join("package-lock.json"),
        root.join("yarn.lock"),
        root.join("pnpm-lock.yaml"),
    ];
    for lf in &lockfiles {
        if !lf.exists() {
            continue;
        }
        if let Some(version) = parse_version_from_lockfile(lf, &external.raw_path) {
            return ExternalClassification::Classified {
                package_name: external.raw_path.clone(),
                version: Some(version),
                language: external.language,
                source: DependencySource::Lockfile,
            };
        }
    }

    let mf = root.join("package.json");
    if mf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: parse_version_from_package_json(&mf, &external.raw_path),
            language: external.language,
            source: DependencySource::Manifest,
        };
    }

    // Search immediate subdirectories for package.json (monorepo layout).
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let subdir = entry.path();
            if !subdir.is_dir() {
                continue;
            }
            let lock_path = subdir.join("package-lock.json");
            if lock_path.exists()
                && let Some(version) = parse_version_from_lockfile(&lock_path, &external.raw_path)
            {
                return ExternalClassification::Classified {
                    package_name: external.raw_path.clone(),
                    version: Some(version),
                    language: external.language,
                    source: DependencySource::Lockfile,
                };
            }
            let pkg_path = subdir.join("package.json");
            if pkg_path.exists() {
                return ExternalClassification::Classified {
                    package_name: external.raw_path.clone(),
                    version: parse_version_from_package_json(&pkg_path, &external.raw_path),
                    language: external.language,
                    source: DependencySource::Manifest,
                };
            }
        }
    }

    ExternalClassification::Unresolved {
        raw_path: external.raw_path.clone(),
        reason: "no Node.js lockfile or package.json found".into(),
    }
}

fn classify_rust(external: &ExternalNode, root: &Path) -> ExternalClassification {
    let lf = root.join("Cargo.lock");
    if lf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: parse_version_from_cargo_lock(&lf, &external.raw_path),
            language: LangId::Rust,
            source: DependencySource::Lockfile,
        };
    }

    let mf = root.join("Cargo.toml");
    if mf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: None,
            language: LangId::Rust,
            source: DependencySource::Manifest,
        };
    }

    ExternalClassification::Unresolved {
        raw_path: external.raw_path.clone(),
        reason: "no Cargo.lock or Cargo.toml found".into(),
    }
}

fn classify_go(external: &ExternalNode, root: &Path) -> ExternalClassification {
    let lf = root.join("go.sum");
    if lf.exists()
        && let Some(version) = parse_version_from_go_sum(&lf, &external.raw_path)
    {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: Some(version),
            language: LangId::Go,
            source: DependencySource::Lockfile,
        };
    }

    let mf = root.join("go.mod");
    if mf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: parse_version_from_go_mod(&mf, &external.raw_path),
            language: LangId::Go,
            source: DependencySource::Manifest,
        };
    }

    ExternalClassification::Unresolved {
        raw_path: external.raw_path.clone(),
        reason: "no go.sum or go.mod found".into(),
    }
}

fn classify_ruby(external: &ExternalNode, root: &Path) -> ExternalClassification {
    let lf = root.join("Gemfile.lock");
    if lf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: parse_version_from_gemfile_lock(&lf, &external.raw_path),
            language: LangId::Ruby,
            source: DependencySource::Lockfile,
        };
    }

    let mf = root.join("Gemfile");
    if mf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: None,
            language: LangId::Ruby,
            source: DependencySource::Manifest,
        };
    }

    ExternalClassification::Unresolved {
        raw_path: external.raw_path.clone(),
        reason: "no Gemfile.lock or Gemfile found".into(),
    }
}

fn classify_c_cpp_best_effort(external: &ExternalNode, root: &Path) -> ExternalClassification {
    // C/C++ has no universal convention. Try conanfile.txt, then vcpkg.json.
    // If neither exists, silently fall back to Unresolved.
    if root.join("conanfile.txt").exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: None,
            language: external.language,
            source: DependencySource::Manifest,
        };
    }
    if root.join("vcpkg.json").exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: None,
            language: external.language,
            source: DependencySource::Manifest,
        };
    }

    tracing::trace!(path = %external.raw_path, "C/C++ external dependency unresolved");
    ExternalClassification::Unresolved {
        raw_path: external.raw_path.clone(),
        reason: "no C/C++ manifest convention found (conanfile.txt, vcpkg.json)".into(),
    }
}

// ── Lockfile parsing helpers ───────────────────────────────────────

/// Best-effort version extraction from a lockfile by searching for the
/// package name followed by a version-like string. Returns None if the
/// package isn't found or the file can't be read.
/// True when the line names exactly this entry, in any lockfile grammar:
/// `name = "x"`, `x==1.2.3`, `"x": {`, `x (1.2.3)`, `"x@^1.2.3":`.
fn names_entry(line: &str, package: &str) -> bool {
    let trimmed = line.trim().trim_start_matches('-').trim();
    for token in trimmed.split(|c: char| {
        matches!(
            c,
            '=' | ' ' | '"' | '(' | ')' | '[' | ']' | ':' | ',' | '\'' | '{'
        )
    }) {
        let token = token.trim();
        if token == package {
            return true;
        }
        if let Some(rest) = token.strip_prefix(package)
            && matches!(
                rest.chars().next(),
                Some('@') | Some('^') | Some('~') | Some('>')
            )
        {
            return true;
        }
    }
    false
}

/// A `version = "x.y.z"` style assignment, whatever the surrounding syntax.
fn version_assignment(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_start_matches('-').trim();
    let rest = trimmed.strip_prefix("version")?.trim_start();
    if !(rest.starts_with('=') || rest.starts_with(':') || rest.starts_with('"')) {
        return None;
    }
    extract_semver(rest)
}

fn parse_version_from_lockfile(path: &Path, package: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut inside_entry = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // A new section or entry header closes the open entry.
        if trimmed.starts_with('[') {
            inside_entry = false;
        }

        if names_entry(trimmed, package) {
            if let Some(version) = version_assignment(trimmed) {
                return Some(version);
            }
            if (trimmed.contains("==") || trimmed.contains(">=") || trimmed.contains("~="))
                && let Some(version) = extract_semver(trimmed)
            {
                return Some(version);
            }
            inside_entry = true;
            continue;
        }

        if !inside_entry {
            continue;
        }
        if let Some(version) = version_assignment(trimmed) {
            return Some(version);
        }
        // A blank line ends a TOML entry that carried no version.
        if trimmed.is_empty() {
            inside_entry = false;
        }
    }
    None
}

fn parse_version_from_package_json(path: &Path, package: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    // Check dependencies/devDependencies for the package.
    for section in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(version) = json.get(section).and_then(|d| d.get(package))
            && let Some(s) = version.as_str()
        {
            return Some(s.to_string());
        }
    }
    None
}

fn parse_version_from_cargo_lock(path: &Path, package: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // Cargo.lock uses TOML; search for [[package]] sections with name = "..."
    let mut in_package_section = false;
    for line in content.lines() {
        if line.trim_start().starts_with("[[package]]") {
            in_package_section = false;
        }
        if let Some(rest) = line.strip_prefix("name = ") {
            let name = rest.trim().trim_matches('"');
            if name == package {
                in_package_section = true;
            }
        }
        if in_package_section && let Some(rest) = line.strip_prefix("version = ") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn parse_version_from_go_sum(path: &Path, package: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // go.sum format: <module> <version> <hash>, one line per module version.
    for line in content.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() != Some(package) {
            continue;
        }
        if let Some(version) = parts.next() {
            return Some(version.to_string());
        }
    }
    None
}

/// Read a `require` line, in single or block form, and drop the `v` prefix.
fn parse_version_from_go_mod(path: &Path, module: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut inside_block = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("require (") {
            inside_block = true;
            continue;
        }
        if inside_block && trimmed.starts_with(')') {
            inside_block = false;
            continue;
        }
        let entry = if let Some(rest) = trimmed.strip_prefix("require ") {
            rest
        } else if inside_block {
            trimmed
        } else {
            continue;
        };
        let mut parts = entry.split_whitespace();
        if parts.next() != Some(module) {
            continue;
        }
        if let Some(version) = parts.next() {
            return Some(version.trim_start_matches('v').to_string());
        }
    }
    None
}

fn parse_version_from_gemfile_lock(path: &Path, name: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // Gemfile.lock lists each gem as "  <name> (<version>)" under a specs
    // section. The name has no quotes; match the indented line exactly.
    for line in content.lines() {
        let line = line.trim_start();
        if let Some(rest) = line
            .strip_prefix(&format!("{name} ("))
            .and_then(|rest| rest.strip_suffix(')'))
        {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Extract the first semver-like substring from a line.
fn extract_semver(line: &str) -> Option<String> {
    let mut chars = line.chars().peekable();
    let mut start = None;
    let mut i = 0usize;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            // Potential start of a version.
            let mut version = String::new();
            let mut dot_count = 0;
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    version.push(c);
                    chars.next();
                } else if c == '.' {
                    version.push(c);
                    dot_count += 1;
                    chars.next();
                } else {
                    break;
                }
            }
            if dot_count >= 2 && !version.is_empty() {
                return Some(version);
            }
            start = Some(i);
        } else {
            chars.next();
        }
        i += 1;
    }
    let _ = start;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn external_node(raw_path: &str) -> ExternalNode {
        ExternalNode {
            raw_path: raw_path.to_string(),
            language: LangId::Ruby,
            classification: None,
        }
    }

    fn python_node(raw_path: &str) -> ExternalNode {
        ExternalNode {
            raw_path: raw_path.to_string(),
            language: LangId::Python,
            classification: None,
        }
    }

    fn go_node(raw_path: &str) -> ExternalNode {
        ExternalNode {
            raw_path: raw_path.to_string(),
            language: LangId::Go,
            classification: None,
        }
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("meta_ast_ruby_dep_{name}"));
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_version_from_gemfile_lock_extracts_version() {
        let dir = test_dir("parse");
        let lf = dir.join("Gemfile.lock");
        std::fs::write(
            &lf,
            "GEM\n  remote: https://rubygems.org/\n  specs:\n    rails (7.0.8.4)\n      actioncable (= 7.0.8.4)\n",
        )
        .unwrap();

        assert_eq!(
            parse_version_from_gemfile_lock(&lf, "rails"),
            Some("7.0.8.4".to_string())
        );
        assert_eq!(parse_version_from_gemfile_lock(&lf, "missing"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classify_ruby_uses_gemfile_lock() {
        let dir = test_dir("classify_lock");
        std::fs::write(
            dir.join("Gemfile.lock"),
            "GEM\n  specs:\n    rails (7.0.8.4)\n",
        )
        .unwrap();

        let classification = classify_ruby(&external_node("rails"), &dir);
        match classification {
            ExternalClassification::Classified {
                package_name,
                version,
                language,
                source,
            } => {
                assert_eq!(package_name, "rails");
                assert_eq!(version.as_deref(), Some("7.0.8.4"));
                assert_eq!(language, LangId::Ruby);
                assert_eq!(source, DependencySource::Lockfile);
            }
            other => panic!("expected Classified, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn classify_ruby_unresolved_without_gemfile() {
        let missing = std::env::temp_dir().join("meta_ast_ruby_dep_missing_dir");
        let classification = classify_ruby(&external_node("rails"), &missing);
        match classification {
            ExternalClassification::Unresolved { raw_path, reason } => {
                assert_eq!(raw_path, "rails");
                assert_eq!(reason, "no Gemfile.lock or Gemfile found");
            }
            other => panic!("expected Unresolved, got {other:?}"),
        }
    }

    /// A lockfile entry name must match the package exactly, not as a substring.
    #[test]
    fn lockfile_match_requires_the_exact_entry() {
        let dir = test_dir("lockfile_boundary");
        let lf = dir.join("uv.lock");
        std::fs::write(
            &lf,
            "[[package]]\nname = \"requests-toolbelt\"\nversion = \"4.0.0\"\n",
        )
        .unwrap();

        assert_eq!(
            parse_version_from_lockfile(&lf, "requests"),
            None,
            "requests must not match the requests-toolbelt entry"
        );
        assert_eq!(
            parse_version_from_lockfile(&lf, "requests-toolbelt").as_deref(),
            Some("4.0.0")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A lockfile that does not list the package must not be reported as the
    /// source of a resolved dependency.
    #[test]
    fn lockfile_without_the_entry_is_not_a_lockfile_hit() {
        let dir = test_dir("lockfile_missing_entry");
        std::fs::write(
            dir.join("uv.lock"),
            "[[package]]\nname = \"requests-toolbelt\"\nversion = \"4.0.0\"\n",
        )
        .unwrap();

        let classification = classify_python(&python_node("requests"), &dir);
        assert!(
            !matches!(
                classification,
                ExternalClassification::Classified {
                    source: DependencySource::Lockfile,
                    ..
                }
            ),
            "an entry the lockfile does not contain is not a lockfile hit: {classification:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A go.sum module path must match exactly, not as a prefix.
    #[test]
    fn go_sum_requires_an_exact_module_path() {
        let dir = test_dir("go_sum_boundary");
        let lf = dir.join("go.sum");
        std::fs::write(&lf, "github.com/foo/bar/baz v1.0.0 h1:AAAA=\n").unwrap();

        assert_eq!(
            parse_version_from_go_sum(&lf, "github.com/foo/bar"),
            None,
            "a prefix must not match a different module"
        );
        assert_eq!(
            parse_version_from_go_sum(&lf, "github.com/foo/bar/baz").as_deref(),
            Some("v1.0.0")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// go.mod `require` lines carry the version.
    #[test]
    fn go_mod_require_line_yields_a_version() {
        let dir = test_dir("go_mod_require");
        std::fs::write(
            dir.join("go.mod"),
            "module example.com/app\n\ngo 1.21\n\nrequire github.com/foo/bar v1.2.3\n",
        )
        .unwrap();

        let classification = classify_go(&go_node("github.com/foo/bar"), &dir);
        match classification {
            ExternalClassification::Classified {
                version,
                language,
                source,
                ..
            } => {
                assert_eq!(language, LangId::Go);
                assert_eq!(source, DependencySource::Manifest);
                let version = version.unwrap_or_default();
                assert!(
                    version.contains("1.2.3"),
                    "the go.mod require version must be read, got {version:?}"
                );
            }
            other => panic!("expected Classified, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every source form the resolver table lists must keep resolving the same
    /// way: which file wins, whether a version is found, and the source label.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Expectation {
        Lockfile(Option<&'static str>),
        Manifest(Option<&'static str>),
        Unresolved,
    }

    struct SourceCase {
        name: &'static str,
        language: LangId,
        package: &'static str,
        file: &'static str,
        content: &'static str,
        nested: bool,
        expectation: Expectation,
    }

    #[test]
    fn ecosystem_sources_resolve_through_one_table() {
        use LangId::{C, Cpp, Go, JavaScript, Python, Ruby, Rust as RustLang};

        let cases = [
            SourceCase {
                name: "uv lock",
                language: Python,
                package: "requests",
                file: "uv.lock",
                content: "[[package]]\nname = \"requests\"\nversion = \"2.32.3\"\n",
                nested: false,
                expectation: Expectation::Lockfile(Some("2.32.3")),
            },
            SourceCase {
                name: "poetry lock",
                language: Python,
                package: "requests",
                file: "poetry.lock",
                content: "[[package]]\nname = \"requests\"\nversion = \"2.32.3\"\n",
                nested: false,
                expectation: Expectation::Lockfile(Some("2.32.3")),
            },
            SourceCase {
                name: "requirements manifest",
                language: Python,
                package: "requests",
                file: "requirements.txt",
                content: "requests==2.32.3\n",
                nested: false,
                expectation: Expectation::Manifest(None),
            },
            SourceCase {
                name: "pyproject manifest",
                language: Python,
                package: "requests",
                file: "pyproject.toml",
                content: "[project]\nname = \"app\"\n",
                nested: false,
                expectation: Expectation::Manifest(None),
            },
            SourceCase {
                name: "pyproject in a subdirectory",
                language: Python,
                package: "requests",
                file: "pyproject.toml",
                content: "[project]\nname = \"app\"\n",
                nested: true,
                expectation: Expectation::Manifest(None),
            },
            SourceCase {
                name: "no python source",
                language: Python,
                package: "requests",
                file: "",
                content: "",
                nested: false,
                expectation: Expectation::Unresolved,
            },
            SourceCase {
                name: "yarn lock",
                language: JavaScript,
                package: "express",
                file: "yarn.lock",
                content: "\"express@^4.18.2\":\n  version \"4.18.2\"\n",
                nested: false,
                expectation: Expectation::Lockfile(Some("4.18.2")),
            },
            SourceCase {
                name: "pnpm lock",
                language: JavaScript,
                package: "express",
                file: "pnpm-lock.yaml",
                content: "  express@4.18.2:\n    version: 4.18.2\n",
                nested: false,
                expectation: Expectation::Lockfile(Some("4.18.2")),
            },
            SourceCase {
                name: "package json",
                language: JavaScript,
                package: "express",
                file: "package.json",
                content: "{\"dependencies\": {\"express\": \"4.18.2\"}}\n",
                nested: false,
                expectation: Expectation::Manifest(Some("4.18.2")),
            },
            SourceCase {
                name: "package json in a subdirectory",
                language: JavaScript,
                package: "express",
                file: "package.json",
                content: "{\"dependencies\": {\"express\": \"4.18.2\"}}\n",
                nested: true,
                expectation: Expectation::Manifest(Some("4.18.2")),
            },
            SourceCase {
                name: "no node source",
                language: JavaScript,
                package: "express",
                file: "",
                content: "",
                nested: false,
                expectation: Expectation::Unresolved,
            },
            SourceCase {
                name: "cargo lock",
                language: RustLang,
                package: "serde",
                file: "Cargo.lock",
                content: "[[package]]\nname = \"serde\"\nversion = \"1.0.203\"\n",
                nested: false,
                expectation: Expectation::Lockfile(Some("1.0.203")),
            },
            SourceCase {
                name: "cargo manifest",
                language: RustLang,
                package: "serde",
                file: "Cargo.toml",
                content: "[dependencies]\nserde = \"1\"\n",
                nested: false,
                expectation: Expectation::Manifest(None),
            },
            SourceCase {
                name: "no rust source",
                language: RustLang,
                package: "serde",
                file: "",
                content: "",
                nested: false,
                expectation: Expectation::Unresolved,
            },
            SourceCase {
                name: "go sum",
                language: Go,
                package: "github.com/pkg/errors",
                file: "go.sum",
                content: "github.com/pkg/errors v0.9.1 h1:abc\n",
                nested: false,
                expectation: Expectation::Lockfile(Some("v0.9.1")),
            },
            SourceCase {
                name: "go mod",
                language: Go,
                package: "github.com/pkg/errors",
                file: "go.mod",
                content: "module app\n\nrequire (\n\tgithub.com/pkg/errors v0.9.1\n)\n",
                nested: false,
                expectation: Expectation::Manifest(Some("0.9.1")),
            },
            SourceCase {
                name: "no go source",
                language: Go,
                package: "github.com/pkg/errors",
                file: "",
                content: "",
                nested: false,
                expectation: Expectation::Unresolved,
            },
            SourceCase {
                name: "gemfile lock",
                language: Ruby,
                package: "rails",
                file: "Gemfile.lock",
                content: "GEM\n  specs:\n    rails (7.0.8.4)\n",
                nested: false,
                expectation: Expectation::Lockfile(Some("7.0.8.4")),
            },
            SourceCase {
                name: "gemfile",
                language: Ruby,
                package: "rails",
                file: "Gemfile",
                content: "source \"https://rubygems.org\"\ngem \"rails\"\n",
                nested: false,
                expectation: Expectation::Manifest(None),
            },
            SourceCase {
                name: "no ruby source",
                language: Ruby,
                package: "rails",
                file: "",
                content: "",
                nested: false,
                expectation: Expectation::Unresolved,
            },
            SourceCase {
                name: "conan manifest",
                language: C,
                package: "zlib/1.3.1",
                file: "conanfile.txt",
                content: "[requires]\nzlib/1.3.1\n",
                nested: false,
                expectation: Expectation::Manifest(None),
            },
            SourceCase {
                name: "vcpkg manifest",
                language: Cpp,
                package: "zlib",
                file: "vcpkg.json",
                content: "{\"dependencies\": [\"zlib\"]}\n",
                nested: false,
                expectation: Expectation::Manifest(None),
            },
            SourceCase {
                name: "no c source",
                language: C,
                package: "sqlite3",
                file: "",
                content: "",
                nested: false,
                expectation: Expectation::Unresolved,
            },
        ];

        for case in cases {
            let dir = test_dir(case.name);
            let target = if case.nested {
                let nested = dir.join("pkg");
                std::fs::create_dir_all(&nested).unwrap();
                nested
            } else {
                dir.clone()
            };
            if !case.file.is_empty() {
                std::fs::write(target.join(case.file), case.content).unwrap();
            }

            let external = ExternalNode {
                raw_path: case.package.to_string(),
                language: case.language,
                classification: None,
            };
            let result = classify_external(&external, &dir);
            let (source, version) = match &result {
                ExternalClassification::Classified {
                    source, version, ..
                } => (Some(*source), version.clone()),
                ExternalClassification::Unresolved { .. } => (None, None),
            };

            match case.expectation {
                Expectation::Lockfile(expected) => {
                    assert_eq!(
                        source,
                        Some(DependencySource::Lockfile),
                        "{}: expected a lockfile hit, got {result:?}",
                        case.name
                    );
                    assert_eq!(version.as_deref(), expected, "{}: version", case.name);
                }
                Expectation::Manifest(expected) => {
                    assert_eq!(
                        source,
                        Some(DependencySource::Manifest),
                        "{}: expected a manifest hit, got {result:?}",
                        case.name
                    );
                    assert_eq!(version.as_deref(), expected, "{}: version", case.name);
                }
                Expectation::Unresolved => assert_eq!(
                    source, None,
                    "{}: expected no classification, got {result:?}",
                    case.name
                ),
            }

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// The ecosystems are not symmetric, and the descriptor table has to keep
    /// every difference: a lockfile without the entry still names the lockfile
    /// for Rust and Ruby, Python, Node and Go fall through to the manifest, and
    /// only Python and Node look inside an immediate subdirectory.
    #[test]
    fn ecosystem_lockfile_asymmetries_are_preserved() {
        let node = |language: LangId, raw: &str| ExternalNode {
            raw_path: raw.to_string(),
            language,
            classification: None,
        };
        let hit = |result: &ExternalClassification| match result {
            ExternalClassification::Classified {
                source, version, ..
            } => (Some(*source), version.clone()),
            ExternalClassification::Unresolved { .. } => (None, None),
        };
        let scratch = [
            "asymmetry_rust",
            "asymmetry_ruby",
            "asymmetry_python",
            "asymmetry_go",
            "asymmetry_node_subdir",
            "asymmetry_node_nested_yarn",
        ];
        for name in scratch {
            let _ = std::fs::remove_dir_all(test_dir(name));
        }

        // A Cargo.lock that does not carry the crate still names the lockfile.
        let dir = test_dir("asymmetry_rust");
        std::fs::write(
            dir.join("Cargo.lock"),
            "[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        assert_eq!(
            hit(&classify_external(&node(LangId::Rust, "serde"), &dir)),
            (Some(DependencySource::Lockfile), None),
            "a Cargo.lock without the crate stays the source for Rust"
        );

        // The same holds for Gemfile.lock.
        let dir = test_dir("asymmetry_ruby");
        std::fs::write(
            dir.join("Gemfile.lock"),
            "GEM\n  specs:\n    other (1.0.0)\n",
        )
        .unwrap();
        assert_eq!(
            hit(&classify_external(&node(LangId::Ruby, "rails"), &dir)),
            (Some(DependencySource::Lockfile), None),
            "a Gemfile.lock without the gem stays the source for Ruby"
        );

        // Python falls through a lockfile that does not carry the entry.
        let dir = test_dir("asymmetry_python");
        std::fs::write(
            dir.join("uv.lock"),
            "[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("pyproject.toml"), "[project]\nname = \"app\"\n").unwrap();
        assert_eq!(
            hit(&classify_external(&node(LangId::Python, "requests"), &dir)),
            (Some(DependencySource::Manifest), None),
            "Python falls through a lockfile without the entry"
        );

        // Go falls through go.sum to go.mod, which carries the version.
        let dir = test_dir("asymmetry_go");
        std::fs::write(dir.join("go.sum"), "github.com/other/mod v1.0.0 h1:AAAA=\n").unwrap();
        std::fs::write(
            dir.join("go.mod"),
            "module example.com/app\n\nrequire github.com/foo/bar v1.2.3\n",
        )
        .unwrap();
        assert_eq!(
            hit(&classify_external(
                &node(LangId::Go, "github.com/foo/bar"),
                &dir
            )),
            (Some(DependencySource::Manifest), Some("1.2.3".to_string())),
            "Go falls through go.sum to go.mod"
        );

        // Node answers from an immediate subdirectory, but only for
        // package.json and package-lock.json.
        let dir = test_dir("asymmetry_node_subdir");
        let nested = dir.join("pkg");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("package.json"),
            "{\"dependencies\":{\"react\":\"18.3.1\"}}",
        )
        .unwrap();
        assert_eq!(
            hit(&classify_external(&node(LangId::JavaScript, "react"), &dir)),
            (Some(DependencySource::Manifest), Some("18.3.1".to_string())),
            "a nested package.json answers for Node"
        );

        let dir = test_dir("asymmetry_node_nested_yarn");
        let nested = dir.join("pkg");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            nested.join("yarn.lock"),
            "react@^18:\n  version \"18.3.1\"\n",
        )
        .unwrap();
        assert_eq!(
            hit(&classify_external(&node(LangId::JavaScript, "react"), &dir)),
            (None, None),
            "a nested yarn.lock is not consulted"
        );

        for name in scratch {
            let _ = std::fs::remove_dir_all(test_dir(name));
        }
    }
}
