# Module Catalog

Status: deferred platform proposal; render-engine v0.1 is defined in
[the current design](../design/render-engine-v0.1.md)

This document is the ownership map. Each crate below is one logical module with one job. The job sentence is normative: if new behavior does not fit it, that behavior does not belong in the module.

“Individually usable” means a caller can invoke the module's interface directly with canonical inputs. It does not mean that every module has zero dependencies.

## Catalog

| Module | Its only job | Canonical input | Canonical output |
|---|---|---|---|
| **point-contracts** | Define and validate lossless Point values and stable spatial provenance. | None | IDs, Point Batches, spatial metadata, provenance |
| **foundation-runtime** | Standardize bounded execution control for long foundation operations. | Operation closure or producer | Jobs, batch streams, budgets, progress, cancellation |
| **point-source** | Provide verified bounded canonical read access to one immutable Source. | Source candidate, expectation, verification policy, and read request | VerifiedSource, SourceRecord, and Point Batches |
| **point-index** | Provide a rebuildable persistent mapping from spatial requests to candidate Source ranges. | PointSource, index target, expectation, and build options | Complete Spatial Index and bounded read plans |
| **point-set** | Materialize exact Point Identities as immutable bounded-memory Point Sets. | Exact Point Batch stream with terminal Snapshot provenance | Spillable Point Set handle |
| **point-revisions** | Persist sparse Edits and resolve immutable Revision state. | Revision target, Operation Identity, expected Revision, and Edit Batch | Commit resolution and Revision view |
| **point-query** | Provide bounded revision-pinned reads from one Snapshot. | Source, Spatial Index, Revision view, read request | Snapshot, exact Point or Breakline streams, ViewInput |
| **render-protocol** | Represent generation-safe renderer-neutral point and mesh updates. | Point or mesh display data | Validated frame tokens and render deltas |
| **point-view** | Turn one frozen View into progressive renderer-neutral View updates. | Query-ready Snapshot and View specification | View-update stream and summary |
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

pub struct Job<T> { /* opaque */ }

impl<T> Future for Job<T> {
    type Output = Result<T>;
}

pub enum StreamEvent<T, S> {
    Batch(T),
    Progress(Progress),
    Complete(S),
}

pub trait BatchStream<T, S> {
    fn next(
        &mut self,
        cancel: &CancellationToken,
    ) -> Result<Option<StreamEvent<T, S>>>;
}
~~~

It uses standard Future and Waker contracts rather than requiring Tokio or another host runtime.

**Independent proof:** drive synthetic Jobs and streams through success, cancellation, budget exhaustion, blocking wait, asynchronous wait, and fused terminal behavior without loading point data.

## 3. point-source

**Job:** provide verified bounded canonical read access to one immutable Source.

The module defines the proven Source seam and common validation. Format behavior lives in adapter crates:

- **source-las** decodes LAS and LAZ record order, headers, VLRs, EVLRs, and Attributes;
- **source-copc** decodes local COPC hierarchy order, byte ranges, and Attributes; and
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
pub trait SourceCandidate: Send + Sync {
    fn preview(&self) -> &SourcePreview;
    fn verify(
        self: Arc<Self>,
        expectation: SourceExpectation,
        policy: VerificationPolicy,
    ) -> Job<VerifiedSource>;
}

pub struct VerifiedSource {
    pub source: Arc<dyn PointSource>,
    pub record: SourceRecord,
    pub level: VerificationLevel,
}

pub trait PointSource: Send + Sync {
    fn identity(&self) -> &SourceId;
    fn metadata(&self) -> &SourceMetadata;
    fn read(
        &self,
        spans: &[SourceSpan],
        fields: FieldMask,
        budget: ReadBudget,
    ) -> Box<dyn BatchStream<PointBatch, SourceReadSummary>>;
}
~~~

PointSource is always verified. SourceRecord is a versioned, serializable value owned by **point-source**; it binds Source Identity to the Full fingerprint and the adapter-specific facts required for Fast verification. SourceExpectation is either New or Recorded(SourceRecord). VerifiedSource returns the reader, the record safe to persist for later reopen, and the achieved VerificationLevel. Engine::create forces Full verification with New; Engine::open supplies Recorded from the manifest and the requested Fast or Full policy. No Workspace Snapshot can hold SourceCandidate.

The logical ordinal is part of the adapter contract:

- LAS and LAZ use point-record order.
- COPC uses canonical hierarchy-key order followed by record offset within the node.

