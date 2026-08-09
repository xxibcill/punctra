# ADR-0003: CPU values are authoritative geometry

## Status

Proposed

## Context

Survey coordinates can be very large while meaningful differences can be millimeters. Current portable GPU shader types do not provide the same dependable 64-bit geometry path as the CPU, and GPU algorithms may vary across devices and drivers.

## Decision

Preserve quantized Source positions and use deterministic 64-bit CPU values or exact/adaptive predicates for Point selection, Edits, terrain topology, Profiles, and Exports.

View preparation subtracts a 64-bit world origin and emits disposable 32-bit relative positions. GPU results may provide visual output and pick hints but never become persistent or analytical input.

## Consequences

- Persistent and exported results do not depend on GPU vendor or display precision.
- View rendering remains fast and portable through floating origins.
- Exact selection must be confirmed by a CPU Query.
- Terrain work is harder to accelerate and may initially be slower.
- A future GPU acceleration path needs a conformance proof against the CPU reference before it can affect authoritative output.
