# Requirements Specification

## 1. Purpose

Define normative requirements for `meta-ast`, a standalone Rust static analyzer for
polyglot source trees. The tool extracts symbol surfaces, builds cross-file dependency
graphs, and computes SCCs. MetaCall FaaS deployment manifest generation is an
optional capability behind the `metacall-deploy` feature flag.

## 2. Functional requirements

### FR-1: Parsing and language support

The analyzer shall parse source files for a growing set of languages, starting with:

- Python
- JavaScript
- TypeScript (including TSX)
- C
- C++
- Rust
- Go

Additional languages (C#, Java, and others) are planned in later phases.

### FR-2: Symbol extraction

The analyzer shall extract top-level symbols and language-appropriate nested symbols
into a normalized intermediate representation:

- Functions
- Classes / structs / interfaces / traits
- Objects (constants, globals, module-level bindings)

### FR-3: Dependency graph

The analyzer shall construct a directed graph of symbol-level dependencies:

- File import/usage edges
- Intra-project symbol references
- Cross-language reference candidates

### FR-4: SCC and Deployment Unit identification

The analyzer shall compute Strongly Connected Components (Tarjan) and annotate each
SCC as a Deployment Unit, classifying it as:

- Independent (acyclic, Function Mesh separation candidate)
- Co-deployment required (cyclic, must remain grouped)

### FR-5: Incremental updates (Planned)

The analyzer is designed to support update workflows from file changes with a target incremental response of under 100ms for files below 5k LOC. Note: This is planned for Phase 4 of the roadmap and is not yet implemented.

### FR-6: CLI and library modes

The project shall expose:

- Rust library interface
- CLI entrypoint for project analysis and output emission

### FR-7: C ABI (Planned)

The project will provide a stable C ABI header (`mc_ast.h`) for embedding scenarios in a later phase.

### FR-8: Datagraph export (Planned)

The project will support exporting a datagraph model suitable for external graph sinks (including Dgraph) in a later phase.

### FR-9: Deploy Manifest generation (feature-gated: `metacall-deploy`, Planned)

When built with `--features metacall-deploy`, the `deploy` subcommand will support generating manifests:

- Scan source files for Cross-Language Call Sites
  (`metacall_load_from_file`, `metacall_load_from_memory`,
  `metacall_load_from_package`, `metacall_load_from_configuration`)
- Extract `(language_tag, script_paths[])` pairs from call site arguments
- Generate one Deploy Manifest (`metacall.{tag}.json`) per detected language group
- Generate a Root Manifest (`metacall.json`) referencing all per-language manifests
- Inline referenced `metacall_load_from_configuration` targets when present in tree;
  emit a low-confidence annotation when the target is absent
- Validate an existing `metacall.json` against static analysis when `--check` is passed

*Note: This feature is planned for Phase 5 of the roadmap and is not yet implemented.*

### FR-10: Mesh Annotation (feature-gated: `metacall-deploy`, Planned)

When built with `--features metacall-deploy`, the `deploy` subcommand will also emit
`metacall.mesh.json` containing:

- SCC-derived Deployment Units with constituent symbol lists
- Cross-language boundary flags per unit
- Independent mesh candidate classification per unit

*Note: This feature is planned for Phase 5 of the roadmap and is not yet implemented.*

## 3. Non-functional requirements

### NFR-1: Correctness

Incorrect symbol labeling is a high-severity defect. Correctness takes priority over
throughput.

### NFR-2: Resilience

Malformed source files shall not crash analysis. Partial extraction shall be allowed
when parser recovery is possible. Unresolvable Cross-Language Call Site arguments
(dynamic values) shall be annotated, not silently discarded.

### NFR-3: Portability

Build and test shall pass on Linux, macOS, and Windows.

### NFR-4: Determinism

Given identical input set and tool version, emitted output shall be deterministic.

### NFR-5: Observability

The implementation shall provide structured diagnostics suitable for CI and local
debugging.

## 4. Acceptance criteria

1. Parse all supported languages and emit valid symbol JSON.
2. Correct SCC identification for multi-language project fixtures.
3. CI pipeline green on Linux/macOS/Windows.
4. *(Planned)* Incremental-update target under 100ms for files below 5k LOC.
5. *(Planned with `metacall-deploy`)* Deploy Manifests generated match expected fixtures for all example projects in `assets/examples/`.
6. *(Planned with `metacall-deploy`)* Mesh Annotation correctly classifies Deployment Units for the `auth-function-mesh` fixture.

## 5. Out-of-scope for MVP

- Incremental watch-mode and C ABI embedding (post-MVP).
- `metacall-deploy` manifest/mesh generation and `--check` mode (post-MVP).
- Dgraph sink/export adapter (post-MVP).
- Full inter-procedural global dataflow with alias analysis.
- Sound cross-language type inference.
- Mandatory online graph database dependency.
- Dynamic Cross-Language Call Site resolution (runtime tag/path values).

## 6. Versioning policy

Breaking changes to output schema or graph semantics require:

1. ADR update.
2. Traceability matrix update.
3. Migration note in roadmap/changelog.
