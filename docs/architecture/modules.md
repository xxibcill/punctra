# Module Catalog

Status: frozen module ownership through the completed v0.9 repository trust and
version-1 compatibility candidate, with the v0.10 professional inspection View
and repository-verified v0.11 exact-review technical slice plus the v0.12
explicit spatial-reference and package-publication repository slice; external
evidence gates and broader terrain/export modules remain outstanding

This is the ownership map for implemented crates. Each crate has one public
job. Several private files may cooperate behind one deep interface; private
file boundaries are not public module promises.

The v0.9 compatibility classification, caller obligations, side effects,
limits, and recovery modes are frozen in the
[v0.9 public interface review](v0.9-interface-review.md).

## Catalog

| Module | Its only job | Canonical input | Canonical output |
|---|---|---|---|
| `point-contracts` | Define and validate lossless Point and spatial provenance values. | None | IDs, Source metadata, Point Batches, bounds, hashes |
| `foundation-runtime` | Standardize runtime-neutral bounded long-operation control. | Work closure or producer | Jobs, streams, progress, cancellation |
| `point-source` | Provide verified bounded canonical reads from one immutable Source. | Adapter capability and read request | Verified `Source`, bounded Point Batches, terminal summary |
| `source-memory` | Supply deterministic in-memory Sources. | Canonical metadata and columns | `point-source` capability |
| `source-las` | Decode supported local LAS/LAZ through the Source contract. | LAS/LAZ path | `point-source` capability |
| `point-index` | Prepare and query one rebuildable persistent spatial index. | Verified Source, target, limits | Complete `PreparedIndex`, candidates, display reads |
| `point-workspace` | Make narrow exact classification selection/history durable and stream exact effective Point rows for one Source. | Complete index, schema, selection/row/commit requests | Workspace, Snapshot, Point rows, Point Set, commit/recovery outcomes |
| `point-review` | Compose one pinned Snapshot with renderer-neutral screen or Point identity input for exact CPU review. | Snapshot, Camera, Viewport, rectangle or PointId, limits | Confirmed Point or exact spillable Point Set plus terminal facts |
| `point-terrain` | Derive and evaluate the narrow Terrain Surface and encode or reconcile its supported deliverable. | Snapshot, Terrain Recipe, detached Check Points | `TerrainSurface`, QA report, LandXML receipt |
| `render-protocol` | Define generation-safe renderer-neutral point display state. | Camera and display values | Validated updates and frame values |
| `point-view` | Plan one frozen View over a host-owned hierarchy without I/O. | Camera, viewport, hierarchy/residency, budget | Demand, requests, retention, retirements |
| `render-wgpu` | Maintain and draw one wgpu representation of render-protocol state. | Render updates, frame, host target | Recorded commands, report, provisional picks |
| `renderer-demo` | Exercise indexed LAS/LAZ View-to-render composition, exact review/correction, and local viewing measurement. | CLI, permitted corpus manifest, generated inputs, or an existing Workspace | Interactive demo, exact review outcome, GPU-free process smoke, or canonical Viewing Report |
| `terrain-demo` | Own one recoverable headless LAS/LAZ-to-terrain Workflow Run and its private post-Run qualification. | Caller-owned paths, identities, baseline, correction/QA intent, limits, and returned LandXML declaration | Eight-frame journal, Revision, Terrain/QA evidence, LandXML/report, and separate Round-Trip Evidence |

`source-copc`, constrained or persisted terrain, general LandXML, general
application UI, bindings, and remote storage are not implemented modules in
v0.12. Display and correction workflow policy remain private to
`renderer-demo`; v0.12 adds no public display-policy or mutation-facade crate.

## 1. point-contracts

**Job:** define and validate lossless Point and spatial provenance values.

It owns stable Source-aware `PointId`, `SourceId`, `AttributeId`, Source schema,
coordinate reference, position transform, integer ticks, world bounds, Point
Batches, Source spans, and content hashes. It does not read files, schedule
work, persist an index or Workspace, plan a View, or allocate GPU resources.

