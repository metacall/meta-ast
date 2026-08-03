# Crate Decisions

## 1. Decision principles

- Correctness and stability over novelty.
- Keep runtime dependencies minimal for CLI/library users.
- Use ecosystem-standard crates with strong maintenance signals.

## 2. Selected crates by concern

### Parsing

- `tree-sitter` + language crates (`c`, `cpp`, `python`, `javascript`, `typescript`, `rust`, `go`)
- Rationale: robust incremental parsing and grammar-level extraction.
- Language crates provide battle-tested queries and node definitions.
- `python`, `javascript`, `typescript` as a start in every iteration.

### Graph and SCC

- `petgraph`
- Rationale: mature directed graph algorithms and built-in Tarjan SCC.

### Serialization

- `serde`, `serde_json`, `yaml_serde`
- Rationale: stable, standard JSON and YAML contract tooling.

### CLI and watch

- `clap`, `notify`, `notify-debouncer-mini`, `blake3`
- Rationale: battle-tested CLI ergonomics with color, derive, and env support. `notify` and `notify-debouncer-mini` provide OS file events for watch mode. `blake3` provides fast, deterministic cryptographic content hashing for file change fingerprinting.

### Parallelism

- `rayon`
- Rationale: data-parallel file processing with work-stealing. A thread-local pool of `Parser` instances (`thread_local!`) enables safe parallel parse + extract per-file without non-`Sync` parser contention.

### Enum utilities

- `strum`
- Rationale: derive macros for Display, EnumIter, EnumString on enums.

### Filesystem

- `ignore`, `dunce`
- Rationale: gitignore-aware file walking (respecting .gitignore and .ignore files) and cross-platform path canonicalization.

### Browser

- `webbrowser`
- Rationale: auto-open HTML dashboard in the user's default browser.

### Error handling

- `thiserror` (library errors), `anyhow` (application boundary)
- Rationale: explicit typed errors + practical context propagation.

## 3. Development dependencies

- `insta` - snapshot testing for JSON output contracts.
- `criterion` - benchmark gating (`pipeline`, `graph`, and `incremental`).
- `tempfile` - isolated filesystem fixtures for integration testing.

## 4. Feature flags

- `watch` - debounced file-system watch mode with incremental re-analysis and BLAKE3 fingerprinting.
- `dataflow` - data/flow node tracking and def-use graph extraction.
- `metacall-deploy` - includes deploy scanner/manifest/mesh generators for MetaCall deployment manifest generation.

## 5. Recommended additions

- `tracing`, `tracing-subscriber` - structured observability.
- `cbindgen` - C ABI header generation when ABI phase begins.

## 6. Alternatives and trade-offs

- Graph: custom adjacency maps can be faster but increase maintenance cost.
- CLI: smaller parsers reduce binary size but lose feature depth.
- JSON: high-performance serializers are unnecessary before proven bottleneck.
- Parallelism: crossbeam scopes are an alternative but rayon's work-stealing is better suited for file-level data parallelism.
- Language dispatch: trait objects allow runtime plugins but lose compile-time completeness checking; enum dispatch chosen (see `STRUCTURE.md`).

## 7. Risk register

- Grammar drift risk (low): mitigate with fixtures + snapshots.
- Watch-mode debounce edge cases (low): mitigate with integration tests.
- Over-scoping optional sinks (medium): keep feature-gated.

## 8. Policy

Crate upgrades that affect behavior must include:

1. CI pass on all platforms.
2. Snapshot/fixture update.
3. Documentation update in this file and `specs/symbol-extraction.md`.
