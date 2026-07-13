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

    for edge_idx in graph.graph.edge_indices() {
        let weight = &graph.graph[edge_idx];
        if weight.kind != crate::graph::EdgeKind::Import {
            continue;
        }
        let Some((src, dst)) = graph.graph.edge_endpoints(edge_idx) else {
            continue;
        };

        // Source must be a file in a known pod.
        let src_fid = match &graph.graph[src] {
            crate::graph::NodeData::File(f) => f.id,
            crate::graph::NodeData::Symbol(s) => s.file_id,
            _ => continue,
        };
        let Some(&pod_id) = file_to_pod.get(&src_fid) else {
            continue;
        };

        // Target must be an ExternalNode.
        let ext = match &graph.graph[dst] {
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
        if lf.exists() {
            return ExternalClassification::Classified {
                package_name: external.raw_path.clone(),
                version: parse_version_from_lockfile(lf, &external.raw_path),
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
        if lf.exists() {
            return ExternalClassification::Classified {
                package_name: external.raw_path.clone(),
                version: parse_version_from_lockfile(lf, &external.raw_path),
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
            if lock_path.exists() {
                return ExternalClassification::Classified {
                    package_name: external.raw_path.clone(),
                    version: parse_version_from_lockfile(&lock_path, &external.raw_path),
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
    if lf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: parse_version_from_go_sum(&lf, &external.raw_path),
            language: LangId::Go,
            source: DependencySource::Lockfile,
        };
    }

    let mf = root.join("go.mod");
    if mf.exists() {
        return ExternalClassification::Classified {
            package_name: external.raw_path.clone(),
            version: None,
            language: LangId::Go,
            source: DependencySource::Manifest,
        };
    }

    ExternalClassification::Unresolved {
        raw_path: external.raw_path.clone(),
        reason: "no go.sum or go.mod found".into(),
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
fn parse_version_from_lockfile(path: &Path, package: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    // Search for the package name, then grab the next quoted/hyphenated
    // version-like token on the same or next line.
    for line in content.lines() {
        if line.contains(package) {
            // Look for a semver-like pattern on this line or the next few.
            for candidate in content.lines().skip_while(|l| !l.contains(package)).take(5) {
                if let Some(v) = extract_semver(candidate) {
                    return Some(v);
                }
            }
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
    // go.sum format: <module> <version> <hash>
    // Take the first line matching the package.
    for line in content.lines() {
        if line.starts_with(package) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return Some(parts[1].to_string());
            }
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
