# Cross-Module Contracts and Invariants

Status: deferred platform proposal; the v0.1 renderer, v0.2 adaptive View,
v0.3 Real Sources, and narrow v0.4 Spatial Index contracts are implemented
under the accepted designs in [`docs/design`](../design)

This document defines the semantics that every module must preserve. Rust names are illustrative; the behavior is normative.

The types shown here live with the owner named in [modules.md](modules.md): Point values in **point-contracts**, execution primitives in **foundation-runtime**, render deltas in **render-protocol**, and behavior-specific requests and errors in their behavior module. This document is a semantic index, not one giant contracts crate.

## Contract design rules

1. Values crossing a seam are owned, immutable, and valid by construction.
2. Source-scale collections cross seams as bounded streams, never as one required allocation.
3. Source-dependent operations name a Source Identity. Revision-dependent operations name a Revision or receive a Snapshot that pins it.
4. Persisted values carry a schema version. In-memory interface versioning and persisted-format versioning are separate.
5. Display values can lose precision; persisted and analytical values cannot.
6. Partial Coverage is explicit. Silence never means “probably complete.”

## Identity

### Source Identity

A Source adapter produces a stable opaque Source Identity and a verification record. The identity is tied to immutable source content and logical record order, not to a path or URL.

Initial Source registration computes and records a complete content fingerprint before Edits can be committed. Reopening supports two explicit verification policies:

- **Fast** checks the recorded file identity, length, modification metadata, header digest, and any available per-span checksums before use.
- **Full** recomputes the complete content fingerprint before exposing a Snapshot.

A Fast metadata mismatch returns VerificationRequired; it does not assign a new identity. Full verification then either confirms the recorded identity or returns SourceChanged.

The contract requires:

- the same unchanged Source reopens with the same identity;
- mutation is detected according to the selected verification policy before affected Point values are returned;
- replacing or re-encoding a Source creates a new identity; and
- untrusted or adversarial storage uses Full verification.

Fast verification is for ordinary accidental file replacement and damage; it is not claimed to detect a hostile same-length, same-metadata rewrite of an unread span.

v0.1 permits exactly one Source per Workspace. Point Identity retains SourceId so the lower data contracts do not need redesign when multi-Source composition is specified later.

### Point Identity

~~~rust
pub struct PointId {
    pub source: SourceId,
    pub ordinal: u64,
}
~~~

Point Identity is stable across:

- process restarts;
- index construction and rebuilding;
- cache deletion;
- thread-count and scheduling changes;
- LOD selection; and
- supported engine upgrades.

The ordinal is logical Source order, never an index-node offset or GPU slot:

- LAS and LAZ: point-record order.
- Proposed COPC adapter: canonical hierarchy-key order, then record offset
  within that node.

Equivalent-looking re-encoded files do not preserve Point Identity.

### Revision and Artifact identity

Revision, Recipe, and Artifact identifiers are opaque. Callers may compare and persist them but may not infer ordering from their representation. Explicit parent and sequence metadata express Revision order.

Revision Identity is globally collision-resistant. RevisionStore::create generates a random private lineage nonce; every root and child Revision Identity commits that nonce plus its canonical history input. Independently created Revision stores over the same Source therefore cannot produce equal Revision Identities. A byte-for-byte recovered or intentionally cloned journal retains its lineage and Revision Identities.

~~~rust
pub struct ArtifactId(/* opaque */);
~~~

Artifact Identity is a collision-resistant digest over the producing module kind, versioned provenance, normalized construction parameters, algorithm version, and canonical Artifact content. Only the module that constructs the Artifact assigns it.

Every Artifact identity includes, directly or through its provenance:

- an input identity: Source Identity plus either the relevant Revision or a detached content hash and Coordinate Reference;
- normalized construction parameters;
- algorithm identifier and version; and
- relevant contract or schema version.

An Artifact derived from logical Workspace state also includes the pinned Revision and normalized Recipe. A Spatial Index is explicitly Revision-independent.

~~~rust
pub enum DataProvenance {
    Snapshot(SnapshotProvenance),
    Detached {
        source: SourceId,
        content_hash: ContentHash,
        coordinate_reference: CoordinateReference,
    },
}
~~~

