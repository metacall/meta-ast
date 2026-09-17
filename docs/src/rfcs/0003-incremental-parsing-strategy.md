# RFC 0003: Incremental Parsing Strategy

## Status

Accepted

## Context

Proposal target includes incremental responsiveness (<100ms for files under 5k LOC).

## Decision

Adopt a staged approach:

1. Correct baseline implementation first.
2. Incremental optimization with `InputEdit` and changed-range narrowing behind benchmark evidence.

## Alternatives considered

1. Full incremental complexity from day one.
2. Full reparse forever.

## Rationale

Correctness-first minimizes early defect risk; benchmark-driven optimization avoids premature complexity.

## Consequences

- Early versions may reparse more broadly.
- Optimization milestones are explicitly measurable.

## Incremental cache contract

- A file that cannot be read keeps its cached extraction and counts as unchanged.
- A retained file stays in the merged output with the same `Arc` handle.
- The stale sweep never treats an unreadable file as removed.
- A first-tick read failure has no entry to keep and claims no counter.
- Oversized files and language changes re-extract and never duplicate a reused handle.
