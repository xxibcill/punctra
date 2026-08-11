# Module Catalog

Status: broader platform proposal deferred; the current renderer, View,
Source, and narrow Spatial Index modules are implemented under the
[v0.1 renderer](../design/render-engine-v0.1.md),
[v0.2 planning](../design/adaptive-view-planning-v0.2.md),
[v0.3 Real Sources](../design/real-sources-v0.3.md), and
[v0.4 Out-of-core View](../design/out-of-core-view-v0.4.md) scopes

This document is the ownership map. Each crate below is one logical module with one job. The job sentence is normative: if new behavior does not fit it, that behavior does not belong in the module.

“Individually usable” means a caller can invoke the module's interface directly with canonical inputs. It does not mean that every module has zero dependencies.

## Catalog

| Module | Its only job | Canonical input | Canonical output |
|---|---|---|---|
| **point-contracts** | Define and validate lossless Point values and stable spatial provenance. | None | IDs, Point Batches, spatial metadata, provenance |
| **foundation-runtime** | Standardize bounded execution control for long foundation operations. | Operation closure or producer | Jobs, batch streams, budgets, progress, cancellation |
| **point-source** | Provide verified bounded canonical read access to one immutable Source. | Source candidate, open options, and read request | Opaque verified Source, Source Record, and Point Batches |
| **point-index** | Provide a rebuildable persistent mapping from spatial requests to candidate Source ranges. | Verified Source, target path, and `PrepareLimits` | Complete `PreparedIndex`, conservative plans, and bounded display streams |
| **point-set** | Materialize exact Point Identities as immutable bounded-memory Point Sets. | Exact Point Batch stream with terminal Snapshot provenance | Spillable Point Set handle |
| **point-revisions** | Persist sparse Edits and resolve immutable Revision state. | Revision target, Operation Identity, expected Revision, and Edit Batch | Commit resolution and Revision view |
| **point-query** | Provide bounded revision-pinned reads from one Snapshot. | Source, Spatial Index, Revision view, read request | Snapshot, exact Point or Breakline streams, ViewInput |
| **render-protocol** | Represent generation-safe renderer-neutral camera and point-display contracts. | Camera and point display data | Validated camera values and render updates |
| **point-view** | Plan one frozen View over a host-owned hierarchy without performing I/O. | Camera, viewport, node snapshot, and hard budget | Prioritized requests, required retention, and safe retirements |
| **terrain-model** | Derive one deterministic Terrain Surface from Points and constraints. | Point Batch stream, Breaklines, Recipe | Immutable Terrain Surface Artifact |
| **landxml** | Encode one Terrain Surface as validated LandXML. | Terrain Surface and export options | Bounded XML byte stream and validation report |
| **point-workspace** | Manage the coherent lifecycle of one Source and its revisioned Workspace. | One Source, options, Queries, Edits | Open result, Snapshots, commit outcomes, preparation jobs |
| **render-wgpu** | Maintain and draw one wgpu representation of render-protocol state. | Render updates, frozen frame, GPU target | Frame report and provisional pick hints |
| application adapters | Translate one host environment into workspace and rendering calls. | CLI, desktop, or language-host input | Calls and host-native results |

## 1. point-contracts

**Job:** define and validate lossless Point values and stable spatial provenance.

This is the stable point-model module. It is intentionally free of execution control, Source decoding, index algorithms, persistence, triangulation, GPU work, and orchestration. Its leverage comes from making point-bearing modules interoperable without sharing private representations.

It owns:

- stable identifier value types;
- Coordinate Reference and unit value types;
- Source and Attribute metadata, including the persisted Revision Source Contract;
- Point Batch and stable provenance schemas;
- Breakline geometry and bounded Breakline Batch schemas;
- shared Region, field-mask, and content-hash value types; and
- persisted schema version identifiers.

It does not own:

- behavior that reads, writes, indexes, queries, edits, derives, or renders data;
- module-owned Query, Edit, View, terrain, and export request types;
- execution control, Jobs, or progress reporting;
- format-specific values that never cross a seam; or
- a general utility collection.

Conceptual interface:

~~~rust
pub struct SourceId(/* opaque */);
pub struct ArtifactId(/* opaque */);
pub struct RevisionId(/* globally unique, opaque */);

pub struct RevisionSourceContract {
    pub source: SourceId,
    pub point_count: u64,
    pub editable_attributes: AttributeSchema,
    pub coordinate_reference: CoordinateReference,
}