Exact Query streams use Snapshot provenance. Synthetic direct-use streams use an immutable in-memory Source Identity with Detached provenance and cannot claim a Workspace Revision.

### Operation Identity

Operation Identity is an opaque value chosen by the caller for one persistent commit request. Before starting the commit Job, the caller durably records the identity with the Workspace Identity needed to reopen and reconcile it. The caller does not canonicalize or hash the Edit payload.

The initial representation is 128 opaque bits. foundation-runtime can generate it from operating-system entropy or validate caller-supplied bytes. Callers never infer time or ordering from it and never intentionally reuse it for a distinct request.

Before the logical commit point, **point-revisions** canonicalizes and hashes the request while copying the full payload, including streamed Point Identities, into verified journal-owned staging. Incomplete staging cannot create a Revision. The same recorded Operation Identity and canonical payload are idempotent and return the recorded resolution. Reusing a recorded Operation Identity with a different canonical payload is Rejected as OperationIdentityConflict. A commit Job uses the caller-supplied Operation Identity as its stable Job identity.

The canonical staging encoding and digest algorithm are versioned parts of the Revision journal schema. Reopen either supports that version or fails before accepting a retry; callers never implement this encoding.

## Coordinate and precision contract

LAS and LAZ positions are quantized; a proposed COPC adapter would preserve the
same property. Canonical Point Batches preserve that quantization:

~~~rust
pub struct QuantizedPositions {
    pub offset: [f64; 3],
    pub scale: [f64; 3],
    pub ticks: Column<[i64; 3]>,
}

impl QuantizedPositions {
    pub fn world_f64(&self, row: usize) -> [f64; 3];
}
~~~

Rules:

- source ticks, scale, and offset are preserved exactly when the format permits;
- CPU geometry uses deterministic conversion to 64-bit floating-point world values or exact integer predicates where appropriate;
- v0.1 does not silently reproject Sources;
- the Workspace Source must use its declared Coordinate Reference, or opening fails;
- an unknown Coordinate Reference is permitted for viewing and dimensionless operations;
- unit-sensitive Derivations and Exports require an explicit Coordinate Reference and units; and
- no module guesses a horizontal reference, vertical reference, axis order, or unit.

View Batches use a separate disposable representation:

~~~rust
pub struct ViewBatch {
    pub view_generation: ViewGenerationKey,
    pub key: ViewBatchKey,
    pub world_origin: [f64; 3],
    pub relative_positions: Column<[f32; 3]>,
    pub point_ids: IdColumn,
    pub attributes: AttributeColumns,
    pub geometric_error_world: f64,
    pub coverage_after: Coverage,
}
~~~

Each display position equals the authoritative world position minus the batch origin, rounded to 32-bit floating point. A GPU result never flows back into an Edit, exact selection, Derivation, Profile, or Export.

## Point Batch

~~~rust
pub struct PointBatch {
    pub source: SourceId,
    pub ids: IdColumn,
    pub positions: QuantizedPositions,
    pub attributes: AttributeColumns,
}
~~~

A valid Point Batch:

- is non-empty; stream completion is represented by a terminal stream summary, never an empty batch;
- has equal row counts in every included column;
- contains unique Point Identities;
- reports which Attributes are present;
- preserves the Source adapter's canonical values and flags;
- fits the hard negotiated batch-memory limit; and
- is ordered as declared by the producing interface.

Classification changes do not silently alter LAS synthetic, key-point, withheld, or overlap flags.

Attribute values unknown to the foundation remain representable as typed extra columns. An adapter must fail with an actionable unsupported-schema error rather than discard a requested Attribute.

## Bounded stream

~~~rust
pub trait BatchStream: Send {
    type Batch;
    type Summary;
    type Error;

    fn next(&mut self) -> Result<Option<Self::Batch>, Self::Error>;
    fn summary(&self) -> Option<&Self::Summary>;
    fn handle(&self) -> OperationHandle;
}
~~~

All implementations guarantee:

