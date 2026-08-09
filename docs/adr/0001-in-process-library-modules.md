# ADR-0001: Independent modules are in-process libraries

## Status

Proposed

## Context

The foundation must let each module work individually. That phrase could lead to separate executables communicating over network or process seams. Point-cloud Queries and Views move high-volume data, and a learning-oriented foundation benefits from simple debugging and deterministic tests.

## Decision

Implement independently usable modules as Rust library crates in one Cargo workspace. Modules communicate through owned canonical values and bounded streams. No network process is required for local operation.

A future distributed adapter may be added only when a real remote implementation and a local implementation prove the seam.

## Consequences

- Each module can be built, tested, benchmarked, and reused directly.
- Callers avoid serialization and deployment overhead on hot data paths.
- The dependency graph, not process isolation, enforces direction.
- A crash can affect the host process, so input validation, panic containment at foreign interfaces, and crash-safe persistence remain essential.
- Remote scale-out is deferred rather than prohibited.
