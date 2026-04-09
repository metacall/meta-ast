# Roadmap

## Phase 1 — Core & MVP symbols

Goals:

- Parser lifecycle implementation per language "python,JS/TS".
- Symbol extraction and normalized IR.
- Inspect-compatible JSON output (`funcs`, `classes`, `objects`).

Exit gates:

1. All target languages parse on fixtures.
2. Stable JSON output for representative projects.
3. Contract tests for required keys pass.

## Phase 2 — Dependency graph & SCC

Goals:

- Build directed dependency/reference graph.
- Compute SCCs and annotate deployability hints.

Exit gates:

1. SCC results match fixture expectations.
2. Cross-file dependency mapping validated on mixed-language samples.

## Phase 3 — Datagraph & optional sink

Goals:

- Extend model with optional data/flow nodes.
- Provide portable graph export contract.

Exit gates:

1. Export format validated.
2. Snapshot/version semantics documented and tested.

## Phase 4 — CLI polish, C ABI, incremental optimization

Goals:

- Watch mode and incremental-update strategy.
- C ABI scaffolding and header generation.

Exit gates:

1. Watch-mode stability tests pass.
2. C ABI smoke tests pass.
3. Incremental performance target evidence captured.

## Phase 5 — Validation and delivery

Goals:

- CI/CD hardening.
- Documentation completion.
- Benchmark and portability evidence.

Exit gates:

1. Green CI matrix on Linux/macOS/Windows.
2. Benchmarks and docs published.
3. Candidate demo narrative aligns with delivered artifacts.

## Scope boundaries

- MVP priority: correctness, parity output, graph/SCC, portability.
- Stretch priority: deep dataflow, live sink integration, advanced resolution heuristics.