- backpressure: the producer does not require the consumer to retain prior batches;
- a hard maximum batch size, including all columns;
- a module-specific hard limit for concurrently active transient blocks;
- cancellation checks between batches and within long-running phases;
- successful exhaustion returns `None`, makes exactly one immutable summary available through `summary`, and then remains fused;
- a cancellation or failure error leaves no summary and is followed by fused `None`;
- progress and cancellation are observed through a separate cloneable `OperationHandle`, never as data-stream events; and
- no source-scale work in stream construction itself unless the interface returns a Job.

Async adapters may expose the same semantics through an asynchronous stream. A synchronous reference path remains available for deterministic tests and simple CLI use.

## Job, progress, and cancellation

~~~rust
pub struct Job<T, E> { /* opaque */ }

impl<T, E> Job<T, E> {
    pub fn handle(&self) -> OperationHandle;
    pub fn blocking_wait(self) -> Result<T, E>;
}

impl<T, E> Future for Job<T, E> {
    type Output = Result<T, E>;
}

impl OperationHandle {
    pub fn progress(&self) -> ProgressSnapshot;
    pub fn cancel(&self);
}
~~~

A Job:

- has one process-local operation identity;
- reports an ordered phase and monotonic counters;
- may coalesce progress events;
- honors the caller's CPU, memory, and temporary-storage budgets;
- exposes uncertain commit acknowledgement only as Indeterminate with an Operation Identity; and
- records whether cancellation completed before or after a commit point.

`Job` implements the standard Future contract without requiring a particular
async runtime. A cloneable `OperationHandle` is the caller capability for
progress observation and cancellation. It cannot publish progress; producers
receive a restricted reporter, while the unique operation owner alone can
publish terminal progress. CLI and foreign-language adapters use
`blocking_wait`; Rust async callers use `.await`.

For a persistent commit, cancellation is accepted before the commit point. Once the commit is known durable, the Job returns Committed even if cancellation was requested concurrently. If acknowledgement fails after the commit point may have been crossed, it returns Indeterminate rather than reporting ordinary cancellation or rejection.

## Point Set contract

A Point Set is represented by an opaque PointSetHandle owned by **point-set**:

~~~rust
pub struct PointSetMetadata {
    pub source: SourceId,
    pub materialized_at: RevisionId,
    pub exact_count: u64,
    pub content_hash: ContentHash,
}
~~~

Rules:

- materialization consumes an exact Query stream as a cancellable Job and accepts no separately supplied provenance;
- the handle derives Source Identity and materialization Revision from the terminal Snapshot provenance;
- every Point Batch Source Identity must equal the terminal Snapshot Source Identity or the entire materialization fails;
- Point Identities are deduplicated and stored in canonical ordinal order;
- memory use obeys PointSetBudget and spills to checked temporary storage when necessary;
- iteration is bounded and repeatable for the handle's documented lifetime;
- the handle is immutable and its content hash covers identity ordering and provenance;
- dropping the last handle may delete ephemeral spill files;
- an expired, corrupt, or missing spill returns an explicit error;
- detached stream provenance is rejected; and
- v0.1 commit requires materialized_at to equal the expected head and rejects implicit rebasing.

The Edit journal consumes Point Identities as bounded batches from the handle and verifies them while writing journal-owned durable staging. It never collects the whole Point Set in memory, and it cannot cross the logical commit point until every staged batch and the canonical payload digest are durable.

## Source interface

~~~rust
pub struct SourceCandidate { /* opaque, unverified */ }

impl SourceCandidate {
    pub fn preview(&self) -> &SourcePreview;
    pub fn open(self, options: OpenOptions) -> SourceJob;
}

pub type SourceJob = Job<Source, SourceError>;

pub struct Source { /* opaque, verified, cloneable */ }

impl Source {
    pub fn identity(&self) -> SourceId;
    pub fn metadata(&self) -> &SourceMetadata;
    pub fn provenance(&self) -> &SourceProvenance;
    pub fn record(&self) -> &SourceRecord;
    pub fn points(&self) -> Result<PointBatches, SourceError>;
    pub fn read(&self, request: ReadRequest) -> Result<PointBatches, SourceError>;
}

