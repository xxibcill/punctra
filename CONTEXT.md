# Domain Context: Point-Cloud Foundation

Status: renderer, adaptive View-planning, and Real Sources terms are
implemented; broader Workspace and terrain terms are deferred

Punctra v0.3 implements the reusable render engine, renderer-neutral View
planner, and verified Source path described in the accepted [v0.1 renderer
design](docs/design/render-engine-v0.1.md), [v0.2 planning
design](docs/design/adaptive-view-planning-v0.2.md), and [v0.3 Real Sources
design](docs/design/real-sources-v0.3.md). The definitions of Source, Source
Identity, Point, Point Identity, Attribute, Point Batch, Coordinate Reference,
View, Coverage, and View Batch below are canonical. The remaining terms are
retained as vocabulary research for possible host projects and do not imply
current product scope.

## Artifact

An immutable result produced from a Source, Snapshot, or explicitly detached input with recorded construction parameters, such as a Spatial Index, Terrain Surface, or Profile. Its provenance identifies the Source and either the Workspace Revision or detached input content that was used, plus its construction version.

_Avoid:_ output blob, generated thing, cached result

## Attribute

A named value associated with a Point, such as classification, return number, intensity, color, or a survey flag. An Attribute retains its source meaning unless an Edit explicitly changes it.

_Avoid:_ property, metadata when referring to a per-point value

## Breakline

An ordered line whose vertices constrain the shape of a Terrain Surface. A Breakline may represent a ridge, curb, channel, wall, or another discontinuity that triangulation must respect.

_Avoid:_ polyline when its terrain-constraining meaning matters

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

## Export

The act of encoding a Snapshot or Artifact for an external tool or file format. Export does not alter the Workspace.

_Avoid:_ save when producing an external deliverable

## Operation Identity

An opaque caller-chosen identity for one canonical commit request. The caller records it before starting the commit so recovery can determine whether that request committed, was rejected, or was never recorded.

_Avoid:_ Job handle, random retry ID

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

## Region

A spatial constraint used by a Query or Derivation. A Region is interpreted in a declared Coordinate Reference.

_Avoid:_ bounds when the shape may be more than an axis-aligned box

## Revision

An immutable, ordered state of a Workspace after a committed Edit. A Revision has exactly one logical predecessor in the initial model.

_Avoid:_ version, generation, save point

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

_Avoid:_ database, source of truth

## Terrain Surface

An immutable triangulated Artifact representing terrain for one explicit input provenance and Recipe. Its Artifact Identity, vertices, faces, constraints, Coordinate Reference, and provenance are explicit.

_Avoid:_ mesh when the terrain semantics and provenance matter

## View

A camera- and viewport-based request for progressive visual representation. A View may use partial Coverage and level of detail; it is not an exact Query.

_Avoid:_ scene when referring to a request

## View Batch

A bounded, renderer-neutral group of origin-relative display values and stable Point Identities produced for a View. A View Batch is disposable and is never authoritative geometry.

_Avoid:_ GPU buffer, render chunk

## Workspace

The revisioned logical document that relates one Source and its Edits to Snapshots.

_Avoid:_ project when referring to the domain object, session

## Workspace Identity

The stable opaque identity stored in a Workspace manifest. Moving the Workspace directory does not change it; copying with intent to create a distinct Workspace does.

_Avoid:_ directory path, display name
