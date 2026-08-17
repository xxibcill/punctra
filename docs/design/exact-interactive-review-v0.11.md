# Exact Interactive Review and Ground Correction Design (v0.11)

Status: **Accepted and Complete — repository-verified technical slice; field
activation evidence and every external product gate remain outstanding**

This design is authoritative for the narrow Punctra v0.11 repository slice.
Its exact base is commit `30ea9ff`, the canonical `0.10.0-alpha.1` tree. The
user instruction on 2026-08-13 accepted this bounded design and activated
repository implementation. That instruction is authority to implement this
technical slice; it is not evidence that v0.10 field observation found
interactive inspection or classification correction to be a material source
of attended time or rework. The ROADMAP activation evidence is still absent
and must remain reported as outstanding.

Repository completion and evidence maturity are independent. Generated
fixtures, maintainer-operated examples, local GPU tests, and a compileable
third-party-style host prove repository behavior only. They do not establish
licensed production-data behavior, professional preference, independent
adoption, paid use, reduced rework, interoperability, partner acceptance, or
support qualification.

## Outcome

Connect one progressive renderer View to CPU-authoritative inspection and one
reversible effective-classification correction path without making sampled GPU
data authoritative.

The accepted vertical slice is:

1. treat one `render-wgpu::PickHit` as a provisional display hint;
2. confirm its `PointId` against one pinned `point_workspace::Snapshot` and
   return exact Source ticks, world position, effective classification, and a
   one-Point `PointSet`;
3. materialize an inclusive screen-through rectangle as an exact spillable
   `PointSet` by scanning every Source Point on the CPU, optionally filtering
   by effective classification at that same Snapshot;
4. derive the complete renderer highlight set only by bounded iteration of the
   resulting `PointSet`;
5. commit one existing durable `SetClassification` request for ground
   correction, inspect its Revision Audit and Edit Footprint, and support only
   the existing immediate-head Revert and Operation reconciliation rules; and
6. demonstrate the public composition from a minimal host-owned renderer
   example that imports no `renderer-demo` private state.

The deletion test is specific. Removing v0.11 would remove the public exact
screen-selection and one-Point confirmation seams, the exact-review example,
and their documentation and tests. Existing Source, Spatial Index, Workspace
classification persistence, renderer updates, GPU picking, terrain, QA, and
export behavior would remain. The slice therefore deepens existing modules
instead of introducing a second document, renderer, or workflow framework.

## Evidence state

| Evidence | State at repository completion |
|---|---|
| Accepted bounded repository design | Present |
| Explicit user activation of repository implementation | Present — 2026-08-13 |
| Canonical v0.10 base | Present — `30ea9ff` |
| Complete one-commit repository qualification | Present — 2026-08-13 |
| v0.10 observation that correction materially costs time or rework | Outstanding |
| Permitted production Source and observed professional correction workflow | Outstanding |
| Independent third-party adopter using the renderer example | Outstanding |
| Partner tolerance, accepted-deliverable, or paid-use evidence | Outstanding |
| Inherited v0.9 complete one-commit candidate record | Outstanding |

The accepted repository work is **Repository-verified** because every local
gate in this design passed from one commit. It must not be described as
Field-qualified, Partner-validated, or Support-qualified without the
corresponding evidence.

## Terms

### Provisional Pick

One `PickHit` produced from a particular `RecordedFrame`. It identifies the
View generation, resident batch, batch version, and sampled Point Identity
whose rasterized splat won the GPU depth test. It is disposable presentation
metadata. It does not prove that the Point belongs to a Workspace Snapshot or
that any sampled position or Attribute is authoritative.

### Confirmed Point

One exact row read by canonical `PointId` from a pinned Snapshot. It contains
the Snapshot provenance, exact signed Source ticks and transform, decoded
finite `f64` world position, effective classification at that Revision, and a
one-Point process-scoped Point Set with the same provenance and before-value.
A Confirmed Point is authoritative only for that immutable Snapshot.