impl PointBatches {
    pub fn next(&mut self) -> Result<Option<PointBatch>, SourceError>;
    pub fn summary(&self) -> Option<&SourceReadSummary>;
    pub fn handle(&self) -> OperationHandle;
}

impl OpenOptions {
    pub fn identify() -> Self;
    pub fn match_record(record: SourceRecord, policy: VerificationPolicy) -> Self;
}
~~~

`SourceCandidate` is unverified and cannot be placed in a Snapshot. Successful
opening publishes one opaque, already verified `Source`; ordinary callers do
not receive an adapter trait object or a separate verification witness.
`SourceRecord` is the versioned persistable verification record: it binds
Source Identity, the Full content fingerprint, logical-order rule and adapter
version, record count and schema digests, and the adapter-owned facts needed to
evaluate Fast verification. `OpenOptions::identify` forces Full verification;
`OpenOptions::match_record` carries the recorded expectation and requested
policy. Success returns the same caller-visible `Source` type for every
adapter, with its record available for later persistence.

Concrete adapter crates may expose convenience `open` functions that construct
and identify their candidate, but those functions still return `SourceJob` and
publish the same opaque `Source`.

Required behavior:

- creating a Source Identity forces Full verification;
- reopening verifies the recorded expectation at the requested Fast or Full level;
- reads are bounded and deterministic;
- overlapping spans do not cause duplicate rows;
- a requested field is returned exactly or rejected explicitly;
- Source mutation produces a SourceChanged error;
- corruption identifies the failing span when possible; and
- network-backed range reads, if later added, preserve identical semantics.

Every official adapter runs the same Source conformance suite.

## Spatial Index contract

~~~rust
pub fn prepare(
    source: Source,
    target: impl AsRef<Path>,
    limits: PrepareLimits,
) -> Job<PreparedIndex, IndexError>;

impl PreparedIndex {
    pub fn descriptor(&self) -> &IndexDescriptor;
    pub fn hierarchy(&self) -> &IndexHierarchy;
    pub fn prepare_report(&self) -> &PrepareReport;

    pub fn candidates(
        &self,
        bounds: WorldBounds,
        limits: CandidateLimits,
    ) -> Result<CandidatePlan, IndexError>;

    pub fn read_node(
        &self,
        node: IndexNodeId,
        budget: NodeReadBudget,
    ) -> Result<IndexPointBatches, IndexError>;
}

impl CandidatePlan {
    pub fn spans(&self) -> &[SourceSpan];
    pub fn candidate_point_count(&self) -> u64;
    pub fn visited_node_count(&self) -> u64;
}
~~~

`prepare` is the only public construction/opening operation. It reads the
opaque verified `Source` directly and returns only a complete `PreparedIndex`:

- a compatible target is fully checked and opened without Source reads;
- a missing target resumes a compatible checksummed work file or builds from
  Source ordinal zero;
- an existing incompatible or corrupt target fails and is never replaced; and
- cancellation or failure before publication leaves no partial complete target.

The returned handle retains the verified Source. `PrepareReport` records
`Opened`, `Built`, or `Resumed`, durable Points reused, Source Points read by
that call, and final artifact bytes. The deterministic work frames checkpoint
consecutive Source blocks; recovery truncates only an invalid suffix and resumes
from the last valid ordinal boundary.

The v0.4 persisted recipe is fixed: consecutive Source blocks contain at most
65,536 Points, a longest-centroid-extent median split builds one binary BVH,
node identities are nonzero and root-first, and each internal node retains at
most 4,096 deterministic exact `(ordinal, ticks)` display samples. The disk and
recipe versions are separate from the Cargo version.

`IndexDescriptor` exposes the bound Source Identity and point count, exact
position transform and optional world bounds, recipe and disk versions,
node/leaf counts, and checksum. `IndexHierarchy` is one complete resident
snapshot. Its `IndexNode` values expose identity, optional parent, inclusive
finite bounds, covered and displayed Point counts, conservative geometric
error, and sampled or complete display Coverage. Persisted pages, work frames,
child-sample merge buffers, and Source adapter details remain private.

