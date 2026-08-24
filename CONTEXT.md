# Domain Context: Point-Cloud Foundation

Status: the narrow v0.1 through v0.9 repository slices are Complete; the v0.10
professional inspection View repository implementation is complete while
field, partner-validation, adoption-publication, and support evidence remain
outstanding; the v0.11 exact interactive review technical slice is complete
and repository-verified while field activation and independent-adoption
evidence remain outstanding; the v0.12 explicit spatial-reference and library-
packaging repository slice is complete while its production-corpus,
downstream, adoption, and support evidence remains outstanding; the accepted
pre-v0.13 renderer-quality corrective checkpoint is complete and repository-
verified while permitted field execution remains outstanding; v0.13:
Complete and repository-verified for the bounded persistent-terrain slice;
field activation, production-scale accuracy, true out-of-core adoption,
independent adoption, partner validation, and support qualification
outstanding; v0.14 exact Terrain QA/correction, v0.15 local browser-foundation,
v0.16 bounded HTTP Range streaming, and v0.17 bounded framework-neutral browser
viewer API slices are Complete and repository-verified while their declared
external exits remain outstanding; broader selection, arbitrary browser Source
delivery, supported SDK packaging, and product terms remain deferred

Punctra v0.11 builds on the reusable render engine, renderer-neutral View
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
The implemented [v0.8 repository interoperability qualification
design](docs/design/design-partner-mvp-v0.8.md) fixes the meanings of
Downstream Declaration, Interoperability Qualification, Round-Trip Evidence,
and Tolerance Profile for the post-Run verifier. The bounded streaming
comparator and canonical Run-bound pass/fail evidence path exist, but those
terms do not imply that a downstream product was exercised or that a firm
accepted the result.
The implemented [v0.9 Trust and v1 Candidate
design](docs/design/trust-v1-candidate-v0.9.md) fixes the repository meanings of
artifact support class, version-1 compatibility, rebuild policy, recovery
certainty, platform evidence, and v1 candidate. Here, v1 candidate means only
that the frozen repository surface passed its recorded local qualification; it
does not mean `1.0.0`, production support, product readiness, or external
acceptance.
The accepted [v0.10 Field Qualification and Professional Inspection View
design](docs/design/field-inspection-view-v0.10.md) fixes Display Mode and
Display Mapping as disposable host-owned presentation policy over explicit
Source or position values. A displayed color never becomes authoritative Point
data. The implementation retains position-only disk-v1 samples for neutral and
elevation display and adds a narrow disk-v2 inspection sample for raw RGB,
intensity, and classification. Perspective and orthographic projections,
progressive loading/Coverage facts, structured View failures, and local corpus
measurements remain application policy rather than exact Query semantics.
The accepted [v0.11 Exact Interactive Review and Ground Correction
design](docs/design/exact-interactive-review-v0.11.md) fixes one separate CPU-
authoritative review composition. `point-review` confirms a provisional Point
Identity against one pinned Snapshot or scans every exact Snapshot row through
one inclusive physical-pixel Screen Rectangle, with an optional effective-
classification equality predicate. Exact Point Sets, not Pick tokens or
resident LOD samples, supply bounded renderer highlights. Classification
commit, immediate-head Revert, Revision Audit/Edit Footprint, and uncertain-
Operation reconciliation retain their existing Workspace meanings. Polygon,
brush, visible-only, and occlusion selection, arbitrary Attribute/position
editing, general UI, and automatic recovery are not accepted meanings.
The accepted [v0.12 Explicit Spatial Reference and Package Publication
design](docs/design/explicit-spatial-reference-v0.12.md) fixes one complete
projected survey-coordinate profile and its provenance. The structured profile
may be decoded from complete verified Source metadata or declared by a caller;
neither provenance is downstream acceptance. Metre, international foot, and US
survey foot can be represented, while the current Terrain, QA, and LandXML path
supports metre/metre only and performs no transformation. Opaque WKT and
unknown references remain explicit rather than guessed.

