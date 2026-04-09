# ADR 0002: Error Semantics and Recovery

## Status

Accepted

## Context

Static analysis should remain useful even on partially invalid source code.

## Decision

Treat parse errors as recoverable when Tree-sitter yields partial trees; treat unrecoverable extraction/config errors as scoped failures with diagnostics.

## Alternatives considered

1. Fail-fast on first parse anomaly.
2. Silent permissive mode with minimal diagnostics.

## Rationale

Recoverable parsing preserves developer feedback loops while maintaining explicit failure signaling.

## Consequences

- Partial results are possible and expected.
- Diagnostics become part of core product quality.
