# Terrain and QA Benchmark Design (v0.6)

Status: accepted and implemented on 2026-08-10; repository technical slice
locally verified

This design is authoritative for Punctra v0.6. It records one narrow technical
slice from a revision-pinned LAS/LAZ Workspace to one CPU-authoritative terrain
deliverable. The roadmap remains evidence-led: accepting this design does not
claim licensed production-data, design-partner, downstream-application, or
human-time evidence that has not been collected.

## Outcome

v0.6 adds one deep `point-terrain` crate that:

1. reads exact position and effective-classification rows from one immutable
   Workspace Snapshot;
2. derives one deterministic, unconstrained, in-memory 2.5D triangulated
   Terrain Surface from the explicitly selected ground class and optional
   inclusive world bounds;
3. compares bounded detached Check Points with that surface using one explicit
   signed-residual convention; and
4. atomically creates one narrowly scoped LandXML 1.2 metric-metre TIN export.

One headless `terrain-demo` application is the real caller. It composes local
LAS/LAZ Source opening, Spatial Index preparation, Workspace open/create,
existing reversible classification correction, terrain Derivation, Check
Point QA, and LandXML export. It does not establish a product UI or a reusable
application framework.

The release also earns one narrow `Snapshot::point_rows` interface in
`point-workspace`. Terrain does not receive a Source, Spatial Index, and
Workspace overlay separately, and callers do not reconstruct that join.

## Evidence boundary

The implemented repository evidence proves deterministic topology, exact Snapshot
input, hard resource ceilings, explicit degenerate-geometry failures, Check
Point arithmetic, atomic create-new export, independent semantic XML parsing,
and unchanged Source bytes on generated fixtures.

It cannot prove any of the following without external evidence:

- behavior on licensed production LAS/LAZ or Sources above 500 million Points;
- acceptance against a design partner's coordinate, accuracy, or deliverable
  tolerances;
- import and round-trip through a named Civil 3D, Bentley, or other downstream
  application version;
- time to first use, human attention, rework, or accepted-deliverable time; or
- five-times faster startup or 50-percent lower attended production time.

Those claims remain outstanding even after all repository gates pass.

## One deep terrain module

`point-terrain` has one public job: derive and evaluate the narrow v0.6 Terrain
Surface and encode its one supported deliverable.

Private implementation modules may own Snapshot ingestion, exact predicates,
triangulation, canonicalization, point location, Check Point evaluation,
LandXML encoding, limits, and errors. Those are not public construction seams.
There is no public triangulator trait, terrain-input trait, QA adapter, or
exporter registry. One Snapshot input and one LandXML encoding do not justify
hypothetical adapters.

The allowed dependency direction is:

```text
point-contracts/foundation-runtime
                ^
                |
        point-workspace
                ^
                |
          point-terrain
                ^
                |
          terrain-demo
```

`point-terrain` depends on `point-workspace`, `point-contracts`,
`foundation-runtime`, and narrow private algorithm/encoding dependencies. It
does not depend directly on `point-source`, `point-index`, a Source adapter,
wgpu, or an application crate. `terrain-demo` owns composition with
`source-las`, `point-index`, and `point-workspace`.

The deletion test is deliberate: deleting `point-terrain` removes terrain
input normalization, topology, QA, and LandXML complexity. That complexity
does not reappear in every caller or enlarge the Workspace persistence model.

## Exact Snapshot Point rows

The v0.5 Point Set interface exposes stable membership and private effective
before-values, but not positions. Iterating its IDs and asking the Source to
join them would duplicate Workspace logic, materialize an unnecessary Point
Set, and can exceed the Source raw-span ceiling for large sparse membership.

v0.6 therefore adds this narrow pull stream to `point-workspace`:

```rust,ignore
impl Snapshot {
    pub fn point_rows(
        &self,
        query: PointQuery,
        limits: PointRowLimits,
    ) -> Result<SnapshotPointBatches, WorkspaceError>;
}

pub struct SnapshotPointBatch { /* bounded ordered columns */ }

impl SnapshotPointBatch {
    pub fn source(&self) -> SourceId;
    pub fn ordinals(&self) -> &[u64];
    pub fn positions(&self) -> &QuantizedPositions;
    pub fn effective_classifications(&self) -> &[u8];
    pub fn point_id(&self, row: usize) -> Option<PointId>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}

impl SnapshotPointBatches {
    pub fn source_metadata(&self) -> &SourceMetadata;
    pub fn handle(&self) -> OperationHandle;
    pub fn next(
        &mut self,
    ) -> Result<Option<SnapshotPointBatch>, WorkspaceError>;
    pub fn summary(&self) -> Option<&SnapshotPointSummary>;
}
```

