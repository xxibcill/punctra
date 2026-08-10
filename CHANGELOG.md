# Changelog

All notable changes to Punctra are documented here. The project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.6.0 - 2026-08-10

- Added the exact classification-aware `Snapshot::point_rows` pull stream with
  exact ticks, effective classification, stable Point Identity, deterministic
  ordering and hashes, cumulative hard limits, fused cancellation, and a
  complete-only terminal summary. The `point-workspace` package now has 67
  tests, including six public Point-row integration tests.
- Added one deep `point-terrain` crate. Its single-worker robust triangulation
  derives a deterministic unconstrained in-memory 2.5D `TerrainSurface` with
  canonical `SurfaceVertex` and `SurfaceFace` values, complete descriptor
  provenance and hashes, explicit degeneracy failures, and hard resource and
  cancellation gates.
- Added bounded detached Check Point QA with closed-boundary coverage,
  explicit gaps, observed-Z-minus-surface-Z residuals, stable caller ordering,
  compensated statistics, and complete-only publication.
- Added the private durable create-new metric-metre LandXML 1.2 points/faces
  encoder with explicit date/time, deterministic bytes, no-replace
  publication, conservative post-publication certainty, and independent
  `roxmltree` semantic verification.
- Added a public Source-to-Terrain example, a generated 10k/100k/1M-capable
  terrain benchmark, and the GPU-free `terrain-demo` LAS/LAZ process caller.
  `point-terrain` has 41 package tests—15 unit/private and 26 integration—plus
  one documentation test;
  `terrain-demo` has one generated LAS/LAZ process test.
- Recorded the local 10,000-Point baseline: Derivation 11.983–12.049 ms,
  detached QA 94.907–95.164 us for three Check Points and 19,604 face tests,
  and durable 1,030,118-byte LandXML creation 18.020–18.311 ms. Descriptor
  accounting reported 135,790,592 peak working bytes, 1,034,176 retained bytes,
  and 521,494 topology steps; QA reported 336 peak working bytes. The benchmark
  names `jjaes-MacBook-Pro.local` and separately reports one-shot Derivation/
  QA/LandXML times of 13,371/125/14,656 us. It explicitly reports
  `worker_heap_measurement: null`, so no worker-heap measurement is claimed.
- Terrain persistence, Breaklines, Profiles, classifiers, CRS transformation,
  general LandXML, downstream-application round trips, licensed production or
  above-500-million-Point evidence, partner validation, paid use, and human-
  workflow claims remain outstanding.

## 0.5.0 - 2026-08-10

- Added one deep `point-workspace` crate over a complete `PreparedIndex` and
  its retained verified Source, with exclusive local Workspace locking and
  immutable root and historical Snapshots.
- Added exact All, inclusive world-box, and bounded explicit-Point-ID selection
  with effective-classification equality, conservative index planning, exact
  Source rechecks, and cumulative hard limits.
- Added immutable process-scoped Point Sets with deterministic ordering,
  repeatable bounded Point-ID batches, automatic checksummed spill, corruption
  detection, and final-handle cleanup.
- Added sparse uniform `U8` classification Revisions, no-op rejection,
  immediate-head Revert as a new inverse Revision, and unchanged Source bytes
  and non-classification values.
- Added caller-owned durable Operation Identity, immutable ready/rejection
  records, no-replace Revision publication, retry without a live Point Set,
  and fail-closed committed/rejected/retryable/not-recorded/indeterminate
  reconciliation.
- Added 61 package tests—19 integration tests through the public interface and
  42 unit, fault-injection, and allocation gates—plus generated LAS/LAZ
  end-to-end coverage, a direct classification/Revert example, and a default
  one-million-Point resource and Revision benchmark.
- Recorded one-machine generated-fixture evidence; licensed production-data,
  above-500-million-Point, and design-partner evidence remain outstanding.

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
