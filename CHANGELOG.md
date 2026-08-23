# Changelog

All notable changes to Punctra are documented here. The project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased - 0.16.0-alpha.1

- Completed and locally repository-verified the accepted v0.16 slice at
  implementation commit `68020dc80e1a0ca95f6746df04862b3f3013ca13`. The
  [v0.16 verification record](docs/releases/v0.16.0.md) pins the exact native
  and browser environments, complete local command matrix, bounded browser
  observations, generated benchmark intervals, and unsupported external exits.
- Added the bounded private [HTTP Range streaming
  slice](docs/design/http-range-streaming-v0.16.md): one versioned deployment
  manifest binds an immutable remote LAS representation, strong validators,
  exact byte lengths, Source identity, and a compatible disk-v2 Punctra index.
  Bare LAS/LAZ URLs are rejected instead of triggering a full download or scan.
- Added strict worker-owned Range response validation, bounded retry and
  cancellation, Source-change detection, per-range SHA-256 integrity, disk-v2
  header/root validation, attributed-sample decoding, transferable 1,024-Point
  batches, and deterministic failures with one safe recovery action.
- Added host-selected none, memory, and persistent Cache API policies. Cache
  namespaces and entries include the deployment schema, Source identity,
  strong validator, index digest, resource, and exact range; explicit
  invalidation is scoped to that namespace, and quota/API failures never
  silently change policy.
- Added a deterministic 70,000-Point LAS 1.2 fixture, its fully verified Source
  record, compatible 172,808-byte disk-v2 index, deployment manifest, fixture
  regeneration verifier, strict local Range/CORS server, and Node module tests
  for transport, cache, retry, cancellation, corruption, and recovery rules.
- Extended the private WebAssembly browser host with an identity-bound sampled
  View generation and four independently bounded renderer batches. The local
  acceptance page now proves the inherited v0.15 lifecycle first, then cold
  streaming, explicit viewer/worker recreation, and a zero-binary-request warm
  cache path while reporting network, cache, queue, staging, transfer, main-
  thread, canvas, and renderer facts separately.
- Kept the boundary private: v0.16 does not add arbitrary URL loading, general
  browser LAS/LAZ decoding, exact browser Queries, Source rewriting, a public
  viewer/networking API, SDK/framework packages, or broad browser/support
  qualification.

## Unreleased - 0.15.0-alpha.1

- Completed and locally repository-verified the private `browser-demo`
  WebAssembly/WebGPU acceptance host at implementation commit
  `6fd3906d5386029f95c4e273cc9f6333c653854c`. The [v0.15 verification
  record](docs/releases/v0.15.0.md) pins the exact native/browser environments,
  commands, generated observations, benchmark comparisons, and nonclaims. The
  host remains under the bounded [v0.15 browser-foundation
  design](docs/design/browser-foundation-v0.15.md). It builds through
  `wasm32-unknown-unknown` and pinned `wasm-bindgen` ES modules, runs without a
  framework or bundler, and renders one deterministic 1,089-Point large-world
  scene in a browser canvas.
- Exercised the existing `point-view`, `render-protocol`, and `render-wgpu`
  contracts in the browser with exact request/retention, View-generation,
  batch-version, logical-residency, provisional-pick, visibility, resize/DPR,
  shutdown, and explicit-recreation checks. `render-wgpu` now selects
  `web_time::Instant` on bare WebAssembly so frame timing no longer traps at
  runtime; native timing remains `std::time::Instant`.
- Added explicit capability and safe-action failures plus separate logical
  vertex, host surface, and renderer transient-texture accounting. The checked-
  in static harness publishes an inspectable pass/unsupported/fail state and a
  restrained engineering-console view documented in the [local browser
  guide](docs/guides/browser-foundation.md).
- Kept the evidence boundary explicit: generated local browser success is not
  HTTP Range delivery, browser LAS/LAZ decoding, a supported SDK, broad browser
  qualification, observed process/GPU memory, independent adoption, support
  qualification, or release-candidate status.

## Unreleased - 0.14.0-alpha.1

- Completed and locally repository-verified the bounded [Exact Terrain QA and
  Correction Loop design](docs/design/exact-terrain-qa-correction-v0.14.md) at
  implementation commit `b8b767df87241eaab4dded82720e38ba8d410670`.
  Exact environment, command, example, benchmark, and nonclaim facts are in the
  [v0.14 verification record](docs/releases/v0.14.0.md). `point-terrain`
  now evaluates one exact Snapshot/Surface pair with a combined Source-Point
  Query, detached Check Points, and evenly stationed profile under one explicit
  asymmetric vertical tolerance. Reports retain authoritative ticks,
  classifications, faces, elevations, signed residuals, gaps, units, spatial
  reference, Snapshot/Surface provenance, input/result hashes, resource facts,
  and explicit freshness state.