**Independent proof:** value validation, deterministic hashing, serde
round-trips, and contract tests run without I/O or a GPU.

## 2. foundation-runtime

**Job:** standardize runtime-neutral bounded long-operation control.

It owns `Job`, `OperationHandle`, `OperationControl`, progress phases,
cancellation, direct parent-linked child waits, and bounded pull-stream
conventions. It does not own durable
Workspace `OperationId`, domain algorithms, storage, an async runtime, or a
thread pool policy exposed to callers.

**Independent proof:** Jobs can be awaited or blocking-waited, cancellation is
fused, progress is monotonic, panics become bounded runtime errors, and stream
terminal rules are tested with generated producers.

## 3. point-source and adapters

**Job:** provide verified bounded canonical reads from one immutable Source.

`point-source` owns Source capability validation, normalized read semantics,
Source Records, verification policy, exact terminal summaries, and shared
conformance behavior. Adapters alone own format decoding.

`source-memory` proves the seam with deterministic canonical columns and opt-in
fault fixtures. `source-las` supports LAS record formats 0–10 and LAZ formats
0–8 while preserving exact ticks, supported Attributes, ordered VLR/EVLR
metadata, opaque Coordinate Reference WKT, strict complete direct GeoTIFF
profiles, and Source order. LAZ formats 9 and 10 are explicitly unsupported
until WavePacket14 can be decoded exactly.

No Source crate owns a Workspace, Spatial Index, View, renderer, or Source
rewrite.

**Independent proof:** shared conformance tests run against memory, generated
LAS, and generated LAZ; direct examples and one-million-Point read benchmarks
use the public Source interface.

## 4. point-index

**Job:** prepare and query one rebuildable persistent mapping from space to
candidate Source ranges.

It owns deterministic fixed Source blocks, BVH topology, append-only resumable
work frames, checksummed complete artifacts, no-replace publication,
conservative candidate plans, immutable hierarchy facts, bounded display
samples, and exact Source-backed leaf reads.

Primary shape:

~~~rust,ignore
let index = point_index::prepare(
    source,
    target,
    PrepareLimits::default(),
).blocking_wait()?;

let inspection_index = point_index::prepare_with_recipe(
    attributed_source,
    inspection_target,
    IndexRecipe::InspectionV1(attribute_ids),
    PrepareLimits::default(),
).blocking_wait()?;

let measured_cold_index = point_index::prepare_fresh_with_recipe(
    measured_source,
    absent_target,
    IndexRecipe::PositionOnlyV1,
    PrepareLimits::default(),
).blocking_wait()?;

let source = index.source();
let candidates = index.candidates(bounds, CandidateLimits::default())?;
~~~

`prepare` is the unchanged position-only disk-v1 path.
`prepare_with_recipe` can select the disk-v2 inspection profile for raw
intensity, classification, and optional all-or-none RGB display samples. It
does not own color mapping, exact classification predicates, Revision overlays,
camera LOD, renderer updates, or Workspace persistence.
`prepare_fresh_with_recipe` is the ownership-safe cold-build variant used by
the index and viewing benchmarks and the field-corpus runner. It never consumes
an existing resumable work family merely to discover that a measurement was
not cold.

**Independent proof:** memory-source oracles, frozen v1/v2 complete/work
fixtures, cold/resumed/warm persistence reads, corruption/limit/incompatible-
target tests, generated LAS/LAZ process smoke, a direct-use example, and the
index benchmark run without a Workspace or GPU.

## 5. point-workspace

**Job:** make narrow exact classification selection and reversible
classification history durable, and stream exact effective Point rows, for one
Source.

The crate is deliberately deep. Its private `selection`, `point_set`, and
`persistence` files do not define separately usable public crates. Keeping
candidate planning, exact Source rechecks, effective overlays, temporary Point
Set storage, Revision records, and Operation reconciliation together gives the
caller one coherent authority boundary.

