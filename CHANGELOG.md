# Changelog

All notable changes to Punctra are documented here. The project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.9.0 - 2026-08-13

- Completed the narrow Trust and v1 Candidate repository design from the exact
  `9a8363a0d807990209f8252d93229c7f9464c923` v0.8 base. The completed v0.8
  repository interoperability-qualification slice remains distinct from its
  still-outstanding external product-MVP gates.
- Froze owner-local version-1 Source Record, Spatial Index, Workspace, Workflow
  Run, report, LandXML, and Round-Trip Evidence fixtures with exact manifests,
  reopen/reconciliation coverage, and future-version, truncation, checksum,
  lineage, binding, and semantic mutation cases appropriate to each artifact
  class.
- Hardened index, Workspace, journal, LandXML, report, and evidence persistence
  around descriptor-bound no-replace publication, open target witnesses,
  destination and parent-directory durability, late binding checks, retained
  private stages, and conservative post-publication certainty without deleting
  replaceable pathnames.
- Preserved filesystem and publication failures through the private Workflow
  taxonomy with bounded diagnostics and one safe recovery action. Recoverable
  I/O remains distinct from invalid input, conflict, semantic failure, and
  indeterminate publication.
- Qualified the existing Complete-Run LandXML verifier with bounded streaming,
  canonical pass/fail evidence, strict read-only Run/report/input binding, a
  frozen generated corpus, exact-existing reconciliation, and adversarial
  parser/resource/publication coverage.
- Froze the version-1 support matrix and public-interface review, documented
  upgrade/rebuild/recovery policy, and recorded the complete local formatting,
  lint, test, rustdoc, fuzz, example, Criterion, and forced-GPU acceptance
  sequence. No new format, Edit, terrain, UI, networking, transformation, or
  product feature family was added.

## 0.8.0-alpha.1 - 2026-08-13

- Implemented the narrow v0.8 repository interoperability-qualification
  design. The private `terrain-demo` path compares a caller-returned semantic
  LandXML 1.2 TIN with one Complete v0.7 Run and emits a separate bounded
  canonical Round-Trip Evidence record.
- Fixed the caller declaration, bounded fail-closed parsing, explicit metric-
  metre unit checks, unique tolerance mapping, ambiguity rejection, exact face-
  topology comparison, no-overwrite evidence publication, and external-
  evidence boundaries.
- Implemented the file-comparison delivery slices: bounded regular-file and
  DOM-backed LandXML subset parsing, unique tolerance matching, normalized TIN
  topology comparison, focused portability and semantic regressions, and an
  explicitly non-Run-bound `compare-landxml` CLI/process path. Its output
  states that canonical evidence was not published and external application
  execution was not verified.
- Added strict read-only Complete-Run binding, streaming LandXML coverage for
  the full v0.7 export ceiling, stable semantic reason codes, canonical pass and
  fail evidence, exact-existing reconciliation, no-replace publication, and
  publication-fault/process regressions through `verify-round-trip`.
- Bumped workspace version metadata to `0.8.0-alpha.1`. This repository slice
  does not alter the v0.7 eight-frame journal or `audit.json`, complete the
  product MVP, or claim actual Civil 3D, Bentley, partner, paid-pilot,
  conversion, or labor-savings evidence.

## 0.7.0-alpha.1 - 2026-08-11

- Implemented the v0.7 technical partner-alpha readiness design as a repository
  technical-readiness slice. `foundation-runtime` Jobs can now wait with direct
  parent-linked cancellation, without a polling thread or async runtime.
- Added bounded exact `Workspace::revision_audit` reconstruction from immutable
  Revision rows and exact Source positions. Audits expose Revision facts,
  sorted classification transitions, changed Point membership and content
  hashes, Edit Footprint, and resource accounting without changing Workspace
  disk schema.
- Added recovery-oriented `TerrainSurface::ensure_landxml`. It creates a missing
  supported LandXML target or reconciles an exact regular file, fails closed on
  conflicts and non-regular targets, and never overwrites caller data. Canonical
  Workflow evidence records stable `ensured_exact` semantics rather than the
  attempt-dependent create/reconcile disposition.
- Replaced the one-shot `terrain-demo` path with a bounded durable Workflow
  facade and thin `start`, `resume`, and journal-only `inspect` CLI. The fixed
  Run root contains `run.pwf`, `run.lock`, `terrain.xml`, and `audit.json`; the
  journal has exactly eight monotonic checksummed frames from `Intent` through
  `Complete`. Inspect can repair only a provably torn final suffix and
  revalidates Run-root identity before reporting status.
- Added canonical report encoding with exact identities, request and semantic-
  result hashes, Revision Audit/Edit Footprint, baseline/changed Terrain facts,
  conservative Surface Change Envelope, detached QA, stable LandXML facts, 115
  semantic limit facts, and explicit external-evidence nonclaims.
- Added bounded structured Workflow failures with stable code, stage,
  publication certainty, known Run/Source/Workspace/Operation/Revision
  identities, and exactly one safe recovery action.
- Added 35 `terrain-demo` package tests—18 unit/private, 14 public workflow-
  facade, and three process tests—covering every eight-frame resume prefix,
  single-Revision reconciliation, exact report conflict handling, 12 public
  limit families,
  LAS/LAZ semantic projection with honest identity differences, Source
  immutability, stale/mismatched state, Retryable intent, cancellation,
  identity-bearing Run-root validation, dropped-Workflow recovery, and CLI
  diagnostics. Private tests exhaust the application-defined journal Intent
  publication, `Complete` append-before-write/before-sync/after-sync lost-
  acknowledgement, and report post-link boundary sets. Report pre-link
  cancellation/failure, `AlreadyExists` races, post-link replacement, target
  kind, staging/working limits, and stage/parent identity cases are
  representative; every possible OS fault is not claimed.
- Added the public `Workspace::schema` accessor used to enforce Source
  classification Attribute 6 before Run or Workspace mutation, an empty
  baseline-to-Revert Surface Change Envelope regression, and post-link LandXML
  cancellation certainty coverage.
- Added a five-mode generated 10,000-Point Workflow benchmark. Local intervals
  (lower/estimate/upper) were 153.38/157.84/161.25 ms cold,
  113.23/114.88/117.08 ms after a committed Edit,
  123.76/126.67/129.66 ms from a Retryable intent,
  96.871/97.629/98.365 ms for XML/report reconciliation, and
  87.233/88.181/89.112 ms for Complete revalidation. The completed journal was
  2,804 bytes and the canonical report was 11,490 bytes across eight frames.
  Worker peak heap was not measured.
- Deferred durable Breaklines because they require both a Workspace persisted-
  schema evolution and a new constrained-triangulation kernel. External
  partner, licensed-data, downstream, paid-use, and human-time evidence remains
  outstanding and is not a v0.7 repository acceptance claim.

## 0.6.0 - 2026-08-10

- Added the exact classification-aware `Snapshot::point_rows` pull stream with
  exact ticks, effective classification, stable Point Identity, deterministic
  ordering and hashes, cumulative hard limits, fused cancellation, and a
  complete-only terminal summary. The `point-workspace` package now has 72
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
  `point-terrain` has 46 package tests—17 unit/private and 29 integration—plus
  one documentation test;
  `terrain-demo` has two process tests covering generated LAS/LAZ and failed
  changed-Surface Derivation recovery.
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
- Added one validated renderer-neutral viewport contract shared by planning
  and rendering.
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
