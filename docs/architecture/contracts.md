# Cross-Module Contracts and Invariants

Status: v0.1 through the narrow v0.7 technical-readiness contracts implemented;
broader terrain/export contracts remain deferred

The versioned designs in [`docs/design`](../design) control exact release
scope. This document summarizes the invariants that cross current crate seams.

## Contract design rules

- Public values describe domain meaning, not storage layout.
- Every Source-scale operation is bounded, cancellable where safe, and complete
  or an explicit error.
- Persisted identities and format versions are checked before publication.
- Display Coverage never substitutes for an exact result.
- Source bytes are immutable; Workspace Edits are overlays.
- Durable certainty is reported conservatively.
- A Workflow checkpoint records a revalidated durable fact, never authority
  independent of the module that owns that fact.

## Identity

### Source Identity

`SourceId` identifies one immutable ordered Source. It binds Point ordering,
metadata, position transform, Attributes, and content verification. Moving a
file does not change its identity; changing or re-encoding content does.

Fast and Full verification policies may differ in cost, but neither may return
Point values from content that contradicts the accepted Source Record.

### Point Identity

One Point is identified by `(SourceId, zero-based ordinal)`. Index rebuilding,
read batching, LAZ chunk layout, Point Set spill, Revision depth, View LOD, and
GPU packing cannot change it. Display-node-local offsets are never durable
Point Identity.

### Workspace Identity

`WorkspaceId` is a nonzero opaque 16-byte lineage value generated at create
time and stored in the manifest. Moving the Workspace directory preserves it.
Two independently created Workspaces over the same Source intentionally have
different identities and root Revisions.

### Revision Identity

`RevisionId` is a nonzero opaque 32-byte tagged hash. It binds Workspace
lineage, parent, sequence, Operation Identity, canonical request digest, and
delta digest. Parent and sequence define linear history; byte ordering does
not.

The root Revision represents unmodified Source classification. Every later
Revision is immutable and has one parent. Historical Snapshots remain
addressable after later commits and reopen.

### Operation Identity

`OperationId` is a caller-owned nonzero opaque 16-byte durable identity for one
canonical commit intent. It is not a runtime `JobId`. The caller retains
`(WorkspaceId, OperationId)` before starting the commit and reuses it for
reconciliation or retry.

One Operation Identity can bind only one canonical intent. Reusing it with
different content is a definitive `OperationConflict`; matching reuse is
idempotent and can publish at most one Revision.

### Workflow Run Identity

The `terrain-demo` Run Identity is a caller-owned nonzero opaque 16-byte value
for one complete durable Workflow intent. It is distinct from Workspace
Identity, Workspace Operation Identity, and runtime Job Identity. The first
`Intent` frame binds it to the canonical request and Source, index, Workspace,
and Run-root path hashes. Start or resume with different meaning fails before
Workflow mutation.

## Coordinates and exact values

Canonical Source positions retain integer ticks plus the declared finite scale
and offset. Exact Workspace box selection evaluates finite `f64` world
coordinates and includes all six boundaries. The final Source predicate removes
conservative index false positives.

Render batches use a finite `f64` origin plus finite `f32` relative positions.
Those display values are not authoritative input to exact selection, Edit, or
Terrain Derivation. Terrain consumes exact Source ticks plus the verified
position transform through `Snapshot::point_rows`.

Coordinate Reference may explicitly be unknown. No module guesses a CRS,
vertical reference, axis order, or units.

## Source contract

`point-source` exposes one verified opaque `Source` with immutable metadata and
bounded read operations. A read request contains normalized half-open Source
spans, projection, and an explicit `ReadBudget`. Batches preserve ascending
Source order and finish with one exact summary. Cancellation, corruption,
changed input, unsupported format, and resource exhaustion are errors rather
than partial success.

Implemented adapters are:

- `source-memory` for deterministic conformance and fault fixtures; and
- `source-las` for LAS point-data record formats 0–10 and LAZ formats 0–8.

