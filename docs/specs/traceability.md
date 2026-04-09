# Traceability Matrix

## Deliverables to docs mapping

| Proposal deliverable | Documentation source | Validation target |
|---|---|---|
| Rust lib + CLI | `ARCHITECTURE.md`, `ROADMAP.md` | build/test/CLI fixture checks |
| Language packs | `specs/symbol-extraction.md` | language fixture + snapshot tests |
| Dataflow (stretch) | `specs/graph-model.md`, ADR-0007 | optional feature tests |
| Stable C ABI | ADR-0005 + `ROADMAP.md` | C ABI smoke tests |
| Dgraph sink | ADR-0007 + `specs/graph-model.md` | export contract validation |
| Inspect parity | `specs/requirements.md`, ADR-0005 | JSON schema/snapshot tests |

## Acceptance criteria mapping

| Acceptance criterion | Spec source | Verification plan |
|---|---|---|
| Parse all MVP languages | `specs/requirements.md` FR-1 | per-language fixture parsing tests |
| Correct SCC identification | `specs/graph-model.md` | graph fixture SCC assertions |
| Keys parity (`funcs`,`classes`,`objects`) | `specs/requirements.md` FR-3 | output contract tests |
| Incremental update target | `ARCHITECTURE.md` + ADR-0003 | benchmark harness and thresholds |
| Linux/macOS/Windows portability | `CI_CD.md` | CI matrix status |

## Decision traceability

| Decision area | ADR |
|---|---|
| Language loading model | `rfcs/0001-language-loading-model.md` |
| Error semantics | `rfcs/0002-error-semantics-and-recovery.md` |
| Incremental strategy | `rfcs/0003-incremental-parsing-strategy.md` |
| Graph representation | `rfcs/0004-graph-representation-and-scc.md` |
| Output contract policy | `rfcs/0005-output-contract-policy.md` |
| Type inference scope | `rfcs/0006-type-inference-scope.md` |
| Dgraph scope | `rfcs/0007-dgraph-integration-scope.md` |
