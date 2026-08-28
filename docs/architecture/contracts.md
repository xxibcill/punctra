# Cross-Module Contracts and Invariants

Status: frozen through the completed v0.9 repository trust and version-1
compatibility candidate, with the v0.10 professional inspection View and
repository-verified v0.11 exact-review technical slice plus the v0.12 explicit
spatial-reference and packaging repository slice; v0.13: Complete and
repository-verified for the bounded persistent-terrain slice; field activation,
production-scale accuracy, true out-of-core adoption, independent adoption,
partner validation, and support qualification outstanding; broader terrain,
export, and product contracts remain outstanding; v0.14 bounded exact Terrain
QA and correction-loop slice Complete and repository-verified; v0.15 browser
foundation, v0.16 Range/cache/Worker streaming, v0.17 viewer/exact-Point,
v0.18 packed SDK/React lifecycle, v0.19 exact local qualification, and v0.20
packed-consumer integration contracts Complete and repository-verified; v0.21
private visual-baseline contracts Complete and repository-verified for the
bounded exact local attended lane, while broader browser/device support,
physical-display presentation, independent-human/adopter evidence, improved or
final visual quality, API stability, support qualification, beta, v1, and
release-candidate status remain outstanding

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

One Source carries exactly one Coordinate Reference: explicitly unknown,
bounded opaque WKT, or a complete `SpatialReferenceProfile`. The structured
profile binds horizontal and vertical EPSG identities,
easting/northing/elevation axes, separate linear units, and declaration
provenance. Exact Source scale and offset remain the coordinate precision
contract. Adapters may publish the profile only from complete verified facts;
missing or contradictory facts are never inferred.

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
`prepare_with_recipe` adds one explicit recipe choice. `PositionOnlyV1` is the
unchanged v1 path. `InspectionV1` validates and retains only `U16` intensity,
`U8` classification, and optional all-or-none `U16` RGB values.
`prepare_fresh_with_recipe` applies the same recipe contract only when both
the complete and work paths are absent. It rejects and preserves existing or
racing paths instead of opening or resuming them, so corpus and benchmark cold
build measurements cannot silently include warm-open or recovery work. A
successful build retains its recognized rebuildable work cache rather than
risk deleting a caller replacement through a non-atomic pathname cleanup.

The v0.4 persisted recipe fixes Source blocks at no more than 65,536 Points,
uses a longest-centroid-extent median-split binary BVH with nonzero root-first
node identities, and retains at most 4,096 deterministic exact `(ordinal,
ticks)` samples per internal node. Disk version 1 stores multibyte integers and
persisted `f64` bit patterns little-endian; magic values, Source identities,
and BLAKE3 checksums retain their declared byte order.

Disk/recipe version 2 retains the hierarchy and bottom-k ordinals, adds a
checksummed 32-byte Attribute-profile header extension, and stores exact
42-byte inspection samples. Internal reads require no Source replay; leaf
reads project the bound Attributes in one contiguous Source span. V1 and v2
targets remain mutually incompatible and are never automatically replaced.

`PreparedIndex` retains the verified Source and exposes it by shared reference.
Its candidate plan is a complete, sorted, disjoint set of Source spans that may
contain false positives but must not omit any exact world-box match.

Internal-node samples and optional raw display Attributes provide bounded
display Coverage only. Complete leaf reads come from the retained Source.
Neither samples nor hierarchy membership define Point Identity or exact
Workspace Query results.

Opening verifies deterministic bottom-k sample ordinals against descendant
Source spans without rereading Source Points. Complete index artifacts are
trusted local rebuildable caches: their unkeyed BLAKE3 checksums detect
accidental corruption and concurrent mutation, not adversarial rewriting.
Incompatible or corrupt targets are preserved and rejected. The caller may
move/delete the rebuildable cache family and rebuild from the verified Source.

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

