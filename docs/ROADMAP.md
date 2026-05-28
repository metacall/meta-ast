# Roadmap

## Phase 1 - Core & MVP symbols [COMPLETE]

Goals:

- Parser lifecycle implementation for all 8 languages: Python, JavaScript, TypeScript, TSX, C, C++, Rust, Go.
- Symbol extraction and normalized IR.
- Inspect-compatible JSON output (`funcs`, `classes`, `objects`).

Exit gates:

1. All target languages parse on fixtures.
2. Stable JSON output for representative projects.
3. Contract tests for required keys pass.

## Phase 2 - Dependency graph & SCC [COMPLETE]

Goals:

- Build directed dependency/reference graph.
- Compute SCCs and annotate deployability hints.

Exit gates:

1. SCC results match fixture expectations.
2. Cross-file dependency mapping validated on mixed-language samples.
3. ReferenceEdges appear in graph output with confidence scores in cross-file resolution tests.

## Phase 3 - Datagraph & optional sink [NOT STARTED]

Goals:

- Extend model with optional data/flow nodes.
- Provide portable graph export contract.

Exit gates:

1. Export format validated.
2. Snapshot/version semantics documented and tested.

## Phase 4 - CLI polish, output formats, visualization [NOT STARTED]

Goals:

- Structured output (JSON + YAML) with `--format` flag.
- Interactive HTML dashboard with Cytoscape.js via `--html` flag.
- `--self-contained` embeds Cytoscape.js offline (requires `embed-cytoscape` feature).
- Watch mode and incremental-update strategy.
- C ABI scaffolding and header generation.

Exit gates:

1. `--format json|yaml` works for both inspect and graph subcommands.
2. `--html` generates a self-contained dashboard with SCC coloring, auto-opens in browser.
3. `--self-contained` embeds Cytoscape.js offline (requires `embed-cytoscape` feature).
4. Watch-mode stability tests pass.
5. C ABI smoke tests pass.
6. Incremental performance target evidence captured.

## Phase 5 - Validation and delivery [NOT STARTED]

Goals:

- CI/CD hardening.
- Documentation completion.
- Benchmark and portability evidence.

Exit gates:

1. Green CI matrix on Linux/macOS/Windows.
2. Benchmarks and docs published.
3. Candidate demo narrative aligns with delivered artifacts.
4. Release artifacts (binaries, crates) published and verified.
5. Release announcement drafted and scheduled.

## Scope boundaries

- MVP priority: correctness, parity output, graph/SCC, portability.
- Stretch priority: cross-Language Support,more depth,deep dataflow, live sink integration, advanced resolution heuristics.
