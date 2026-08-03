# RFC 0006: Type Inference Scope

## Status

Accepted

## Context

Cross-language full type inference is high complexity and not required for MVP parity.

## Decision

Limit MVP to extraction of declared/type-hint metadata and best-effort symbol linking. Defer sound cross-language inference to future milestones.

## Alternatives considered

1. Full type system in MVP.
2. No type metadata at all.

## Rationale

Keeps scope feasible while preserving useful metadata for analysis.

## Consequences

- Some cross-language links remain heuristic.
- Future advanced inference can be introduced without breaking core contracts.