LAZ formats 9 and 10 fail explicitly pending exact layered WavePacket14 codec
support. COPC and remote reads remain deferred.

## Spatial Index contract

`point-index::prepare` consumes one verified `Source`, one target, and
`PrepareLimits`. It returns a complete `PreparedIndex` by opening a compatible
artifact or deterministically building/resuming one. A complete artifact is
checksummed, Source-bound, synced, and published without replacement.

`PreparedIndex` retains the verified Source and exposes it by shared reference.
Its candidate plan is a complete, sorted, disjoint set of Source spans that may
contain false positives but must not omit any exact world-box match.

Internal-node samples provide bounded display Coverage only. Complete leaf
reads come from the retained Source. Neither samples nor hierarchy membership
define Point Identity or exact Workspace Query results.

## Workspace contract

`point-workspace` accepts exactly one complete `PreparedIndex`. Supplying the
index rather than separate Source and index values prevents mismatched
capabilities. Create chooses one Source `AttributeId` whose exact type must be
`U8`; open revalidates that schema and the persisted Source contract.
`Workspace::schema()` returns that validated `WorkspaceSchema` without exposing
private manifest representation. `terrain-demo` requires its
`schema().classification()` to be Source Attribute 6.

The public lifecycle is:

~~~rust,ignore
let workspace = point_workspace::create(
    root,
    index,
    WorkspaceSchema::new(classification_attribute),
    OpenLimits::default(),
).blocking_wait()?;

let reopened = point_workspace::open(
    root,
    reopened_index,
    OpenLimits::default(),
).blocking_wait()?;
~~~

One Workspace, Snapshot, or Point Set handle retains the exclusive local
Workspace lock. A second open fails. Multi-process readers and writers are not
claimed.

### Snapshot and exact selection

A `Snapshot` is pinned to one immutable Revision and exposes exact Workspace,
Source, and Revision provenance. The implemented Query grammar supports:

- `PointQuery::all()`;
- `PointQuery::within(inclusive_world_bounds)`;
- optional `.classification_is(value)` against the effective classification;
  and
- `Snapshot::select_point_ids` for bounded explicit Point Identities.

All/box selection starts with a complete conservative index plan. Explicit
Point IDs are bounded, Source-validated, sorted, deduplicated, and normalized
to spans. Both paths read exact Source positions/classification, apply every
overlay through the pinned Revision, run final predicates, and publish a Point
Set only after complete terminal Source verification.

View samples, GPU picks, visibility, depth, and occlusion never exclude a
Point. v0.7 has no polygon, corridor, frustum, screen-through, brush,
visible-only, or occlusion Query.

### Exact Snapshot Point rows

`Snapshot::point_rows(PointQuery, PointRowLimits)` returns one pull-based
`SnapshotPointBatches` stream. Each nonempty `SnapshotPointBatch` contains one
Source identity, strictly increasing ordinals, exact quantized positions, and
effective `U8` classification values with equal column lengths. Rows reflect
every overlay through the pinned Revision and are identical across Source
batch partitioning.

The stream is provisional until `next()` returns terminal `None` and
`summary()` exposes `SnapshotPointSummary`. That summary binds Snapshot
provenance, normalized Query, candidate and exact counts, ordered Point-ID
hash, and full row-content hash. Error or cancellation is fused and publishes
no summary. Point-row streaming does not materialize a Point Set and exposes no
general Attribute or overlay-storage seam.

### Point Set

A public `PointSet` is an immutable process-scoped collection of ordered unique
Point Identities captured at one Snapshot. Metadata contains provenance, exact
count, a membership-stable Point-ID hash, and a provenance/before-value content
hash.

Private records also retain the effective classification observed during
selection. That value is the authoritative before-value for the first
classification commit.

The Point Set retains bounded records in memory and automatically spills to a
checksummed append-only file when its resident ceiling is crossed. Iteration
through `PointSet::ids(PointIdReadLimits)` is repeatable and bounded. Missing,
changed, truncated, or corrupt spill storage fails before false completion.
The final handle removes the spill. A Point Set is not a durable named
selection or recovery record.