impl RevisionSourceContract {
    pub fn from_verified_source(
        identity: &SourceId,
        metadata: &SourceMetadata,
    ) -> Result<Self>;
}

pub struct PointId {
    pub source: SourceId,
    pub ordinal: u64,
}

pub struct PointBatch {
    pub source: SourceId,
    pub ids: IdColumn,
    pub positions: PositionColumns,
    pub attributes: AttributeColumns,
}

pub struct SnapshotProvenance {
    pub source: SourceId,
    pub revision: RevisionId,
    pub coordinate_reference: CoordinateReference,
}

pub enum DataProvenance {
    Snapshot(SnapshotProvenance),
    Detached {
        source: SourceId,
        content_hash: ContentHash,
        coordinate_reference: CoordinateReference,
    },
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

**Independent proof:** contract tests can round-trip every persisted value and verify compatibility without opening any Source.

## 2. foundation-runtime

**Job:** standardize bounded execution control for long foundation operations.

It owns:

- runtime-neutral Job handles that implement Future and blocking_wait;
- bounded batch-stream terminal semantics;
- cancellation tokens and commit-point reporting;
- operation identities;
- progress phases and monotonic counters;
- CPU, memory, temporary-storage, and batch budgets; and
- a structured error envelope that retains a module-owned error code and context.

It does not own:

- a process-global executor;
- domain requests or results;
- module-specific error meaning;
- persistence or recovery; or
- scheduling policy inside another module.

Conceptual interface:

~~~rust
pub struct OperationId([u8; 16]);

impl OperationId {
    pub fn generate() -> Result<Self>;
    pub fn from_bytes(bytes: [u8; 16]) -> Result<Self>;
    pub fn as_bytes(&self) -> &[u8; 16];
}

pub struct Job<T, E> { /* opaque */ }

impl<T, E> Job<T, E> {
    pub fn handle(&self) -> OperationHandle;
    pub fn blocking_wait(self) -> Result<T, E>;
}

impl<T, E> Future for Job<T, E> {
    type Output = Result<T, E>;
}

pub trait BatchStream: Send {
    type Batch;
    type Summary;
    type Error;