It owns:

- one Workspace manifest and exclusive local lock;
- one complete `PreparedIndex` and its retained verified Source;
- one explicitly selected Source `U8` classification Attribute;
- immutable root, head, and historical Snapshots;
- exact All, inclusive world-box, and explicit Point-ID selection;
- optional effective-classification equality;
- exact ordered Snapshot Point-row streaming with ticks, effective
  classification, complete-only hashes, and cumulative limits;
- process-scoped spillable Point Sets and bounded repeated ID reads;
- sparse uniform classification assignment;
- immediate-head Revert as a new inverse Revision;
- immutable ready, rejection, and Revision files; and
- caller-owned Operation reconciliation and retry.

Primary shape:

~~~rust,ignore
let workspace = point_workspace::create(
    root,
    index,
    WorkspaceSchema::new(classification_attribute),
    OpenLimits::default(),
).blocking_wait()?;

let snapshot = workspace.head();
let points = snapshot.select(
    PointQuery::within(bounds).classification_is(2),
    PointSetLimits::default(),
).blocking_wait()?;

let operation = OperationId::generate()?;
let outcome = workspace.commit(
    CommitRequest::set_classification(operation, points, 1),
    CommitLimits::default(),
).blocking_wait()?;
let revision = match outcome {
    CommitOutcome::Committed(receipt) => receipt.revision(),
    other => return handle_noncommitted(other),
};

let audit = workspace
    .revision_audit(revision, RevisionAuditLimits::default())
    .blocking_wait()?;
~~~

Public types include `Workspace`, `Snapshot`, `PointSet`, `PointSetEntry`,
`PointSetEntryBatch`, `PointSetEntryBatches`, `PointQuery`,
`SnapshotPointBatch`, `SnapshotPointBatches`, `SnapshotPointSummary`,
`RevisionAudit`, `ClassificationTransition`, `WorkspaceSchema`, the commit and
Operation-resolution variants, explicit limit types, and bounded diagnostics.
`PreparedIndex::source()` is the only Source seam needed by construction.
`Workspace::schema()` exposes the selected classification Attribute without
exposing private manifest storage.

It does not own Source discovery/verification, index building, screen
projection, general Attribute streaming or position edits, named Point Sets,
branches, compaction, terrain, rendering, or application policy.

**Independent proof:** headless public interface tests cover lifecycle,
selection, forced spill, commit, Revert, historical Snapshot, reopen, retry,
resolution, corruption, lock conflict, hard limits, and generated LAS/LAZ.
Private fault tests exercise every persistence publication class. Six public
Point-row tests cover root/Edit/history/Revert overlays, partition-independent
hashes, generated LAS/LAZ, cumulative limits, complete no-match streams, fused
error, and cancellation.

The v0.7 audit seam adds `RevisionAudit`, `ClassificationTransition`, and
`RevisionAuditLimits` behind `Workspace::revision_audit`. It validates immutable
Revision structure and hashes, joins exact Source positions, and publishes
complete transitions, Point membership/content hashes, and Edit Footprint only
after bounded completion. It adds no persisted audit cache or schema change.

## 5a. point-review

**Job:** turn renderer-neutral screen or Point identity input into exact facts
from one immutable Workspace Snapshot.

It owns normalized finite `ScreenRect` values, a Camera/Viewport-bound
`ScreenSelection`, explicit review limits, f64 perspective and orthographic
projection, complete screen-through evaluation, one-Point confirmation, and
complete-only review summaries. It scans CPU-authoritative Snapshot rows and
materializes accepted identities through `Snapshot::select_point_ids`; it does
not consume GPU positions, depth, visibility, or residency.

It does not own `PickHit` generation validation, input gestures, windows, wgpu,
Workspace creation, commits, Revert, recovery policy, or renderer highlights.
Those remain host decisions composed from existing public seams.

