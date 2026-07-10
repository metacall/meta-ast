# Deploy Module (`metacall-deploy`)

> Feature-gated. Build with `--features metacall-deploy`.

## Overview

The `deploy` subcommand scans a polyglot project for cross-language `metacall_load_from_*` call sites and generates the three artifacts that `metacall-deploy` expects:

| Artifact | Description |
|---|---|
| `metacall.json` | Root manifest composing all language groups |
| `metacall.<tag>.json` | Per-language manifest (e.g. `metacall.py.json`) |
| `metacall.mesh.json` | Function Mesh annotation derived from SCC analysis |

## Usage

```bash
# Build with the feature enabled
cargo build --release --features metacall-deploy

# Generate manifests
meta-ast deploy <project_path> --out <output_dir>

# Validate existing manifests against source (CI mode)
meta-ast deploy <project_path> --check
```

### Options

| Flag | Default | Description |
|---|---|---|
| `--out <dir>` | `.` | Directory to write generated artifacts |
| `-f, --format <json\|yaml>` | `json` | Serialization format |
| `--check` | off | Diff mode - exits non-zero on divergence |

---

## Pipeline

```
run_deploy()
  1. discover_files()             - language-routed file list
  2. scanner::scan_file()         - tree-sitter queries per language
  3. pipeline::analyze_graph()    - full symbol + SCC analysis
  4. inject cross-language edges  - MetaCall call sites -> graph edges
  5. SCC recompute                - with new cross-language edges
  6. manifest::generate_manifests()
  7. mesh::generate_mesh_annotation()
  8. write artifacts  (or check::check_manifests() in --check mode)
```

### Module map

```
src/deploy/
├── mod.rs          Entry point: run_deploy(), DeployConfig
├── scanner.rs      tree-sitter call site detection, CallSite, CallSiteVariant
├── manifest.rs     DeployManifest, RootManifest, generate_manifests()
├── mesh.rs         MeshAnnotation, DeploymentUnit, CrossLanguageEdge, generate_mesh_annotation()
├── check.rs        check_manifests() - diff generated vs on-disk manifests
└── tags.rs         LangId <-> MetaCall tag mapping (py / node / ts / c / cpp / rs / go)
```

---

## Call Site Scanner

`scanner::scan_file` runs language-specific tree-sitter queries to detect four variants of the MetaCall load API.

### Supported variants

| Variant | Detected functions |
|---|---|
| `LoadFromFile` | `metacall_load_from_file`, `LoadFromFile` (Go) |
| `LoadFromMemory` | `metacall_load_from_memory`, `LoadFromMemory` |
| `LoadFromPackage` | `metacall_load_from_package`, `LoadFromPackage` |
| `LoadFromConfiguration` | `metacall_load_from_configuration`, `LoadFromConfiguration` |

### Confidence scoring

| Argument type | Score |
|---|---|
| String literal | `1.0` |
| Variable / expression | `0.4` |

Dynamic arguments (variables, computed paths) are not dropped - they are emitted as low-confidence call sites so the mesh annotation still captures the dependency.

### Language coverage

Queries exist for all 8 supported languages: Python, JavaScript, TypeScript, TSX, C, C++, Rust, Go.

---

## Manifest Generation

`manifest::generate_manifests` groups all discovered files and call-site script references by their MetaCall language tag.

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

### Primary language heuristic

The root manifest's `language_id` is inferred as the caller language from the first call site found. If no call sites exist, it defaults to `node` or the language of files in the project root.

### `metacall_load_from_configuration` handling

When a `LoadFromConfiguration` call site is detected, the target path is read from disk and parsed as either a `RootManifest` or `DeployManifest`. The contained scripts are merged into the language groups of the current project - allowing nested manifest composition.

---

## Mesh Annotation

`mesh::generate_mesh_annotation` maps SCC analysis output onto a topology description for MetaCall's Function Mesh.

### Output types

```rust
MeshAnnotation {
    version: String,                      // "1.0"
    deployment_units: Vec<DeploymentUnit>,
    cross_language_edges: Vec<CrossLanguageEdge>,
    stats: MeshStats,
}

DeploymentUnit {
    id: usize,                   // SCC component index
    symbols: Vec<UnitSymbol>,
    is_cross_language: bool,     // SCC spans >1 language
    is_mesh_candidate: bool,     // SCC hint == Independent
    deployability: String,       // "independent" | "cyclic"
}

CrossLanguageEdge {
    from_unit: usize,
    to_unit: usize,
    from_language: String,
    to_language: String,
    confidence: f64,
}
```

A unit where `is_mesh_candidate = true` and `is_cross_language = false` can be deployed independently as a Function Mesh service. Cross-language edges in `cross_language_edges` represent explicit MetaCall runtime boundaries.

---

## Check Mode

`check::check_manifests` compares the freshly generated manifests against whatever `metacall.json` / `metacall.<tag>.json` files exist on disk.

Diagnostic conditions:
- Root manifest missing (`metacall.json` absent).
- `language_id` mismatch between generated and existing root manifest.
- Script present in generated manifest but absent from existing file.
- Script present in existing file but absent from generated manifest.
- JSON parse failure on any existing manifest file.

Exit behavior: `run_deploy` calls `anyhow::bail!` with a count of divergences when any diagnostics are found, producing a non-zero exit code. Suitable for CI enforcement.

---

## Example: `auth-function-mesh`

Source layout:

```
auth-function-mesh/
├── __init__.py
├── auth.py          # calls metacall_load_from_file('node', ['auth/auth.js'])
└── auth/
    └── auth.js
```

`auth.py` excerpt:
```python
from metacall import metacall_load_from_file, metacall

metacall_load_from_file('node', ['auth/auth.js'])

def encrypt(text):
    return metacall('sign', text)
```

Generated `metacall.json`:
```json
{
  "language_id": "py",
  "path": ".",
  "scripts": [],
  "packages": {
    "py": ["__init__.py", "auth.py"],
    "node": ["auth/auth.js"]
  }
}
```

Generated `metacall.mesh.json` (excerpt):
```json
{
  "version": "1.0",
  "cross_language_edges": [
    { "from_unit": 2, "to_unit": 0, "from_language": "py", "to_language": "node", "confidence": 1.0 }
  ],
  "stats": {
    "total_units": 9,
    "cross_language_units": 0,
    "independent_candidates": 7,
    "languages": ["py", "node"]
  }
}
```

---

## Extending the Scanner

To add a new language's call site detection:

1. Add a `static <LANG>_QUERY: LazyLock<Query>` in `scanner.rs` using the language's tree-sitter grammar.
2. Add the new arm to the `match id` dispatch in `scan_file`.
3. Add the language tag mapping in `tags.rs` (`metacall_tag` and `from_metacall_tag`).
4. Add a fixture test in the `#[cfg(test)]` block in `scanner.rs`.