    fn next(&mut self) -> Result<Option<Self::Batch>, Self::Error>;
    fn summary(&self) -> Option<&Self::Summary>;
    fn handle(&self) -> OperationHandle;
}
~~~

It uses standard Future and Waker contracts rather than requiring Tokio or
another host runtime. Jobs and streams expose a cloneable `OperationHandle`
for progress observation and cancellation. Progress is never interleaved with
data batches, and callers cannot publish producer progress or terminal state.

**Independent proof:** drive synthetic Jobs and streams through success, cancellation, budget exhaustion, blocking wait, asynchronous wait, and fused terminal behavior without loading point data.

## 3. point-source

**Job:** provide verified bounded canonical read access to one immutable Source.

The module defines the proven Source seam and common validation. Format behavior lives in adapter crates:

- **source-las** decodes LAS formats 0–10 and LAZ formats 0–8 in point-record
  order, including headers, VLRs, EVLRs, and supported Attributes; LAZ formats
  9 and 10 are explicitly unsupported pending exact layered WavePacket14 codec
  support;
- the proposed, deferred **source-copc** would decode local COPC hierarchy
  order, byte ranges, and Attributes; and
- **source-memory** supplies deterministic fixtures and fault injection.

It owns:

- Source Identity verification;
- metadata and Attribute-schema normalization;
- stable mapping from logical record position to Point Identity;
- enforcement of bounded reads;
- validation of canonical adapter output; and
- detection of truncated, corrupt, unsupported, or changed Sources.

It does not own:

- format decoding, which belongs to the concrete adapter;
- spatial indexing;
- Edits or overlays;
- filtering beyond fields needed for decoding;
- Coordinate Reference guessing or reprojection;
- LOD selection; or
- GPU packing.

Conceptual interface:

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

`SourceCandidate` is unverified. Opening it with `OpenOptions::identify` forces
Full verification; `OpenOptions::match_record` carries persisted
`SourceRecord` evidence and the requested Fast or Full policy. Success returns
one opaque, already verified `Source`, regardless of the concrete adapter.
`SourceRecord` is a versioned, serializable value owned by **point-source**; it
binds Source Identity to the Full fingerprint and the adapter-specific facts
required for Fast verification. The verification policy is an opening input,
not a second reader type or an achieved-level witness. No Workspace Snapshot
can hold `SourceCandidate`.

Concrete adapter crates may expose convenience `open` functions that construct
and identify their candidate, but those functions still return `SourceJob` and
publish the same opaque `Source`.

The logical ordinal is part of the adapter contract:

- LAS and LAZ use point-record order.
- A future COPC adapter would use canonical hierarchy-key order followed by
  record offset within the node.

An index may reorder storage for speed, but it must carry the original Point Identity.

**Independent proof:** open a fixture through one adapter, request record spans, and receive the same canonical Point Batches without constructing a Workspace or Spatial Index.

## 4. point-index

**Job:** provide a rebuildable persistent mapping from spatial requests to candidate Source ranges.

It owns:

- deterministic 65,536-Point Source-block construction and binary BVH planning;
- exact node bounds, covered/display counts, stable identities, and conservative
  geometric-error summaries;
- bounded deterministic internal display samples and complete Source-backed
  leaf reads;
- append-only checksummed work, valid-prefix recovery, complete artifact
  verification, and no-replace publication; and
- deterministic conservative inclusive-box traversal to sorted disjoint Source
  spans.

It does not own:

- decoding point records;
- exact attribute filtering;
- Revision overlays;
- authoritative Point Identity;
- camera/View policy, renderer packing, or GPU state; or
- Source bytes.

Implemented interface:

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
~~~

`prepare` verifies and opens a compatible complete target, otherwise resumes a
compatible work file or builds from the beginning. Corrupt or incompatible
existing targets fail without replacement. It publishes only after a complete
artifact is synced and atomically hard-linked to a previously absent target;
there is no public builder, open-status enum, filesystem abstraction, or page
interface.

`PreparedIndex` retains its verified `Source`. `IndexDescriptor` exposes Source
binding, transform/bounds, recipe/disk versions, counts, and artifact checksum.
`IndexHierarchy` exposes a complete root-first immutable node snapshot.
`CandidatePlan` is complete or an error and contains sorted disjoint Source
spans. `IndexPointBatches` emits exact-position `IndexSample` values: sampled
internal-node Coverage comes from the artifact, while complete leaves are read
from the Source. These are display-only batches, not exact Query results.

The proposed foundation index deliberately treats a future COPC adapter through
the same canonical Source seam; native-hierarchy import waits for a second real
producer before gaining a seam. **point-view** owns camera culling, screen error,
point budgets, priority, and refinement policy.

**Independent proof:** build an index from **source-memory** and compare every complete candidate plan with a sequential-scan oracle. No file decoder, Revision store, Workspace, or renderer is required.

## 5. point-set

**Job:** materialize exact Point Identities as immutable bounded-memory Point Sets.

It owns:

- deterministic Source-ordinal ordering and duplicate removal;
- compressed in-memory identity segments;
- spill files when a memory budget is reached;
- Source Identity and materialization-Revision provenance;
- exact count, content hash, integrity checks, and lifetime;
- bounded identity iteration; and
- cleanup after the last handle is released.

It does not own:

- deciding which Points match a Query;
- Source decoding or Spatial Index access;
- applying an Edit;
- Revision persistence; or
- long-term named selection storage.

Conceptual interface:

~~~rust
pub fn materialize(
    points: impl BatchStream<
        Batch = PointBatch,
        Summary = ExactPointSummary,
        Error = QueryError,
    >,
    budget: PointSetBudget,
) -> Job<PointSetHandle, PointSetError>;

impl PointSetHandle {
    pub fn provenance(&self) -> &SnapshotProvenance;
    pub fn exact_count(&self) -> u64;
    pub fn content_hash(&self) -> ContentHash;
    pub fn ids(&self, budget: ReadBudget) -> PointIdBatchStream;
}
~~~

Point Set provenance is derived from the exact stream's terminal summary; the caller cannot label it separately. Every Point Batch Source Identity must match that summary, and Detached provenance is rejected. The handle is process-scoped and remains valid until its owning process releases it. A commit consumes the handle through bounded iteration and durably stages its canonical Point Identities inside **point-revisions** before any ambiguous commit point; it never requires every Point Identity in memory. A Point Set is not itself the crash-recovery record.

**Independent proof:** materialize generated Point Batches under tiny memory limits, force several spills, reopen the handle within its lifetime, and iterate the same canonical identities and hash.

## 6. point-revisions

**Job:** persist sparse Edits and resolve immutable Revision state.

It owns:

- the append-only edit journal;
- atomic compare-and-swap commit;
- monotonically ordered Revision identities;
- sparse Point-Attribute overlays;
- versioned Breakline records;
- immutable Revision-view resolution;
- checksums, write-ahead records, and crash recovery; and
- compaction that preserves every committed Revision in v0.1.

It does not own:

- Source bytes or Source decoding;
- spatial query planning;
- terrain invalidation policy;
- user-interface undo behavior; or
- derived caches.

Conceptual interface:

~~~rust
impl RevisionStore {
    pub fn create(
        target: RevisionTarget,
        source: RevisionSourceContract,
        options: RevisionOptions,
    ) -> Job<Self, RevisionError>;

