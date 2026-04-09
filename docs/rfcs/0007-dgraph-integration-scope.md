# ADR 0007: Dgraph Integration Scope

## Status

Accepted

## Context

The proposal includes Dgraph sink capability, but standalone analyzer operation is mandatory.

## Decision

Treat Dgraph integration as optional/export-layer capability. Core analysis must not depend on external graph database availability.

## Alternatives considered

1. Dgraph as required runtime dependency.
2. No graph sink pathway.

## Rationale

Optional integration preserves standalone usability and reduces operational burden in MVP.

## Consequences

- Export contracts must remain sink-agnostic.
- Dgraph adapter can evolve independently behind feature boundaries.