Each nonempty batch has equal-length ordinal, position, and effective-
classification columns. Rows are ordered by strictly increasing Source
ordinal across the complete stream and never duplicate a Point. Position ticks
and the Source transform remain exact. Classification is the value after every
overlay through the pinned Revision.

The stream supports only the existing `PointQuery` grammar: All or one
inclusive finite `WorldBounds`, followed by optional equality against the
Workspace's one effective `U8` classification Attribute. It does not expose
general Attributes, predicates, Source spans, overlay rows, or position Edits.

Returned batches are provisional observations until `next` reaches terminal
`None` and `summary` publishes complete provenance, normalized Query facts,
and exact row count. Failure or cancellation publishes no terminal summary.
Terrain may stage private partial work, but it never publishes a Terrain
Surface from an incomplete row stream.

`PointRowLimits` separately bound candidate nodes, candidate spans and Points,
Source batch Points and payload, adapter working bytes, overlay segments and
bytes, emitted rows, output batch rows and payload, and peak incremental
working bytes. Point Set resident/spill limits are not reused because this
stream retains no complete Point Set.

## Public terrain interface

The public surface is intentionally small and intent-shaped:

```rust,ignore
pub type TerrainJob = Job<TerrainSurface, TerrainError>;
pub type CheckPointJob = Job<CheckPointReport, TerrainError>;
pub type LandXmlJob = Job<LandXmlReceipt, TerrainError>;

pub fn derive(
    snapshot: Snapshot,
    recipe: TerrainRecipe,
    limits: TerrainLimits,
) -> TerrainJob;

impl TerrainRecipe {
    pub fn new(ground_classification: u8) -> Self;
    pub fn within(self, bounds: WorldBounds) -> Self;
}

impl TerrainSurface {
    pub fn descriptor(&self) -> &TerrainDescriptor;
    pub fn vertices(&self) -> &[SurfaceVertex];
    pub fn faces(&self) -> &[SurfaceFace];

    pub fn check_points<I>(
        &self,
        check_points: I,
        limits: CheckPointLimits,
    ) -> CheckPointJob
    where
        I: IntoIterator<Item = CheckPoint> + Send + 'static;

    pub fn export_landxml(
        &self,
        target: impl AsRef<Path>,
        options: LandXmlOptions,
        limits: LandXmlLimits,
    ) -> LandXmlJob;
}
```

Exact field shapes may be adjusted while preserving this interface meaning.
The design does not authorize additional behavior under generic request enums.

`derive` owns the `Snapshot` for the Job lifetime. The resulting
`TerrainSurface` retains only immutable in-memory topology and copied
provenance; it does not retain the Workspace session or lock. A caller that
needs its Snapshot concurrently clones the immutable handle explicitly.

The normalized Recipe contains exactly:

- one explicit effective-classification byte considered ground;
- optional inclusive `WorldBounds`; and
- the fixed v0.6 terrain algorithm version.

Worker count, Source batch partitioning, memory limits, Job scheduling, path
names, and timestamps are execution facts and never Recipe or topology facts.
The implementation supports one worker in v0.6 and exposes no worker-count
option.

`TerrainDescriptor` records the Snapshot provenance, normalized Recipe and
digest, terrain algorithm version, Source transform and Coordinate Reference,
input/vertex/face/hull counts, inclusive terrain bounds, canonical input,
geometry, topology, and Artifact hashes, and accounted peak working bytes,
retained Surface bytes, and topology steps. Cargo version, terrain algorithm
version, and LandXML subset version evolve independently.

`SurfaceVertex` records one one-based `SurfaceVertexId`, one Source-aware Point
Identity, and its exact Source position ticks. `SurfaceFace` records one one-
based `SurfaceFaceId` and three one-based vertex identities in counter-clockwise
order. No face owns copied positions.

## Ground Input

Terrain derives its own exact Query from the Recipe:

```rust,ignore
let query = match recipe.bounds() {
    Some(bounds) => PointQuery::within(bounds),
    None => PointQuery::all(),
}
.classification_is(recipe.ground_classification());
```

Every returned Snapshot Point row is Ground Input. No classifier, confidence,
screen selection, display sample, or index approximation can add or remove a
Point. Inclusive bounds and effective-classification equality are rechecked on
canonical Source positions and the pinned Revision.

The Spatial Index remains a rebuildable accelerator. Deleting it after opening
the Snapshot cannot change the intended Terrain meaning. GPU/View state is
never terrain input.

## Deterministic unconstrained TIN

v0.6 derives an unconstrained 2.5D triangulation. Terrain vertices are the
Ground Input Points; XY defines topology and Z supplies elevation.

Before triangulation, the implementation sorts Ground Input by the exact key:

```text
(x tick, y tick, z tick, Point Identity)
```

It then rejects, rather than guesses around, all unsupported degeneracy:

- fewer than three Ground Input Points;
- any duplicate XY position, including duplicate XYZ observations;
- duplicate XY with different Z as conflicting elevation evidence;
- all vertices collinear in XY;
- an exact-predicate or topology-work limit breach; or
- any topology that cannot satisfy the declared invariants.

Orientation and in-circle decisions use deterministic robust signs, never a
caller-selected epsilon. Canonical insertion order is independent of Source
batch partitioning. A cocircular diagonal tie is resolved by the
lexicographically smaller undirected pair of canonical vertex keys.

Every emitted face:

- references three distinct vertices;
- has positive counter-clockwise XY orientation;
- is rotated so its lowest vertex index appears first; and
- participates in a final lexicographic face sort.

The final surface has no duplicate faces, crossing edges, non-manifold edges,
or faces outside the convex hull. Its boundary is the convex hull of Ground
Input. The module creates no boundary polygon, hole, island, Breakline, Steiner
vertex, vertical face, or extrapolated face.

Single-worker execution and fixed operation order are part of v0.6's
determinism contract. Repeated runs with equal Snapshot provenance and Recipe
must return byte-identical public vertices, faces, descriptor hashes, and
LandXML semantics. A Revert that restores equal effective classification may
produce equal geometry/topology hashes even though the newer Snapshot has a
different Revision provenance.

## Detached Check Point QA

v0.6 supports only detached Check Points supplied explicitly by the caller.
It does not Query residuals for Source Points or infer Check Points from an
Attribute.

```rust,ignore
pub struct CheckPoint {
    /* bounded caller identity plus finite [x, y, z] */
}

pub struct CheckPointResult {
    /* input identity/position, sampled elevation or gap, residual */
}
```

Check Point identities are unique within one request. Positions must be finite
and already expressed in the Terrain Surface's Coordinate Reference, axis
order, and units. No coordinate, datum, or unit transformation occurs.

Surface location uses deterministic XY predicates. A Point inside a face or on
its closed boundary receives one interpolated surface elevation. On a shared
edge or vertex, deterministic face choice cannot change the elevation because
the incident planar faces share the same edge vertices. A Point outside the
convex hull produces an explicit Terrain Gap; it is not extrapolated and does
not contribute a numeric residual.

For a covered Check Point, signed residual is exactly:

```text
observed Check Point z - interpolated Terrain Surface z
```

A positive residual is above the surface and a negative residual is below it.
The report preserves caller order and contains one result per Check Point plus
covered/gap counts and deterministic min, max, mean, and root-mean-square
residuals for covered values. Empty covered sets expose absent statistics
rather than fabricated zeroes.

`CheckPointLimits` cap input Check Points, retained results, result bytes,
point-location work, and peak incremental memory. A limit or invalid input
publishes no partial report.

## Reversible ground correction

`point-terrain` never changes classification or Source bytes. The headless host
uses the existing v0.5 interface:

1. obtain caller-approved Point Identities;
2. materialize them with `Snapshot::select_point_ids`;
3. record an `OperationId` before commit;
4. commit `CommitRequest::set_classification` to the chosen ground or
   non-ground value;
5. reconcile any indeterminate Operation exactly as v0.5 requires; and
6. derive a new Terrain Surface from the committed head Snapshot.

