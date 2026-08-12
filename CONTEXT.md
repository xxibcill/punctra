# Domain Context: Point-Cloud Foundation

Status: renderer, adaptive View planning, Real Sources, Spatial Index/out-of-
core View, the narrow Workspace, and the v0.6 Terrain and Check Point QA slice
are implemented; the narrow v0.7 technical-readiness Workflow Run is also
implemented; broader terrain and product terms remain deferred

Punctra v0.7 builds on the reusable render engine, renderer-neutral View
planner, and verified Source path described in the accepted [v0.1 renderer
design](docs/design/render-engine-v0.1.md), [v0.2 planning
design](docs/design/adaptive-view-planning-v0.2.md), and [v0.3 Real Sources
design](docs/design/real-sources-v0.3.md). The accepted [v0.4 Out-of-core View
design](docs/design/out-of-core-view-v0.4.md) additionally implements Spatial
Index and rebuildable Artifact behavior for its narrow scope. The accepted
[v0.5 Durable document core design](docs/design/durable-document-core-v0.5.md)
implements the narrow classification meanings of Workspace, Snapshot, Query,
Point Set, Edit, Revert Edit, Revision, and Operation Identity behind the one
deep `point-workspace` crate. Broader uses of those terms remain vocabulary
research and do not imply product scope. The implemented [v0.6 Terrain and QA
benchmark design](docs/design/terrain-qa-benchmark-v0.6.md) fixes the narrow
meanings of Ground Input, Terrain Surface, Check Point, Residual, and Terrain
Gap used by `Snapshot::point_rows`, `point-terrain`, and `terrain-demo`.
The implemented [v0.7 technical-readiness
design](docs/design/technical-alpha-readiness-v0.7.md) additionally fixes the
narrow meanings of Workflow Run, Run Checkpoint, Revision Audit, Edit
Footprint, Surface Change Envelope, and Recovery Action. Those terms describe
one headless technical path and do not imply a partner-facing product.

## Artifact

An immutable result produced from a Source, Snapshot, or explicitly detached input with recorded construction parameters, such as a Spatial Index, Terrain Surface, or Profile. Its provenance identifies the Source and either the Workspace Revision or detached input content that was used, plus its construction version.

_Avoid:_ output blob, generated thing, cached result

## Attribute

A named value associated with a Point, such as classification, return number, intensity, color, or a survey flag. An Attribute retains its source meaning unless an Edit explicitly changes it.

_Avoid:_ property, metadata when referring to a per-point value

## Breakline

An ordered line whose vertices constrain the shape of a Terrain Surface. A Breakline may represent a ridge, curb, channel, wall, or another discontinuity that triangulation must respect.

_Avoid:_ polyline when its terrain-constraining meaning matters

## Check Point

A detached surveyed position used as independent evidence about a Terrain
Surface. A Check Point is not a Source Point and has its own caller-provided
identity.

_Avoid:_ control point, Point when the observation is detached from the Source

## Coordinate Reference

The declared horizontal reference, vertical reference, axis order, and units needed to interpret positions. A Coordinate Reference may explicitly be unknown; it is never guessed.

_Avoid:_ projection when referring to the whole reference, assumed CRS

## Coverage

A statement of how much of a requested result is currently represented. Complete Coverage is exact for the request; partial Coverage must be explicit.

_Avoid:_ done, loaded when completeness is what matters

## Derivation

The act of producing an Artifact from one explicit input provenance and one Recipe. The input is either a Snapshot or detached immutable Source content. A Derivation does not modify its input.

_Avoid:_ processing when the specific operation is derivation

## Edit

An intentional logical change to Workspace state, such as changing Point classifications or adding a Breakline. An Edit is recorded rather than applied to Source bytes.

_Avoid:_ mutation when referring to Source data, patch without domain context

## Edit Footprint

The inclusive axis-aligned world bounds of the Source Points whose effective
classification actually changed in one immutable Revision. It is derived from
exact Source positions and Revision rows. It does not describe every Terrain
face affected by the Edit.

_Avoid:_ changed region, Surface Change Envelope, exact change polygon

## Effective Attribute Value

The value obtained by applying every relevant overlay through one pinned
Revision to the immutable Source value. In v0.5 only the chosen `U8`
classification Attribute can have an effective value different from Source.

_Avoid:_ current value without naming the Snapshot, mutated Source value

## Export

The act of encoding a Snapshot or Artifact for an external tool or file format. Export does not alter the Workspace.

_Avoid:_ save when producing an external deliverable

## Ground Input

The complete set of Snapshot Points selected by one explicit effective ground
classification and optional Region for a Terrain Derivation.

_Avoid:_ visible ground, display Points, inferred terrain Points

## Operation Identity

An opaque caller-chosen identity for one canonical commit request. The caller records it before starting the commit so recovery can determine whether that request committed, was rejected, or was never recorded.

_Avoid:_ Job handle, random retry ID

## Pick Hint

A provisional Point Identity obtained from partial View residency. A Pick Hint
may be confirmed through an exact explicit-Point-ID Query, but it never proves
that a View or screen region contains no other matching Points.

_Avoid:_ exact selection, Query result, visibility proof

## Point

One observed spatial sample with a position and zero or more Attributes. A Point belongs to exactly one Source.

_Avoid:_ vertex unless it is a Terrain Surface vertex

## Point Batch

A bounded, ordered group of Points used to move exact point data between modules. A Point Batch is part of a stream and does not imply that the whole Source is resident.

_Avoid:_ chunk when referring to the public data contract

## Point Identity