    pub fn open_and_recover(
        target: RevisionTarget,
        expected_source: RevisionSourceContract,
        options: RevisionOptions,
    ) -> Job<Self, RevisionError>;

    pub fn source_contract(&self) -> &RevisionSourceContract;
    pub fn head(&self) -> RevisionId;
    pub fn view(&self, revision: RevisionId) -> Result<RevisionView>;

    pub fn commit(
        &self,
        operation: OperationId,
        expected_head: RevisionId,
        edits: EditBatch,
    ) -> Job<CommitOutcome, RevisionError>;

    pub fn resolve_operation(
        &self,
        operation: OperationId,
    ) -> Result<CommitResolution>;
}

impl RevisionView {
    pub fn overlays(&self, ids: &[PointId], fields: FieldMask) -> OverlayBatch;
    pub fn breaklines(&self, region: &Region) -> BreaklineStream;
}
~~~

create and open_and_recover are the only public entry points that create or interpret the Revision journal. create generates the private lineage nonce from which globally unique Revision Identities are derived. Recovery verifies that nonce, the persisted Revision Source Contract, schema, frames, checksums, operation records, and head before exposing a RevisionStore. That contract binds Source Identity, the valid ordinal interval `0..point_count`, the editable Attribute schema, and Coordinate Reference, so commit can reject unknown Point Identities and invalid patches without depending on **point-source**.

The caller chooses and durably records the Operation Identity and Workspace Identity before starting commit. The caller does not reproduce Revision-store canonicalization. The store canonicalizes and hashes the request while copying the complete Edit payload, including every referenced Point Identity, into journal-owned durable staging. Only after staging is verified may it cross the logical commit point. A crash before a complete operation record is recovered as Rejected or NotRecorded, both of which guarantee that no Revision was created. Reusing a recorded identity with the same expected head and canonical Edit Batch is idempotent and returns its recorded resolution; reusing it with different content is Rejected. Committed means the new Revision is durable. Rejected means the previous head is unchanged. Indeterminate means an I/O failure prevented acknowledgement after the durable commit point may have been crossed; the caller resolves the same Operation Identity after reopen. Recovery always exposes a complete old or new Revision, never a partial one.

A Point Set's Source Identity and materialization Revision must equal the commit Source and expected head in v0.1. Rebase is an explicit future policy, not an implicit use of stable IDs.

Every committed Revision remains addressable after reopen in v0.1. Live Revision views pin any storage needed by compaction; pruning is not implemented.

**Independent proof:** commit Edits against synthetic Point Identities, reopen the store, and resolve identical overlays. A fault-injection adapter tests torn writes and disk-full behavior.

## 7. point-query

**Job:** provide bounded revision-pinned reads from one Snapshot.

It owns:

- turning an exact Query into an index candidate plan;
- reading candidate Source spans;
- applying Snapshot overlays;
- exact Region and Attribute predicates;
- stable result ordering;
- exact-count and terminal-provenance accounting; and
- enforcing query memory and cancellation budgets.

It does not own:

- index construction;
- Source decoding rules;
- journal persistence;
- visual LOD;
- terrain generation; or
- GPU selection.

Conceptual interface:

~~~rust
impl QueryEngine {
    pub fn new(
        source: Source,
        index: Option<Arc<PreparedIndex>>,
        revisions: Arc<RevisionStore>,
    ) -> Result<Self>;

    pub fn pin(&self, revision: RevisionId) -> Result<Snapshot>;
}

impl Snapshot {
    pub fn revision(&self) -> RevisionId;
    pub fn provenance(&self) -> &SnapshotProvenance;

    pub fn query(
        &self,
        query: PointQuery,
    ) -> Box<dyn BatchStream<
        Batch = PointBatch,
        Summary = ExactPointSummary,
        Error = QueryError,
    >>;

    pub fn breaklines(
        &self,
        region: Region,
    ) -> Box<dyn BatchStream<
        Batch = BreaklineBatch,
        Summary = ExactBreaklineSummary,
        Error = QueryError,
    >>;

