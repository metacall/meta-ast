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
5. Dependency graph construction.
6. SCC analysis (Tarjan) and deployability annotation.
7. Output emission (JSON; optional sink/export adapters).

## 3. Component boundaries

- **Input layer:** path discovery, filtering, language routing.
- **Parser layer:** Tree-sitter parser lifecycle and tree ownership.
- **Extractor layer:** language-specific query packs and capture mapping.
- **Model layer:** normalized symbol/domain structs.
- **Graph layer:** directed graph assembly + SCC algorithms.
- **Output layer:** inspect-compatible serialization and optional adapters.
- **Interface layer:** CLI + library API (future: C ABI).

## 4. Data contracts (summary)

Primary inspect-compatible entity groups:

- `funcs`
- `classes`
- `objects`

Static extensions:

- `source_range`
- `docstring` (where available)
- `complexity_score` (defined policy-driven; may be nullable initially)

Detailed graph contract is defined in `specs/graph-model.md`, including `language_id`, project-root-relative `path`, `snapshot_id`, `file_id`, `visibility`, and `DataNode` semantics.

## 5. Error handling model

- Parse errors are recoverable when Tree-sitter yields partial trees.
- Extraction errors are scoped to file/language unit where possible.
- Fatal process-level errors are reserved for invalid configuration or unrecoverable IO/system failures.

## 6. Incremental analysis model

- Baseline mode: re-parse changed file and recompute impacted graph region.
- Optimized mode: apply `InputEdit` + changed range reduction.
- Optimization is benchmark-triggered and must not compromise correctness.

## 7. Compatibility and integration

- Output contract remains stable for MetaCall inspect-style consumers.
- Optional integration layers (C ABI, Dgraph) are feature-scoped and do not block standalone operation.
