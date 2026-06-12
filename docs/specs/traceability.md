# Traceability Matrix

## Deliverables to docs mapping

| Deliverable | Documentation source | Validation target |
|---|---|---|
| Rust lib + CLI | `ARCHITECTURE.md`, `ROADMAP.md` | build/test/CLI fixture checks |
| Language packs | `specs/symbol-extraction.md` | language fixture + snapshot tests |
| Dependency graph + SCC | `specs/graph-model.md`, ADR-0004, ADR-0008, ADR-0009 | graph fixture SCC assertions |
| Deployment Unit annotation | `CONTEXT.md`, `specs/graph-model.md` | SCC classification fixture tests |
| Dataflow (stretch) | `specs/graph-model.md`, ADR-0007 | optional feature tests |
| Stable C ABI | ADR-0005 + `ROADMAP.md` | C ABI smoke tests |
| Dgraph sink | ADR-0007 + `specs/graph-model.md` | export contract validation |
| Deploy Manifests (`metacall-deploy`) | `specs/requirements.md` FR-9 | fixture manifest comparison tests |
| Mesh Annotation (`metacall-deploy`) | `specs/requirements.md` FR-10 | SCC unit classification tests |

## Acceptance criteria mapping

| Acceptance criterion | Spec source | Verification plan |
|---|---|---|
| Parse all supported languages | `specs/requirements.md` FR-1 | per-language fixture parsing tests |
| Correct SCC identification | `specs/graph-model.md` | graph fixture SCC assertions |
| Deployment Unit classification | `specs/requirements.md` FR-4 | independent vs. co-deploy fixture assertions |
| Incremental update target | `ARCHITECTURE.md` + ADR-0003 | benchmark harness and thresholds |
| Linux/macOS/Windows portability | `CI_CD.md` | CI matrix status |
| Deploy Manifest fixture match | `specs/requirements.md` FR-9 | `assets/examples/` manifest comparison |
| Mesh Annotation correctness | `specs/requirements.md` FR-10 | `auth-function-mesh` unit classification |

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
| Graph module design | `rfcs/0008-graph-module.md` |
| Cross-file dependency mapping | `rfcs/0009-cross-file-dependency-mapping.md` |