Immediate-head Revert remains the only reversal. Reverting classification
creates another immutable Revision and a later Derivation; it never mutates an
earlier Terrain Surface. There is no provisional classifier in v0.6.

## LandXML 1.2 metric-metre subset

LandXML encoding is private to `point-terrain`. One encoder and one caller do
not justify a public exporter seam or another crate.

The accepted subset creates one UTF-8 LandXML 1.2 document containing:

- one metric Units declaration with metres as the linear unit;
- caller-supplied, strictly validated document date and time values required by
  the LandXML 1.2 root element;
- one caller-bounded Surface name;
- one TIN Surface Definition;
- one deterministic point entry per canonical Terrain vertex; and
- one deterministic face entry per canonical Terrain face.

Point identifiers are consecutive and derived only from canonical vertex
order. Faces reference those identifiers and preserve canonical face order.
World coordinates are computed from the exact Source ticks and transform,
then written with a deterministic finite round-tripping decimal encoding. The
profile assumes Source X is easting, Y is northing, and Z is elevation; the
LandXML coordinate tuple is encoded in its required northing, easting,
elevation order. The caller must use this export only when the Source
coordinates are already metric metres and must make that assertion explicitly.
The exporter performs no unit or CRS transformation and never interprets the
opaque declared-or-unknown Coordinate Reference.

Document date and time are explicit `LandXmlOptions` facts. The encoder never
reads the system clock, infers them from filesystem metadata, or injects another
nondeterministic timestamp. Equal Terrain Surface and options therefore produce
equal XML bytes.

The encoder does not emit Breaklines, boundaries, contours, COGO Points,
alignments, profiles, grids, volume surfaces, parcels, or application-specific
extensions. It does not import LandXML.

Export uses create-new publication:

1. reject a pre-existing target without opening it for mutation;
2. encode a bounded sibling staging file;
3. flush, sync, close, and reopen the stage read-only;
4. verify the expected length and content hash;
5. publish to the target without replacement;
6. sync the parent directory; and
7. remove disposable staging state.

Before publication, cancellation or failure leaves no target. A failure after
publication begins reports an explicit indeterminate export with the expected
content hash; the caller inspects the target rather than overwriting it.
`LandXmlReceipt` records the Terrain descriptor hashes, output content hash,
byte length, and vertex/face counts.

`LandXmlLimits` separately cap vertices, faces, output bytes, write-buffer
bytes, staging bytes, XML token bytes, and peak incremental working bytes.

Independent acceptance uses `roxmltree` only in integration-test/support code,
not encoder helpers. The parser must independently verify the LandXML version,
metric-metre declaration, one-Surface shape, unique consecutive point IDs,
finite coordinates, valid face references, exact vertex and face counts, and a
semantic digest matching the in-memory Terrain Surface. This proves the
repository encoder/parser separation. It does not claim import into a named
downstream application.

## No terrain persistence

`TerrainSurface` is an immutable in-memory Artifact. v0.6 defines no terrain
file, open/reopen interface, work journal, resume behavior, cache, migration,
or Workspace-owned terrain record. Process exit discards the surface and the
caller derives it again from the immutable Snapshot and Recipe.

The only new durable output is the caller-requested LandXML file. LandXML is an
Export, not authoritative Workspace state. Source bytes, Workspace manifest,
Revisions, and Spatial Index bytes remain unchanged by terrain Derivation,
Check Point QA, and export.

## Resource contracts

Limits remain operation-specific:

- `PointRowLimits` own exact Snapshot input planning and streaming ceilings;
- `TerrainLimits` own Ground Input rows, vertices, faces, exact-predicate and
  topology steps, resident result bytes, overlapping working allocations, and
  Job progress/cancellation cadence;
- `CheckPointLimits` own detached inputs, location work, retained report bytes,
  and peak working memory; and
- `LandXmlLimits` own staging/output bytes, element counts, buffers, and peak
  working memory.

The in-memory Terrain result counts toward `TerrainLimits` retained bytes.
Temporary capacity accounting includes old and new allocations that overlap
during sorting or growth. Capacity is charged before allocation and checked
against actual capacity after allocation where the allocator may over-reserve.
No resource fallback changes membership, decimates Points, relaxes predicates,
returns partial topology, or omits XML elements.