View samples, GPU picks, visibility, depth, and occlusion never exclude a Point
from Workspace Queries. The Workspace grammar itself has no polygon, corridor,
frustum, screen, brush, visible-only, or occlusion Query. v0.11's sibling
`point-review` crate composes `Snapshot::point_rows` with renderer-neutral
Camera/Viewport values to implement one inclusive CPU screen-through rectangle;
it does not change the Workspace Query grammar.

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
The spill is retained as bounded private debris rather than unlinked through a
replaceable pathname. It is ignored by recovery and may be removed only as
offline caller-owned maintenance while no related handle, job, or process is
live. A Point Set is not a durable named selection or recovery record.

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
checksummed. Complete scratch values are staged and synced, made read-only,
reopened and revalidated, then published as independent descriptor-bound
no-replace copies. The ready Operation to Revision path alone intentionally
hard-links one authoritative identity. Directory sync establishes the durable
commit point. Recovery validates contiguous linear history and every published
operation record under `OpenLimits`; it fails closed on gaps, forks,
lineage/Source mismatch, duplicate identities, or corruption.

Recognized incomplete scratch files are per-attempt bounded and ignored by
recovery. They are retained because automatic pathname deletion cannot exclude
a racing replacement; offline cleanup is permitted only with no live
Workspace, Snapshot, Point Set, or job. Recovery never opens a published
immutable value for mutation.

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

The accepted v0.13 `point-terrain::prepare` contract returns one
`TerrainPrepareJob` whose success is `PreparedTerrainSurface`; it requires an
explicit inclusive AOI, `TerrainPrepareLimits`, and one caller-owned target.
Its four attempt dispositions distinguish opening a compatible complete target,
publishing a compatible final stage, resuming topology from compatible input
work, and building when the complete target, stage, and work paths are absent.
Complete verified Ground Input and a complete final Surface stage are the only
durable resume checkpoints. Sorting and the existing canonical triangulator may
rerun after input recovery, but Source rows are not reread after the verified
input checkpoint.

Surface disk-v1 is an immutable rebuildable Artifact, not Workspace authority
or Workflow Run-v1 state. It binds Snapshot/Source/Workspace/Revision,
Recipe/AOI, transform, spatial reference, algorithm, counts, bounds, and
canonical input/geometry/topology/Artifact hashes. Fixed 4,096-record checksum
blocks protect work input, vertices, and faces; a checksum stored in the footer
covers every byte preceding that footer. Open validates every
binding, block directory, checksum, and structural fact before exposing a
prepared handle, and bounded reads revalidate touched blocks before yielding
canonical vertex/face records.

Publication syncs and verifies an owned stage, creates the target without
replacement, syncs the parent, and revalidates the published path before
acknowledgement. Stale, incompatible, corrupt, symlinked, non-regular, and
racing targets are preserved. Post-commit uncertainty carries the expected
complete-payload/footer checksum for same-request reconciliation. A valid
historical Artifact remains valid for its pinned Snapshot even when the
Workspace head advances.

Successful publication deliberately retains the verified final stage and any
input-work pathname. The implementation holds identity witnesses for files it
verified, but `ResumedPublication` does not inspect an arbitrary work sibling or
make it trusted. No portable unlink can be conditioned on the verified open
inode; a later check-then-unlink could remove a racing replacement. The
complete target takes precedence during a warm open, and siblings may be
removed only as explicit owner-controlled offline maintenance when no related
handle, job, or process is live.

The prepared handle retains bounded metadata and file access, not all vertices
and faces. `SurfaceArtifactDescriptor` is semantic identity;
`TerrainPrepareReport` and its `Built`/`ResumedInput`/`ResumedPublication`/
`Opened` disposition describe only the attempt. `SurfaceReadLimits`
independently bound batch records, decoded payload, checksum buffer, retained
working bytes, and whole-stream work for `SurfaceVertexBatches` and
`SurfaceFaceBatches`. The full-AOI single-worker triangulator still retains the
complete AOI under hard memory
limits; disk persistence is not a true external-memory topology algorithm.

## Detached Check Point QA contract

`TerrainSurface::check_points` accepts finite, uniquely identified detached
observations already expressed in the Surface coordinate system and units.
The Coordinate Reference must be the supported easting/northing/elevation
metre/metre profile; a structured foot profile and every unstructured
reference fail before evaluation.
Closed face boundaries are covered; a point outside the convex hull produces
an explicit `CheckPointOutcome::Gap`. For coverage, residual is observed Z
minus interpolated Surface Z. Results preserve caller order and statistics use
deterministic compensated accumulation. Failure, cancellation, or any limit
breach publishes no partial `CheckPointReport`.

