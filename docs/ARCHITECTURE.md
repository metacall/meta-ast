# Architecture

## 1. Design goals

- Standalone-first static analysis (no runtime execution).
- Deterministic and resilient extraction under partial syntax errors.
- Incremental-by-design workflow for watch/update scenarios.
- Language-agnostic core; MetaCall deployment support is an opt-in feature.

## 2. High-level pipeline

1. Source discovery and language detection.
2. Tree-sitter parse per file.
3. Query-based symbol extraction per language pack.
4. Intermediate symbol model normalization.
5. Dependency graph construction (initial node + file edges).
6. Import path resolution via stateful resolvers implementing the `ImportResolver` trait (mapping import strings to file paths/IDs, supporting stateful configs like `tsconfig.json` and disk caches).
7. Cross-file reference resolution via `FlattenedScopeCache` (DFS the import graph once per file, then O(1) scope lookups).
8. SCC analysis (Tarjan) and Deployment Unit annotation.
9. Output emission (JSON, YAML, or interactive HTML dashboard).
10. _(Roadmap Phase 5, requires `metacall-deploy` feature)_ Cross-Language Call Site detection, Deploy Manifest generation, and Mesh Annotation emission.

## 3. Component boundaries

- **Input layer:** path discovery, filtering, language routing.
- **Parser layer:** Tree-sitter parser lifecycle and tree ownership.
- **Extractor layer:** language-specific query packs and capture mapping.
- **Model layer:** normalized symbol/domain structs.
- **Graph layer:** directed graph assembly + SCC algorithms. External dependencies (stdlib, third-party packages) that are referenced but not part of the project are represented as `ExternalNode` entries (`graph/node.rs:85`), carrying the raw import path and language. They appear in the graph but have no file-backed symbol data.
- **Pipeline layer:** full graph analysis orchestration (`pipeline.rs`).
- **Resolver layer:** cross-file reference resolution via `FlattenedScopeCache` (`graph/resolver.rs`).
- **Output layer:** serialization and optional adapters.
- **Interface layer:** CLI + library API (future: C ABI).
- **Deploy layer** _(PLANNED - Roadmap Phase 5, feature-gated: `metacall-deploy`)_: Cross-Language Call Site scanner, Deploy Manifest writer, Root Manifest assembler, Mesh Annotation emitter. **Not yet implemented.**

Detailed module layout, data structures, and dependency direction are defined in `STRUCTURE.md`.

## 4. Data contracts (summary)

Primary symbol extraction output:

- `funcs`
- `classes`
- `objects`

Static extensions:

- `source_range`
- `docstring` (where available)

Deploy output _(PLANNED - Roadmap Phase 5, feature-gated: `metacall-deploy`)_:

- `metacall.{tag}.json` per detected language group (Deploy Manifest)
- `metacall.json` root manifest (Root Manifest)
- `metacall.mesh.json` (Mesh Annotation - SCC-derived Deployment Unit advisory)

**Note:** The `metacall-deploy` feature is not yet implemented. No code exists behind this feature gate.

Detailed graph contract is defined in `specs/graph-model.md`.

## 5. Error handling model

- Parse errors are recoverable when Tree-sitter yields partial trees.
- Extraction errors are scoped to file/language unit where possible.
- Unresolvable Cross-Language Call Sites (dynamic tag/path arguments) are annotated as low-confidence entries in the Mesh Annotation, not silently dropped.
- Fatal process-level errors are reserved for invalid configuration or unrecoverable IO/system failures.

## 6. Incremental analysis model _(PLANNED)_

- Baseline mode: re-parse changed file and recompute impacted graph region.
- Optimized mode: apply `InputEdit` + changed range reduction.
- Optimization is benchmark-triggered and must not compromise correctness.

**Current status:** Incremental parsing is not yet implemented. The pipeline currently runs full re-analysis on every invocation. This section describes the planned architecture for watch/update scenarios (Roadmap Phase 4 pending items).

Parallel parse + extract uses rayon per-file; graph assembly is sequential. See `STRUCTURE.md` section 5 for pipeline phase details.

## 7. Output formats

The CLI supports JSON and YAML for programmatic consumption, plus an interactive HTML dashboard for visual analysis.

- **JSON / YAML:** Controlled by the `--format` flag. JSON is the default. YAML requires no extra setup - just pass `--format yaml`.
- **HTML dashboard:** Separate concern, activated with `--html`. Generates a single `.html` file with an embedded Cytoscape.js graph. The browser auto-opens unless you redirect. Use `--self-contained` (requires `embed-cytoscape` feature) to bundle the JS library directly for offline use.

The dashboard turns SCC analysis into something you can actually see. Nodes in cyclic clusters (co-deployment required) are colored red. Independent Deployment Units are green. This is the difference between "your code has cycles" and "here is the exact knot you need to untangle before you can split this into independent mesh units."

## 8. Compatibility and integration

- Optional integration layers (C ABI, `metacall-deploy`, Dgraph) are feature-scoped and do not block standalone operation. **Note:** C ABI and `metacall-deploy` are planned but not yet implemented.
- Discussion and contributions: [Discord](https://discord.gg/VvSZRsBK)