The one-worker implementation checks cooperative cancellation at bounded row,
predicate, face, Check Point, and XML intervals. Cancellation before a result
or export publication exposes no partial public value.

## Error taxonomy

`TerrainError` is non-exhaustive and distinguishes:

- invalid Recipe, Check Point, or LandXML caller input;
- incompatible Snapshot provenance or coordinate assumptions;
- insufficient Ground Input;
- duplicate XY and conflicting-elevation evidence;
- collinear or otherwise unsupported degenerate geometry;
- deterministic topology-invariant failure;
- explicit resource-limit exhaustion;
- wrapped Workspace Point-row failure;
- target-already-exists conflict;
- unsupported metric-metre LandXML export;
- bounded filesystem I/O;
- cooperative cancellation;
- runtime Job/progress failure; and
- indeterminate LandXML publication.

Diagnostics are bounded and do not retain unbounded external strings or paths.
An outside-surface Check Point is a successful explicit Terrain Gap, not an
error. There are no terrain persistence corruption or recovery errors because
v0.6 persists no Terrain Surface.

## Caller flow

The complete headless composition is:

```rust,ignore
let source = source_las::open("survey.laz").blocking_wait()?;
let index = point_index::prepare(
    source,
    "survey.laz.pidx",
    PrepareLimits::default(),
).blocking_wait()?;

let workspace = open_or_create_workspace(index)?;
let snapshot = workspace.head();

let recipe = TerrainRecipe::new(2);
let terrain = point_terrain::derive(
    snapshot.clone(),
    recipe,
    TerrainLimits::default(),
).blocking_wait()?;

let qa = terrain.check_points(
    load_detached_check_points()?,
    CheckPointLimits::default(),
).blocking_wait()?;
report_check_points(&qa)?;

let receipt = terrain.export_landxml(
    "existing-ground.xml",
    LandXmlOptions::metric_metres(
        "Existing Ground",
        "2026-08-10",
        "00:00:00Z",
    )?.assert_coordinates_are_metric_metres(),
    LandXmlLimits::default(),
).blocking_wait()?;
println!("exported {} bytes", receipt.byte_length());
```

Classification correction remains a separate explicit host action before the
next `derive` call. `terrain-demo` owns its Source/index/Workspace/export paths,
optional built-in covered/gap QA sample, explicit LandXML date/time and unknown-
CRS metric assertion, optional exact-ordinal classification correction/Revert
exercise, reporting, and exit codes. Those policies do not enter foundation
interfaces.

## Verification and acceptance

All verification runs locally. The completed v0.6 technical slice satisfies
these repository acceptance gates:

- public Point-row tests show exact positions and effective classification at
  root, changed, historical, and Revert Snapshots;
- row results are identical across Source batch partitions and generated LAS
  and LAZ, and a terminal summary is absent after error or cancellation;
- Terrain topology is byte-identical across repeated single-worker runs and
  varied Point-row batch sizes;
- a separate small-fixture oracle validates vertex membership, positive face
  orientation, canonical ordering, manifold incidence, convex-hull boundary,
  and local unconstrained Delaunay decisions;
- fixtures cover fewer-than-three Points, duplicate XY, conflicting Z,
  collinear input, cocircular ties, extreme valid ticks, and every declared
  resource family;
- cancellation and allocation failure publish no partial Terrain Surface;
- analytic planar fixtures prove Check Point interpolation, positive/negative/
  zero signed residual, boundary inclusion, and explicit outside gaps;
- classification commit, re-Derivation, immediate-head Revert, and another
  Derivation prove unchanged Source bytes and restored geometry meaning;
- LandXML create-new publication never replaces an existing path and injected
  failures expose no partial acknowledged Export;
- independent `roxmltree` parsing reproduces the in-memory point/face semantics;
- `terrain-demo` exercises generated LAS and LAZ through Workspace, terrain,
  QA, and export without a GPU; and
- formatting, strict lint, workspace tests, warning-free documentation,
  examples, package benchmarks, existing process smoke, and required local GPU
  regressions pass as documented in `CONTRIBUTING.md`.

The terrain benchmark records input/vertex/face counts, retained and accounted
peak working bytes, topology steps, Derivation time, Check Point time and face-
location work, LandXML time and bytes, and the named local machine. It is
generated-fixture technical evidence, not a production-scale or workflow-value
claim. Its `worker_heap_measurement` is explicitly null, so the benchmark does
not claim an observed worker-heap value.