### Classification Revision and Revert

`CommitRequest::set_classification(operation, points, value)` derives its
expected head from Point Set provenance. Changed rows are stored in ascending
ordinal order as `(ordinal, before_u8, after_u8)`; no-op rows are omitted. If
all selected values already equal the request, the Operation is recorded as
`Rejected(NoChanges)` and no Revision is created.

`CommitRequest::revert_head(operation, expected_head)` is accepted only for the
current non-root head. It appends a new child with the immediate head's rows
inverted. It does not move the head backward or erase history. Reverting the
inverse acts as redo.

Position and every non-classification Attribute remain Source-authoritative.
The implementation never rewrites a LAS/LAZ or in-memory Source.

### Commit certainty and reconciliation

The custom `CommitJob` exposes only errors known to precede durable
publication. Once Operation or Revision publication begins, panic, I/O failure,
cancellation, or lost acknowledgement maps conservatively to
`CommitOutcome::Indeterminate`, and that session is mutation-poisoned.

After dropping the entire session graph and reopening, resolution is one of:

- `Committed(CommitReceipt)` after the Revision is validated and directory
  durability is established;
- `Rejected(RecordedRejection)` for a synced immutable rejection;
- `Retryable(Box<RecordedIntent>)` for a complete ready payload with current
  expected head;
- `NotRecorded` when no durable record exists; or
- `Indeterminate(CommitUncertainty)` when safe certainty cannot be proved.

`retry_operation` revalidates and links the complete ready payload. It does not
need the expired Point Set.

### Exact Revision Audit

`Workspace::revision_audit(revision, RevisionAuditLimits)` rebuilds one
immutable `RevisionAudit` without changing Workspace state. The root audit is
canonically empty. A non-root audit validates the complete Revision structure
and digest, streams exact Source positions for every strictly increasing changed
ordinal, and publishes only after all joins and limits succeed.

The audit reports `RevisionInfo`, sorted unique `(before, after, count)`
classification transitions, changed Point count, the inclusive world-space Edit
Footprint, ordered Source-aware Point membership hash, full content hash, and
resource facts. A Revert audit has the same membership and footprint as the
Revision it inverts, with reversed transitions. Historical results do not
change after later commits.

## Persistence contract

The current private Workspace layout is:

~~~text
workspace.pcw/
  manifest.pwm
  workspace.lock
  operations/
    <operation-id>.ready
    <operation-id>.reject
  revisions/
    <sequence>-<revision-id>.pwr
  scratch/
    ...
~~~

Manifest, ready, rejection, and Revision values are immutable, versioned, and
checksummed. Complete values are staged and synced, closed, made read-only,
reopened and revalidated, then hard-linked without replacement. Directory sync
establishes the durable commit point. Recovery validates contiguous linear
history and every published operation record under `OpenLimits`; it fails
closed on gaps, forks, lineage/Source mismatch, duplicate identities, or
corruption.

Recognized incomplete scratch files are disposable. Recovery never opens a
published immutable value for mutation.

## Terrain Surface contract

`point-terrain::derive` owns one immutable `Snapshot`, normalized
`TerrainRecipe`, and `TerrainLimits` for the Job lifetime. It derives its Ground
Input through `Snapshot::point_rows`: every exact row matching the explicit
effective ground classification and optional inclusive bounds becomes one
canonical `SurfaceVertex`. The result does not retain the Workspace session.

The single-worker algorithm sorts exact tick/Point-Identity keys, rejects fewer
than three Points, duplicate XY, conflicting elevation, collinear input,
unsupported numeric ranges, and resource exhaustion, and uses deterministic
robust orientation and in-circle signs. Every `SurfaceFace` is counter-
clockwise, uses three distinct one-based `SurfaceVertexId` values, is
canonically rotated and sorted, and belongs to one unconstrained manifold
Delaunay disk over the convex hull. No partial Surface is published.

