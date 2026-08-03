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

## Phase 3 - Datagraph & optional sink [COMPLETE]

Goals:

- Extend model with optional data/flow nodes (DataNode, FlowEdge, DataScope, FlowKind).
- Implement intra-procedural def-use extraction for Rust (let bindings, parameters).
- Provide portable graph export contract with schema versioning (v1).
- Pluggable sink adapters (GraphSink trait + JsonSink).
- CLI integration: `--datagraph` flag on graph subcommand.
- Unified GraphOutput serialization (replaces separate datagraph module).

Exit gates:

1. Export format validated via integration tests (JSON roundtrip, field checks).
2. Snapshot/version semantics documented and tested (SCHEMA_VERSION = 1).
3. End-to-end pipeline extracts data nodes from real Rust fixtures.
4. Flow edges created for def-use chains (param→usage, let→let shadowing).

## Phase 4 - CLI polish, output formats, visualization [COMPLETE]

Goals:

- Structured output (JSON + YAML) with `--format` flag.
- Interactive HTML dashboard with Cytoscape.js via `--html` flag.
- Watch mode and incremental-update strategy.
- C ABI scaffolding and header generation (scoped out, see below).

Exit gates:

1. ~~`--format json|yaml` works for analysis output.~~ DONE
2. ~~`--html` generates a dashboard with SCC/Deployment Unit coloring,
   auto-opens in browser.~~ DONE
3. ~~Watch-mode stability tests pass.~~ DONE
4. ~~Incremental performance target evidence captured.~~ DONE
5. C ABI smoke tests. DROPPED - issue #21 closed NOT_PLANNED; the C ABI
   interface was proposed in RFC 0011 but not implemented. The exit gate is
   removed from scope and tracked as post-GSoC future work.

## Phase 5 - MetaCall Deploy Manifests [COMPLETE]

_Requires `--features metacall-deploy`. Full documentation in [DEPLOY.md](DEPLOY.md)._

Goals:

- Implement cross-language call-site detection across all 9 supported language ports
  (`metacall_load_from_file`, `metacall_load_from_memory`, `metacall_load_from_package`,
  `metacall_load_from_configuration`), including CommonJS `require()` for JS/TS and
  bare-name call detection for Rust after `use` import.
- Partition files into same-language pods via Union-Find over dependency edges.
- Resolve external dependencies per-language from lockfiles (preferred for exact
  version pinning) and package manifests (fallback).
- Generate pod manifest (`metacall.pods.json`) with per-pod deployments, inter-pod
  edges with fused confidence scores, and scoped dependency lists.
- Emit mesh annotation (`metacall.mesh.json`) from SCC deployment unit analysis,
  classifying independent Function Mesh candidates vs. co-deployment-required groups
  with cross-language call-site attribution.
- Implement `--check` validation mode: fairness check ensuring every cut edge has a
  corresponding RPC stub entry in the manifest (bijection check, ADR 0003 pattern).

Exit gates:

1. Pod manifests generated match expected fixtures for all projects in
   `tests/fixtures/mixed/`. DONE
2. Mesh annotation correctly classifies deployment units for `auth-function-mesh`
   fixture with call-site attribution. DONE
3. `--check` detects missing RPC stubs for cut edges and reports structured diagnostics. DONE
4. Dynamic call-site arguments emit low-confidence annotation rather than hard failure. DONE
5. External dependency resolution identifies `jsonwebtoken` from `package.json`/lockfile
   in the `auth-function-mesh` fixture with exact version pinning. DONE

## Phase 6 - Language expansion [COMPLETE]

Goals:

- Extend language support beyond the initial 8, prioritizing C# and Java.
- Each new language requires: grammar crate, query pack (symbols + imports +
  references), import resolver, visibility rules, and fixture tests.
- Cross-language Call Site detection extended to new language ports as they ship.

Outcome:

- Ruby shipped end to end: grammar, query pack, resolver, visibility rules,
  fixtures, snapshots, and `metacall-deploy` call-site and lockfile coverage.
- C# (issue #23) and Java (issue #24) were evaluated and closed NOT_PLANNED.
  Ruby was the third language added, bringing the catalog to nine.

Exit gates:

1. New language parses on fixtures. DONE (Ruby)
2. New language pack passes extraction and cross-file dependency tests. DONE
3. `metacall-deploy` feature detects call sites in the new port bindings. DONE

## Phase 7 - Validation and delivery [COMPLETE]

Goals:

- CI/CD hardening.
- Documentation completion.
- Benchmark and portability evidence.

Exit gates:

1. Green CI matrix on Linux/macOS/Windows. DONE
2. Benchmarks and docs published. DONE - see [BENCHMARKS.md](BENCHMARKS.md) and
   the mdbook site (GitHub Pages).
3. Candidate demo narrative aligns with delivered artifacts. DONE - see
   [DEMO.md](DEMO.md).
4. Release artifacts (binaries, crates) published and verified. DONE - v0.5.0
   on GitHub Releases (7 targets x core + deploy binaries) and crates.io.
5. Release announcement drafted and scheduled. DONE - v0.5.0 release notes and
   the [Final Report](FINAL_REPORT.md).

## Scope boundaries

- MVP priority: correctness, symbol extraction, graph/SCC, portability.
- `metacall-deploy` priority: Cross-Language Call Site detection, Deploy Manifest
  generation, Mesh Annotation from SCC analysis.
- Stretch priority: more languages, deeper dataflow, live sink integration, advanced
  resolution heuristics.
