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
10. _(Requires `metacall-deploy` feature)_ Cross-language call-site scanning, pod partitioning, dependency resolution from lockfiles, pod-and-mesh manifest generation, and CI fairness checking. See [DEPLOY.md](DEPLOY.md).

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
- **Deploy layer** _(feature-gated: `metacall-deploy`)_: Cross-language call-site scanner (`scanner.rs`), pod partitioning via Union-Find over same-language edges (`pod.rs`), cross-language SCC cut detection and oversized-pod rebalancing (`cut.rs`), per-language external dependency resolution from lockfiles and manifests (`dependency.rs`), pod manifest generation (`manifest.rs`), Function Mesh annotation (`mesh.rs`), and CI fairness checking for RPC-converted cut edges (`check.rs`). See [DEPLOY.md](DEPLOY.md).

Detailed module layout, data structures, and dependency direction are defined in `STRUCTURE.md`.

## 4. Data contracts (summary)

Primary symbol extraction output:

- `funcs`
- `classes`
- `objects`

Static extensions:

- `source_range`
- `docstring` (where available)

Deploy output _(feature-gated: `metacall-deploy`)_:

- `metacall.pods.json` - pod manifest with per-pod deployments, inter-pod edges, dependency lists, and AST node metrics
- `metacall.mesh.json` - Function Mesh topology annotation with SCC-derived deployment units and cross-language call-site attribution

See [DEPLOY.md](DEPLOY.md) for schema details and the call site scanner reference.

Detailed graph contract is defined in `specs/graph-model.md`.

## 5. Error handling model

- Parse errors are recoverable when Tree-sitter yields partial trees.
- Extraction errors are scoped to file/language unit where possible.
- Unresolvable Cross-Language Call Sites (dynamic tag/path arguments) are annotated as low-confidence entries in the Mesh Annotation, not silently dropped.
- Fatal process-level errors are reserved for invalid configuration or unrecoverable IO/system failures.

## 6. Incremental analysis model

- Baseline mode: re-parse changed file and recompute the full graph.
- Optimized mode: apply `InputEdit` + changed range reduction (planned, benchmark-triggered).

**Current status:** Baseline incremental analysis is implemented behind the `watch`
feature flag (`--features watch`). The `watch` module (`src/watch/mod.rs`) provides:

- `incremental_reanalyze()` - pure, deterministic, testable re-analysis step.
- `run_watch()` - debounced OS-level watcher loop (via `notify` + `notify-debouncer-mini`).
- BLAKE3 cryptographic content-hash fingerprinting (`Fingerprint([u8; 32])`) for change detection.
- Cached `Arc<FileExtraction>` per file: zero-allocation pointer sharing for unchanged files; graph rebuilt from scratch each tick (graph + SCC rebuild is sub-ms).
- CLI integration via `meta-ast graph <path> --watch [--watch-debounce <ms>]`.

The `InputEdit` / changed-range narrowing optimization is deferred per RFC 0003.

Parallel parse + extract uses rayon per-file; graph assembly is sequential. See `STRUCTURE.md` section 5 for pipeline phase details.

## 7. Output formats

The CLI supports JSON and YAML for programmatic consumption, plus an interactive HTML dashboard for visual analysis and datagraph JSON exports.

- **JSON / YAML:** Controlled by the `-f, --format` flag. JSON is the default. YAML requires no extra setup - just pass `--format yaml`.
- **HTML dashboard:** Separate concern, activated with `--html`. Generates a single `.html` file with an interactive Cytoscape.js graph loaded from a CDN (cached by the browser after first fetch). The browser auto-opens unless you redirect.
- **Datagraph JSON:** Activated with `--datagraph` (requires `--features dataflow`). Exports detailed data/flow node definitions and def-use relations.
- **Language Override:** Force a specific language with `-l, --language <lang>`.

The dashboard turns SCC analysis into something you can actually see. Nodes in cyclic clusters (co-deployment required) are colored red. Independent Deployment Units are green. This is the difference between "your code has cycles" and "here is the exact knot you need to untangle before you can split this into independent mesh units."

## 8. Compatibility and integration

- Optional integration layers (C ABI, `metacall-deploy`, Dgraph) are feature-scoped and do not block standalone operation. The `metacall-deploy` feature is implemented (see [DEPLOY.md](DEPLOY.md)). C ABI is planned but not yet implemented.
- Discussion and contributions: [Discord](https://discord.gg/VvSZRsBK)