- Added exact semantic Surface comparison by authoritative face Point
  identities, including deterministic added/removed counts and hashes plus a
  conservative incident-vertex changed-region envelope. The existing
  `point-workspace` commit, Operation reconciliation, and immediate-head Revert
  contracts remain the only correction model.
- Added bounded exact QA over checksummed file-backed disk-v1 Surfaces. Prepared
  vertices and faces materialize only under explicit Surface-read,
  materialization, result, face-test, and combined working-byte ceilings; the
  in-memory and prepared paths produce identical semantic evidence.
- Added the public `exact_terrain_qa` example and [traceability
  guide](docs/guides/exact-terrain-qa.md). The example seeds a generated
  defect, inspects it, corrects classification, proves old evidence stale,
  re-derives, compares, rechecks, emits JSON/SVG evidence, Reverts, and proves
  baseline topology restoration.
- Kept the evidence boundary explicit: field activation, observed professional
  workflow timing, independent publication/adoption, partner validation, and
  support qualification remain outstanding. Generated fixtures do not satisfy
  those exits.

## Unreleased - 0.13.0-alpha.1

- Completed and locally repository-verified the bounded [Persistent Bounded-AOI
  Terrain design](docs/design/persistent-production-scale-terrain-v0.13.md) at
  implementation commit `d99ed34324e8938fd0211344fbf65d539bb37178`; the
  exact environment, command outcomes, fixtures, and generated observations
  are in the [v0.13 verification record](docs/releases/v0.13.0.md). The scope
  preserves the exact single-worker full-AOI triangulator and legacy in-memory
  `derive` path while adding checksummed Ground-Input/final checkpoints,
  resume, no-replace disk-v1 Surface publication, warm reopen, stale-binding
  detection, and bounded file-backed vertex/face streams.
- Retained the verified final stage and any Surface work sibling after
  successful publication because no portable unlink can be conditioned on the
  open owned inode. Work is trusted only after verification; a warm open gives
  the complete target precedence, and cleanup remains an explicit owner-
  controlled offline action.
- Kept the evidence boundary explicit: **Complete and repository-verified for
  the bounded persistent-terrain slice; field activation, production-scale
  accuracy, true out-of-core adoption, independent adoption, partner
  validation, and support qualification outstanding.** Persistence is bounded-
  memory rather than true out-of-core triangulation. The two unrelated-firm
  above-500-million-Point projects and production AOI/workstation measurements
  remain outstanding. Version `0.13.0-alpha.1` remains Unreleased; this record
  does not claim a tag, registry publication, or independent package execution.

## Unreleased - 0.12.0-alpha.1

- Completed the repository implementation of the private pre-v0.13 field
  evidence lane. Opt-in manifests require the five-mode/two-projection
  matrix, five projects from three firms, permissions, and declared known-
  feature outcomes; every pose must settle within a bounded ceiling and remain
  quiet for 300 rendered frames. Reports now retain settlement, cumulative lifecycle,
  transient-resource, adaptive-appearance, depth-cue/fallback, and declared
  feature-outcome inputs while explicitly stating that those inputs were not
  verified by viewing operations. Permitted field execution and human
  interpretation evidence remain outstanding.
- Added a bounded on-canvas inspection status panel with package-derived View
  version, display/projection/loading state, truthful display Coverage and
  Point counts, exact-selection and resident-locator state, clear/recovery
  actions, north orientation, target-plane scale and cursor coordinates, and
  mode-specific palette meaning. The detailed engineering transcript is
  printed to standard output while the title stays compact, and the required
  panel fits the minimum window at 200% interface scaling.
- Added a bounded renderer appearance policy: projected-density-aware 1–4
  physical-pixel splats, exact eight-presented-frame parent/child color
  transitions, and optional eye-dome lighting with an explicit unenhanced
  fallback. Presentation weight and depth enhancement do not change Point
  position, depth coverage, or provisional pick identity, and enhanced
  rendering is capped at eight transient texture bytes per physical pixel.
- Activated the bounded pre-v0.13 renderer-quality corrective design. The
  public View planner now reconstructs still-valid refinement history before
  considering new refinements and budgets only still-demanded in-flight work;
  the synchronous private Scene bridge admits one new request per pump. The
  investigated 2560-by-1664 physical-pixel synthetic View now settles within
  the accepted 1,024-frame ceiling and remains unchanged for a 300-frame
  observation window instead of alternating coarse and fine request cuts.