## Exact Snapshot/Surface QA and comparison contract

`TerrainSurface::exact_qa` accepts one Snapshot whose provenance exactly
matches the Surface plus a nonempty combination of one exact Source
`PointQuery`, uniquely identified detached Check Points, and one evenly
stationed profile. `PreparedTerrainSurface::exact_qa` has identical semantic
results after bounded materialization of checksummed disk-v1 records. Both
paths require the complete supported easting/northing/elevation metre profile.

Source and Check Point residual is observed world Z minus interpolated Surface
Z. Lower and upper tolerance magnitudes are finite, nonnegative, metre-valued,
and inclusive. Profiles report exact CPU Surface elevation or an explicit gap
at each declared station; connected visualization between stations is not a
continuous plane/TIN intersection. Reports bind Snapshot, Recipe, spatial
reference, Surface hashes, tolerance, completed Source-row facts, canonical
input/result hashes, every outcome, and resource accounting. A historical
report remains valid for its exact pair. Freshness against a caller-declared
current Snapshot alone makes no Surface claim; freshness against a declared
current Snapshot/Surface pair additionally distinguishes stale Snapshot and
Surface state after an Edit. The five outcomes keep Snapshot-only evidence
distinct from evidence whose current Surface was checked.

`compare_surfaces` requires common Workspace/Source lineage, Recipe, position
transform, and spatial reference. It compares faces by three authoritative
Point Identities rather than Surface-local vertex IDs. Added/removed counts and
hashes are exact; optional changed bounds conservatively enclose every vertex
incident to a changed face and are not an exact change polygon. QA and
comparison limits fail without partial reports, decimation, tolerance changes,
or suppressed gaps.

## LandXML export contract

`TerrainSurface::export_landxml` privately encodes one deterministic UTF-8
LandXML 1.2 metric-metre TIN with explicit caller-supplied date/time, one
Surface, consecutive point IDs, and canonical faces. Coordinates are written
as northing, easting, elevation. A supported structured profile emits one
matching `CoordinateSystem`. An unsupported or unstructured reference fails;
no caller assertion can override the Source contract. No unit or CRS
transformation occurs.

Export stages and syncs a per-attempt bounded sibling file, reopens and
verifies it, then publishes an independent descriptor-bound copy with atomic
no-replace semantics and syncs the parent. The verified macOS path uses
`fclonefileat`; the unverified Linux path copies into an unnamed `O_TMPFILE`
and links that descriptor exactly once. Unsupported filesystems or platforms
fail closed. The separately encoded named stage is retained as recognized
debris because pathname cleanup cannot conditionally unlink the owned open
file. Before publication, failure
leaves no target. Once publication starts, verification, sync, or terminal-
progress failure is conservatively `ExportIndeterminate`; a `LandXmlReceipt`
is returned only after durable completion. Publication retains an open witness
for the published leaf, syncs that destination file before the parent
directory, and revalidates both the open file and target name after directory
sync and terminal progress. Reconciliation retains and revalidates the same
kind of leaf witness through its final acknowledgement boundary. The
independent `roxmltree`
acceptance parser is test-only and shares no encoder helpers.

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
  run.lock      # workflow-exclusive / qualification-shared process lock
  terrain.xml   # exactly ensured LandXML target
  audit.json    # exactly ensured canonical report
~~~

`start_run` publishes the caller's complete `WorkflowRunIntent` before Point
selection or commit. `resume_run` requires identical paths and intent, resolves
the same Workspace Operation Identity, recomputes immutable work, and appends or
validates exactly these monotonic frames: `Intent`, `RevisionResolved`,
`AuditObserved`, `SurfaceObserved`, `QaObserved`, `ExportEnsured`,
`ReportEnsured`, and `Complete`. `inspect_and_repair_run` verifies the journal
format, hash chain, semantic links, and Run lock without opening external
workflow state; it may durably repair a torn final suffix to the last verified
frame. It then
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

## Run-bound interoperability qualification contract

`terrain-demo verify-round-trip` opens exactly one Complete eight-frame Run
without repairing or mutating it. It retains and revalidates stable witnesses
for the Run root, `run.pwf`, `terrain.xml`, `audit.json`, the caller-returned
LandXML, and the evidence parent. The Complete checkpoint and canonical report
must agree on Run, request, LandXML, and report facts before evaluation.

