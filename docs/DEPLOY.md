# Deploy Module (`metacall-deploy`)

> Feature-gated. Build with `--features metacall-deploy`.

## Overview

The `deploy` subcommand scans a polyglot project for cross-language `metacall_load_from_*` call sites, partitions files into same-language pods, resolves external dependencies from lockfiles, and generates two artifacts for the MetaCall Function Mesh deployment model:

| Artifact | Description |
|---|---|
| `metacall.pods.json` | Pod manifest: language-based deployment units, inter-pod edges with fused confidence scores, per-pod dependency lists with pinned versions, and AST node metrics |
| `metacall.mesh.json` | Function Mesh topology annotation with SCC-derived deployment units and cross-language call-site attribution |

## Usage

```bash
# Build with the feature enabled
cargo build --release --features metacall-deploy

# Generate manifests
meta-ast deploy <path> --out <output_dir>

# CI validation: verify every cut edge has a corresponding RPC stub entry
meta-ast deploy <path> --check
```

### Options

| Flag | Default | Description |
|---|---|---|
| `--out <dir>` | `.` | Directory to write generated artifacts |
| `-f, --format <json\|yaml>` | `json` | Serialization format |
| `--check` | off | Fairness check mode - exits non-zero on missing RPC stubs |

---

## Pipeline

```
run_deploy()
  1. discover_files()                               - language-routed file list
  2. pipeline::analyze_graph()                      - full symbol + import + SCC analysis
  3. scanner::scan_file() per file (rayon parallel) - MetaCall call-site detection
  4. inject MetaCall import edges into graph        - add_metacall_edge with path resolution
  5. SCC recompute with new edges                   -
  6. pod::partition_into_pods()                     - Union-Find over same-language edges
  7. metrics::compute_file_metrics()                - AST node counts per file/pod
  7. cut::find_cross_language_cuts()                - cheapest-edge split for cross-lang SCCs
  8. cut::find_oversized_pod_cut() per pod          - second-pass rebalancing
  9. dependency::resolve_dependencies()             - per-language lockfile/manifest parsing
 10. manifest::generate_pod_manifest()              - PodManifest serialization
 11. mesh::generate_mesh_annotation()               - SCC-derived topology
 12. write artifacts or check::check_cut_fairness() in --check mode
```

### Module map

```
src/deploy/
├── mod.rs          Entry point: run_deploy(), DeployConfig, add_metacall_edge()
├── scanner.rs      tree-sitter call-site detection, CallSite, CallSiteVariant
├── pod.rs          Union-Find partition_into_pods(), PodPartition, InterPodEdge
├── cut.rs          find_cross_language_cuts(), find_oversized_pod_cut(), CutEdge
├── dependency.rs   classify_external(), resolve_dependencies(), per-language resolvers
├── metrics.rs      compute_file_metrics(), compute_pod_metrics(), FileMetrics
├── manifest.rs     generate_pod_manifest(), PodManifest, ManifestEdge
├── mesh.rs         generate_mesh_annotation(), DeploymentUnit, CrossLanguageEdge
├── check.rs        check_cut_fairness() -- bijection check between cuts and rpc_stub edges
└── tags.rs         LangId <-> MetaCall tag mapping (py / node / ts / c / cpp / rs / go)
```

---

## Call Site Scanner

`scanner::scan_file` runs language-specific tree-sitter queries to detect four variants of the MetaCall load API.

### Supported variants

| Variant | Detected functions |
|---|---|
| `LoadFromFile` | `metacall_load_from_file`, `LoadFromFile` (Go), bare-name after `use` import (Rust) |
| `LoadFromMemory` | `metacall_load_from_memory`, `LoadFromMemory` |
| `LoadFromPackage` | `metacall_load_from_package`, `LoadFromPackage` |
| `LoadFromConfiguration` | `metacall_load_from_configuration`, `LoadFromConfiguration` |

### Confidence scoring

| Argument type | Score |
|---|---|
| String literal | `1.0` |
| Variable / expression | `0.4` |

Dynamic arguments (variables, computed paths) emit low-confidence call sites rather than being dropped.

### Language coverage

Queries exist for all 8 supported languages: Python, JavaScript, TypeScript, TSX, C, C++, Rust, Go.

---

## Pod Partitioning

`pod::partition_into_pods` uses Union-Find to group files into same-language deployment units.

A pod is a set of files connected by Import or Reference edges where every file shares the same `LangId`. Cross-language edges are never unioned, becoming inter-pod edges by construction. The algorithm runs in O(n + e) where n is the file count and e is the SCC-participating edge count.

Ownership edges are excluded from unioning -- they represent file containment, not dependency.

### Inter-pod edge confidence fusion

When both an Import edge (from the scanner) and a Reference edge (from scope resolution) exist between the same pod pair, their confidences are multiplied to produce a single combined weight clamped to [0.0, 1.0]. If only one edge type exists, its confidence is used directly.

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