`TerrainDescriptor` binds Snapshot provenance, normalized Recipe and hash,
algorithm version, Source transform and Coordinate Reference, input/geometry/
topology/artifact hashes, exact counts and bounds, and accounted resource
facts. Equal Snapshot meaning and Recipe produce equal geometry and topology;
Revision provenance remains distinct even after an Edit and Revert restore the
same effective geometry.

## Detached Check Point QA contract

`TerrainSurface::check_points` accepts finite, uniquely identified detached
observations already expressed in the Surface coordinate system and units.
Closed face boundaries are covered; a point outside the convex hull produces
an explicit `CheckPointOutcome::Gap`. For coverage, residual is observed Z
minus interpolated Surface Z. Results preserve caller order and statistics use
deterministic compensated accumulation. Failure, cancellation, or any limit
breach publishes no partial `CheckPointReport`.

## LandXML export contract

`TerrainSurface::export_landxml` privately encodes one deterministic UTF-8
LandXML 1.2 metric-metre TIN with explicit caller-supplied date/time, one
Surface, consecutive point IDs, and canonical faces. Coordinates are written
as northing, easting, elevation. The caller must establish that Source units
are metres; an unknown Coordinate Reference requires an explicit metric-metre
assertion. No unit or CRS transformation occurs.

Export stages and syncs a bounded sibling file, reopens and verifies it, then
publishes by no-replace hard link and syncs the parent. Before publication,
failure leaves no target. Once publication starts, verification, sync, cleanup,
or terminal-progress failure is conservatively `ExportIndeterminate`; a
`LandXmlReceipt` is returned only after durable completion. The independent
`roxmltree` acceptance parser is test-only and shares no encoder helpers.

`TerrainSurface::ensure_landxml` applies the same deterministic encoding and
publication limits but also reconciles a pre-existing regular target. An exact
length-and-content-hash match returns `ReconciledExisting`; any different,
symlinked, or non-regular target fails without replacement. A raced target is
rechecked through the same exact-existing path. `Created` versus
`ReconciledExisting` describes one attempt; durable Workflow evidence records
only stable `ensured_exact` semantics.

## Durable terrain Workflow contract

`terrain-demo` owns one application-level Run root with fixed recognized files:

~~~text
run-root/
  run.pwf       # checksummed append-only journal
  run.lock      # exclusive process lock
  terrain.xml   # exactly ensured LandXML target
  audit.json    # exactly ensured canonical report
~~~

`start_run` publishes the caller's complete `WorkflowRunIntent` before Point
selection or commit. `resume_run` requires identical paths and intent, resolves
the same Workspace Operation Identity, recomputes immutable work, and appends or
validates exactly these monotonic frames: `Intent`, `RevisionResolved`,
`AuditObserved`, `SurfaceObserved`, `QaObserved`, `ExportEnsured`,
`ReportEnsured`, and `Complete`. `inspect_run` verifies the journal format, hash
chain, semantic links, and Run lock without opening external workflow state;
it may durably repair a torn final suffix to the last verified frame. It then
revalidates Run-root identity. Replacement after repair is
`PWF_PUBLICATION_INDETERMINATE` at `inspect` with publication phase
`journal-checkpoint`, never false inspection success.

The caller must create the Workspace separately and bind the intent to its
current `workspace.head().provenance().revision()`. Its selected `U8` Attribute
must be Source Attribute 6, the `source-las` classification column.
Start/resume open only; an absent Workspace is `PWF_INVALID_REQUEST` before Run
creation or Workspace mutation.

A synced frame is only a checkpoint. Resume revalidates it against Workspace
state, recomputed Audit/Terrain/QA meaning, or exact XML/report bytes. Torn final
suffixes can be repaired to the last complete frame; an invalid complete frame,
gap, reordering, wrong version, path mismatch, or semantic mismatch fails
closed. A classification Revision remains committed if a later phase fails.

