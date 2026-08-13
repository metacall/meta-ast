# Deploy Module (`metacall-deploy`)

Feature-gated. Build with `--features metacall-deploy`.

## Overview

The `deploy` subcommand scans polyglot projects for MetaCall load and client call sites. It partitions files into same-language pods, resolves external dependencies from lockfiles, and generates two deployment artifacts:

| Artifact | Description |
|---|---|
| `metacall.pods.json` | Pod manifest with deployment units, inter-pod edges, dependency lists, and AST metrics |
| `metacall.mesh.json` | Function Mesh topology with SCC deployment units and call-site attribution |

## Usage

```bash
# Build with the feature enabled
cargo build --release --features metacall-deploy

# Generate manifests
meta-ast deploy <path> --out <output_dir>

# CI validation: verify every cut edge has an RPC stub entry
meta-ast deploy <path> --check
```

### Options

| Flag | Default | Description |
|---|---|---|
| `-o, --out <dir>` | `.` | Directory to write generated artifacts |
| `-f, --format <json\|yaml>` | `json` | Serialization format |
| `--check` | off | Fairness check mode: exits non-zero on missing RPC stubs |
| `--max-pod-size <N>` | `20` | Files per pod before rebalancing triggers |

---

## Pipeline

```
run_deploy()
  1. discover_files()                               - language-routed file list
  2. pipeline::analyze_graph()                      - symbol, import, and SCC analysis
  3. scanner::scan_file() per file (rayon parallel) - MetaCall call-site detection
  4. inject MetaCall import edges into graph        - add_metacall_edge with path resolution
  5. resolve client calls (two-phase)               - client_call::resolve_client_calls
  6. SCC recompute with new edges                   - update SCC analysis
  7. pod::partition_into_pods()                     - Union-Find over same-language edges
  8. metrics::compute_file_metrics()                - AST node counts per file/pod
  9. cut::find_cross_language_cuts()                - cheapest-edge split for cross-lang SCCs
 10. cut::find_oversized_pod_cut() per pod          - second-pass rebalancing
 11. dependency::resolve_dependencies()             - lockfile and manifest parsing
 12. manifest::generate_pod_manifest()              - PodManifest serialization
 13. mesh::generate_mesh_annotation()               - SCC-derived topology
 14. write artifacts or check::check_cut_fairness() in --check mode
```

### Module map

```
src/deploy/
├── mod.rs          Entry point: run_deploy(), DeployConfig, add_metacall_edge()
├── scanner.rs      tree-sitter call-site detection, CallSite, CallSiteVariant
├── client_call.rs  resolve_client_calls(), resolve_script_to_file() - two-phase client invocation resolution
├── pod.rs          Union-Find partition_into_pods(), PodPartition, InterPodEdge
├── cut.rs          find_cross_language_cuts(), find_oversized_pod_cut(), CutEdge
├── dependency.rs   classify_external(), resolve_dependencies(), per-language resolvers
├── metrics.rs      compute_file_metrics(), compute_pod_metrics(), FileMetrics
├── manifest.rs     generate_pod_manifest(), PodManifest, ManifestEdge
├── mesh.rs         generate_mesh_annotation(), DeploymentUnit, CrossLanguageEdge
├── check.rs        check_cut_fairness() - bijection check between cuts and rpc_stub edges
└── tags.rs         LangId <-> MetaCall tag mapping (py, node, ts, c, cpp, rs, go)
```

---

## Call Site Scanner

`scanner::scan_file` runs tree-sitter queries to detect MetaCall API load variants and client calls.

### Supported variants

| Variant | Detected functions |
|---|---|
| `LoadFromFile` | `metacall_load_from_file`, `LoadFromFile` (Go), bare `use` import (Rust), `load::from_single_file` (Rust) |
| `LoadFromMemory` | `metacall_load_from_memory`, `LoadFromMemory` |
| `LoadFromPackage` | `metacall_load_from_package`, `LoadFromPackage` |
| `LoadFromConfiguration` | `metacall_load_from_configuration`, `LoadFromConfiguration` |
| `ClientCall` | `metacall`, `metacall_await`, `metacallfms` (all); `metacallv`, `metacallt`, `metacall_function` (C/C++); Go `metacall.Call` / `metacall.Await`; Rust `metacall::metacall`, `metacall_no_arg`, `metacall_untyped`. Note: `metacall_handle` is excluded because argument layout varies per port |

### Confidence scoring

| Case | Score |
|---|---|
| String literal argument | `1.0` |
| Unique match in load-confirmed files (Phase A) | `1.0` |
| Multiple matches in load-confirmed files (Phase A) | `0.8` |
| Unique match in global name index (Phase B) | `0.6` |
| Multiple matches in global name index (Phase B) | `0.5` |
| Computed argument / function name | `0.4` |

### Language coverage

Queries cover all 9 supported languages: Python, JavaScript, TypeScript, TSX, C, C++, Rust, Go, and Ruby.

### Client invocation resolution

`client_call::resolve_client_calls` resolves `ClientCall` targets after scanning:

- **Phase A (Load-aware):** Maps caller load sites to candidate target files, matching function names within loaded scope.
- **Phase B (Global fallback):** Searches a project-wide index of symbol names when Phase A finds no match. Emits a Warning diagnostic if no match exists.