**Independent proof:** deterministic memory-Source interface tests cover both
projection kinds, classification overlays, inclusive boundaries, invalid
rectangles and identities, limits, cancellation, resident/forced-spill Point
Sets, and exact confirmed Point values. The generated benchmark measures the
complete CPU scan without claiming production latency.

## 6. point-terrain

**Job:** derive and evaluate the narrow Terrain Surface and encode its one
supported deliverable.

`point-terrain` owns exact Ground Input ingestion through
`Snapshot::point_rows`, deterministic robust unconstrained 2.5D triangulation,
canonical `SurfaceVertex` and `SurfaceFace` values, immutable descriptor and
resource facts, detached Check Point QA, and the private metric-metre LandXML
1.2 encoder. A supported structured v0.12 profile is retained in the descriptor
and emitted as one LandXML `CoordinateSystem`; structured non-metre profiles
fail before QA/export. Its private derivation, triangulation, QA, and XML files
do not form adapter seams.

Primary shape:

~~~rust,ignore
let surface = point_terrain::derive(
    snapshot,
    TerrainRecipe::new(2),
    TerrainLimits::default(),
).blocking_wait()?;

let report = surface
    .check_points(check_points, CheckPointLimits::default())
    .blocking_wait()?;
let receipt = surface
    .ensure_landxml(target, options, LandXmlLimits::default())
    .blocking_wait()?;
~~~

It does not own Source/index discovery, Workspace edits, Breaklines,
constrained topology, terrain persistence, coordinate transformation, general
LandXML, rendering, or host recovery policy.

**Independent proof:** package and documentation tests cover the public
interface, robust topology/oracle agreement, degeneracy and every resource
family, large-world and extreme finite numeric behavior, detached QA,
overflow-safe residual statistics, exact-existing reconciliation, injected
durable LandXML certainty boundaries, and independent XML semantics. A public
example and generated 10k/100k/1M-capable benchmark compose only public seams;
`terrain-demo` is the real LAS/LAZ process caller.

## 7. render-protocol

**Job:** define and validate generation-safe renderer-neutral point display
state.

It owns camera/frame values, stable batch keys, monotonically versioned atomic
Upserts, conditional Removes, Reset generations, independently bounded complete
highlight input, and CPU state-model validation. It owns no GPU, I/O, Source,
index, LOD, or Workspace behavior.

**Independent proof:** contract and state-model tests run without a GPU.

## 8. point-view

**Job:** plan one frozen View over a host-owned hierarchy without performing
I/O.

It owns frustum culling, screen-space-error LOD, hysteresis, deterministic
priority, point/byte/batch reservation, demanded-node reporting, progressive
parent Coverage, retention, and exact conditional retirement. The host owns
hierarchy acquisition, request execution, and renderer update application.

**Independent proof:** generated hierarchy tests and the planner benchmark run
without a Source, Workspace, or GPU.

## 9. render-wgpu

**Job:** maintain and draw one wgpu representation of render-protocol state.

It owns GPU buffers, shaders, pipelines, depth, command-encoded uploads,
draw/pick recording, resource-pinning `RecordedFrame`, and logical residency
reports. The host owns the device, queue, encoder, target, submission, polling,
and device-loss policy.

It does not perform Source I/O, LOD planning, exact selection, Edit, automatic
eviction, or queue submission.

**Independent proof:** required local offscreen GPU tests apply generated
render-protocol updates; renderer-neutral tests remain GPU-free.

## 10. renderer-demo

**Job:** exercise one complete application composition without turning host
policy into foundation interfaces.

The demo can use generated hierarchy data or Full-verify a supported LAS/LAZ,
build/open its index, materialize demanded nodes, and apply atomic renderer
updates. Its `--smoke` mode accepts one complete CPU-model Upsert without a GPU.
The v0.10 private display policy preserves neutral color by default and can map
exact sampled world Z or raw inspection Attributes deterministically without
changing Point Identity, geometry, or Coverage. Point-index owns only the
versioned bounded raw samples; renderer-demo owns presentation mapping.