    pub fn view_input(&self) -> Result<ViewInput>;
}

impl ViewInput {
    pub fn provenance(&self) -> &SnapshotProvenance;

    pub fn hierarchy(
        &self,
        request: HierarchyRequest,
    ) -> Result<IndexNodeBatch>;

    pub fn materialize(
        &self,
        selection: NodeSelection,
        fields: FieldMask,
    ) -> Box<dyn BatchStream<
        Batch = PointBatch,
        Summary = ViewReadSummary,
        Error = QueryError,
    >>;
}
~~~

`QueryEngine::new` verifies that the opaque `Source` metadata, optional
`PreparedIndex`, and `RevisionStore` carry the same Source Identity and that
Source point count, editable Attribute schema, and Coordinate Reference equal
the persisted Revision Source Contract. It rejects the composition before any
Snapshot is created. `pin` accepts only a Revision Identity and obtains its
`RevisionView` from that validated store, so a view from another store cannot
be injected.

A Query uses the already pinned Snapshot. Concurrent commits cannot change its output. Point Queries are exact and complete in v0.1; partial Coverage belongs to Views.

When no compatible index exists, a complete Query may sequentially scan the Source. A View requires a compatible index because a full scan cannot satisfy an interactive LOD contract.

`ViewInput` is a deferred platform capability: a future host adapter may use it
to obtain revision-pinned hierarchy facts and materialize selected node keys.
That adapter can translate the facts and materialization state into the
host-owned `AvailableNodes` snapshot accepted by the current **point-view**
contract. **point-view** itself does not depend on **point-query**, accept a
`ViewSpec` or `FrameToken`, or emit renderer resets.

**Independent proof:** use **source-memory** with real index and Revision stores in temporary directories, then compare streamed results against a simple sequential oracle.

## 8. render-protocol

**Job:** represent generation-safe renderer-neutral camera and point-display
contracts.

It owns:

- validated perspective `Camera` values in 64-bit world coordinates;
- caller-selected View, generation, Point, batch, and batch-version identities;
- origin-relative `RenderPoint` and `PointBatch` values;
- fixed point, estimated-byte, and batch residency accounting;
- Reset, Upsert, conditional Remove, and complete highlight-set updates; and
- a CPU state model that validates generations, versions, and hard limits.

It does not own:

- hierarchy, LOD, loading, or eviction policy;
- Source or Snapshot access;
- GPU allocation or drawing;
- mesh or Terrain Surface display contracts; or
- exact selection.

Primary interface:

~~~rust
impl Camera {
    pub fn perspective(
        eye: [f64; 3],
        target: [f64; 3],
        up: [f64; 3],
        vertical_field_of_view_radians: f32,
        near_distance: f32,
        far_distance: f32,
    ) -> Result<Self, CameraError>;
}

pub enum RenderUpdate {
    Reset { view_generation: ViewGenerationKey },
    Upsert { batch: PointBatch },
    Remove {
        view_generation: ViewGenerationKey,
        key: BatchKey,
        expected_version: BatchVersion,
    },
    SetHighlights {
        view_generation: ViewGenerationKey,
        point_ids: Vec<PointId>,
    },
}

impl RenderStateModel {
    pub fn new(limits: RenderLimits) -> Self;
    pub fn apply<'update>(
        &mut self,
        update: &'update RenderUpdate,
    ) -> Result<AppliedUpdate<'update>, ProtocolError>;
    pub fn snapshot(&self) -> RenderSnapshot;
}
~~~

`RenderStateModel` is a small CPU reference for protocol validation; it owns no
GPU resources. A Reset explicitly begins one `ViewGenerationKey`. Upserts must
advance a batch version, conditional removal cannot remove a newer replacement,
and hard point, estimated-byte, and batch limits fail without changing state.
The renderer and protocol tests use these same transition rules.

**Independent proof:** apply generated Reset, Upsert, replacement, conditional
Remove, highlight, stale-generation, and resource-limit sequences and inspect
the deterministic public snapshot without creating a GPU.

## 9. point-view

**Job:** plan one frozen View over a host-owned hierarchy without performing
I/O.

It owns:

- frustum and screen-error planning;
- point, byte, and batch-budget allocation;
- stable LOD priority and refinement;
- parent Coverage during progressive replacement;
- hysteresis across successive plans; and
- exact generation-safe retirement decisions.

It does not own:

