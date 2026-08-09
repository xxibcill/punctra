# Changelog

All notable changes to Punctra are documented here. The project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- Started the v0.3 Real Sources implementation with canonical Source and Point
  contracts, runtime-neutral bounded execution, a verified Source interface,
  and an in-memory conformance adapter.
- Promoted Point Identity to the Source-aware `(Source Identity, ordinal)`
  contract used by renderer picking and highlighting.

## 0.2.0 - 2026-08-09

- Added renderer-neutral adaptive View planning with frustum culling and
  screen-space-error LOD selection.
- Added point, byte, and batch-aware request planning with progressive parent
  Coverage and LOD hysteresis.
- Added deterministic, generation-safe retention and retirement decisions.
- Upgraded the synthetic demo to represent more than 10 million logical Points
  under fixed renderer residency limits.
- Added local CPU planner benchmarks and planner-to-renderer GPU acceptance
  coverage.

## 0.1.0 - 2026-08-09

- Added the renderer-neutral, generation-safe streaming protocol.
- Added hard point-buffer byte, point-count, and batch-count limits.
- Added the embeddable wgpu point renderer with circular splats and depth.
- Added 64-bit world origins with camera-relative 32-bit GPU positions.
- Added identity-preserving highlights and asynchronous point picking.
- Added a progressive million-point interactive demo and GPU acceptance tests.
