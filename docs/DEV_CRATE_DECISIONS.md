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

- `serde`, `serde_json`
- Rationale: stable, standard JSON contract tooling.

### CLI and watch

- `clap`, `notify`
- Rationale: battle-tested CLI ergonomics and cross-platform FS events.

### Parallelism

- `rayon`
- Rationale: data-parallel file processing with work-stealing. Tree-sitter `Parser` and `Tree` are `Send + Sync`, enabling safe parallel parse + extract per-file.

### Error handling

- `thiserror` (library errors), `anyhow` (application boundary) , `dunce` (path normalization)
- Rationale: explicit typed errors + practical context propagation.

### Optional caching

- `moka`
- Rationale: cache strategy candidate for incremental performance tuning.

## 3. Development dependencies

- `insta` - snapshot testing for JSON output contracts.
- `jsonschema` - output schema validation in tests.
- `criterion` - benchmark gating (Phase 4).

## 4. Recommended additions

- `tracing`, `tracing-subscriber` - structured observability.
- `cbindgen` - C ABI header generation when ABI phase begins.


## 5. Alternatives and trade-offs

- Graph: custom adjacency maps can be faster but increase maintenance cost.
- CLI: smaller parsers reduce binary size but lose feature depth.
- JSON: high-performance serializers are unnecessary before proven bottleneck.
- Parallelism: crossbeam scopes are an alternative but rayon's work-stealing is better suited for file-level data parallelism.
- Language dispatch: trait objects allow runtime plugins but lose compile-time completeness checking; enum dispatch chosen (see `structure.md`).

## 6. Risk register

- Grammar drift risk (low): mitigate with fixtures + snapshots.
- Watch-mode debounce edge cases (low): mitigate with integration tests.
- Over-scoping optional sinks (medium): keep feature-gated.

## 7. Policy

Crate upgrades that affect behavior must include:

1. CI pass on all platforms.
2. Snapshot/fixture update.
3. Documentation update in this file and `specs/symbol-extraction.md`.
