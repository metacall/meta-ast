# MetaCall Deploy Manifests

_Research synthesis. 2026-06-24._

## Status

Implemented

---

## 1. What We Know

### 1.1 The Gap MetaCall Has

MetaCall's runtime makes cross-language calls transparent at runtime but invisible at
deployment time. The canonical `metacall.json` manifest is deliberately minimal:

```json
{ "language_id": "py", "path": ".", "scripts": ["__init__.py"] }
```

It declares **what files to load** but not **how they relate to each other**. Call
topology is discovered at runtime by the core loader/inspector via reflection. This
means:

- No pre-deployment validation of cross-language call paths.
- No topology visualization before deploy.
- No SCC detection across language boundaries.
- No static check that referenced scripts actually exist.

**meta-ast already produces everything needed to fill this gap:**
cross-file dependency graphs, SCC analysis, language identification, symbol
extraction with types, and deployability classification. Phase 5 wires this
data into deploy manifest generation.

### 1.2 What the Codebase Already Has

| Component | Status | Location |
|-----------|--------|----------|
| `DeployabilityHint` enum | Built | `src/graph/scc.rs:38-46` |
| `DeployabilityStats` | Built | `src/output/graph.rs:31-36` |
| `SerializedScc` | Built | `src/output/graph.rs:81-90` |
| `GraphAnalysis` (graph + scc) | Built | `src/pipeline.rs:7-11` |
| `GraphOutput::from_graph()` | Built | `src/output/graph.rs:113` |
| `LangId` (8 variants) | Built | `src/language/mod.rs:67-80` |
| `CodeGraph.external_index` | Built | `src/graph/mod.rs:15-22` |
| `ExternalNode.language` | Built | `src/graph/node.rs:85-90` |
| Cross-language confidence (0.6) | Built | `src/graph/resolver.rs:67` |

### 1.3 The Reference Fixture

`assets/examples/auth-function-mesh/` demonstrates the target pattern:

- `auth.py` (Python) calls `metacall_load_from_file('node', ['auth/auth.js'])` to
  load Node.js functions, then calls `metacall('sign', text)` and
  `metacall('verify', token)` to invoke them.
- `auth/auth.js` (Node.js) exports `sign` and `verify` using `jsonwebtoken`.

The deploy tool must detect this `metacall_load_from_file('node', [...])` call
and generate:

1. A Python-side manifest listing the scripts.
2. A Node.js-side manifest listing `auth/auth.js`.
3. A root manifest composing both.
4. A mesh annotation showing two cross-language deployment units.

### 1.4 MetaCall Language Tags vs meta-ast LangId

MetaCall uses short runtime tags. meta-ast uses descriptive `LangId` variants.
A mapping is required:

| meta-ast `LangId` | MetaCall `language_id` |
|-------------------|----------------------|
| `Python`          | `"py"`               |
| `JavaScript`      | `"node"`             |
| `TypeScript`      | `"ts"`               |
| `Tsx`             | `"ts"`               |
| `C`               | `"c"`                |
| `Cpp`             | `"cpp"`              |
| `Rust`            | `"rs"`               |
| `Go`              | `"go"`               |

_Note: Ruby (`rb`) and Java (`java`) are MetaCall-supported but not currently in meta-ast's 8-language extraction engine. Deployment manifests for these will be generated if they are targets of `metacall_load_from_*` calls, but meta-ast will not extract symbols from their source files._

---

## 2. Architecture

### 2.1 Data Flow

```
Input Discovery
     |
     v
Parallel Parse + Extract (rayon)
     |
     v
Graph Assembly (GraphBuilder::from_extractions)
     |                                     \
     v                                      \
SCC Analysis (Tarjan)                        v
     |                               [NEW] Preserve Vec<FileExtraction>
     v                                      |
Mesh Annotation Emitter                      v
from SCC Deployment Units         [NEW] Call Site Scanner
     |                             (per-file metacall_load_from_* detection)
     v                                      |
Deploy Manifest Generator <-----------------+
     |
     v
Root Manifest Assembler
     |
     v
[optional] --check validation against existing metacall.json
```

### 2.2 Module Layout

```
src/deploy/
  mod.rs           DeployConfig, DeployError, public API
  tags.rs          LangId -> MetaCall tag mapping, tag validation
  scanner.rs       Cross-Language Call Site detection (tree-sitter queries)
  manifest.rs      DeployManifest, RootManifest types + generation
  mesh.rs          MeshAnnotation type + SCC -> deployment unit converter
  check.rs         --check validation mode (diff against existing manifests)
```

### 2.3 Feature Gate