The verifier streams both LandXML inputs under the full supported v0.7 export
ceiling and builds only the bounded metric-metre single-TIN semantic model.
Presentation order, identifiers, triangle winding, and supported bounded
metadata do not change meaning. XML, subset, Coordinate-Reference, unit,
Point-count, unique-vertex-mapping, tolerance, and topology outcomes use stable
semantic reason codes. Once both
files are completely witnessed, a supported semantic rejection is a canonical
failed evaluation; inability to witness or completely evaluate an input,
resource exhaustion, cancellation, or changed input is operational failure and
does not become success.

Canonical pass or fail evidence uses schema
`punctra.terrain-demo.landxml-round-trip-evidence.v1` and is published outside
the Run root by exact-existing reconciliation or descriptor-bound no-replace
publication. Different existing bytes are never overwritten. Caller-provided
application, version, and settings are opaque declarations, not proof that a
downstream product ran or accepted the deliverable.

## View and renderer contracts

`point-view` is synchronous and renderer-neutral. For one frozen camera,
viewport, hierarchy snapshot, residency snapshot, and budget, it returns
deterministic demanded nodes, prioritized new requests, required retention, and
conditional safe retirements. It performs no I/O.

`render-protocol` validates generation-safe Reset/Upsert/Remove effects.
`render-wgpu` records work into the host's encoder and never owns queue
submission or device polling. A `RecordedFrame` pins exactly the displayed
resources used by asynchronous picking. Picks are provisional Point hints.
Complete highlight-update input has an independent host-selected count ceiling
and is rejected atomically before duplicate removal.

`point-review` accepts no `PickHit` and therefore cannot validate display
generation. A host first rejects a hit whose `ViewGenerationKey` is not active,
then pins a Snapshot and confirms only the hinted `PointId`. Confirmation
returns exact ticks, transform, world position, effective classification, and a
one-Point Point Set from that Snapshot. A miss is never negative Query evidence.

A `ScreenSelection` binds a normalized finite physical-pixel rectangle to one
validated Camera and Viewport. Its bounds must lie in inclusive
`[0,width] x [0,height]`. Perspective and orthographic projection use f64 CPU
math; near/far, clip, and rectangle boundaries are inclusive. Selection is
screen-through: residency, occlusion, splat size, and depth-test winners do not
alter membership. Only terminal success publishes a complete Point Set.

`CameraProjection` is explicit. Perspective uses vertical field of view;
orthographic uses vertical world height. Orthographic frustum and
screen-space-error calculations are depth-independent, while both modes retain
the same large-world origin, near/far, depth, and Point-identity contracts.

The private `renderer-demo` maps exactly one host-selected mode. Neutral and
elevation require a position-only disk-v1 index; RGB, intensity, and
classification require the matching disk-v2 inspection contract. RGB absence
is an explicit invalid request, not a fallback. Every mapping changes only
RGBA8: identity, position, geometry, generation/version, and Coverage remain
equal across modes.

The browser host owns its canvas, WebGPU device/queue/surface lifecycle,
visibility/DPR policy, Source deployment choice, recovery, and disposal. The
public viewer composes only the accepted lifecycle, camera, streaming,
presentation, provisional-pick, and exact fixture bridge. Range transport,
Worker decoding, cache policy, raw Wasm publication, and qualification remain
private implementation or repository-host policy.

The v0.21 Visual Trial seam is also private and closed over nine checked-in
identifiers. Record mode may create only the canonical baseline PNGs and a
commit-free baseline-input manifest that cross the implementation pin. Final
evidence must come from a later verify-mode run of that exact pinned build.
Rubric input follows capture and visible loading of its exact bound images. The
verify page accepts one nonrepeating query tuple: a full lowercase 40-hex
implementation commit, positive decimal verifier byte length, and lowercase
64-hex verifier SHA-256. It fixes the verifier path and attended-lane identity;
the visible Run control remains disabled for an absent or malformed tuple. The
USTAR bundle preserves repository-relative paths for private transport but is
not itself evidence. Its standard browser Blob download may fall back to an
explicitly enabled same-origin local-server POST only; that fallback is bounded
to the same archive, rejects existing targets rather than overwriting them, and
does not create evidence. Canonical images and feature facts remain presentation-
only and cannot change Source, Point, Coverage, selection, or Query authority.