An index may reorder storage for speed, but it must carry the original Point Identity.

**Independent proof:** open a fixture through one adapter, request record spans, and receive the same canonical Point Batches without constructing a Workspace or Spatial Index.

## 4. point-index

**Job:** provide a rebuildable persistent mapping from spatial requests to candidate Source ranges.

It owns:

- resumable index construction for Sources without a useful hierarchy;
- spatial node bounds, point counts, Source spans, and geometric-error summaries;
- checksummed index persistence and recovery; and
- deterministic conservative Region traversal and hierarchy reads.

It does not own:

- decoding point records;
- exact attribute filtering;
- Revision overlays;
- authoritative Point Identity;
- camera rendering; or
- Source bytes.

Conceptual interface:

~~~rust
pub enum IndexOpen {
    Ready(IndexArtifact),
    Missing,
    Incompatible(IndexMismatch),
}

pub fn open_index(
    target: IndexTarget,
    expectation: IndexExpectation,
) -> Job<IndexOpen>;

impl IndexBuilder {
    pub fn build_or_resume(
        source: Arc<dyn PointSource>,
        target: IndexTarget,
        options: IndexOptions,
    ) -> Job<IndexArtifact>;
}

impl IndexArtifact {
    pub fn descriptor(&self) -> &IndexDescriptor;

    pub fn exact_candidates(
        &self,
        region: &Region,
    ) -> Result<Box<dyn BatchStream<SourceSpanBatch, ExactPlanSummary>>>;

    pub fn hierarchy(
        &self,
        request: HierarchyRequest,
    ) -> Result<IndexNodeBatch>;
}
~~~

open_index is the only public reader of the persisted index representation. It verifies identity, schema, completeness, and checksums before returning Ready. IndexDescriptor exposes immutable Artifact Identity, Source Identity, Source point count, build options, and index schema version without exposing nodes or pages. Missing and Incompatible are explicit rebuildable states; corrupt or interrupted data is never returned as a partial IndexArtifact.

An exact read plan is a bounded stream of conservative candidate-span batches. The Query module performs exact spatial and Attribute tests. False positives are allowed; false negatives are not. An incomplete index returns IndexIncomplete instead of an exact plan.

The builder reads stable logical Source spans through PointSource and checkpoints the next ordinal, builder state, and Source Identity. Calling build_or_resume after restart verifies the checkpoint and resumes the scan. v0.1 deliberately builds the same foundation index for COPC; native-hierarchy import waits for a second real producer before gaining a seam.

The index exposes hierarchy facts only. **point-view** owns camera culling, screen error, point budgets, priority, and refinement policy.

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
    points: impl BatchStream<PointBatch, ExactPointSummary>,
    budget: PointSetBudget,
) -> Job<PointSetHandle>;

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
    ) -> Job<Self>;

    pub fn open_and_recover(
        target: RevisionTarget,
        expected_source: RevisionSourceContract,
        options: RevisionOptions,
    ) -> Job<Self>;

    pub fn source_contract(&self) -> &RevisionSourceContract;
    pub fn head(&self) -> RevisionId;
    pub fn view(&self, revision: RevisionId) -> Result<RevisionView>;

    pub fn commit(
        &self,
        operation: OperationId,
        expected_head: RevisionId,
        edits: EditBatch,
    ) -> Job<CommitOutcome>;

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
        source: Arc<dyn PointSource>,
        index: Option<Arc<IndexArtifact>>,
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
    ) -> Box<dyn BatchStream<PointBatch, ExactPointSummary>>;

    pub fn breaklines(
        &self,
        region: Region,
    ) -> Box<dyn BatchStream<BreaklineBatch, ExactBreaklineSummary>>;

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
    ) -> Box<dyn BatchStream<PointBatch, ViewReadSummary>>;
}
~~~

QueryEngine::new verifies that the PointSource metadata, optional IndexArtifact, and RevisionStore carry the same Source Identity and that Source point count, editable Attribute schema, and Coordinate Reference equal the persisted Revision Source Contract. It rejects the composition before any Snapshot is created. pin accepts only a Revision Identity and obtains its RevisionView from that validated store, so a view from another store cannot be injected.

A Query uses the already pinned Snapshot. Concurrent commits cannot change its output. Point Queries are exact and complete in v0.1; partial Coverage belongs to Views.

When no compatible index exists, a complete Query may sequentially scan the Source. A View requires a compatible index because a full scan cannot satisfy an interactive LOD contract.