Two cut strategies share one implementation in `cut.rs`:

**Cross-language SCC cuts (mandatory):** When an SCC spans multiple languages, the single lowest-confidence internal edge is selected as a cut point. The cut is recorded with `CutReason::CrossLanguageScc`. This edge must be converted to an RPC stub in the generated manifest.

**Oversized pod cuts (greedy):** After language-based partitioning, any pod exceeding `DEFAULT_MAX_POD_SIZE` (20 files) receives a single cut at its lowest-confidence internal edge. This is a single-pass greedy strategy, not iterative repartitioning.

---

## External Dependency Resolution

`dependency::resolve_dependencies` walks Import edges from each pod's files to `ExternalNode` targets, classifies each external, and groups results by pod ID.

Per-language resolvers dispatch via exhaustive `match` on `LangId`:

| Language(s) | Resolver | Lockfile (first) | Manifest (fallback) |
|---|---|---|---|
| Python | `classify_python` | `uv.lock`, `poetry.lock`, `Pipfile.lock` | `pyproject.toml`, `requirements.txt` |
| JS / TS / TSX | `classify_node_ecosystem` | `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml` | `package.json` |
| Rust | `classify_rust` | `Cargo.lock` | `Cargo.toml` |
| Go | `classify_go` | `go.sum` | `go.mod` |
| C / C++ | `classify_c_cpp_best_effort` | -- | `conanfile.txt`, `vcpkg.json` |

Lockfiles are always preferred over manifests because they carry exact pinned versions. Manifest fallbacks return `version: None`. C/C++ is best-effort only and silently falls back to `Unresolved` when no manifest convention is found.

Each pod's `dependencies` list in the manifest contains only the externals actually imported by files within that pod.

---

## Pod Manifest Schema

`manifest::generate_pod_manifest` produces a single `metacall.pods.json`:

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

- **`deployments`**: One entry per language pod. `files` lists all source files in the pod. `metrics` uses AST node count as the primary quantitative signal, with symbol count as context. `dependencies` lists externals scoped to this pod only.
- **`edges`**: Inter-pod edges with fused confidence weights. `is_cross_language` is true when the two pods use different languages. `cut_annotation` is present only on edges that were identified as cut points by the cut detection pass.
- **`metrics`**: Global counts across all pods.

The `source` field in dependency entries indicates where version information came from: `Lockfile` when a lockfile supplied a pinned version, `Manifest` when only a manifest file was found without an exact version.

---

## Mesh Annotation

`mesh::generate_mesh_annotation` maps SCC analysis output onto a topology description for MetaCall's Function Mesh. ExternalNode-only SCCs are excluded -- they have no file or symbol content to deploy.

`deployment_units` carries sequential ids (0, 1, 2, ...) over only the emitted units; SCC components that are file-only or ExternalNode-only are skipped during emission. Every `cross_language_edges` endpoint is therefore remapped through file ownership so it always resolves to a real unit id -- edges that land on a skipped component anchor to the unit that owns that file's symbols, and only truly orphaned edges are dropped.

```rust
MeshAnnotation {
    version: String,                          // "1.0"
    deployment_units: Vec<DeploymentUnit>,
    cross_language_edges: Vec<CrossLanguageEdge>,
    stats: MeshStats,
}

DeploymentUnit {
    id: usize,                      // sequential emitted-unit id (0-based)
    symbols: Vec<UnitSymbol>,
    is_cross_language: bool,        // SCC spans >1 language
    is_mesh_candidate: bool,        // SCC hint == Independent
    deployability: String,          // "independent" | "acyclic_dependency" | ...
}

CrossLanguageEdge {
    from_unit: usize,              // always a real deployment_units id
    to_unit: usize,                // always a real deployment_units id
    from_language: String,
    to_language: String,
    call_site: Option<String>,      // source file path of the metacall_load_from_* call
    confidence: f64,
}
```

A unit where `is_mesh_candidate = true` and `is_cross_language = false` can be deployed independently as a Function Mesh service. Cross-language edges in `cross_language_edges` carry `call_site` attribution when the edge originated from a detected MetaCall load call, and both endpoints always reference an emitted deployment unit.

---

## Check Mode (Fairness)

`check::check_cut_fairness` validates that every cut edge has a corresponding RPC stub entry in the manifest, following the same no-silent-drop principle as ADR 0003:

1. Every cut edge must appear in `manifest.edges[]` with a `cut_annotation`.
2. Every cut edge must have `kind: "rpc_stub"`.
3. No non-cut edge may carry a `cut_annotation`.

Exit behavior: `run_deploy` calls `anyhow::bail!` with a count of violations when any diagnostics are found, producing a non-zero exit code suitable for CI enforcement.

---

