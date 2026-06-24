# Roadmap

## Phase 1 - Core & MVP symbols [COMPLETE]

Goals:

- Parser lifecycle implementation for all initial languages: Python, JavaScript,
  TypeScript, TSX, C, C++, Rust, Go.
- Symbol extraction and normalized IR.
- Structured JSON/YAML output (`funcs`, `classes`, `objects`).

Exit gates:

1. All target languages parse on fixtures.
2. Stable JSON output for representative projects.
3. Contract tests for required keys pass.

## Phase 2 - Dependency graph & SCC [COMPLETE]

Goals:

- Build directed dependency/reference graph.
- Compute SCCs and annotate Deployment Units (independent vs. co-deployment required).

Exit gates:

1. SCC results match fixture expectations.
2. Cross-file dependency mapping validated on mixed-language samples.
3. ReferenceEdges appear in graph output with confidence scores in cross-file
   resolution tests.

## Phase 3 - Datagraph & optional sink [NOT STARTED]

Goals:

- Extend model with optional data/flow nodes.
- Provide portable graph export contract.

Exit gates:

1. Export format validated.
2. Snapshot/version semantics documented and tested.

## Phase 4 - CLI polish, output formats, visualization [IN PROGRESS]

Goals:

- Structured output (JSON + YAML) with `--format` flag.
- Interactive HTML dashboard with Cytoscape.js via `--html` flag.
- `--self-contained` embeds Cytoscape.js offline (requires `embed-cytoscape` feature).
- Watch mode and incremental-update strategy.
- C ABI scaffolding and header generation.

Exit gates:

1. ~~`--format json|yaml` works for analysis output.~~ DONE
2. ~~`--html` generates a self-contained dashboard with SCC/Deployment Unit coloring,
   auto-opens in browser.~~ DONE
3. ~~`--self-contained` embeds Cytoscape.js offline (requires `embed-cytoscape` feature).~~ DONE
4. Watch-mode stability tests pass. NOT YET STARTED
5. C ABI smoke tests pass. NOT YET STARTED
6. Incremental performance target evidence captured. NOT YET STARTED

## Phase 5 - MetaCall Deploy Manifests [NOT STARTED]

_Requires `--features metacall-deploy`._

Goals:

- Implement Cross-Language Call Site detection across all supported language ports
  (`metacall_load_from_file`, `metacall_load_from_memory`, `metacall_load_from_package`,
  `metacall_load_from_configuration`).
- Generate per-language Deploy Manifests (`metacall.{tag}.json`) and Root Manifest
  (`metacall.json`).
- Emit Mesh Annotation (`metacall.mesh.json`) from SCC Deployment Unit analysis,
  classifying independent Function Mesh candidates vs. co-deployment-required groups.
- Implement `--check` validation mode: diff generated manifests against any existing
  `metacall.json` in the project tree.
- Inline `metacall_load_from_configuration` targets when present; annotate as
  low-confidence when target is absent from the analyzed tree.

Exit gates:

1. Deploy Manifests generated match expected fixtures for all projects in
   `assets/examples/`.
2. Mesh Annotation correctly classifies Deployment Units for `auth-function-mesh`
   fixture (two independent units: `{encrypt, sign}` and `{decrypt, verify}`).
3. `--check` detects manifest/source divergence and reports it as a structured
   diagnostic.
4. Dynamic call site arguments (non-literal tag or path) emit low-confidence
   annotation rather than a hard failure.

## Phase 6 - Language expansion [NOT STARTED]

Goals:

- Extend language support beyond the initial 8, prioritizing C# and Java.
- Each new language requires: grammar crate, query pack (symbols + imports +
  references), import resolver, visibility rules, and fixture tests.
- Cross-language Call Site detection extended to new language ports as they ship.

Exit gates:

1. C# and Java parse on fixtures.
2. All new language packs pass extraction and cross-file dependency tests.
3. `metacall-deploy` feature detects call sites in new port bindings.

## Phase 7 - Validation and delivery [NOT STARTED]

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

- MVP priority: correctness, symbol extraction, graph/SCC, portability.
- `metacall-deploy` priority: Cross-Language Call Site detection, Deploy Manifest
  generation, Mesh Annotation from SCC analysis.
- Stretch priority: more languages, deeper dataflow, live sink integration, advanced
  resolution heuristics.