### Screen Rectangle

Two finite physical-pixel-space endpoints captured with one `Viewport`. The
endpoints are normalized componentwise to inclusive minimum and maximum
bounds. Coordinates use a top-left origin and pixel-edge units. The valid
domain is `0..=viewport.width()` horizontally and
`0..=viewport.height()` vertically. A zero-width or zero-height rectangle is
valid and matches only exact projected centers on that line or point.

### Screen-through Selection

The complete ordered set of Points whose exact CPU-projected centers lie
inside the Screen Rectangle and the Camera clip volume, regardless of GPU
residency, sample LOD, splat radius, transparency, or occlusion. "Through"
means that every matching depth between the inclusive near and far planes is
eligible; it does not mean Points behind the Camera or outside its clip range.

### Ground Correction

One existing Workspace `SetClassification` Revision applied to an exact Point
Set. The reference workflow uses effective classification `2` for Ground. A
host that removes Points from Ground must supply its non-Ground classification
explicitly; Punctra does not guess a replacement value or assign universal
semantics to other `u8` values. This is not a new Edit grammar.

## Accepted scope

The repository implementation is limited to:

- CPU confirmation of exactly one provisional `PickHit` identity against one
  pinned Snapshot;
- perspective and orthographic screen projection using
  `render_protocol::Camera` and `render_protocol::Viewport`;
- one inclusive axis-aligned Screen Rectangle in physical pixel-edge space;
- a complete exact CPU scan with an optional equality predicate on the
  Snapshot's effective classification;
- process-scoped resident or spilled Point Sets with existing provenance,
  ordering, hashes, limits, cancellation, and cleanup behavior;
- renderer highlights populated only after complete bounded Point Set identity
  iteration;
- the existing durable classification commit, immediate-head Revert, Revision
  Audit, Edit Footprint, and Operation-resolution interfaces;
- one minimal `render-wgpu` example built only from public library interfaces;
- focused rustdoc and repository documentation; and
- Cargo version `0.11.0-alpha.1`, interface tests, local benchmarks where
  relevant, and required local CPU/GPU verification.

## Explicit non-goals

v0.11 does **not** add or imply:

- polygon, lasso, fence, brush, flood, radius, or freehand selection;
- occlusion-aware, visible-only, front-most-only, or splat-coverage selection;
- GPU-compute selection, GPU readback of bulk identities, or selection from
  resident LOD samples;
- arbitrary Attribute editing, position editing, Source rewriting, or a
  generalized Edit command system;
- a general desktop UI, window/event-loop framework, command palette, undo
  stack, selection algebra, persistent named Point Set, or multi-document host;
- Workspace branching, merge, multi-writer operation, compaction, or a new
  persisted format;
- automatic retry, automatic Revert, automatic stale-selection replay, or an
  automatic recovery policy;
- selection acceleration, latency guarantees, or an assertion that a full
  Source scan fits every viewing-scale dataset;
- a second renderer backend, plugin interface, custom shader interface, or
  `renderer-demo` public-state extraction;
- changed-region per-Point overlays derived from an Audit hash alone; or
- any external product, professional, adoption, interoperability, or support
  claim.

## Module ownership and dependency direction

Existing deep modules retain their responsibilities:

- `point-contracts` continues to own exact Source-aware `PointId`, signed
  position ticks, transforms, and finite world-coordinate values;
- `render-protocol` continues to own renderer-neutral `Camera`, `Viewport`,
  View generation, batch, update, and highlight contracts;
- `render-wgpu` continues to own disposable rasterization and asynchronous
  provisional picking while the host owns the wgpu instance, adapter, device,
  queue, command encoder, render target, submission, and polling;
- `point-workspace` remains headless and owns exact Snapshot rows, effective-
  classification lookup, Point Set construction/spill, and durable Edit state;
- `point-review` is the narrow renderer-neutral composition above
  `point-workspace` and `render-protocol`. It owns exact projection evaluation,
  screen/pick confirmation facts, and review-specific limits and failures. It
  must not depend on `render-wgpu` or accept `PickHit` directly;