- hierarchy construction or persistence;
- node materialization, request execution, or cancellation;
- GPU buffers, shaders, or device state;
- exact selection membership;
- Source decoding;
- Edits;
- Terrain Surface derivation; or
- automatic renderer eviction.

Primary interface:

~~~rust
impl ViewPlanner {
    pub fn plan(
        &mut self,
        camera: &Camera,
        viewport: Viewport,
        available_nodes: AvailableNodes<'_>,
        budget: PlanningBudget,
    ) -> Result<ViewPlan, PlanError>;
}
~~~

The host supplies immutable hierarchy facts and current missing, requested, or
resident state. `ViewPlanner` retains only hysteresis history. It materializes
no point data and performs no side effects. A later platform adapter may
translate a Spatial Index hierarchy into this node snapshot without creating a
new public seam inside the planner.

Requests are ordered by visual priority with stable node-key tie breaking.
Resident parents remain retained until all selected visible replacements are
resident. Every retirement copies the observed View generation, batch key, and
batch version, so applying a delayed retirement cannot remove newer data.

**Independent proof:** run generated hierarchy and residency snapshots through
the public planning interface and inspect the deterministic plan without
creating a GPU device or materializing a Point Batch.

## 10. terrain-model

**Job:** derive one deterministic Terrain Surface from Points and constraints.

It owns:

- surface candidate thinning;
- duplicate and coincident-XY policy;
- Breakline normalization and intersection noding;
- robust orientation and in-circle predicates;
- constrained triangulation;
- boundary and hole handling;
- topology validation and canonical ordering; and
- complete Recipe and provenance recording.

It does not own:

- point decoding, indexing, or Snapshot lookup;
- Classification editing;
- GPU geometry authority;
- LandXML semantics; or
- interactive rendering.

Conceptual interface:

~~~rust
pub fn derive(
    input: TerrainInput,
    recipe: TerrainRecipe,
    limits: TerrainLimits,
) -> Job<SurfaceArtifact, TerrainError>;

impl SurfaceArtifact {
    pub fn identity(&self) -> &ArtifactId;
    pub fn vertex_batches(&self, budget: ReadBudget) -> SurfaceVertexStream;
    pub fn face_batches(&self, budget: ReadBudget) -> SurfaceFaceStream;
}
~~~

TerrainInput carries one DataProvenance for both the exact Point stream and Breakline stream; mismatches are rejected. Snapshot input records Source Identity and Revision. Detached synthetic input records its in-memory Source Identity, a combined input-content hash, and Coordinate Reference, but cannot claim a Workspace Revision. Every Point Batch Source Identity must match that provenance. SurfaceProvenance adds a canonical digest of the consumed streams, the fully normalized TerrainRecipe, applied TerrainLimits, and terrain contract version.

The first implementation is CPU-authoritative and has explicit limits for input Points, vertices after constraint noding, faces, diagnostics, memory, and temporary storage. It may use parallel preparation, but the recorded numeric model, robust predicates, and canonical tie-breaking determine the result. Given identical normalized input, Recipe, and algorithm version, topology is byte-for-byte deterministic on supported platforms.

**Independent proof:** derive a Terrain Surface directly from generated Point Batches and Breaklines, then validate topology and compare a golden hash. No Workspace, Source adapter, Spatial Index, or GPU is required.

## 11. landxml

**Job:** encode one Terrain Surface as validated LandXML.

It owns:

- LandXML version and unit encoding;
- deterministic vertex and face identifiers;
- explicit face, boundary, and Breakline serialization;
- XML-schema and semantic validation;
- warning and error reporting; and
- deterministic byte ordering where the format permits it.

It does not own:

- terrain derivation;
- Point Queries;
- file replacement policy;
- CAD automation; or
- a generic exporter registry.

Conceptual interface:

~~~rust
impl LandXml {
    pub fn encode(
        surface: &SurfaceArtifact,
        options: LandXmlOptions,
    ) -> Box<dyn BatchStream<
        Batch = ByteChunk,
        Summary = LandXmlReport,
        Error = LandXmlError,
    >>;
}
~~~

The module emits explicit vertices and faces so consumers receive the derived topology rather than silently retriangulating points. A host adapter that writes a file is responsible for temporary-file creation, consuming the bounded byte stream, flush, sync, and atomic target replacement.

**Independent proof:** encode a stored Terrain Surface fixture into an in-memory byte buffer and validate it without loading any point cloud.

## 12. point-workspace

**Job:** manage the coherent lifecycle of one Source and its revisioned Workspace.