For candidate plans:

- false positives are allowed and removed by **point-query**;
- false negatives are forbidden;
- Source spans refer to stable logical records;
- output spans are sorted, nonempty, disjoint, and deterministic;
- `CandidateLimits` separately caps visited nodes, final spans, candidate Points,
  and working bytes; and
- the result is complete or an error, never partial Coverage.

`IndexPointBatches` is a bounded fused batch stream. Internal nodes read their
checksummed persisted samples; leaves read every Point in their one contiguous
span from the retained verified Source. Each `IndexPointBatch` carries Source
Identity, transform, node identity, and sorted unique `IndexSample` values.
These sparse display values deliberately are not canonical `PointBatch` values
and do not claim complete Query Coverage. The terminal `IndexReadSummary`
reports emitted display count, covered Source count, Source provenance, and
sampled or complete Coverage. Failure and cancellation publish no summary.

`PrepareLimits`, `CandidateLimits`, and `NodeReadBudget` keep Source batch,
adapter, builder, artifact, hierarchy, candidate, index-buffer, display-batch,
and emitted-Point ceilings separate. **point-view**, not **point-index**, owns
camera culling, screen-error policy, budgets, priority, and refinement.

The complete artifact is checksummed and Source/version-bound. A synced
temporary sibling is published with an atomic no-replace hard link; an existing
or racing target is rejected rather than overwritten. The parent directory is
synced before disposable work, sample-spool, and temporary files are removed.
This is not rename-and-replace behavior.

## Revision and Snapshot contract

~~~rust
pub struct RevisionSourceContract {
    pub source: SourceId,
    pub point_count: u64,
    pub editable_attributes: AttributeSchema,
    pub coordinate_reference: CoordinateReference,
}

pub struct EditBatch {
    pub operations: Vec<Edit>,
    pub message: Option<String>,
}

pub enum Edit {
    PatchPoints {
        points: PointSetHandle,
        patch: PointPatch,
    },
    UpsertBreakline {
        layer: LayerId,
        feature: FeatureId,
        geometry: Breakline,
    },
    RemoveBreakline {
        layer: LayerId,
        feature: FeatureId,
    },
}

pub enum CommitOutcome {
    Committed { revision: RevisionId },
    Rejected { reason: CommitRejection },
    Indeterminate { operation: OperationId },
}

pub enum CommitResolution {
    Committed { revision: RevisionId },
    Rejected { reason: CommitRejection },
    NotRecorded,
}
~~~

Rules:

- RevisionSourceContract is derived from verified SourceMetadata, persists Source Identity, point count, editable Attribute schema, and Coordinate Reference at store creation, and is matched exactly on reopen;
- valid Point ordinals are `0..point_count`, and Point patches are limited to its editable Attribute schema;
- Source bytes are immutable; Point changes are sparse overlays;
- PointPatch cannot change position in v0.1;
- EditBatch has a hard maximum operation count, while Point Identities remain streamed through PointSetHandle;
- Edit operations execute in batch order, and a later operation wins when fields overlap;
- a Point Set's Source Identity and materialization Revision must equal the commit Source and expected head;
- commit receives a caller-chosen Operation Identity that was retained before the Job started;
- the Revision store binds the Operation Identity to its internally canonicalized Source Identity, expected head, ordered Edit Batch, and referenced Point Set contents;
- repeating the same identity and canonical payload is idempotent, while different content is Rejected as OperationIdentityConflict;
- the Revision store durably stages the entire canonical payload before the logical commit point;
- incomplete staging is discarded or recovered as Rejected and cannot create a Revision;
- commit uses compare-and-swap against an expected head;
- a stale expected head is Rejected with the actual head and writes nothing;
- Committed creates exactly one complete durable Revision;
- Indeterminate means the commit point may have been crossed but acknowledgement failed;
- every commit has an Operation Identity whose status can be reconciled after reopen;
- NotRecorded reconciliation means no durable operation record exists and guarantees that the request did not create a Revision;
- a Snapshot is immutable and remains readable while later commits occur;
- recovery exposes the old head or the complete new head, never half of either; and
- an unknown Point Identity, invalid Attribute, or invalid Breakline rejects the whole Edit Batch.