## Example: `auth-function-mesh`

Mirrored as a committed fixture at `tests/fixtures/mixed/auth_microservice/` and the
reference input used by the deploy integration tests. Source layout:

```
auth-function-mesh/
├── __init__.py
├── auth.py          # calls metacall_load_from_file('node', ['auth.js'])
└── auth/
    ├── auth.js      # requires('jsonwebtoken'); exports sign/verify
    └── package.json # declares jsonwebtoken dependency
```

Running `meta-ast deploy auth-function-mesh --out ./out` produces:

`metacall.pods.json`:
```json
{
  "version": "1.0",
  "deployments": [
    {
      "id": 0,
      "language": "py",
      "files": ["__init__.py", "auth.py"],
      "metrics": { "total_ast_nodes": 63, "file_count": 2, "symbol_count": 2 },
      "dependencies": []
    },
    {
      "id": 1,
      "language": "node",
      "files": ["auth/auth.js"],
      "metrics": { "total_ast_nodes": 49, "file_count": 1, "symbol_count": 2 },
      "dependencies": [
        { "name": "jsonwebtoken", "version": "9.0.0", "language": "java_script", "source": "Lockfile" }
      ]
    }
  ],
  "edges": [
    {
      "from_pod": 0,
      "to_pod": 1,
      "kind": "import",
      "confidence": 1.0,
      "is_cross_language": true
    }
  ],
  "metrics": {
    "total_pods": 2,
    "cross_language_edges": 1,
    "total_ast_nodes": 112
  }
}
```

`metacall.mesh.json`:
```json
{
  "version": "1.0",
  "deployment_units": [
    { "id": 5, "symbols": [{"name": "sign", ...}], "deployability": "independent", "is_mesh_candidate": true },
    { "id": 6, "symbols": [{"name": "verify", ...}], "deployability": "independent", "is_mesh_candidate": true },
    { "id": 7, "symbols": [{"name": "encrypt", ...}], "deployability": "independent", "is_mesh_candidate": true },
    { "id": 8, "symbols": [{"name": "decrypt", ...}], "deployability": "independent", "is_mesh_candidate": true }
  ],
  "cross_language_edges": [
    { "from_unit": 3, "to_unit": 1, "from_language": "py", "to_language": "node", "call_site": "auth.py", "confidence": 1.0 }
  ],
```
> Note: unit ids are sequential over emitted units (0-based), and every
> edge endpoint resolves to one of them. The exact ids depend on SCC
> ordering, but the edge never references a skipped component index.
  "stats": { "total_units": 4, "independent_candidates": 4, "languages": ["py", "node"] }
}
```

---

## Edge-case fixtures

The deploy integration tests (`tests/integration/deploy_mixed_test.rs`) drive
the entire module against committed fixtures under `tests/fixtures/mixed/`.
Each fixture is deliberately shaped to exercise a different branch of the
cut/scanner/cut-fairness machinery. A star-shaped fixture (one loader, N
leaf callees, no cycles) is insufficient: it never produces a non-trivial
SCC, so the SCC-driven cut paths stay dead. The tiered fixtures cover that:

| Fixture | What it stresses |
| --- | --- |
| `auth_microservice` | Baseline: acyclic star (py loads go/node/ts). Validates shape, not cuts. |
| `auth_microservice_level2` | Cross-language SCC cycle (py loads go AND go loads py back) -> `CrossLanguageScc` cut; intra-language cycle (orchestrator<->callback) collapse into one py pod; dynamic (`LoadFromMemory`) and config-driven (`LoadFromConfiguration`) loads. |
| `auth_microservice_level3` | Full-module stress: all four load variants (`LoadFromFile`, `LoadFromMemory`, `LoadFromPackage`, `LoadFromConfiguration`); an intra-language 3-file cycle (orchestrator<->cache<->queue); a py<->go metacall round-trip cut; external dependency classification (lockfile/manifest) for `express` and builtins. |

The `CutReason::OversizedPod` path is not reachable from a file fixture
(`DEFAULT_MAX_POD_SIZE` is 20 files); it is covered by a synthetic-graph
unit test in `src/deploy/cut.rs` (`oversized_pod_cut_fires_below_threshold`)
that passes a small `max_size` so a 4-file pod triggers the cut.

Add a new fixture when you extend a cut branch or a scanner variant. The
fixture must contain a real cycle (not just a star) for the cut code to run.

---

## Extending the Scanner

To add a new language's call site detection:

1. Add a `static <LANG>_QUERY: LazyLock<Query>` in `scanner.rs` using the language's tree-sitter grammar.
2. Add the new arm to the `match id` dispatch in `scan_file`.
3. Add the language tag mapping in `tags.rs` (`metacall_tag` and `from_metacall_tag`).
4. Add a fixture test in the `#[cfg(test)]` block in `scanner.rs`.