The accepted [v0.13 Persistent Bounded-AOI Terrain
design](docs/design/persistent-production-scale-terrain-v0.13.md) fixes one
rebuildable disk-v1 Surface Artifact for an explicit inclusive AOI. It preserves
the existing exact Ground Input and canonical single-worker full-AOI topology,
checkpoints complete input and final staging, publishes without replacement,
reopens compatible Surfaces, and exposes bounded file-backed vertex/face
streams. Persistence is not true out-of-core triangulation: the complete AOI
still must fit the declared triangulation-memory limit. After publication, the
verified stage and any work sibling remain because identity-conditioned
pathname cleanup is not portable. A work sibling becomes resumable only after
verification; none of these files is Workspace authority.

The accepted [v0.14 Exact Terrain QA and Correction Loop
design](docs/design/exact-terrain-qa-correction-v0.14.md) fixes one exact,
CPU-authoritative QA report for a frozen Snapshot/Surface pair and one semantic
Surface comparison. Rendered colors and profile drawings remain presentation;
correction and Revert retain the existing `point-workspace` Operation meanings.

The accepted [v0.15 WebAssembly and WebGPU Browser Foundation
design](docs/design/browser-foundation-v0.15.md) fixes Browser Host as one
private application composition over the existing renderer protocol, View
planner, and wgpu renderer. The JavaScript caller owns the canvas element and
lifecycle policy; the private Rust adapter owns WebGPU resources on its behalf
as the host of `render-wgpu`. Browser display and picking remain progressive
and provisional. Local generated execution does not mean remote LAS/LAZ
delivery, a supported SDK, broad Browser Qualification, or exact CPU Query.

The completed [v0.16 HTTP Range Streaming, Browser Caching, and Worker Decoding
design](docs/design/http-range-streaming-v0.16.md) fixes Remote Deployment as a
trusted versioned binding between one immutable LAS representation, its strong
HTTP validator and Source identity, and one compatible disk-v2 Spatial Index.
The private worker may publish only the root node's bounded Sampled Coverage;
the manifest and cache do not become Source authority, and browser display does
not repeat the complete `source-las` verification used to prepare the binding.
An arbitrary raw URL is not a Remote Deployment.
The [v0.16 repository verification record](docs/releases/v0.16.0.md) owns the
exact local implementation, environment, command, browser, and nonclaim facts.

The completed [v0.17 Browser Viewer API
design](docs/design/browser-viewer-api-v0.17.md) fixes a coherent checked-in
JavaScript and TypeScript integration boundary over the private WebAssembly
viewer and streaming worker. It owns lifecycle, viewport, camera, five display
modes, render scheduling, bounded state, provisional pick/highlight generation
semantics, and exact-Query handoff. The host owns interaction policy; exact
Point values come only from the separate immutable-LAS record bridge. This is
not SDK packaging, an arbitrary-Source adapter, a framework promise, or broad
browser qualification.
The [v0.17 repository verification record](docs/releases/v0.17.0.md) owns the
exact local implementation, environment, command, browser, exact-record, and
nonclaim facts.

## Artifact

An immutable result produced from a Source, Snapshot, or explicitly detached input with recorded construction parameters, such as a Spatial Index, Terrain Surface, or Profile. Its provenance identifies the Source and either the Workspace Revision or detached input content that was used, plus its construction version. A rebuildable persistent Artifact may be deleted and reproduced from that authority; persistence does not make it Workspace state.

_Avoid:_ output blob, generated thing, cached result

## Attribute

A named value associated with a Point, such as classification, return number, intensity, color, or a survey flag. An Attribute retains its source meaning unless an Edit explicitly changes it.

_Avoid:_ property, metadata when referring to a per-point value

## Breakline

An ordered line whose vertices constrain the shape of a Terrain Surface. A Breakline may represent a ridge, curb, channel, wall, or another discontinuity that triangulation must respect.

_Avoid:_ polyline when its terrain-constraining meaning matters

## Browser Host

The caller-owned application layer that embeds Punctra in a browser. It owns
the canvas element, CSS placement, device-pixel-ratio and visibility policy,
input, scheduling, errors, and recovery decisions. A private Rust adapter may
own WebGPU instance/device/queue/surface resources on its behalf while the
public renderer remains submission-neutral.

_Avoid:_ browser SDK when referring to the private v0.15 acceptance adapter,
Punctra UI when the host owns application policy