- private `renderer-demo` may compose the new interfaces but does not define
  them and is not a dependency of the public example; and
- the third-party-style example lives with `render-wgpu` examples, uses only
  public interfaces, and owns its integration policy locally.

This one-way dependency is deliberate: `Camera` and `Viewport` are
renderer-neutral validated values, whereas `PickHit`, `RecordedFrame`, wgpu
resources, and host state never cross the review seam. The private renderer
host is the first real caller of the deliberately small `point-review`
composition, while its public interface tests establish the renderer-neutral
contract independently. The renderer-only public example proves a different
boundary: `render-wgpu` remains usable without Workspace or review
dependencies. A general callback predicate, public Point Set builder, or
renderer-to-Workspace adapter trait would expose more interface than this
operation needs and is not accepted.

## Public interface shape

The implementation adds a small `point-review` interface with these
semantics; exact Rust naming may follow repository conventions without
broadening it:

1. A validated, copyable `ScreenSelection` contains a `Camera`, `Viewport`, a
   normalized finite `ScreenRect`, and an optional effective-classification
   equality value. Construction rejects an endpoint outside the captured
   Viewport instead of clipping it silently.
2. `point_review::screen_through(&Snapshot, selection, ScreenReviewLimits)` starts a
   cancellable review Job. It returns only a complete Point Set plus terminal
   review facts or a terminal failure; no partial membership escapes.
3. `point_review::confirm_pick(&Snapshot, PointId, ScreenReviewLimits)` starts one
   bounded exact confirmation Job. Success returns a `ConfirmedPoint` and its
   one-Point Point Set. A foreign Source identity, impossible ordinal, changed
   Source, or unavailable exact row is an explicit failure rather than an
   empty or guessed confirmation.
4. `ConfirmedPoint` exposes Snapshot provenance, `PointId`, exact ticks and
   transform, decoded world position, effective classification, and a borrow
   or clone of its Point Set. It exposes no renderer batch or GPU value.

`point-workspace` adds only a bounded Point Set entry reader exposing the exact
effective before-value already retained in its process-scoped Point Set. The
existing `PointSet::ids(PointIdReadLimits)` remains the sole accepted path
from an exact selection to renderer highlights. The existing
`CommitRequest::set_classification`, `CommitRequest::revert_head`,
`Workspace::revision_audit`, and `Workspace::resolve_operation` interfaces
remain the mutation, audit, and reconciliation seams. No second commit facade
or durable interactive-workflow journal is added.

## Canonical projection and boundary semantics

The selection algorithm is version 1 of this repository interface. For each
Source Point in ascending ordinal order it performs the following operations
on the CPU:

1. Read the exact signed position ticks and verified `PositionTransform` from
   the immutable Source, then decode the canonical finite `f64` world position.
2. Subtract `Camera::eye()` componentwise in `f64` and take ordered `f64` dot
   products with the Camera's canonical right, up, and forward basis. Call the
   results `x`, `y`, and positive-forward depth `z`.
3. Convert the Camera's validated `f32` near distance, far distance, and
   perspective field of view to `f64`. Compute aspect ratio as the exact `f64`
   quotient of the integer Viewport width and height, not by reusing a rounded
   display matrix.
4. Reject the Point as outside, without error, when `z` is less than the near
   distance or greater than the far distance. Both clip planes are inclusive.
5. For perspective projection, let
   `t = tan(vertical_field_of_view_radians / 2)`, then compute
   `ndc_x = x / (z * t * aspect)` and `ndc_y = y / (z * t)`. For orthographic
   projection, let `half_height = vertical_world_height / 2` and
   `half_width = half_height * aspect`, then compute
   `ndc_x = x / half_width` and `ndc_y = y / half_height`.
6. Reject the Point as outside when either normalized coordinate is outside
   the inclusive interval `[-1, 1]`.
