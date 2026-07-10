# RFC 0005: Output Contract Policy

## Status

Accepted

## Context

Compatibility with MetaCall-style inspect consumers is a central project requirement.

## Decision

Make inspect-compatible JSON the primary output contract, preserving stable keys:

- `funcs`
- `classes`
- `objects`

## Alternatives considered

1. New schema first, with adapters later.
2. Multiple equal-priority formats.

## Rationale

Contract stability reduces integration risk and aligns with proposal goals.

## Consequences

- Breaking output changes require versioning, migration note, and traceability update.
- Snapshot/schema validation becomes mandatory in CI.
