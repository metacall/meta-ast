# meta-ast Documentation

This directory contains the implementation-facing technical documentation for
`meta-ast`.

The structure is intentionally traceable: specs -> architecture -> structure ->
ADRs -> roadmap -> validation artifacts, so design decisions are easy to trace
back to requirements and forward to tests.

## Document map

- `ARCHITECTURE.md` - system architecture, component boundaries, runtime flow,
  output formats, and the `metacall-deploy` feature layer.
- `STRUCTURE.md` - code structure, data structures, design patterns, module layout,
  and implementation order.
- `DEV_CRATE_DECISIONS.md` - crate selection rationale and trade-offs.
- `CI_CD.md` - CI/CD architecture and quality gates.
- `ROADMAP.md` - phase-aligned implementation milestones and measurable exit gates.

## Specs

- `specs/requirements.md` - normative requirements and acceptance criteria.
- `specs/graph-model.md` - symbol graph and datagraph contracts, including
  `language_id`, project-root-relative `path`, `snapshot_id`, `file_id`,
  `visibility`, and `DataNode` semantics.
- `specs/symbol-extraction.md` - language-pack extraction contracts.
- `specs/traceability.md` - mapping from deliverables to implementation/docs/tests.

## Architecture Decision Records (ADRs)

- `adr/0001-stateful-import-resolver.md` - Stateful import resolver trait seam
- `adr/0002-scope-resolution-heuristics.md` - Scope resolution heuristics
- `adr/0003-unresolved-import-policy.md` - Unresolved import handling policy
- `adr/0004-global-scope-synthetic-symbols.md` - Global scope synthetic symbol generation

## Requests for Comments (RFCs)

- `rfcs/0001-language-loading-model.md` - Language loading model
- `rfcs/0002-error-semantics-and-recovery.md` - Error semantics and recovery
- `rfcs/0003-incremental-parsing-strategy.md` - Incremental parsing strategy
- `rfcs/0004-graph-representation-and-scc.md` - Graph representation and SCC
- `rfcs/0005-output-contract-policy.md` - Output contract policy
- `rfcs/0006-type-inference-scope.md` - Type inference scope
- `rfcs/0007-dgraph-integration-scope.md` - Dgraph integration scope
- `rfcs/0008-graph-module.md` - Graph module design
- `rfcs/0009-cross-file-dependency-mapping.md` - Cross-file dependency mapping

## Scope policy

- **MVP (must ship):** symbol extraction, dependency graph + SCC/Deployment Unit
  analysis, cross-platform CI.
- **`metacall-deploy` feature:** Cross-Language Call Site detection, Deploy Manifest
  generation, Root Manifest assembly, Mesh Annotation from SCC.
- **Stretch:** intra-procedural dataflow beyond simple def-use, live Dgraph sink,
  advanced cross-language type matching, expanded language support.

## Update policy

When implementation changes any public contract (schema, CLI behavior, graph
semantics, language support), update the corresponding file in this directory in
the same pull request.