- Added one explicit projected survey-coordinate profile with horizontal and
  vertical EPSG identities, easting/northing/elevation axes, separate linear
  units, provenance, bounded serialization, and canonical hash bytes. Opaque
  WKT and frozen unknown-reference encodings remain compatible and are never
  guessed into the structured profile.
- Added strict fail-closed LAS/LAZ GeoTIFF key decoding for complete direct
  projected-reference facts. Metre, international-foot, and US-survey-foot
  declarations are preserved; duplicate, indirect, missing, malformed,
  user-defined, unsupported, and WKT-conflicting inputs do not publish a
  structured profile.
- Bound structured or exact opaque reference semantics into Workspace reopen
  and Terrain Artifact identity. Detached QA and LandXML accept the new
  metre/metre profile, reject structured non-metre input, and keep the frozen
  unknown-reference boolean readable only by legacy reconciliation. Current
  workflow, QA, and export paths require the supported profile.
- Emitted and compared one strict LandXML 1.2 `CoordinateSystem` declaration,
  with reference drift rejected before numeric coordinate tolerances in both
  DOM and bounded streaming round-trip readers.
- Prepared all twelve public library crates for local crates.io/docs.rs
  validation with complete package metadata, exact versioned path
  dependencies, empty default features, an explicit MSRV/license policy, a
  dependency-role guide, package-content checks, and clean extracted-package
  builds. The demo applications remain private and no registry upload occurs.
- Preserved every frozen persisted-v1 fixture and the legacy unknown-reference
  identity path. This bounded repository release does not claim a permitted
  production corpus, coordinate transformation, downstream acceptance,
  independent adoption, partner validation, or production support.

## Unreleased - 0.11.0-alpha.1

- Completed and locally repository-verified the evidence-honest v0.11 Exact
  Interactive Review and Ground Correction technical design from canonical
  v0.10 commit `30ea9ff`. Repository verification does not satisfy the
  outstanding field gate or claim professional time savings, independent
  adoption, partner acceptance, or production support.
- Added the headless `point-review` composition for exact CPU confirmation of
  provisional display identities and inclusive perspective/orthographic
  screen-through rectangle selection at one pinned Workspace Revision.
  Selection evaluates authoritative Snapshot rows, supports an optional exact
  effective-classification filter, and publishes only a complete spillable
  Point Set under cumulative hard limits.
- Added bounded Point Set entry reads so exact review can expose the effective
  classification captured with each selected identity without consulting
  sampled display Attributes or changing Point Set persistence.
- Bounded complete renderer highlight input independently from resident Point,
  batch, and byte limits. Oversized highlight updates now fail atomically
  before duplicate removal and preserve prior renderer state.
- Added a minimal public-only offscreen `render-wgpu` host example. It owns the
  wgpu instance, device, queue, target, encoder, submission, and polling;
  records and resolves a provisional pick; and documents pinned CPU
  confirmation and resource ownership without importing `renderer-demo`
  private state.
- Connected the professional inspection demo to explicit existing Workspace
  state for exact review, exact Point Set-derived overlays, caller-owned
  classification Operations, Revision Audit/Edit Footprint reporting,
  immediate-head Revert, and same-identity reconciliation. The path neither
  creates a Workspace nor treats a GPU miss or resident LOD sample as exact.
- Added public-interface projection/oracle/cancellation/spill coverage, a
  generated LAS exact-review process plus inherited generated LAS/LAZ
  identity/persistence regressions, stale-state and complete highlight-handoff
  tests, the full commit/Audit/Revert/reopen identity chain, compiled rustdoc,
  resource-fact benchmarks, and required local GPU coverage. No new persisted
  Workspace, Spatial Index, Run, Point Set spill, or LandXML version was
  introduced.
- Prevented the durable terrain-workflow qualification lane from comparing
  against an unnamed historical Criterion baseline. The benchmark still
  measures filesystem synchronization and reports current intervals, while
  cross-Revision attribution now requires a named same-machine A/B/A run with
  a stable base self-check.

## Unreleased - 0.10.0-alpha.1

- Implemented the repository track of the accepted v0.10 Field Qualification
  and Professional Inspection View design from the v0.9 repository candidate.
  The v0.8 and v0.9 repository slices are complete, but repository work does
  not turn generated tests or declared labels into field, partner, downstream,
  or support evidence.
- Reconciled the six post-base v0.8 comparator correctness and regression
  commits before broadening the View: process-level CLI failure coverage,
  shared coordinate-drift facts, named round-trip geometry, same-window input
  capture, captured-content comparison, and bounded complete XML attributes.
