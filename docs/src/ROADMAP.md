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
2. Snapshot/version semantics documented and tested (SCHEMA_VERSION = 2).
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
   removed from scope and tracked as post-GSoC future work in issue #63.

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

## Phase 8 - Polyglot LSP Server & Shard Indexing (`metacall/lsp`) [IN PROGRESS]

Goals:

- Implement Phase 0 engine prerequisites: symbol coordinates (`source_range`, `file_path`),
  in-memory buffer extraction seam (`extract_text_with_id_gen`), and modular `.metast` v2
  shard and index persistence (`ShardFile`, `ShardEdge`, `ShardManifestRecord`, `ShardHeader`).
- Implement dynamic cache invalidation across all import resolvers (`clear_cache`).
- Enable downstream `metacall/lsp` development for single-language and polyglot navigation:
  - Phase 8a: Synchronous language server (goto-definition, hover, diagnostics).
  - Phase 8b: Cross-language jump-to-definition and reference resolution over `metacall()` boundaries.
  - Phase 8c: Signature enrichment and cross-language stub generation.

Exit gates:

1. Phase 0 engine seams implemented, tested, and schema version bumped to 2. DONE
2. `.metast` v2 modular shards, headers, and manifest files persist and restore graph topology. DONE
3. Resolver cache invalidation handles dynamic configuration updates. DONE
4. `metacall/lsp` language server crate operational against `meta-ast` core library.

## Phase 9 - Engine Refactoring & Graph Reuse [PLANNED]

Goals:

- Zero-allocation resolver dispatch: replace `Box<dyn ImportResolver>` trait objects with
  an enum dispatch model (`Resolver`) to eliminate heap allocation during pipeline runs (issue #41).
- Language module deduplication: introduce declarative macros (`define_language_pack!`) to
  eliminate repetitive spec and query boilerplate across language packs (issue #39).
- Deploy pipeline modularization: extract `DeployOrchestrator` struct from `run_deploy`
  for single-responsibility and independent step reuse by downstream tools (issue #40).
- Reusable graph visitor interfaces over `CodeGraph` for custom static analysis passes.

Exit gates:

1. Zero heap allocations during per-file import resolution dispatch.
2. Language pack boilerplate reduced across Python, Ruby, C, C++, Rust, Go, JS, TS, and TSX.
3. `DeployOrchestrator` exposes individual pipeline stages (scan, partition, cuts, manifests, mesh).

## Phase 10 - Polyglot Security & Taint Flow Analysis (SAST) [PLANNED]

Goals:

- Deliver cross-language taint-flow analysis across MetaCall FFI boundaries (issue #29,
  `metacall/polyglot-sast`).
- Detect untrusted inputs in one language reaching dangerous execution sinks in another language.
- Classify findings into Common Weakness Enumeration (CWE) categories.
- Output native SARIF (v2.1.0) reports for GitHub/GitLab Security tab integration.
- Integrate with MetaSSR as deployment-blocking middleware and dashboard visualization.

Exit gates:

1. Cross-language taint flow correctly traces from Python/JS inputs into C/Rust sinks.
2. Deterministic rule-based engine emits valid SARIF v2.1.0 reports.
3. MetaSSR deploy middleware blocks deployments with critical security findings.

## Phase 11 - Developer Ecosystem & Community Tooling [IN PROGRESS]

Goals:

- Cross-platform distribution scripts: Unix `scripts/install.sh` (issue #46) and Windows
  `scripts/install.ps1`.
- Property-based testing with `proptest` for Tarjan SCC, cycle detection, and edge normalization
  invariants (issue #48).
- Streamline contributor experience: curated "Good First Issues" with detailed task guides.
- CLI output ergonomics: JSON error reporting and enhanced diagnostic formatting (issue #47).

Exit gates:

1. Verified curl/PowerShell installation scripts published for all release artifacts.
2. `proptest` suites validating graph normalization and SCC determinism.
3. Active contributor onboarding through structured issue templates.

## Phase 12 - Deep Expression AST & Full Syntax Trees [PLANNED]

Goals:

- Extend `meta-ast` beyond coarse symbol-level IR into fine-grained expression syntax trees
  and intra-procedural Control Flow Graphs (CFG).
- Extract statement nodes, binary operations, control flow branches, and expression terms across
  all 9 supported languages.
- Maintain a layered representation:
  - *Layer 1 (Default)*: Fast, lightweight symbol & reference graph.
  - *Layer 2 (Opt-in)*: Full expression-level AST with lexical scopes and operator nodes.
- Generate intra-procedural CFGs for abstract interpretation, dead branch elimination, and
  fine-grained taint propagation.

Exit gates:

1. Full expression AST extractable via `--depth full` or `extract_full_ast`.
2. Control Flow Graph (CFG) generated with branch conditions and join nodes.
3. Zero performance regression on default symbol-only extraction passes.

## Phase 13 - Polyglot Code Transformation & Refactoring Engine [PLANNED]

Goals:

- Evolve `meta-ast` from a read-only static analyzer into a bidirectional polyglot code
  transformation and refactoring engine.
- Implement lossless Concrete Syntax Tree (CST) rewriting, preserving whitespace, formatting,
  and comments.
- Deliver cross-language atomic symbol renaming:
  - Renaming a function or method in C, C++, or Rust automatically rewrites and updates all
    cross-language caller sites in Python, JavaScript, and Ruby.
- Implement automated polyglot code migrations, AST rewrite recipes, and FFI/RPC stub generation
  (`meta-ast refactor`, `meta-ast codegen`).
- Provide programmatic transformation APIs for language migration tools and automated refactorings.

Exit gates:

1. Lossless round-trip source rewriting verified across all 9 languages without formatting loss.
2. Cross-language atomic symbol renaming verified on mixed Python/JS/Rust/C fixture codebases.
3. Automated refactoring CLI (`meta-ast refactor`) and FFI stub generator (`meta-ast codegen`).

## Strategic Architecture Evolution

`meta-ast` follows a phased strategic evolution from lightweight symbol graph to a full polyglot
transformation engine:

1. **Current Foundation (Phases 1-11)**:
   - High-speed, read-only static analysis and symbol-level IR.
   - Cross-language dependency graph, import resolution, and Tarjan SCC cycle detection.
   - Language Server (LSP) seams, shard index persistence (`.metast` v2), and security analysis (SAST).
2. **Deep Syntax Expansion (Phase 12)**:
   - Full expression-level syntax trees and Control Flow Graphs (CFG) layered over the symbol graph.
3. **Bidirectional Transformation (Phase 13)**:
   - Lossless CST source rewriting, cross-language atomic refactoring, and automated code generation.

## Scope boundaries

- Core priority: general-purpose symbol extraction, cross-language dependency graph, cycle detection, shard persistence, zero-cost abstractions.
- Tooling priority: Polyglot LSP server (`metacall/lsp`), IDE integration, general-purpose CI gates.
- Evolution priority: Full expression AST (Phase 12), Polyglot code transformation & refactoring (Phase 13), SAST security analysis (`metacall/polyglot-sast`).