`audit.json` uses fixed UTF-8 key order and includes exact identities and
request hashes, a named source-independent semantic-results hash, Revision
Audit/Edit Footprint, baseline and changed Terrain facts, conservative Surface
Change Envelope, QA, stable `ensured_exact` LandXML facts, all semantic limit
facts, and explicit partner/downstream/human-acceptance nonclaims. Machine and
elapsed-time observations do not enter canonical bytes.

## View and renderer contracts

`point-view` is synchronous and renderer-neutral. For one frozen camera,
viewport, hierarchy snapshot, residency snapshot, and budget, it returns
deterministic demanded nodes, prioritized new requests, required retention, and
conditional safe retirements. It performs no I/O.

`render-protocol` validates generation-safe Reset/Upsert/Remove effects.
`render-wgpu` records work into the host's encoder and never owns queue
submission or device polling. A `RecordedFrame` pins exactly the displayed
resources used by asynchronous picking. Picks are provisional Point hints.

## Jobs, cancellation, and progress

Runtime-neutral `Job<T, E>` values implement both `Future` and
`blocking_wait`. Their cloneable handles expose monotonic progress and fused
cancellation. Cancellation is observed only at boundaries where the operation
can still report its durable certainty truthfully.

`Job::blocking_wait_cancelled_by` links a synchronously awaited child directly
to one parent `CancellationToken`. Parent cancellation is then visible to child
control, reporters, and streams without polling or a hidden runtime. Child
cancellation does not cancel the parent, and an uncooperative child retains the
existing detached-worker limitation.

Point and byte progress describe physical work; it does not imply semantic
completion. Exact values are returned only after their terminal validation.

## Resource contract

Every zero limit permits zero use. Capacity is checked before allocation or
publication. Separate ledgers cover:

- Source batches, payload, decoder work, spans, and total Points;
- index work/artifact bytes, resident metadata, candidates, and node reads;
- Workspace persisted counts/bytes, metadata, checksum buffers, and work;
- selection candidates, input IDs, output Points, overlays, resident records,
  working memory, and cumulative spill bytes;
- Point-row candidate facts, Source batches, overlays, emitted rows, batch
  payload, working memory, and total rows;
- Point-ID count, batch payload, read buffer, and working memory;
- commit selected/changed Points, input frames, block sizes, work, temporary
  bytes, Revision bytes, and total durable bytes;
- Ground Input rows, vertices/faces, topology work, overlapping working
  allocations, and retained Surface bytes;
- detached Check Point inputs/results, location work, and report bytes;
- LandXML vertices/faces, output/staging/token/buffer bytes, and publication
  work; and
- Workflow intent counts, journal/frame/path bytes, Revision Audit, Surface
  Change Envelope, canonical report output/staging/buffer bytes, and combined
  live orchestrator working bytes.

Temporary and durable storage are distinct. Overlapping old/new allocations
are charged together. An indivisible block that cannot fit fails explicitly.

## Errors and diagnostics

Errors distinguish invalid caller input, unsupported format or geometry,
incompatible or changed Source, corruption, resource exhaustion, cancellation,
lock/target conflict, I/O, prepublication runtime failure, poisoned mutation
capability, and indeterminate durable certainty. Diagnostics are bounded and
never embed unbounded external payloads. A Check Point gap is a successful
explicit outcome, not an error.

`WorkflowFailure` additionally exposes a stable `PWF_*` code, Workflow stage,
certainty category, optional indeterminate publication phase, every known
Run/Source/Workspace/Operation/Revision identity, and exactly one recovery
action. The CLI prints the same bounded structured information and does not
automatically retry uncertain mutation or replace a conflicting target.

## Deferred contracts

Breaklines, constrained or persisted terrain, general Attribute Point-row
streams, general LandXML/import, persisted migration, multi-Source Workspaces,
remote storage, and screen projection require later accepted designs. Their
vocabulary in the roadmap is not a current public API promise.