- Added strict read-only `terrain-demo verify-round-trip` qualification over an
  exact Complete Run, with shared locking, artifact revalidation, canonical
  no-replace evidence, stable `PRT_*` outcomes, and explicit caller-declaration
  and acceptance nonclaims.
- Replaced the inherited whole-file DOM comparator with an exact-byte bounded
  local XML stream/parser, using borrowed `quick-xml` token views, at the 4-GiB,
  10-million-Point, and 20-million-face export ceilings. Added lexical token and
  namespace-stack guarding, changed/extended
  input detection on every pass, fallible prechecked parser/retained/comparison
  growth, deterministic accounted peaks, inclusive/over-limit coverage, and
  generated pass/topology evidence goldens. Accounted peaks are algorithm
  charges, not allocator metadata/slack, process RSS, or observed heap.
- Made the first Spatial Index work header ownership-safe and retryable: the
  complete synced header is now published from a uniquely owned temporary by a
  no-replace link, while empty, racing, or caller-owned `.work` paths are
  preserved and fail closed instead of being claimed or deleted.
- Retained byte-identical Spatial Index disk v1 for position-only samples and
  added disk v2 for bounded raw `U16` intensity, `U8` classification, and
  optional all-or-none `U16` RGB display samples. Explicit recipes, frozen v1
  and v2 complete/work fixtures, exact 32/42-byte accounting, cold/resumed/warm
  reads, incompatible-target preservation, and observed temporary-disk facts
  cover the new cache contract.
- Added private deterministic neutral, elevation, RGB, intensity, and
  classification mappings to `renderer-demo`. Mode changes preserve Point
  Identity, geometry, and Coverage; attributed modes require the v2 inspection
  recipe and RGB fails explicitly when all channels are unavailable.
- Added perspective and target-plane-scale-preserving orthographic projection,
  middle-drag pan, projection toggle/reset behavior, matching frustum/SSE
  planning, large-world depth/picking coverage, and a planner benchmark case.
- Breaking alpha API migration: `Camera::vertical_field_of_view_radians()` was
  replaced by `Camera::projection()`. Callers now match
  `CameraProjection::Perspective { vertical_field_of_view_radians }` or
  `CameraProjection::Orthographic { vertical_world_height }`; an orthographic
  camera deliberately has no nominal field of view. Exhaustive `CameraError`
  matches must also handle the new `InvalidOrthographicWorldHeight` variant
  (or use a wildcard arm). Spatial Index persistence
  facts are recipe-specific in v0.10, so callers inspect
  `IndexDescriptor::{recipe_version,disk_version}` instead of assuming one
  process-global version pair. `SourceReadSummary::provenance` and
  `IndexReadSummary::provenance` remain ordinary accessors but are no longer
  callable in const evaluation; this lets summaries share detached provenance
  without copying bounded text. Their `source` accessors remain const.
- Split View truth into demanded/candidate/issued/retained/retired work,
  queue/staging/residency facts, and Sampled versus Complete Coverage. Added
  bounded stable `PVIEW_*` diagnostics with owning phases and exactly one safe
  recovery action.
- Added the permission-gated local `renderer-demo corpus` runner, bounded
  manifest/navigation trace, GPU viewing measurements, canonical no-replace
  report, recorded effective limits, and explicit false nonclaims. Private
  paths and opaque project/firm identifiers are not copied into the report.
- Added CPU, process, persistence, planner, and local GPU regression coverage,
  plus a five-minute first-LAS/LAZ guide and a non-sensitive corpus-manifest
  template. No benchmark number, production-corpus completion, observed
  professional preference, approved screenshot, repository topic/homepage
  publication, or partner acceptance is claimed.

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
  evidence boundaries before the pending Run-bound publication slice.
- Implemented the first two private delivery slices: bounded regular-file and
  DOM-backed LandXML subset parsing, unique tolerance matching, normalized TIN
  topology comparison, focused portability and semantic regressions, and an
  explicitly non-Run-bound `compare-landxml` CLI/process path. Its output
  states that canonical evidence was not published and external application
  execution was not verified. This historical v0.8 implementation was
  superseded by the bounded streaming reader in the v0.10 entry above.
- Added strict read-only Complete-Run binding, streaming LandXML coverage for
  the full v0.7 export ceiling, stable semantic reason codes, canonical pass and
  fail evidence, exact-existing reconciliation, no-replace publication, and
  publication-fault/process regressions through `verify-round-trip`.
- This repository slice does not alter the v0.7 eight-frame journal or
  `audit.json`, complete the product MVP, or claim actual Civil 3D, Bentley,
  partner, paid-pilot, conversion, or labor-savings evidence.

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