## Remote Deployment

A trusted, versioned browser-host statement binding one immutable HTTP LAS/LAZ
representation to its exact byte length, strong validator, Punctra Source
identity, compatible Spatial Index, display-sample recipe, and integrity facts.
It authorizes only the declared bounded progressive display path. It is not
Source authority, URL discovery, a credential policy, or permission to scan or
download an arbitrary file in full.

_Avoid:_ remote Source when the binding is absent, URL loader, browser Source
adapter, cache manifest

## Check Point

A detached surveyed position used as independent evidence about a Terrain
Surface. A Check Point is not a Source Point and has its own caller-provided
identity.

_Avoid:_ control point, Point when the observation is detached from the Source

## Confirmed Point

One provisional renderer Point Identity resolved from exact Source ticks and
the effective classification at a pinned Workspace Snapshot. Its world
position, provenance, and one-Point Point Set come from CPU-authoritative
Snapshot reads; GPU position, color, depth, and batch membership are not
confirmation facts.

_Avoid:_ Pick Hit, sampled display Point, current Point without a Revision

## Coordinate Reference

The declared horizontal reference, vertical reference, axis order, units, and
provenance needed to interpret positions. The v0.12 structured profile uses
nonzero EPSG identities and easting/northing/elevation axes while exact Source
scale/offset retains coordinate precision. A Coordinate Reference may instead
be opaque WKT or explicitly unknown; it is never guessed.

_Avoid:_ projection when referring to the whole reference, assumed CRS

## Coverage

A statement of how much of a requested result is currently represented. Complete Coverage is exact for the request; partial Coverage must be explicit.

_Avoid:_ done, loaded when completeness is what matters

## Derivation

The act of producing an Artifact from one explicit input provenance and one Recipe. The input is either a Snapshot or detached immutable Source content. A Derivation does not modify its input. Durable checkpoints may resume implementation work without changing the Derivation's semantic input or result.

_Avoid:_ processing when the specific operation is derivation

## Display Mapping

A deterministic CPU conversion from an explicit sampled position or raw
Attribute value to the RGBA8 bytes carried by a View Batch. It changes only
presentation and cannot create an exact Attribute value, Query result, or
Workspace Edit.

_Avoid:_ Attribute conversion, shader truth, measurement palette

## Display Mode

The host-selected Display Mapping used for one View. v0.10 modes are neutral,
elevation, RGB, intensity, and classification. Unsupported or unavailable
inputs fail explicitly rather than causing a guessed fallback.

_Avoid:_ layer when no data layer changes, classification when referring to color

## Display Sample

A bounded identity-preserving index value used for progressive display.
Internal-node samples have Sampled Coverage; complete leaf reads have Complete
Coverage for that node. Neither is an exact Workspace Query merely because its
stored values came from the authoritative Source.

_Avoid:_ Query result, complete Source, authoritative GPU Point

## Downstream Declaration

The caller's exact application label, version label, and settings associated
with one returned deliverable. It records what the caller asserts and is not
proof that the application ran or applied those settings.

_Avoid:_ supported application, verified configuration, application evidence

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

## Field Corpus

A bounded local manifest of permitted Sources, opaque observation identities,
declared machine context, display/projection choices, and navigation traces
used to reproduce viewing measurements. Permission to inspect and measure is
explicit per Source; redistribution permission is separate.

_Avoid:_ public dataset, benchmark proof, production evidence without permission

## Ground Input

The complete set of Snapshot Points selected by one explicit effective ground
classification and optional Region for a Terrain Derivation. The v0.13
persistent path requires one explicit inclusive world-bounds AOI and may
checkpoint this complete verified set before topology work.

_Avoid:_ visible ground, display Points, inferred terrain Points

## Interoperability Qualification

A bounded semantic comparison between one authoritative exported deliverable
and one caller-returned deliverable under an explicit Tolerance Profile. It
qualifies the compared artifacts, not an application, vendor, firm, or product.

_Avoid:_ certification, application support, partner acceptance

## Operation Identity

