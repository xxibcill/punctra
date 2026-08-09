# Changelog

All notable changes to Punctra are documented here. The project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

- Started v0.4 Out-of-core View under an accepted design: one rebuildable
  persistent Spatial Index, conservative Source-span lookup, bounded display
  samples, and a private host-owned real-cloud materialization path.

## 0.3.0 - 2026-08-10

- Added canonical Source and Point contracts, runtime-neutral bounded execution,
  a verified Source interface, and deterministic in-memory conformance adapter.
- Added bounded opening and reads for LAS point-data record formats 0–10 and LAZ
  formats 0–8, preserving exact position ticks, supported Attributes,
  Coordinate Reference WKT, and ordered VLR/EVLR payloads. LAZ formats 9 and 10
  return an explicit unsupported-format error pending exact layered
  WavePacket14 codec support.
- Added versioned Source Records, Full/Fast reopen semantics, stable corruption
  and change errors, fused cancellation, normalized spans, projection, exact
  summaries, and hard point, payload, and decoder-working-memory limits.
- Added shared memory/LAS/LAZ conformance tests, coverage for every supported
  LAS and LAZ point format, a file inspection example, and one-million-Point
  LAS/LAZ benchmarks.
- Promoted Point Identity to the Source-aware `(Source Identity, ordinal)`
  contract used by renderer picking and highlighting.
- Migration: replace `PointId::new(zero_based_ordinal)` with
  `PointId::new(source_id, zero_based_ordinal)` and retain that Source Identity
  anywhere Point IDs are persisted or reconstructed.

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
