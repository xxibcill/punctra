# Module Catalog

Status: current through the narrow v0.6 terrain/QA slice; broader terrain and
export modules deferred

This is the ownership map for implemented crates. Each crate has one public
job. Several private files may cooperate behind one deep interface; private
file boundaries are not public module promises.

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
| `point-terrain` | Derive and evaluate the narrow v0.6 Terrain Surface and encode its supported deliverable. | Snapshot, Terrain Recipe, detached Check Points | `TerrainSurface`, QA report, LandXML receipt |
| `render-protocol` | Define generation-safe renderer-neutral point display state. | Camera and display values | Validated updates and frame values |
| `point-view` | Plan one frozen View over a host-owned hierarchy without I/O. | Camera, viewport, hierarchy/residency, budget | Demand, requests, retention, retirements |
| `render-wgpu` | Maintain and draw one wgpu representation of render-protocol state. | Render updates, frame, host target | Recorded commands, report, provisional picks |
| `renderer-demo` | Exercise synthetic or indexed LAS/LAZ View-to-render composition. | CLI or generated inputs | Interactive demo or GPU-free process smoke |
| `terrain-demo` | Exercise the headless LAS/LAZ-to-terrain composition. | LAS/LAZ and caller-owned paths/options | Indexed Workspace, Terrain/QA report, LandXML file |

`source-copc`, constrained or persisted terrain, general LandXML, general
application UI, bindings, and remote storage are not implemented modules in
v0.6.

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
cancellation, and bounded pull-stream conventions. It does not own durable
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
metadata, Coordinate Reference WKT, and Source order. LAZ formats 9 and 10 are
explicitly unsupported until WavePacket14 can be decoded exactly.

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

let source = index.source();
let candidates = index.candidates(bounds, CandidateLimits::default())?;
~~~

It does not own exact classification predicates, Revision overlays, camera LOD,
renderer updates, or Workspace persistence.

**Independent proof:** memory-source oracles, persistence/resume tests, generated
LAS/LAZ process smoke, a direct-use example, and the index benchmark run without
a Workspace or GPU.

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
~~~

Public types include `Workspace`, `Snapshot`, `PointSet`, `PointQuery`,
`SnapshotPointBatch`, `SnapshotPointBatches`, `SnapshotPointSummary`,
`WorkspaceSchema`, the commit and Operation-resolution variants, explicit
limit types, and bounded diagnostics. `PreparedIndex::source()` is the only
Source seam needed by construction.

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

## 6. point-terrain

**Job:** derive and evaluate the narrow v0.6 Terrain Surface and encode its one
supported deliverable.

`point-terrain` owns exact Ground Input ingestion through
`Snapshot::point_rows`, deterministic robust unconstrained 2.5D triangulation,
canonical `SurfaceVertex` and `SurfaceFace` values, immutable descriptor and
resource facts, detached Check Point QA, and the private metric-metre LandXML
1.2 encoder. Its private derivation, triangulation, QA, and XML files do not
form adapter seams.

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
    .export_landxml(target, options, LandXmlLimits::default())
    .blocking_wait()?;
~~~

It does not own Source/index discovery, Workspace edits, Breaklines,
constrained topology, terrain persistence, coordinate transformation, general
LandXML, rendering, or host recovery policy.

**Independent proof:** 41 package tests—15 unit/private and 26 integration—plus
one documentation test cover the public interface, robust topology/oracle
agreement, degeneracy and every resource family, detached QA, injected durable
LandXML certainty boundaries, and independent XML semantics. A public example
and generated 10k/100k/1M-capable benchmark compose only public seams;
`terrain-demo` is the real LAS/LAZ process caller.

## 7. render-protocol

**Job:** define and validate generation-safe renderer-neutral point display
state.

It owns camera/frame values, stable batch keys, monotonically versioned atomic
Upserts, conditional Removes, Reset generations, and bounded CPU state-model
validation. It owns no GPU, I/O, Source, index, LOD, or Workspace behavior.

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

The bridge is private because a second application has not proven a reusable
materialization seam.

## 11. terrain-demo

**Job:** exercise the complete GPU-free LAS/LAZ-to-Terrain composition without
turning host policy into another foundation interface.

The application Full-verifies a supported LAS/LAZ Source, prepares or opens its
Spatial Index, creates or opens one Workspace, derives the class-2 Terrain,
optionally evaluates a built-in covered/gap QA sample, and durably creates the
supported LandXML file with explicit document date/time. The opaque Coordinate
Reference requires the caller's explicit metric-metre assertion. Its optional
`--exercise-correction-revert ORDINAL` path performs one exact classification
correction, re-Derivation, immediate-head Revert, and exact Surface-restoration
check.

**Independent proof:** one process integration test runs generated LAS and LAZ
through the complete composition without a GPU and checks deterministic output
semantics, changed Ground Input, exact post-Revert geometry/topology/vertices/
faces, and byte-identical Source data.

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
| `point-terrain` | `point-workspace`, `point-contracts`, `foundation-runtime`, narrow private algorithm/encoding dependencies |
| `render-protocol` | `point-contracts` |
| `point-view` | `render-protocol` and narrow math/value dependencies |
| `render-wgpu` | `render-protocol`, `point-contracts` |
| `renderer-demo` | only the Source/index/View/render crates it composes |
| `terrain-demo` | only the Source/index/Workspace/terrain crates it composes |

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
5. the one-deep-crate `point-workspace` document and exact Point-row interface;
6. deterministic Terrain Derivation, QA, and supported deliverable creation in
   `point-terrain`;
7. generation-safe `render-protocol` values;
8. deterministic `point-view` planning; and
9. host-owned-lifecycle `render-wgpu` recording.

Index pages, decoder buffers, Point Set frames, overlay tables, Operation
records, Revision blocks, triangulation arenas, XML encoder state, scheduling
details, GPU bindings, and demo staging remain private.