The initial history is linear. Every committed Revision remains addressable after reopen, and live Snapshots pin required storage. Pruning, collaboration, branches, and merge semantics are outside v0.1.

**point-revisions** owns RevisionView, which exposes immutable overlays and Breaklines. **point-query** requests a RevisionView by Revision Identity from its already validated RevisionStore and combines it with the verified Source and compatible-index state to create the public Snapshot. It does not accept a caller-supplied RevisionView. **point-workspace** returns that Snapshot without defining another Snapshot type.

QueryEngine construction fails unless the verified `Source` metadata, optional
complete Spatial Index, and `RevisionSourceContract` agree on Source Identity;
Source point count, editable Attribute schema, and Coordinate Reference must
also equal the `RevisionSourceContract`.

## Query contract

~~~rust
pub struct PointQuery {
    pub region: Region,
    pub filter: PointFilter,
    pub fields: FieldMask,
}

pub struct ExactPointSummary {
    pub provenance: DataProvenance,
    pub exact_count: u64,
}

pub struct ExactBreaklineSummary {
    pub provenance: DataProvenance,
    pub exact_count: u64,
}
~~~

A Query:

- reads one pinned Snapshot;
- applies Source values, then Revision overlays, then predicates;
- returns all exact Region and Attribute matches;
- never substitutes visible LOD samples for exact Points;
- emits batches in Point ordinal order for the Workspace Source;
- completes with Snapshot provenance and exact row count;
- returns no duplicate Point Identity; and
- is unchanged by concurrent commits.

Exact world boxes and polygons include their boundary. Internal index partitions may be half-open, but the final predicate follows the closed public Region. A screen-through Region contains a frozen 64-bit camera including near/far clip, viewport, and screen polygon; it includes Points on the polygon and clip boundaries and ignores occlusion. Visible-only selection is not supported in v0.1.

Screen projection uses the recorded Camera64 matrix, a specified 64-bit operation order, and no depth-buffer input. The Query contract version changes if those numeric semantics change.

GPU picking may produce provisional candidate Point Identities from resident View data. Exact selection ignores that incomplete set, runs a complete CPU Query over the frozen screen-through Region, and then materializes a Point Set.

## View contract

~~~rust
impl ViewPlanner {
    pub fn plan(
        &mut self,
        camera: &Camera,
        viewport: [u32; 2],
        available_nodes: AvailableNodes<'_>,
        budget: PlanningBudget,
    ) -> Result<ViewPlan, PlanError>;
}
~~~

A v0.2 View plan:

- consumes one host-owned, generation-stamped snapshot of hierarchy metadata and
  missing, requested, or resident batch state;
- conservatively culls the 64-bit world-space bounds against a validated
  perspective `Camera`;
- selects LOD by screen-space error with hysteresis and stable node-key tie
  breaking;
- reserves requested and retained point, estimated-byte, and batch costs before
  adding a request;
- keeps resident ancestors or descendants until selected replacement Coverage
  is resident;
- returns requests by descending visual priority, retained nodes by node key,
  and conditional retirements by batch key;
- stamps every result with the snapshot `ViewGenerationKey`, and stamps each
  retirement with the observed batch version; and
- performs no materialization, I/O, renderer update, exact selection, or claim
  of analytical completeness.

`ViewGenerationKey` contains the caller's View identity and generation; it does
not contain a Workspace Revision. A host that derives `AvailableNodes` from a
Snapshot remains responsible for retaining and validating that provenance and
for selecting a new generation when its policy requires one. **point-view** has
no dependency on `ViewInput`, **point-query**, or a Spatial Index.

The host explicitly begins renderer state with `RenderUpdate::Reset`, converts
materialized requested nodes into `PointBatch` Upserts, and applies plan
retirements as conditional Remove updates. A render `Frame` supplies the exact
generation, camera, and viewport to draw; a mismatched generation returns
`ViewGenerationMismatch`. Mesh rendering remains outside the implemented
renderer and planner contracts.

