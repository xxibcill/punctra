# Changelog

All notable changes to Punctra are documented here. The project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

## 0.4.0 - 2026-08-10

- Added `point-index` with one deterministic 65,536-Point fixed-block BVH,
  conservative inclusive-box candidate plans, stable root-first node identities,
  bounded exact-position display samples, and Source-backed complete leaves.
- Added append-only checksummed work frames, valid-prefix recovery, deterministic
  resume, complete artifact validation, and durable no-replace publication by
  hard-linking a synced temporary artifact before removing disposable sidecars.
- Added separate hard limits for Source reads, adapter and builder memory,
  incomplete and complete files, resident hierarchy metadata, candidate plans,
  and node materialization.
- Added validated direct seeks across fixed-size LAZ chunk boundaries. Point-wise
  and variable-chunk LAZ retain bounded cancellable sequential replay.
- Added `ViewPlan::demanded_nodes()` so hosts can retain still-demanded Requested
  work and cancel camera-stale queued work without inferring demand from an
  absent request delta.
- Added the private real-cloud `renderer-demo` bridge and CLI. Supported LAS/LAZ
  files are Full-verified, built/resumed/opened through `point-index`, and
  materialized into exact identity-preserving atomic renderer Upserts under
  explicit staging, planning, hierarchy, and renderer budgets.
- Added direct-use, oracle, persistence, interruption, corruption, cancellation,
  process-level LAS/LAZ smoke, one-million-Point benchmark, measured heap, and
  local GPU acceptance evidence. External licensed production-data and partner
  validation remain outstanding.

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