ViewInput is revision-pinned and opaque: it exposes immutable Snapshot provenance, hierarchy facts, and bounded materialization of selected node keys while hiding Source, Spatial Index, and Revision-store ownership. It always applies the same Snapshot overlays as an exact Query. **point-view** compares ViewSpec's FrameToken with that provenance before emitting Reset.

**Independent proof:** use **source-memory** with real index and Revision stores in temporary directories, then compare streamed results against a simple sequential oracle.

## 8. render-protocol

**Job:** represent generation-safe renderer-neutral point and mesh updates.

It owns:

- frozen FrameToken and ViewGenerationKey values;
- stable point-batch and mesh-batch keys;
- origin-relative display columns;
- Reset, Upsert, Remove, and replacement semantics;
- progressive Coverage values;
- renderer-neutral mesh batches; and
- validation that one delta cannot mix View identity, generation, or Revision.

It does not own:

- LOD or camera policy;
- Source or Snapshot access;
- GPU allocation or drawing;
- Terrain Surface derivation; or
- exact selection.

Conceptual interface:

~~~rust
pub struct MeshBatch {
    pub view_generation: ViewGenerationKey,
    pub key: MeshBatchKey,
    pub artifact: ArtifactId,
    pub world_origin: [f64; 3],
    pub relative_positions: BoundedVec<[f32; 3]>,
    pub triangle_indices: BoundedVec<[u32; 3]>,
}

pub enum RenderDelta {
    Points(ViewDelta),
    UpsertMesh {
        view_generation: ViewGenerationKey,
        batch: MeshBatch,
    },
    RemoveMesh {
        view_generation: ViewGenerationKey,
        key: MeshBatchKey,
    },
}

impl RenderStateModel {
    pub fn apply(&mut self, delta: &RenderDelta) -> Result<()>;
    pub fn active_frame(&self) -> Option<&FrameToken>;
}
~~~

RenderStateModel is a small CPU reference for protocol validation; it owns no GPU resources. The renderer and tests use the same transition rules.

**Independent proof:** apply generated Reset, Upsert, replacement, removal, stale-generation, and mixed-Revision sequences and verify the resulting abstract resident-key set without creating a GPU.

## 9. point-view

**Job:** turn one frozen View into progressive renderer-neutral View updates.

It owns:

- frustum and screen-error planning;
- point-budget allocation;
- stable LOD priority and refinement;
- choosing a floating world origin;
- converting authoritative positions to origin-relative display values;
- progressive Coverage; and
- renderer-neutral picking identities.

It does not own:

- GPU buffers, shaders, or device state;
- exact selection membership;
- Source decoding;
- Edits;
- Terrain Surface derivation; or
- persistent caches outside its own disposable view data.

Conceptual interface:

~~~rust
impl ViewPreparer {
    pub fn prepare(
        input: ViewInput,
        spec: ViewSpec,
    ) -> Box<dyn BatchStream<ViewDelta, ViewSummary>>;
}
~~~

ViewInput is an opaque, index-ready capability created by **point-query**. View preparation fails with IndexIncomplete until the Spatial Index is complete. View ordering is deterministic for a normalized request even if worker completion order is not. A displayed Point is a sample, not proof that all relevant Points have been considered.

ViewSpec's FrameToken Revision must equal ViewInput's pinned Revision. A mismatch fails before Reset is emitted.

**Independent proof:** run camera and LOD fixtures through **source-memory** and the real Query interface, then inspect View Batches without creating a GPU device.

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
) -> Job<SurfaceArtifact>;

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
    ) -> Box<dyn BatchStream<ByteChunk, LandXmlReport>>;
}
~~~

The module emits explicit vertices and faces so consumers receive the derived topology rather than silently retriangulating points. A host adapter that writes a file is responsible for temporary-file creation, consuming the bounded byte stream, flush, sync, and atomic rename.

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
        source: Arc<dyn SourceCandidate>,
        options: CreateOptions,
    ) -> Job<Opened>;

    pub fn open(
        root: WorkspaceRoot,
        source: Arc<dyn SourceCandidate>,
        verification: VerificationPolicy,
        options: OpenOptions,
    ) -> Job<Opened>;
}