### Implemented repository evidence

- `point-workspace` has 67 tests: 42 unit/fault/allocation tests and 25 public
  integration tests, including six exact Point-row stream tests.
- `point-terrain` has 41 package tests—15 unit/private and 26 integration—plus
  one documentation test across interface, topology, resource, detached-QA,
  LandXML, robust-algorithm, and publication-fault suites.
- `terrain-demo` has one process test that runs generated LAS and LAZ through
  the complete GPU-free caller. LAS/LAZ correction, re-Derivation, immediate-
  head Revert, and restored geometry meaning are covered while Source bytes
  remain unchanged.
- Formatting, strict workspace lint, workspace tests, warning-free
  documentation, every declared example/benchmark/process smoke, and required
  local GPU gates complete through the commands in `CONTRIBUTING.md`.

On the local Apple M5 Pro (`Mac17,9`), 24 GiB, arm64, macOS 26.5.2 reference
machine with Rust 1.90.0, the completed 10,000-Point Criterion run measured:

| Evidence | Local value |
|---|---:|
| Input / vertices / faces / hull vertices | 10,000 / 10,000 / 19,602 / 396 |
| Derivation | 11.983–12.049 ms (829.97–834.53 Kpoints/s) |
| Detached QA | 94.907–95.164 us for 3 Check Points / 19,604 face tests |
| Durable LandXML creation | 18.020–18.311 ms / 53.650–54.518 MiB/s |
| LandXML bytes | 1,030,118 B |
| Descriptor accounted peak working bytes | 135,790,592 B |
| Descriptor retained Surface bytes | 1,034,176 B |
| Descriptor topology steps | 521,494 |
| QA accounted peak working bytes | 336 B |
| Evidence record machine | `jjaes-MacBook-Pro.local` (`macos`/`aarch64`) |
| One-shot Derivation / QA / LandXML | 13,371 / 125 / 14,656 us |
| Observed worker heap | unclaimed (`worker_heap_measurement: null`) |

Only the completed 10,000-Point generated run is claimed. The benchmark also
supports 100,000 and 1,000,000 generated Points, but no result at those scales
is inferred here. One-shot values are reported separately from Criterion
intervals. Licensed production, above-500-million-Point, partner,
downstream-application, paid-use, and human-time evidence remains outstanding.

## Delivery slices

Implementation was delivered in four coherent slices:

1. exact `Snapshot::point_rows`, its limits, terminal summary, and root/overlay
   public tests;
2. `point-terrain` values, one-worker deterministic unconstrained TIN,
   degeneracy/resource fixtures, and benchmark;
3. detached Check Point evaluation and private LandXML create-new encoder with
   independent semantic parser tests; and
4. `terrain-demo`, LAS/LAZ end-to-end correction/Revert coverage,
   documentation, and full local verification.

## Explicitly out of scope

v0.6 does not add:

- Terrain Surface persistence, reopen, resume, migration, cache, or Workspace
  ownership;
- Breaklines, constrained triangulation, boundary polygons, holes, islands,
  Steiner vertices, vertical faces, walls, or overhangs;
- Profiles, contours, slopes, volumes, grids, hydrology, smoothing, thinning,
  decimation, or automatic tiling;
- Source-Point residual Queries, residual rasters, QA dashboards, or general
  report serialization;
- a provisional ground classifier, confidence values, or automatic correction;
- position or non-classification Attribute Edits, Source rewrite, or general
  Edit grammar;
- screen, polygon, corridor, frustum, visible-only, or occlusion selection;
- multiple Sources, COPC, remote storage, networking, or multi-process terrain;
- CRS transformation, datum conversion, geoid handling, unit conversion, axis
  inference, or support for non-metre LandXML coordinates;
- general LandXML, LandXML import, application extensions, or overwrite/update
  export;
- downstream Civil 3D, Bentley, or other application compatibility claims;
- a desktop UI, rendering integration, bindings, autosave, or collaboration;
  or
- licensed-production, above-500-million-Point, partner, paid-use, or human-
  workflow evidence.

Future milestones may add only the subset earned by real caller and external
evidence. This design does not scaffold those seams in advance.