```toml
# Cargo.toml
[features]
default = []
embed-cytoscape = ["dep:handlebars"]
metacall-deploy = []
```

All `src/deploy/` code is behind `#[cfg(feature = "metacall-deploy")]`.
The CLI `Deploy` variant is gated. Zero compile-time cost when disabled.

---

## 3. Type System Design

### 3.1 Tag Mapping (`src/deploy/tags.rs`)

Maps meta-ast's `LangId` to MetaCall runtime tags (`py`, `node`, `ts`, `c`, `cpp`, `rs`, `go`).

### 3.2 Cross-Language Call Site (`src/deploy/scanner.rs`)

The scanner uses tree-sitter queries per language pack to find calls to
`metacall_load_from_file`, `metacall_load_from_memory`, etc. It extracts:

- The first argument (language tag string literal or enum) -> `target_lang`.
- The second argument (array of script paths) -> `scripts`.

Literals receive 1.0 confidence. Computed arguments (identifiers, calls, etc.)
receive 0.4 confidence and are captured as their raw text.

#### 3.2.1 Language Specific Patterns

- **Python**: `metacall_load_from_file(tag, scripts)`
- **JS/TS/Tsx**: `metacall_load_from_file(tag, scripts)`
- **C/C++**: `metacall_load_from_file(tag, paths, size)` or `metacall_load_from_file_ex(...)`
- **Rust**: `metacall::load::from_files(Tag::NodeJS, ...)`
- **Go**: `metacall.LoadFromFile("node", ...)`

### 3.3 Deploy Manifest (`src/deploy/manifest.rs`)

Defines `DeployManifest` and `RootManifest` structs compatible with MetaCall's
format, plus an extension for multi-language projects.

### 3.4 Mesh Annotation (`src/deploy/mesh.rs`)

Mesh annotation emitted as `metacall.mesh.json`. Maps SCC-derived deployment
units to a mesh topology with cross-language boundaries and independence
classification.

---

## 4. Scanner Implementation Strategy

### 4.1 Tree-Sitter Queries

Implemented queries for all 8 supported languages using an "anchor and capture
arguments node" strategy to ensure robust extraction of language tags and
script path arrays.

### 4.2 Data Preservation

The current pipeline discards `Vec<FileExtraction>` after graph building.
The implementation uses a **dedicated lightweight scanner** pass that re-parses
files only when the `deploy` command is used, avoiding memory overhead on the
primary analysis path.

---

## 5. Output Formats

### 5.1 Per-Language Manifest (`metacall.{tag}.json`)

Groups discovered project files and detected call sites by target language.

### 5.2 Root Manifest (`metacall.json`)

Identifies the primary entry point language and composes all per-language
packages.

### 5.3 Mesh Annotation (`metacall.mesh.json`)

Emits deployment unit topology derived from SCC analysis, identifying cyclic
clusters and independent candidates across language boundaries.

---

## 6. Check Mode (`--check`)

Diffs generated manifests against any existing `metacall.json` in the project
tree and reports divergences (missing/extra scripts, language mismatches)
as structured diagnostics.

---

## 7. Testing Strategy

### 7.1 Example Validation Matrix

Verified against all fixtures in `assets/examples/`:

| Example | Status | Target Manifests | Mesh Expectation |
|---------|--------|------------------|------------------|
| `auth-function-mesh` | PASS | `metacall.py.json`, `metacall.node.json`, `metacall.json` | 2 units, 1 edge |
| `auth-middleware` | PASS | `metacall.node.json`, `metacall.json` | 1 unit |
| `string-manipulation` | PASS | `metacall.json` | 1 unit (external) |
| `time-app-web` | PASS | `metacall.py.json`, `metacall.json` | 1 unit |

### 7.2 Integration Tests

Implemented in `tests/integration/deploy_test.rs`, verifying manifest
correctness, root composition, and `--check` failure detection.

---

## 8. Risks and Tradeoffs

### 8.1 No Upstream MetaCall Extension

The enriched manifest format (with `packages` field and mesh annotation) is a
meta-ast extension, not an upstream MetaCall feature. MetaCall core will ignore
unknown fields. We target the **meshfunction** project's deployment needs rather
than the core upstream loader.

### 8.2 Extraction Data Loss

Re-scanning files for call sites adds slight latency '1 ms' but preserves a lean
memory model for the primary `GraphAnalysis` pipeline.

### 8.3 Unsupported Languages

Symbol extraction is limited to the 8 languages in `meta-ast`'s core engine.
Other languages (Ruby, Java) are handled as external nodes.

### 8.4 Dynamic Call Sites

Unresolvable arguments are assigned 0.4 confidence and annotated, aligning with
NFR-2 (resilience).