The host also owns perspective/orthographic orbit-pan-zoom controls, truthful
demand/issued/resident and Sampled/Complete Coverage presentation, stable
`PVIEW_*` diagnostics, and the permission-gated bounded corpus manifest and
canonical no-replace Viewing Report. With an explicitly opened existing
Workspace it composes provisional generation-checked picking, pinned exact
confirmation/screen-through review, Point Set-derived highlights, caller-owned
classification Operations, Revision Audit, immediate-head Revert, and
same-identity reconciliation. Its report does not claim production
corpus completion, professional preference, terrain capacity, partner
acceptance, or human-time savings.

The bridge is private because a second application has not proven a reusable
materialization seam.

**Independent proof:** CPU mapping/grammar/state tests, generated LAS/LAZ
process smoke, corpus manifest/report/publication fixtures, renderer-neutral
planner tests/benchmark, and required local offscreen GPU tests cover the
private composition without turning it into a public framework.

## 11. terrain-demo

**Job:** own one recoverable GPU-free LAS/LAZ-to-Terrain Workflow Run and its
read-only post-Run qualifier without turning application policy into another
foundation crate.

The package exposes a small application facade: `WorkflowPaths`,
`WorkflowRunIntent`, `WorkflowLimits`, `WorkflowPhase`, `WorkflowReceipt`,
`WorkflowStatus`, `WorkflowFailure`, `start_run`, `resume_run`, and
`inspect_and_repair_run`. The binary is a thin bounded grammar and presentation layer with
`start`, `resume`, `inspect`, `compare-landxml`, and `verify-round-trip`
commands. Start/resume require the same
caller-owned Run/Operation identities, expected baseline Revision, nonempty
exact Ground-ordinal set, normalized Terrain Recipe, detached Check Points,
LandXML options, four paths, and limits.

The application Full-verifies the supported LAS/LAZ Source, prepares or opens
its Spatial Index, opens the caller-created Workspace, resolves or commits the
recorded classification correction, audits the resulting Revision, derives
the baseline and changed class-2 Surfaces, evaluates detached QA, and ensures
the supported LandXML and canonical report. The opaque Coordinate Reference
requires the caller's explicit metric-metre assertion.

The Workflow never creates a Workspace. The caller creates one through the
public `point-workspace` lifecycle, supplies the current head as the baseline,
uses Source Attribute 6 (`source-las` classification) as the selected `U8`
Attribute, and provides an already existing Run-root directory. An absent
Workspace is an invalid request before Run creation or Workspace mutation.

Private journal, workflow, report, publication, comparison, streaming, evidence,
qualification, and diagnostic modules own the
exclusive Run lock, exact path bindings, eight-frame journal, Operation
resolution, Revision Audit, baseline/changed Derivation, conservative Surface
Change Envelope, QA, LandXML/report reconciliation, and one-action structured
failures. The fixed Run root contains `run.pwf`, `run.lock`, `terrain.xml`, and
`audit.json`. No Terrain Surface or audit cache is persisted.

Qualification strictly reads a Complete Run, original and returned LandXML,
and canonical report without repair or Run-root mutation. It streams the full
supported export ceiling, evaluates the narrow semantic model, and creates or
exactly reconciles separate canonical pass/fail evidence outside the Run root.
The caller declaration is recorded but never treated as observed external
execution.

**Independent proof:** package, frozen-fixture, workflow-facade, and process
tests cover every eight-frame restart prefix, one-Revision idempotence, exact
report/XML/evidence conflict and recovery, public limit families, LAS/LAZ
semantic projection, Complete-Run and immutable-input binding, streaming XML
and semantic reason families, Source immutability, stale/mismatched state,
Retryable intent, cancellation, dropped-Workflow recovery, and CLI diagnostics.
The generated 10k/100k/1M-capable Criterion benchmark has five
cold/restart/reconciliation modes. Exact current results belong to the release
record; generated evidence claims neither worker heap nor external acceptance.