7. Map the projected center into top-left-origin physical pixel-edge space:
   `screen_x = (ndc_x + 1) * viewport.width() / 2` and
   `screen_y = (1 - ndc_y) * viewport.height() / 2`, with integer dimensions
   converted exactly to `f64`.
8. Include the Point exactly when
   `rectangle.min_x <= screen_x <= rectangle.max_x` and
   `rectangle.min_y <= screen_y <= rectangle.max_y`, and when the optional
   effective-classification equality predicate also matches at the pinned
   Revision.

There is no epsilon, hidden expansion, rounding to a pixel index, or
half-open edge. Shared rectangle boundaries can intentionally select the same
Point in adjacent queries. A full-Viewport rectangle includes exact side-plane
and near/far-plane centers. Membership tests the mathematical Point center,
not a renderer splat's radius or alpha.

All finite-input arithmetic is checked. If world-minus-eye, basis projection,
perspective scale, normalized coordinates, or pixel coordinates becomes NaN or
infinite, the complete Job fails with a bounded unsupported-projection
diagnostic naming the stable stage and Point Identity. It must not silently
skip that Point. The sequential CPU oracle uses the same ordered formulas and
comparisons.

"Exact" means complete Source membership, exact ticks, pinned effective
classification, canonical Point Identity, deterministic ordering, and the
specified CPU projection algorithm. It does not mean rational or arbitrary-
precision geometry, and it does not promise bit-for-bit agreement with GPU
rasterization. GPU sample positions, `f32` shader arithmetic, splat coverage,
depth, and edge rules remain provisional presentation behavior.

## Full-scan selection semantics

`screen_through` evaluates every Source ordinal exactly once. It does not use
Spatial Index bounds, current View nodes, resident batches, a Pick table, or a
GPU depth buffer to remove candidates. The complete Source span is charged to
the existing cumulative candidate and Source-read ceilings. Exact overlay
blocks are applied through the pinned Revision before the optional
classification predicate is evaluated.

Matches enter the existing Point Set builder in ascending Source ordinal
order. Resident and forced-spill executions must produce identical ordered
identities, exact counts, membership hashes, content hashes, effective before-
values, and provenance. An empty successful selection is a complete empty
Point Set. Cancellation, Source failure, overlay failure, spill failure, or a
resource limit destroys unpublished state and returns no partial Point Set.

The selection's `SnapshotProvenance` captures Workspace, Source, and Revision.
The `ScreenSelection` captures Camera, Viewport, rectangle, and filter by value at
Job creation. Later camera motion or head changes cannot change an in-flight
result. A host may label the completed result stale relative to its current
View, but it must not relabel or mutate the Point Set's captured provenance.

## Provisional pick confirmation

The host captures the exact `RecordedFrame`, View generation, active Camera and
Viewport, and pinned Snapshot provenance when it encodes a `PickRequest`. It
submits the host-owned command buffer, drives normal device polling, and treats
`PickPoll::Ready(Some(hit))` only as a candidate identity.

Before confirmation the host must reject a result whose View generation no
longer equals the captured active generation or whose captured Snapshot is no
longer the review state the user is acting on. Batch key and batch version are
retained as display diagnostics; they are not Workspace provenance. A stale
result is discarded without Source access, selection, highlight, or mutation.

For a current result the host passes only `hit.point()` to
`point_review::confirm_pick`. The Workspace checks Source identity and ordinal,
reads the exact Source row, applies overlays through the pinned Revision, and
returns the Confirmed Point plus its exact one-Point Point Set. It does not
confirm the sampled GPU position, color, depth, splat radius, or batch payload.
No `PickHit` can directly authorize a commit or a `SetHighlights` update.

## Point Set-derived highlighting

Every v0.11 highlight update follows one atomic host sequence:

1. verify that the Point Set Source and captured review provenance belong to
   the active View policy;
2. check `PointSetMetadata::exact_count()` against an explicit host maximum
   highlight count and checked retained-vector byte ceiling;