## Terrain contract

~~~rust
pub struct TerrainRecipe {
    pub algorithm: AlgorithmVersion,
    pub grid: TerrainGrid,
    pub thinning: ThinningRule,
    pub coincident_xy: CoincidentXyRule,
    pub boundary: BoundaryRule,
    pub maximum_edge: Option<Length>,
}

pub struct TerrainInput {
    pub provenance: DataProvenance,
    pub points: Box<dyn BatchStream<
        Batch = PointBatch,
        Summary = ExactPointSummary,
        Error = QueryError,
    >>,
    pub breaklines: Box<dyn BatchStream<
        Batch = BreaklineBatch,
        Summary = ExactBreaklineSummary,
        Error = QueryError,
    >>,
}

pub struct SurfaceProvenance {
    pub input: DataProvenance,
    pub input_digest: ContentHash,
    pub recipe: TerrainRecipe,
    pub limits: TerrainLimits,
    pub contract_version: TerrainContractVersion,
}

pub struct TerrainLimits {
    pub maximum_input_points: u64,
    pub maximum_vertices_after_noding: u64,
    pub maximum_faces: u64,
    pub maximum_diagnostics: u64,
    pub maximum_memory_bytes: u64,
    pub maximum_temporary_bytes: u64,
}

pub struct SurfaceArtifact {
    pub identity: ArtifactId,
    pub provenance: SurfaceProvenance,
    pub coordinate_reference: CoordinateReference,
    pub vertices: SurfaceVertices,
    pub faces: TriangleFaces,
    pub breaklines: BoundedVec<SurfaceConstraint>,
    pub diagnostics: BoundedVec<Diagnostic>,
}
~~~

Rules:

- Snapshot input requires matching Source Identity, Revision, and Coordinate Reference in both terminal stream summaries;
- detached synthetic streams require the same Source Identity, combined input-content hash, and Coordinate Reference and cannot claim a Workspace Revision;
- every detached Point Batch Source Identity must equal the Source Identity in its Detached provenance;
- SurfaceProvenance records that Snapshot or detached input identity, a batch-partition-independent digest of the consumed Points and Breaklines, the fully normalized Recipe, the applied limits, and the terrain contract version;
- ArtifactId is derived from the versioned SurfaceProvenance and canonical
  topology content, so a future host can label bounded display batches without
  inspecting terrain internals;
- the caller supplies finite Point Batches and raw finite Breaklines;
- a named preset is resolved to complete numeric parameters before recording;
- the module normalizes duplicate vertices and Breakline intersections according to the Recipe;
- input signed zero is canonicalized to positive zero and NaN or infinity is rejected;
- every XY value is already on the recorded TerrainGrid or follows its explicit reject-or-round rule;
- grid rounding is nearest with ties to even;
- Breakline intersections use exact integer or rational construction before the recorded grid-rounding step;
- canonical ordering compares grid coordinates, stable Point Identity, then constraint identity;
- the authoritative path forbids contraction or fused multiply-add where it could change recorded rounding;
- incompatible elevations at the same XY and irreconcilable constraints fail explicitly;
- robust predicates and stable Point Identity tie-breaking determine topology;
- valid faces are nondegenerate, consistently oriented, and reference existing vertices;
- output vertices and faces have canonical ordering;
- every TerrainLimit includes Points and vertices created by Breakline noding; and
- the same normalized inputs, Recipe, and algorithm version produce identical topology on supported platforms.

The grid, floating-point environment, exact-predicate implementation, rounding rules, thinning order, and intersection rules are all part of AlgorithmVersion. The GPU may accelerate disposable visualization or a verified non-authoritative prepass. It is not the source of terrain topology.

## LandXML contract

LandXML encoding:

- consumes a complete Terrain Surface, not a point stream;
- writes explicit vertices and faces;
- records declared units and Coordinate Reference metadata when representable;
- includes boundaries and Breaklines according to explicit options;
- uses deterministic identifiers and ordering;
- validates finite values, unique identifiers, face references, orientation, and degeneracy;
- never silently retriangulates; and
- emits bounded byte chunks followed by one report with counts, warnings, selected format version, and a content hash.

