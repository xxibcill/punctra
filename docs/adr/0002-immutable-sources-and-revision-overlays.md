# ADR-0002: Source bytes are immutable

## Status

Proposed

## Context

In-place changes to multi-gigabyte LAS, LAZ, or COPC files make undo, stable Point Identity, crash recovery, concurrent reads, and reproducible Derivations difficult. Rewriting a compressed Source for every classification Edit is also too expensive.

## Decision

Treat every Source as immutable and tie Point Identity to its Source Identity and logical ordinal. Store logical changes as sparse Edit records in an append-only Revision journal. Queries combine Source values with the overlays from one pinned Snapshot.

Writing modified LAS or LAZ, when added after v0.1, will be an explicit Export that produces a new Source Identity; it will never be an implicit Workspace mutation. That exporter is outside the initial module set.

## Consequences

- Revisions, undo-like workflows, recovery, and concurrent Snapshot reads become tractable.
- Sparse commits scale with changed Points rather than Source size.
- Rebuilding an index cannot change Point Identity.
- Query execution must merge overlays efficiently.
- The journal requires compaction and migration over time.
- Users who need rewritten point files pay the cost once through explicit Export.