impl Workspace {
    pub fn identity(&self) -> &WorkspaceId;
    pub fn head(&self) -> Result<Snapshot>;
    pub fn snapshot(&self, revision: RevisionId) -> Result<Snapshot>;
    pub fn index_status(&self) -> IndexStatus;
    pub fn prepare_index(&self, options: IndexOptions) -> Job<IndexReady>;

    pub fn commit(
        &self,
        operation: OperationId,
        expected: RevisionId,
        edits: EditBatch,
    ) -> Job<CommitOutcome>;

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

- GPU buffer allocation and eviction;
- shaders and render pipelines;
- uploads and draw ordering;
- point appearance and highlighting;
- mesh vertex and index uploads;
- isolation of View identity and generation;
- frame timing and residency-pressure reports; and
- device-loss recovery.

It does not own:

- Source I/O;
- Spatial Index or LOD policy;
- authoritative coordinates;
- exact selection;
- Edits;
- Terrain derivation; or
- Workspace persistence.

Conceptual interface:

~~~rust
impl WgpuRenderer {
    pub fn attach(
        device: wgpu::Device,
        queue: wgpu::Queue,
        options: RenderOptions,
    ) -> Result<Self>;

    pub fn apply(&mut self, update: RenderDelta) -> Result<()>;

    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        frame: &FrameToken,
    ) -> Result<FrameReport>;

    pub fn pick_candidates(
        &self,
        frame: &FrameToken,
        region: ScreenRegion,
    ) -> Result<PickHint>;
}
~~~

The renderer rejects updates from a different View identity or generation until it receives an explicit Reset. Upsert and Remove deltas replace LOD batches by stable batch key. PickHint is provisional and reports resident Coverage; it can provide immediate feedback but can never be committed as an exact Point Set.

The renderer may evict inactive generations after Reset. It never silently evicts an active generation below the protocol state; an upload that exceeds its GPU budget returns ResourceLimit and residency pressure so the host can request a lower-budget new View generation.

Rendering uses whatever point and mesh batches are resident for the requested FrameToken and returns within a frame budget. It never performs Source-scale I/O, LAZ decoding, indexing, or terrain construction synchronously.

**Independent proof:** render synthetic render-protocol point and mesh deltas to an offscreen texture. No Source, Workspace, point-view, or terrain module is required.

## 14. Application adapters

Adapters translate host concepts; they do not reimplement domain behavior.

| Adapter | Its only job | Depends on |
|---|---|---|
| **point-cli** | Translate command-line arguments, progress, and exit codes into module calls. | point-workspace and the Point Set, terrain, or LandXML modules used by the command |
| **viewer-desktop** | Translate window input and application state into Workspace, View, terrain, and renderer calls. | point-workspace, point-set, point-view, terrain-model, render-protocol, render-wgpu |
| **point-python** later | Translate Python values, iteration, exceptions, and cancellation into foundation-module calls. | only the modules exposed by the binding |

The desktop adapter may use a GPU pick as a candidate hint. It creates the durable Point Set only after an exact Query at the frozen frame's Revision.

For Terrain Surface display, the desktop adapter reads the Artifact Identity plus bounded vertex and face ranges from SurfaceArtifact and packs renderer-neutral MeshBatch values. This remains adapter code while there is one caller; a separate seam is earned only when a second producer needs the same policy.

## Allowed dependencies

The dependency allowlist is stricter than Cargo's ability to compile a graph:

| Module | May depend on |
|---|---|
| point-contracts | standard library and narrow value-type dependencies |
| foundation-runtime | standard library and narrow concurrency dependencies |
| point-source | point-contracts, foundation-runtime |
| source-las, source-copc, source-memory | point-source, point-contracts, foundation-runtime |
| point-index | point-contracts, foundation-runtime, point-source |
| point-set | point-contracts, foundation-runtime |
| point-revisions | point-contracts, foundation-runtime, point-set |
| point-query | point-contracts, foundation-runtime, point-source, point-index, point-revisions |
| render-protocol | point-contracts |
| point-view | point-contracts, foundation-runtime, render-protocol, point-index, point-query |
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

Only these seams are intended for third-party adapters in v0.1:

1. the **SourceCandidate/PointSource** seam, because LAS/LAZ, COPC, and memory adapters prove it;
2. bounded stream and render-protocol values, because multiple producers and consumers already exist; and
3. the high-level Workspace interface, because CLI, desktop, and future language adapters share it.

Filesystem storage, scheduling, index page layout, terrain predicates, and journal framing remain private. Publishing those details would reduce locality and freeze decisions before multiple adapters prove a useful seam.