The first target is LandXML 1.2. Supporting another version requires fixtures and semantic round trips; it is not a version-string-only change.

## Structured error contract

~~~rust
pub struct Error {
    pub module: ModuleId,
    pub code: u32,
    pub class: ErrorClass,
    pub operation: Option<OperationId>,
    pub message: String,
    pub context: BoundedVec<ErrorContext>,
    pub recovery: Recovery,
}

pub enum ErrorClass {
    InvalidInput,
    NotFound,
    Conflict,
    CorruptData,
    Unsupported,
    Stale,
    ResourceLimit,
    Unavailable,
    Indeterminate,
    Cancelled,
    Io,
    InternalInvariant,
}
~~~

**foundation-runtime** owns the bounded envelope and broad class. Each behavior module owns and versions its numeric codes. Required named codes include SourceMissing, SourceChanged, SourceContractMismatch, VerificationRequired, UnsupportedFormat, UnsupportedSchema, IndexIncomplete, IndexIncompatible, ExpiredPointSet, ProvenanceMismatch, StaleRevision, UnknownRevision, OperationIdentityConflict, InvalidGeometry, WorkspaceBusy, IndeterminateCommit, ViewGenerationMismatch, GpuUnavailable, and DeviceLost.

Errors are actionable at the module's interface. Lower modules do not format dialogs, terminal text, or Python exceptions. Adapters translate the structured error while retaining module, numeric code, class, and context.

Failure effects:

- failed Source reads do not alter persistent state;
- failed index builds may leave verified resumable checkpoints only;
- Rejected commits leave the previous head intact;
- Indeterminate commits require operation reconciliation and recover to one complete old or new head;
- failed Derivations may leave verified disposable cache entries only;
- failed LandXML encoding does not mutate the Terrain Surface;
- the host atomic-file adapter, not **landxml**, guarantees that failed file output leaves the previous destination; and
- device loss can fail a frame but cannot corrupt a Workspace.

## Ordering and concurrency

- Source reads and exact Query output use Point ordinal order within the single Workspace Source.
- Evaluation jobs may run concurrently.
- Commits serialize and use compare-and-swap semantics.
- One batch stream never invokes its consumer concurrently.
- View updates use stable visibility priority with node-key tie-breaking and explicit replacement.
- Terrain vertices and faces use canonical ordering.
- Progress phases are ordered; elapsed time is not deterministic.
- A Snapshot remains valid across concurrent commits and reads.

Thread safety is declared per concrete type. No module relies on process-global mutable state.

## Persistence and compatibility

Every persisted foundation format has:

- a magic value and explicit schema version;
- Source Identity and relevant Revision;
- creation options and algorithm version where relevant;
- length framing and checksums;
- a documented endianness;
- recovery behavior for an incomplete tail; and
- a compatibility test corpus.

Unknown major versions fail explicitly. Migrations read the old representation and write a new one; they do not reinterpret bytes in place.

Cache keys include all inputs that affect meaning: Source Identity; Revision when logical state matters; normalized request, construction parameters, or Recipe; algorithm version; and persisted-schema version.

## Resource contract

The foundation promises bounded work rather than hardware-independent latency:

- opening an existing Workspace with Fast Source verification is proportional to manifest size and the uncheckpointed journal tail, not total Point count;
- index construction is streaming, resumable, checksummed, and budgeted;
- an indexed exact Query is proportional to visited index nodes plus candidate and emitted Points;
- a Query with no complete index remains exact by sequentially scanning the Source;
- Point Set materialization is bounded by its memory budget and spills within its temporary-storage budget;
- a sparse commit is proportional to edited Point Identities and features, not Source size;
- View work is proportional to visited visible nodes and emitted samples within the point budget;
- terrain memory is bounded by its explicit TerrainLimits;
- LandXML encoding emits bounded byte chunks rather than requiring one output allocation; and
- exceeding a declared budget fails with ResourceLimit rather than growing without bound.

Absolute timing targets belong in benchmark baselines after representative hardware and Sources exist.
