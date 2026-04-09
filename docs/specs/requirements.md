# Requirements Specification

## 1. Purpose

Define normative requirements for `meta-ast`, a standalone Rust static analyzer for polyglot projects with `metacall_inspect`-compatible output.

## 2. Functional requirements

### FR-1: Parsing and language support

The analyzer shall parse source files for:

- Python
- JavaScript
- TypeScript (including TSX)
- C
- C++
- Rust
- Go

### FR-2: Symbol extraction

The analyzer shall extract top-level symbols and language-appropriate nested symbols into a normalized intermediate representation:

- Functions
- Classes / structs / interfaces / traits
- Objects (constants, globals, module-level bindings)

### FR-3: Inspect parity output

The primary JSON output contract shall include keys:

- `funcs`
- `classes`
- `objects`

These keys must remain stable and compatible with MetaCall inspect-style consumers.

### FR-4: Dependency graph

The analyzer shall construct a directed graph of symbol-level dependencies:

- File import/usage edges
- Intra-project symbol references
- Cross-language reference candidates

### FR-5: SCC and deployability insight

The analyzer shall compute strongly connected components (Tarjan) and provide cluster-level deployability insight.

### FR-6: Incremental updates

The analyzer shall support update workflows from file changes with a target incremental response of under 100ms for files below 5k LOC.

### FR-7: CLI and library modes

The project shall expose:

- Rust library interface
- CLI entrypoint for project analysis and JSON emission

### FR-8: C ABI (planned later phase)

The project shall provide a stable C ABI header (`mc_ast.h`) for embedding scenarios.

### FR-9: datagraph export

The project may export a datagraph model suitable for external graph sinks (including Dgraph).

## 3. Non-functional requirements

### NFR-1: Correctness

Incorrect symbol labeling is a high-severity defect. Correctness takes priority over throughput.

### NFR-2: Resilience

Malformed source files shall not crash analysis. Partial extraction shall be allowed when parser recovery is possible.

### NFR-3: Portability

Build and test shall pass on Linux, macOS, and Windows.

### NFR-4: Determinism

Given identical input set and tool version, emitted output shall be deterministic.

### NFR-5: Observability

The implementation shall provide structured diagnostics suitable for CI and local debugging.

## 4. Acceptance criteria

1. Parse all MVP languages and emit valid symbol JSON.
2. Correct SCC identification for multi-language project fixtures.
3. Output parity for keys `funcs`, `classes`, `objects`.
4. Incremental-update target under 100ms for <5k LOC files.
5. CI pipeline green on Linux/macOS/Windows.

## 5. Out-of-scope for MVP

- Full inter-procedural global dataflow with alias analysis.
- Sound cross-language type inference.
- Mandatory online graph database dependency.

## 6. Versioning policy

Breaking changes to output schema or graph semantics require:

1. ADR update.
2. Traceability matrix update.
3. Migration note in roadmap/changelog.
