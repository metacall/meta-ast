# ADR 0001: Language Loading Model

## Status

Accepted

## Context

The analyzer must support multiple Tree-sitter grammars with predictable behavior and low operational complexity.

## Decision

Use compile-time language crate integration with explicit language dispatch.

## Alternatives considered

1. Runtime-loaded grammars via dynamic libraries.
2. Hybrid runtime plugin registry.

## Rationale

Compile-time dispatch maximizes type safety and build determinism for MVP scope.

## Consequences

- New language support requires source-level addition and release.
- Lower runtime complexity and fewer deployment surprises.