Private journal faults exhaust the application-defined Intent-publication and
`Complete` append-before-write, before-sync, and after-sync lost-
acknowledgement boundaries. Private report faults exhaust the application-
defined post-link boundaries. Pre-link cancellation/failure, exact/conflicting
`AlreadyExists` races, post-link replacement, target kind, staging/working
limits, and stage/parent directory identity cases are representative. This is
not a claim about every possible operating-system fault.

The private workflow regression additionally rederives the immediate-head
Revert and proves that its baseline-to-restored Surface Change Envelope is
empty. Retained v0.6 regressions also cover exact changed Ground Input,
post-Revert geometry/topology/vertex/face restoration, byte-identical Source
data, and explicit immediate-head Revert behavior. These regressions preserve
the lower-level terrain and Workspace guarantees without retaining the
superseded one-shot CLI grammar. The v0.7 Workflow leaves a committed
classification Revision in place when a later phase fails.

## Deferred modules

Future Breakline/constrained-terrain behavior, terrain persistence, general
LandXML, desktop UI, bindings, or a COPC adapter require accepted versioned
designs. The roadmap does not authorize empty crate scaffolding.

## Allowed dependency direction

The allowlist is stricter than what Cargo can compile:

| Module | May depend on |
|---|---|
| `point-contracts` | standard library and narrow value dependencies |
| `foundation-runtime` | standard library and narrow concurrency dependencies |
| `point-source` | `point-contracts`, `foundation-runtime` |
| `source-memory`, `source-las` | `point-source`, `point-contracts`, `foundation-runtime` |
| `point-index` | `point-source`, `point-contracts`, `foundation-runtime` |
| `point-workspace` | `point-index`, `point-source`, `point-contracts`, `foundation-runtime` |
| `point-review` | `point-workspace`, `render-protocol`, `point-contracts`, `foundation-runtime` |
| `point-terrain` | `point-workspace`, `point-contracts`, `foundation-runtime`, narrow private algorithm/encoding dependencies |
| `render-protocol` | `point-contracts` |
| `point-view` | `render-protocol` and narrow math/value dependencies |
| `render-wgpu` | `render-protocol`, `point-contracts` |
| `renderer-demo` | only the Source/index/Workspace/review/View/render crates it composes |
| `terrain-demo` | `source-las`, `point-source`, `point-index`, `point-workspace`, `point-terrain`, `point-contracts`, `foundation-runtime`, and narrow checksum, identity-generation, and error dependencies |

Additional rules:

- no crate below the Workspace authority boundary depends on `point-workspace`
  or an application adapter;
- no headless crate depends on wgpu or a windowing library;
- no renderer inspects private Source, index, or Workspace storage;
- no format adapter depends on a Workspace; and
- feature flags cannot change identity, completeness, Revision, or rendering
  correctness semantics.

## Public seams and private locality

The implemented reusable seams are:

1. canonical values in `point-contracts`;
2. runtime-neutral work control in `foundation-runtime`;
3. verified bounded `point-source` access shared by memory/LAS/LAZ;
4. complete persistent `point-index` preparation and reads;
5. the one-deep-crate `point-workspace` document, exact Point-row, and Revision
   Audit interface;
6. exact renderer-neutral Snapshot review in `point-review`;
7. deterministic Terrain Derivation, QA, and supported deliverable ensure in
   `point-terrain`;
8. generation-safe `render-protocol` values;
9. deterministic `point-view` planning; and
10. host-owned-lifecycle `render-wgpu` recording.

Index pages, decoder buffers, Point Set frames, overlay tables, Operation
records, Revision blocks, triangulation arenas, XML encoder state, scheduling
details, GPU bindings, and demo staging remain private.
