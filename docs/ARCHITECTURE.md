# Architecture

## 1. Design goals

- Standalone-first static analysis (no runtime execution).
- Inspect-compatible contract.
- Deterministic and resilient extraction under partial syntax errors.
- Incremental-by-design workflow for watch/update scenarios.

## 2. High-level pipeline
 
1. Source discovery and language detection.
2. Tree-sitter parse per file.
3. Query-based symbol extraction per language pack.
4. Intermediate symbol model normalization.
5. Dependency graph construction (initial node + file edges).
6. Import path resolution via per-language resolvers (mapping import strings to file IDs).
7. Cross-file reference resolution via `FlattenedScopeCache` (DFS the import graph once per file, then O(1) scope lookups).
8. SCC analysis (Tarjan) and deployability annotation.
9. Output emission (JSON, YAML, or interactive HTML dashboard).

## 3. Component boundaries

- **Input layer:** path discovery, filtering, language routing.
- **Parser layer:** Tree-sitter parser lifecycle and tree ownership.
- **Extractor layer:** language-specific query packs and capture mapping.
- **Model layer:** normalized symbol/domain structs.
- **Graph layer:** directed graph assembly + SCC algorithms. External dependencies (stdlib, third-party packages) that are referenced but not part of the project are represented as `ExternalNode` entries (`graph/node.rs:83`), carrying the raw import path and language. They appear in the graph but have no file-backed symbol data.
- **Pipeline layer:** full graph analysis orchestration (`pipeline.rs`).
- **Resolver layer:** cross-file reference resolution via `FlattenedScopeCache` (`graph/resolver.rs`).
- **Output layer:** inspect-compatible serialization and optional adapters.
- **Interface layer:** CLI + library API (future: C ABI).

Detailed module layout, data structures, and dependency direction are defined in `STRUCTURE.md`.

## 4. Data contracts (summary)

Primary inspect-compatible entity groups:

- `funcs`
- `classes`
- `objects`

Static extensions:

- `source_range`
- `docstring` (where available)

Detailed graph contract is defined in `specs/graph-model.md`, including `language_id`, project-root-relative `path`, `snapshot_id`, `file_id`, `visibility`, and `DataNode` semantics.

## 5. Error handling model

- Parse errors are recoverable when Tree-sitter yields partial trees.
- Extraction errors are scoped to file/language unit where possible.
- Fatal process-level errors are reserved for invalid configuration or unrecoverable IO/system failures.

## 6. Incremental analysis model

- Baseline mode: re-parse changed file and recompute impacted graph region.
- Optimized mode: apply `InputEdit` + changed range reduction.
- Optimization is benchmark-triggered and must not compromise correctness.

Parallel parse + extract uses rayon per-file; graph assembly is sequential. See `STRUCTURE.md` section 5 for pipeline phase details.

## 7. Output formats

The CLI supports JSON and YAML for programmatic consumption, plus an interactive HTML dashboard for visual analysis.

- **JSON / YAML:** Controlled by the `--format` flag. JSON is the default. YAML requires no extra setup - just pass `--format yaml`.
- **HTML dashboard:** Separate concern, activated with `--html`. Generates a single `.html` file with an embedded Cytoscape.js graph. The browser auto-opens unless you redirect. Use `--self-contained` (requires `embed-cytoscape` feature) to bundle the JS library directly for offline use.

The dashboard turns SCC analysis into something you can actually see. Nodes in cyclic clusters are colored red. Independent units are green. Instead of reading JSON blobs, you click around, expand symbols, and see exactly where the cycles are. This is the difference between "your code has cycles" and "here, this is the knot you need to untangle."

## 8. Compatibility and integration

- Output contract remains stable for MetaCall inspect-style consumers.
- Optional integration layers (C ABI, Dgraph) are feature-scoped and do not block standalone operation.
- Discussion and contributions: [Discord](https://discord.gg/VvSZRsBK)