The stable identity of one Point within one immutable Source Identity. Rebuilding an index or changing execution order does not change Point Identity.

_Avoid:_ array index, node-local ID, GPU ID

## Point Set

An immutable collection of Point Identities captured at a known Revision. A Point Set may be used as the target of a later Edit.

_Avoid:_ selection when referring to the materialized result rather than the interaction

## Profile

An Artifact containing ordered elevation samples and gaps along a path over a Terrain Surface.

_Avoid:_ cross-section unless that narrower meaning is intended

## Query

A read-only request against one Snapshot that returns every matching value. Progressive or partial display is a View, not a Query.

_Avoid:_ search when spatial and attribute semantics matter

## Recipe

The complete, normalized parameters and algorithm version used by a Derivation. A Recipe is recorded so the same Artifact can be reproduced.

_Avoid:_ preset after its defaults have been resolved

## Recovery Action

The one safe caller action attached to a structured Workflow failure, such as
resuming the same Run, raising a named limit, restoring the expected Source, or
removing a conflicting caller-owned target. It is not an automatic retry
policy.

_Avoid:_ suggestion, best effort, retry everything

## Residual

The signed vertical difference between an observed elevation and the Terrain
Surface elevation at the same horizontal position. Its sign convention must be
declared; v0.6 uses observed elevation minus surface elevation.

_Avoid:_ error when the direction and reference surface are unnamed

## Region

A spatial constraint used by a Query or Derivation. A Region is interpreted in a declared Coordinate Reference.

_Avoid:_ bounds when the shape may be more than an axis-aligned box

## Revision

An immutable, ordered state of a Workspace after a committed Edit. A Revision has exactly one logical predecessor in the initial model.

_Avoid:_ version, generation, save point

## Revision Audit

A rebuildable in-memory Artifact that describes exactly one immutable
Workspace Revision. It contains the Revision facts, ordered classification
transitions, changed Point count and membership hash, and Edit Footprint.

_Avoid:_ audit log, mutable history, Workflow report

## Revert Edit

An Edit that applies the recorded inverse of the current head Revision and
creates a new child Revision. It does not move the head backward, erase the
target Revision, or imply general history rewriting.

_Avoid:_ rollback, head rewind, delete history

## Run Checkpoint

An immutable checksummed frame proving that one Workflow phase fact was
durably synced. A checkpoint must be revalidated against the owning Source,
Workspace, Terrain, export, or report state; it is not an independent source of
truth.

_Avoid:_ cache entry, save point, proof without revalidation

## Snapshot

A read-only view of Workspace state pinned to one Revision. Every Query reads from a Snapshot; a Derivation may read from a Snapshot or explicitly detached input.

_Avoid:_ current data, live state

## Source

An immutable, ordered collection of point records that may be supplied to a Workspace or used directly by a module. A Source may be local, in memory, or remotely addressable, but its content and ordering are fixed for its Source Identity.

_Avoid:_ dataset when Source or Workspace is more precise

## Source Identity

The recorded identity of one immutable Source. Replacing or re-encoding the Source creates a different Source Identity, even if its visible Points appear equivalent.

_Avoid:_ filename, path, URL

## Source Span

A half-open interval of logical Point ordinals within one Source. Equivalent
overlapping Source Spans describe their union and never duplicate a Point.

_Avoid:_ byte range, chunk, index node

## Spatial Index

A rebuildable Artifact that maps spatial requests to candidate Source ranges. A Spatial Index accelerates access but never defines Point Identity or authoritative geometry.

In v0.4, it also stores checksummed exact-position samples for partial View
Coverage at internal hierarchy nodes. Those display samples are not canonical
Point Batches or complete Query results; complete leaf values still come from
the verified Source.

_Avoid:_ database, source of truth

## Surface Change Envelope

Conservative inclusive bounds over vertices incident to Terrain faces added or
removed between the baseline and changed Surfaces. It is application report
evidence, not a persisted Workspace value or exact change polygon.

_Avoid:_ Edit Footprint, affected-area polygon, persisted terrain boundary

## Terrain Surface

An immutable triangulated Artifact representing terrain for one explicit input
provenance and Recipe. Its Artifact Identity, vertices, faces, constraints or
explicit absence of constraints, Coordinate Reference, and provenance are
explicit.

_Avoid:_ mesh when the terrain semantics and provenance matter

## Terrain Gap

An explicit result that a Terrain Surface has no face at a requested horizontal
position. A Terrain Gap is not an elevation and is never silently filled by
extrapolation.

_Avoid:_ zero elevation, missing value when absence of surface Coverage matters

## View

A camera- and viewport-based request for progressive visual representation. A View may use partial Coverage and level of detail; it is not an exact Query.

_Avoid:_ scene when referring to a request

## View Batch

A bounded, renderer-neutral group of origin-relative display values and stable Point Identities produced for a View. A View Batch is disposable and is never authoritative geometry.

_Avoid:_ GPU buffer, render chunk

## Workflow Run

One durable execution intent for the supported classification-to-terrain path.
Its caller-owned Run Identity, Workspace Operation Identity, request meaning,
and Source/index/Workspace/Run-root path bindings are fixed by the first
durable journal frame.

_Avoid:_ process, Job handle, retry attempt

## Workspace

The revisioned logical document that relates one Source and its Edits to Snapshots.

_Avoid:_ project when referring to the domain object, session

## Workspace Identity

The stable opaque identity stored in a Workspace manifest. Moving the Workspace directory does not change it; copying with intent to create a distinct Workspace does.

_Avoid:_ directory path, display name