3. create `PointSet::ids(PointIdReadLimits)` with total, batch, payload,
   read-buffer, and working-memory limits consistent with those host ceilings;
4. consume the stream completely into a checked vector, retaining its
   ascending unique order; and
5. only after terminal success apply one complete
   `RenderUpdate::SetHighlights` for the active View generation.

The host must not seed, supplement, intersect, or replace that vector with
Pick tokens, resident batch contents, display samples, Audit hashes, or
privately cached ordinals. A one-Point confirmation still iterates its
one-Point Point Set. If iteration, allocation, provenance validation, or a
limit fails, the renderer receives no update and its prior highlight state is
left unchanged. Applying a successfully completed empty Point Set explicitly
clears highlights.

Highlights remain identity overlays: selected Points that are not resident do
not become visible merely because they are selected, while later resident
batches with matching identities receive the same accepted highlight state.
This is not visible-only selection and does not make the renderer a selection
authority.

## Durable correction, Revert, audit, and reconciliation

The reference flow retains the existing Workspace contracts:

1. Generate and retain a nonzero caller-owned `OperationId` before mutation.
2. Submit `CommitRequest::set_classification(operation, point_set, value)` with
   explicit `CommitLimits`. The Point Set provenance is the expected parent;
   if another Revision became head, the definitive result is
   `CommitRejection::StaleHead`. The host does not silently rerun the Screen
   Query or transplant membership to the new head.
3. On `Committed`, pin the returned Revision, run
   `Workspace::revision_audit` with explicit `RevisionAuditLimits`, and expose
   the exact transition counts, Point Identity hash, content hash, and Edit
   Footprint. Audit facts are metadata; they do not independently construct a
   renderer highlight set.
4. A user-requested undo creates a new `OperationId` and submits only
   `CommitRequest::revert_head(operation, expected_head)`, where
   `expected_head` is the exact Revision being reverted. Revert is rejected if
   it is no longer the immediate head. There is no arbitrary history undo.
5. Audit the Revert Revision independently and verify the inverse transition,
   restored footprint, lineage, and hashes.

The Workspace remains single-writer and the Source remains byte-for-byte
unchanged. Point Identity must survive provisional hit, confirmation, Point
Set resident/spill storage, highlight iteration, commit, audit, Revert, drop,
reopen, and exact reselection.

`CommitOutcome::Indeterminate` poisons the live Workspace exactly as today.
The host retains the original Operation Identity and canonical intent, drops
all Workspace/Snapshot/PointSet handles, reopens with explicit `OpenLimits`,
and calls `resolve_operation` for that same identity. It reports the exact
`Committed`, `Rejected`, `Retryable`, `NotRecorded`, or `Indeterminate`
resolution. A `Retryable` state may be resumed only by an explicit caller
decision using the existing recorded intent. v0.11 adds no automatic retry,
automatic Revert, guessed success, new Operation Identity, or directory-wide
cleanup.

## Stale-state rules

Staleness is explicit and asymmetric:

- a PickHit from a non-active View generation is disposable and is discarded;
- a completed selection whose Camera no longer matches the displayed Camera
  remains exact for its captured query, but the host labels it stale and does
  not silently present it as the current rectangle;
- a Point Set from a historical Snapshot remains readable and highlightable
  when the host explicitly presents that provenance, but cannot commit over a
  newer head;
- a stale classification commit is a durable definitive rejection with
  expected and actual Revision identities;
- an immediate-head Revert request becomes stale as soon as another Revision
  becomes head; and
- an indeterminate publication is not called stale or failed. It requires
  reopen and Operation reconciliation.

Repinning means obtaining a new immutable Snapshot and explicitly rerunning
confirmation or selection. No result is updated in place.

## Limits and failure model

All work is local, bounded, cancellable before mutation, and fail-closed.

Screen selection and confirmation reuse `PointSetLimits` rather than adding an
unbounded interactive default. The complete operation cumulatively charges:

- complete candidate span and Point counts;
- Source spans, batches, Points, payload, decoder allowance, and read work;
- input Point identities for one-Point confirmation;
- Revision overlay segments and bytes;
- exact output Points;
- combined peak selection/projection/builder working bytes;
- resident Point Set bytes; and
- cumulative spill bytes.

`PointIdReadLimits` separately bounds highlight identity iteration. The example
adds explicit host caps for the final retained identity vector because a
bounded stream does not itself bound a caller's accumulation. `CommitLimits`,
`RevisionAuditLimits`, `OpenLimits`, render residency limits, Pick token space,
readback bytes, and GPU device limits remain independently enforced; one
budget never disguises another.

Expected terminal failures include invalid rectangle coordinates, foreign or
impossible Point Identity, unsupported projection arithmetic, Source or Index
mismatch, overlay or spill corruption, allocation/resource exhaustion,
cooperative cancellation, stale head, no changes, immediate-head Revert
rejection, Operation conflict, and indeterminate publication. Each returns a
structured existing error or one narrow bounded screen-projection category.
Diagnostics do not include unbounded paths, Source data, or private host state.

A GPU or Pick failure occurs before Workspace mutation. A selection or
highlight failure cannot publish a Revision. A prepublication commit failure
publishes no Revision; a post-publication uncertainty is never collapsed into
an error and always follows Operation reconciliation.

## Third-party renderer example and rustdoc

Add one minimal example at
`crates/render-wgpu/examples/third_party_host.rs`. It must not import
`renderer-demo`, copy its private scene/corpus state, own a window loop, or
claim to be an application template. Using one deterministic display point,
public renderer crates, and an offscreen target, it demonstrates:

- host creation and ownership of wgpu instance, adapter, device, queue,
  encoder, target, submission, and polling;
- generation reset and one bounded public point-batch upload;
- rendering one `RecordedFrame` and encoding one provisional PickRequest;
- validation of the returned provisional generation and Point Identity; and
- an explicit documented handoff to pinned Snapshot confirmation under
  separately selected Workspace/Point Set limits.

The public `point-review` interface tests and private `renderer-demo` caller
separately exercise exact confirmation, screen-through review, bounded Point
Set iteration, highlight application, and the existing commit/Audit/Revert
interfaces. Keeping those operations out of the renderer example proves that
the public renderer has no Workspace dependency. The example uses small
generated display data so its behavior is reproducible and redistributable.
Its successful run is library-integration evidence, not independent adoption
or professional workflow evidence. When a local adapter is expected, its
acceptance command sets `PUNCTRA_REQUIRE_GPU=1` or otherwise fails explicitly
rather than reporting a skipped example as exercised.

Focused crate rustdoc must explain:

- which renderer and wgpu resources the host owns;
- why PickHit metadata is provisional;
- how one pinned Snapshot supplies exact confirmation;
- the complete-scan, inclusive-boundary, and screen-through meanings;
- Point Set provenance and bounded identity iteration;
- stale View versus stale Revision behavior;
- commit certainty and same-Operation reconciliation; and
- every limit family the adopter must choose.

Links run in rustdoc and point directly to the example and owning public types.
Documentation must not imply that `renderer-demo` is a public integration
interface.

## Version, persistence, and documentation

The workspace Cargo version becomes `0.11.0-alpha.1`. v0.11 adds no persisted
Workspace, Revision, Operation, Point Set spill, Spatial Index, Run, report,
or LandXML version. Screen Queries, Confirmed Points, Camera capture, and
Point Sets remain process-scoped. Existing version-1 persisted fixtures must
open with identical identities and semantics.

Update README, CHANGELOG, CONTEXT, ROADMAP status, CONTRIBUTING commands,
architecture module/workflow/testing documents, crate manifests, and public
rustdoc together. Documentation must distinguish:

- sampled rendering from exact selection;
- selection membership from renderer residency;
- a pinned Snapshot from the mutable Workspace head;
- selected Points from Points actually changed by a commit;
- Revert from general undo; and
- repository verification from every outstanding external evidence gate.