The host presents demanded nodes, load candidates, actually issued work,
retention/retirement, queue/staging, requested/resident nodes, and Sampled
versus Complete resident Coverage as separate facts. Pausing issues no new
requests. No loading or resident state is called Query completion.

The local corpus manifest is bounded, rejects unknown fields, and requires
explicit inspection and measurement permission per entry. The no-replace
Viewing Report records only observed viewing operations, effective limits, and
explicit false nonclaims. It omits private paths and project/firm identifiers;
Source identity and machine facts remain caller-controlled sensitive data.
Manifest-supplied feature outcomes, project/firm counts, mode/projection
matrix facts, and lane configuration are serialized only under explicit
`declared_*` names. A false nonclaim records that declared feature outcomes
were not verified by viewing operations.

## Jobs, cancellation, and progress

Runtime-neutral `Job<T, E>` values implement both `Future` and
`blocking_wait`. Their cloneable handles expose monotonic progress and fused
cancellation. Cancellation is observed only at boundaries where the operation
can still report its durable certainty truthfully.

`Job::blocking_wait_cancelled_by` links a synchronously awaited child directly
to one parent `CancellationToken`. Pull-based compositions use
`CancellationToken::link_to_parent` and retain its scoped `CancellationLink`
while the child stream is active. Parent cancellation is then visible to child
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
- review row-stream limits, accepted match count, retained identity bytes, and
  terminal Point Set limits;
- commit selected/changed Points, input frames, block sizes, work, temporary
  bytes, Revision bytes, and total durable bytes;
- Ground Input rows, vertices/faces, topology work, overlapping working
  allocations, and retained Surface bytes;
- persistent Terrain Point-row/input counts, full-AOI triangulation memory,
  work/checkpoint/stage/Artifact bytes, checksum/read buffers, prepared-handle
  metadata, stream records/payload/work, and cumulative temporary bytes;
- detached Check Point inputs/results, location work, and report bytes;
- LandXML vertices/faces, output/staging/token/buffer bytes, and publication
  work; and
- Workflow intent counts, journal/frame/path bytes, Revision Audit, Surface
  Change Envelope, canonical report output/staging/buffer bytes, and combined
  live orchestrator working bytes; and
- render highlight-update input independently of resident points, batches, and
  bytes; and
- renderer-demo hierarchy, request queue, staging, renderer residency, corpus
  manifest/report, navigation-trace, index temporary disk, and Source
  verification measurement limits; and
- browser visual renderer/canvas residency, capture/readback, canonical pixels,
  PNG encoding, comparison, evidence JSON, baseline-input manifest, and private
  USTAR transport limits; and
- qualification XML input, node, text/attribute, semantic-model, comparison,
  evidence output/staging/buffer, and retained-witness bytes.

Round-Trip qualification separately accounts parser file/nodes/text,
Point/face/comparison, evidence output, and publication-buffer ceilings.
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

The private LandXML comparator retains the `PRT_INVALID_INPUT`,
`PRT_RESOURCE_LIMIT`, and `PRT_SEMANTIC_MISMATCH` families. Run-bound semantic
evidence adds stable unit, Point-count, unmatched/ambiguous vertex, tolerance,
and topology reason codes without treating caller declarations as observed
application execution.

`renderer-demo` failures expose one stable `PVIEW_*` code, one owning phase,
bounded detail, and exactly one safe recovery action. Source, index,
resource/cancellation, GPU, I/O, request, and internal failures remain distinct.

Persistent Terrain errors additionally distinguish invalid or absent AOI,
corrupt work, corrupt Artifact, unsupported disk/work version, stale binding,
existing/conflicting target, and indeterminate Surface publication. None
returns a partial prepared handle or silently rebuilds a valid mismatched file.

## Deferred contracts

Breaklines, constrained, tiled, true out-of-core, parallel, distributed, or GPU
terrain, general Attribute Point-row streams, general LandXML/import, migration
beyond explicit rebuild, multi-Source Workspaces, remote storage,
polygon/brush/visible-only selection, and general Attribute or position
correction require later accepted designs. Their vocabulary in the roadmap is
not a current public API promise.