This is the normal caller's deep module for coherent document access. It coordinates lower modules but does not absorb their algorithms. Its depth comes from hiding recovery order, Source/index/Revision compatibility, and Snapshot construction behind a small interface.

It owns:

- opening and validating the Workspace manifest;
- connecting concrete adapters to module interfaces;
- distinct create and reopen Jobs;
- Source registration and verification gates;
- index status and an explicit preparation Job;
- propagation of progress, cancellation, and resource budgets to invoked modules;
- lifecycle ordering for open, index attachment, Query, commit, and automatic manifest persistence; and
- construction of query-ready Snapshots.

It does not own:

- file-format decoding;
- index algorithms;
- Revision storage rules;
- exact query predicates;
- terrain topology;
- LandXML encoding; or
- GPU resources.

Conceptual interface:

~~~rust
pub struct WorkspaceId([u8; 16]);

impl WorkspaceId {
    pub fn as_bytes(&self) -> &[u8; 16];
}

impl Engine {
    pub fn create(
        root: WorkspaceRoot,
        source: SourceCandidate,
        options: CreateOptions,
    ) -> Job<Opened, WorkspaceError>;

    pub fn open(
        root: WorkspaceRoot,
        source: SourceCandidate,
        verification: VerificationPolicy,
        options: OpenOptions,
    ) -> Job<Opened, WorkspaceError>;
}

impl Workspace {
    pub fn identity(&self) -> &WorkspaceId;
    pub fn head(&self) -> Result<Snapshot>;
    pub fn snapshot(&self, revision: RevisionId) -> Result<Snapshot>;
    pub fn index_status(&self) -> IndexStatus;
    pub fn prepare_index(
        &self,
        options: IndexOptions,
    ) -> Job<IndexReady, WorkspaceError>;

    pub fn commit(
        &self,
        operation: OperationId,
        expected: RevisionId,
        edits: EditBatch,
    ) -> Job<CommitOutcome, WorkspaceError>;

    pub fn resolve_operation(
        &self,
        operation: OperationId,
    ) -> Result<CommitResolution>;
}
~~~

Opened contains the Workspace, head Snapshot, and IndexStatus. WorkspaceId is generated at create time, persisted in the manifest, and stable across reopen and directory moves. No Snapshot is exposed during initial Source fingerprinting. No raw Spatial Index or QueryEngine leaks through the Workspace interface.

One v0.1 Workspace binds exactly one immutable Source. SourceId remains part of Point Identity and provenance so future multi-Source design does not require changing Point values, but no current interface accepts a Source collection.

Each Snapshot pins the compatible-index state that existed when it was created. After prepare_index completes, the caller asks the Workspace for a new Snapshot at the same Revision before requesting ViewInput. Existing Snapshots remain valid for exact sequential Queries.

The Workspace interface is the public test surface for coherent Source, index, Revision, and Query lifecycle. The Snapshot type and its Query behavior are owned by **point-query** and returned unchanged by **point-workspace**. An application composes Point Set, View, terrain, LandXML, and rendering modules itself.

**Independent proof:** run it headlessly with memory adapters and no renderer. A CLI and a desktop viewer invoke the same interface.

## 13. render-wgpu

**Job:** maintain and draw one wgpu representation of render-protocol state.

It owns:

- point GPU buffers and explicit Reset, Upsert, and Remove effects;
- point draw and pick shaders, pipelines, depth, and render targets;
- command-encoded uploads and draw ordering;
- point appearance and highlighting;
- exact-resource `RecordedFrame` snapshots for provisional picking; and
- frame timing and logical residency reports.

It does not own:

- Source I/O;
- Spatial Index, hierarchy, loading, LOD, or automatic eviction policy;
- authoritative coordinates;
- exact selection;
- Edits;
- Terrain derivation or mesh rendering;
- Workspace persistence; or
- queue submission, device polling, or device-loss recovery.

Primary interface:

~~~rust
impl WgpuRenderer {
    pub fn new(
        device: &wgpu::Device,
        config: RendererConfig,
    ) -> Result<Self, RendererError>;

    pub fn apply(
        &mut self,
        update: &RenderUpdate,
    ) -> Result<UpdateReport, RendererError>;

    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &Frame,
    ) -> Result<RecordedFrame, RendererError>;

    pub fn pick(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        recorded_frame: &RecordedFrame,
        request: PickRequest,
    ) -> Result<PickTicket, RendererError>;
}

