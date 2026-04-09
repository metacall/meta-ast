# ADR 0004: Graph Representation and SCC

## Status

Accepted

## Context

The project requires dependency analysis and SCC computation across polyglot symbol sets.

## Decision

Use a directed graph representation with explicit node/edge kinds and Tarjan SCC as the canonical cycle analysis algorithm.

## Alternatives considered

1. Custom adjacency structures.
2. Relational-first representation.

## Rationale

A typed directed graph model matches analysis semantics and keeps SCC computation straightforward and testable.

## Consequences

- Graph model contracts must stay stable (`specs/graph-model.md`).
- SCC behavior is deterministic and auditable.