An opaque caller-chosen identity for one canonical commit request. The caller
records it before starting the commit so recovery can determine whether that
request committed, was rejected, or was never recorded. A classification
commit and a later immediate-head Revert are distinct requests with distinct
identities; retry or reconciliation of either request retains its original
identity.

_Avoid:_ Job handle, random retry ID

## Pick Hint

A provisional Point Identity obtained from partial View residency. A Pick Hint
records the producing View generation, batch, and batch version. After the host
rejects stale View state, `point-review` may confirm its Point Identity against
a pinned Snapshot. A Pick Hint never proves effective values, Edit eligibility,
or that a View or screen region contains no other matching Points.

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

An immutable collection of Point Identities and effective classification
before-values captured at a known Revision. A Point Set may be resident or
spilled and may be read repeatedly through bounded ordered identity or entry
batches. It may be used as the target of a later Edit. Renderer highlights are
presentation derived from a complete bounded identity read; they are not Point
Set storage or membership authority.

_Avoid:_ selection when referring to the materialized result rather than the interaction

## Profile

An Artifact containing ordered elevation samples and gaps along a path over a Terrain Surface.

_Avoid:_ cross-section unless that narrower meaning is intended

## Projection Mode

The camera projection used for a View. Perspective and orthographic projection
change screen projection and LOD calculations but not Source coordinates,
Point Identity, or Coverage. In v0.11 the same validated Camera projection is
an explicit input to the separately defined exact CPU Screen-through Selection
algorithm; that does not make GPU rasterization authoritative.

_Avoid:_ Coordinate Reference, map projection, CRS

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

## Round-Trip Evidence

An immutable record binding an original Export, a caller-returned deliverable,
its Downstream Declaration, Tolerance Profile, and complete qualification
result. It is technical artifact evidence, not proof of paid use or customer
acceptance.

_Avoid:_ application certification, pilot evidence, acceptance report

## Screen Rectangle

One normalized inclusive rectangle in top-left-origin continuous physical-
pixel coordinates, bound to a specific nonempty Viewport. The complete
Viewport spans `[0, width]` by `[0, height]`; zero-width or zero-height
rectangles retain exact boundary meaning.

_Avoid:_ pixel-index range, lasso, polygon, brush stroke

## Screen-through Selection

An exact revision-pinned CPU Query that scans complete Snapshot rows and
selects every Point center projecting inside one Screen Rectangle and the
Camera's inclusive clip volume. An optional equality predicate uses effective
classification at that Snapshot. GPU residency, splat size, transparency,
occlusion, and visible-surface depth do not remove matches.

_Avoid:_ visible-only selection, GPU selection, Pick region, occlusion query

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
explicit. A v0.13 prepared Terrain Surface may be file-backed and read through
bounded canonical streams; its full-AOI triangulation remains memory-resident
during construction.

_Avoid:_ mesh when the terrain semantics and provenance matter

## Terrain Gap

An explicit result that a Terrain Surface has no face at a requested horizontal
position. A Terrain Gap is not an elevation and is never silently filled by
extrapolation.

_Avoid:_ zero elevation, missing value when absence of surface Coverage matters

## Tolerance Profile

The caller-declared inclusive horizontal and vertical differences permitted by
one Interoperability Qualification. It is recorded exactly and is never
inferred from coordinates, display precision, or a downstream application.

_Avoid:_ fuzz, epsilon, automatic tolerance

## View

A camera- and viewport-based request for progressive visual representation. A View may use partial Coverage and level of detail; it is not an exact Query.

An exact v0.11 Screen-through Selection may capture the same Camera and
Viewport values, but it remains a separate complete CPU operation over one
pinned Snapshot.

_Avoid:_ scene when referring to a request

## View Batch

A bounded, renderer-neutral group of origin-relative display values and stable Point Identities produced for a View. A View Batch is disposable and is never authoritative geometry.

_Avoid:_ GPU buffer, render chunk

## Viewing Report

A bounded local record of one Field Corpus run: declared and observed machine
context, Source/index binding, timings, resource facts, Coverage, trace facts,
failures, and explicit nonclaims. It is measurement evidence only for the
operations recorded; it does not imply terrain capacity, professional
preference, partner acceptance, downstream support, or human-time savings.

_Avoid:_ certification, field qualification, performance promise

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