impl PickTicket {
    pub fn poll(&mut self) -> Result<PickPoll, PickError>;
}
~~~

The host owns the device, queue, command encoder, target texture, submission, and
device polling. An explicit Reset clears the previous generation; the renderer
never silently evicts active batches. Upsert and conditional Remove updates use
stable batch keys and exact versions. An update that exceeds its logical point,
byte, or batch limits returns a resource error without changing residency.

Rendering records the currently resident point batches into the host encoder and
returns a `RecordedFrame` whose report describes that generation. Retaining the
recorded frame pins the exact GPU resources it references. Picking records a
one-pixel provisional ID pass against that value and returns a nonblocking
ticket; it never confirms exact Point Set membership. Neither rendering nor
picking performs Source-scale I/O, decoding, indexing, or terrain construction.

**Independent proof:** apply synthetic render-protocol point updates, render
them to an offscreen texture, and poll provisional picks. No Source, Workspace,
**point-view**, or terrain module is required.

## 14. Application adapters

Adapters translate host concepts; they do not reimplement domain behavior.

| Adapter | Its only job | Depends on |
|---|---|---|
| **renderer-demo** | Privately compose synthetic or indexed LAS/LAZ hierarchy materialization with View planning and rendering. | source-las, point-index, point-view, render-protocol, render-wgpu |
| **point-cli** | Translate command-line arguments, progress, and exit codes into module calls. | point-workspace and the Point Set, terrain, or LandXML modules used by the command |
| **viewer-desktop** | Translate window input and application state into Workspace, View, terrain, and renderer calls. | point-workspace, point-set, point-view, terrain-model, render-protocol, render-wgpu |
| **point-python** later | Translate Python values, iteration, exceptions, and cancellation into foundation-module calls. | only the modules exposed by the binding |

The desktop adapter may use a GPU pick as a candidate hint. It creates the durable Point Set only after an exact Query at the frozen frame's Revision.

Terrain Surface display remains part of the deferred platform proposal. A
future adapter may pack host-owned bounded display batches, but the implemented
**render-protocol** and **render-wgpu** contracts expose no mesh batch type.

## Allowed dependencies

The dependency allowlist is stricter than Cargo's ability to compile a graph:

| Module | May depend on |
|---|---|
| point-contracts | standard library and narrow value-type dependencies |
| foundation-runtime | standard library and narrow concurrency dependencies |
| point-source | point-contracts, foundation-runtime |
| source-las, source-memory | point-source, point-contracts, foundation-runtime |
| proposed source-copc | point-source, point-contracts, foundation-runtime |
| point-index | point-contracts, foundation-runtime, point-source |
| point-set | point-contracts, foundation-runtime |
| point-revisions | point-contracts, foundation-runtime, point-set |
| point-query | point-contracts, foundation-runtime, point-source, point-index, point-revisions |
| render-protocol | point-contracts |
| point-view | render-protocol and narrow in-process math/value dependencies |
| terrain-model | point-contracts, foundation-runtime |
| landxml | point-contracts, foundation-runtime, terrain-model |
| point-workspace | point-contracts, foundation-runtime, point-source, point-index, point-revisions, point-query |
| render-wgpu | point-contracts, render-protocol |
| application adapters | only the foundation and output modules directly composed by the host workflow |

In particular:

- no lower module may depend on **point-workspace**;
- no headless module may depend on wgpu or a windowing crate;
- no renderer may inspect private index or Source storage;
- no format adapter may depend on a Workspace; and
- no cycle is accepted to save a small amount of adapter code.

## Public seams and private locality

The implemented foundation exposes five reusable seams:

1. **render-protocol** camera, point-batch, generation, and versioned update
   values;
2. **point-view**'s `ViewPlanner::plan`, which accepts a host-owned hierarchy
   snapshot and returns requests, required retention, and exact retirements;
3. **render-wgpu**'s explicit update and frame interfaces; and
4. the v0.3 **point-source** opaque verified `Source`, `SourceRecord`, and
   bounded `PointBatches` interface; and
5. v0.4 **point-index** `prepare`, `PreparedIndex`, conservative candidate
   planning, immutable hierarchy facts, and display-only node streams.

The Query, Revision, and Workspace seams described elsewhere in this document
remain part of the deferred broader platform proposal. They are not
prerequisites for using the current Source, index, planner, or renderer.

Filesystem storage, scheduling, index page layout, terrain predicates, and journal framing remain private. Publishing those details would reduce locality and freeze decisions before multiple adapters prove a useful seam.