Ambiguity resolution is deterministic: one edge per matching symbol in path-sorted order. Client-call edges are `EdgeKind::Reference` from calling file to target symbol node.

`run_deploy` also warns on orphaned `metacall.json` files that no `LoadFromConfiguration` call site references.

---

## Pod Partitioning

`pod::partition_into_pods` uses Union-Find to group files into same-language deployment units.

Files sharing the same `LangId` and connected by Import or Reference edges join the same pod. Cross-language edges remain inter-pod edges. Ownership edges are excluded because they express file structure, not dependency.

### Confidence fusion

When both an Import edge and a Reference edge connect the same pod pair, confidence scores multiply to form a combined weight in [0.0, 1.0]. If only one edge type exists, its confidence is used directly.

### Language tag mapping

| Language | Tag |
|---|---|
| Python | `py` |
| JavaScript | `node` |
| TypeScript / TSX | `ts` |
| C | `c` |
| C++ | `cpp` |
| Rust | `rs` |
| Go | `go` |

---

## Cut Detection

`cut.rs` implements two cut rules:

1. **Cross-language SCC cuts:** Cuts the lowest-confidence internal edge when an SCC spans multiple languages (`CutReason::CrossLanguageScc`). The manifest marks these as RPC stubs.
2. **Oversized pod cuts:** Greedy single-pass cut on pods exceeding the pod size limit (`DEFAULT_MAX_POD_SIZE`, 20 files by default). Override the limit with `--max-pod-size <N>`.

---

## External Dependency Resolution

`dependency::resolve_dependencies` collects external imports per pod and inspects project lockfiles and manifests.

| Language(s) | Resolver | Lockfile (preferred) | Manifest (fallback) |
|---|---|---|---|
| Python | `classify_python` | `uv.lock`, `poetry.lock`, `Pipfile.lock` | `pyproject.toml`, `requirements.txt` |
| JS / TS / TSX | `classify_node_ecosystem` | `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml` | `package.json` |
| Rust | `classify_rust` | `Cargo.lock` | `Cargo.toml` |
| Go | `classify_go` | `go.sum` | `go.mod` |
| C / C++ | `classify_c_cpp_best_effort` | - | `conanfile.txt`, `vcpkg.json` |

Lockfiles supply pinned versions (`source: "Lockfile"`). Manifest fallbacks set `version: None` (`source: "Manifest"`).

---

## Pod Manifest Schema

`manifest::generate_pod_manifest` writes `metacall.pods.json`:

```json
{
  "version": "1.0",
  "deployments": [
    {
      "id": 0,
      "language": "py",
      "files": ["auth.py", "__init__.py"],
      "metrics": {
        "total_ast_nodes": 63,
        "file_count": 2,
        "symbol_count": 2
      },
      "dependencies": [
        {
          "name": "requests",
          "version": "2.32.3",
          "language": "python",
          "source": "Lockfile"
        }
      ]
    }
  ],
  "edges": [
    {
      "from_pod": 0,
      "to_pod": 1,
      "kind": "import",
      "confidence": 1.0,
      "is_cross_language": true,
      "cut_annotation": null
    }
  ],
  "metrics": {
    "total_pods": 2,
    "cross_language_edges": 1,
    "total_ast_nodes": 112
  }
}
```

---

## Mesh Annotation

`mesh::generate_mesh_annotation` exports SCC analysis to `metacall.mesh.json`.

```rust
MeshAnnotation {
    version: String,
    deployment_units: Vec<DeploymentUnit>,
    cross_language_edges: Vec<CrossLanguageEdge>,
    stats: MeshStats,
}

DeploymentUnit {
    id: usize,
    symbols: Vec<UnitSymbol>,
    is_cross_language: bool,
    is_mesh_candidate: bool,
    deployability: String,
}

CrossLanguageEdge {
    from_unit: usize,
    to_unit: usize,
    from_language: String,
    to_language: String,
    call_site: Option<String>,
    confidence: f64,
}
```

Units with `is_mesh_candidate = true` and `is_cross_language = false` deploy independently as Function Mesh services.

---

## Check Mode (Fairness)

`check::check_cut_fairness` validates RPC stub contracts:

1. Every cut edge appears in `manifest.edges[]` with a `cut_annotation`.
2. Cut edges have `kind: "rpc_stub"`.
3. Non-cut edges omit `cut_annotation`.

`run_deploy` exits non-zero if fairness checks fail.

---

## Edge-case Fixtures

Integration tests use fixtures in `tests/fixtures/mixed/`:

| Fixture | Coverage |
| --- | --- |
| `auth_microservice` | Baseline acyclic star graph (py loads go/node/ts). |
| `auth_microservice_level2` | Cross-language SCC cycle, intra-language cycle collapse, dynamic and config loads. |
| `auth_microservice_level3` | All four load variants, 3-file cycle, py-go round-trip cut, lockfile classification. |

---

## Extending the Scanner

To add call site detection for a new language:

1. Add a `static <LANG>_QUERY: LazyLock<Query>` in `scanner.rs`.
2. Add a dispatch arm to `scan_file`.
3. Map tags in `tags.rs`.
4. Add unit tests in `scanner.rs`.