## Verification gates

Repository implementation is complete only when all applicable local gates
pass from one exact commit, including the authoritative sequence in
`CONTRIBUTING.md` and these focused additions.

### Projection contract

- Perspective and orthographic fixtures independently calculate every
  projected center from exact ticks.
- Near, far, left, right, top, bottom, rectangle minimum, and rectangle maximum
  boundaries are inclusive; one representable step outside each is excluded.
- Top-left Y inversion, non-square Viewports, one-pixel Viewports, degenerate
  rectangles, signed zero, very large world origins, and extreme finite inputs
  have direct tests.
- A non-finite intermediate fails the complete operation without partial
  membership.

### Exact selection and confirmation

- Every rectangle result matches an independent sequential full-Source CPU
  oracle across varied Source batch sizes, perspective/orthographic Cameras,
  empty and complete Viewports, and optional effective-classification filters.
- Tests prove that occluded, transparent, and nonresident Points still match
  when their exact centers satisfy the query, while Points outside near/far or
  the rectangle do not.
- Resident and forced-spill results have identical membership, ordering,
  provenance, counts, hashes, and before-values.
- Every candidate, Source, overlay, output, working, resident, temporary, and
  highlight-read limit has a focused failure test; cancellation publishes no
  result.
- One-Point confirmation rejects foreign Source, invalid ordinal, stale View
  policy, changed Source, and resource failure, and returns exact ticks,
  transform, world position, effective classification, and Point Set on
  success.

### Identity, highlight, and durable correction

- Point Identity survives PickHit, confirmation, Point Set memory/spill,
  bounded ID iteration, renderer highlight state, commit, audit, Revert,
  handle drop, reopen, and reselection.
- Tests instrument the host composition so no highlight update occurs before
  terminal Point Set iteration and the accepted vector equals exactly the
  Point Set IDs. Failure leaves prior highlights unchanged.
- Stale Point Set commits and stale Reverts return exact expected/actual head
  facts and create no Revision.
- Commit and Revert audits prove transition counts, Point Identity/content
  hashes, Edit Footprints, historical immutability, and Source-byte equality.
- Fault injection covers representative commit publication uncertainty and
  every `OperationResolution` state without automatic retry or guessed result.

### Integration and local qualification

- The third-party-style example compiles without `renderer-demo` and runs on
  the required local GPU adapter.
- Focused rustdoc examples compile with warnings denied.
- Workspace tests cover the new public interface through public seams, not
  private builder shape.
- Existing renderer offscreen, planner, display, Workspace persistence,
  golden-v1, fuzz, benchmark, documentation, formatting, Clippy, test, and GPU
  gates remain green locally.
- The point-review benchmark records full-scan Point count, elapsed time,
  peak accounted working bytes, resident/spill disposition, and temporary
  bytes for generated fixtures. It is not extrapolated to production Sources
  or described as an interaction-latency promise.

No hosted CI is added. Required GPU commands use `PUNCTRA_REQUIRE_GPU=1` so a
missing expected adapter is a failure.

## Completion statement

v0.11 may be marked repository-complete only when its interface, example,
rustdoc, tests, benchmark facts, versioning, and the complete local verification
record agree at one commit. The completion statement must continue to name the
ROADMAP activation evidence, licensed field workflow, independent adoption,
partner validation, and inherited support/candidate records as outstanding
unless separately obtained.

Passing this design proves one bounded technical composition: provisional GPU
identity to pinned CPU confirmation, exact screen-through Point Set, bounded
identity highlight, durable classification Revision, exact Audit, and
immediate-head Revert. It does not prove that this is the right product
workflow, that it saves professional time, or that it is ready for general
production support.

The complete local repository sequence passed on 2026-08-13 from the exact
commit containing this record. That closes only the repository gate. The
activation evidence, permitted field workflow, independent adoption, partner
validation, and inherited support/candidate records in the table above remain
outstanding.
