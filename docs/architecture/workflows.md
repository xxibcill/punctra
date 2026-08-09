# Runtime Workflows

Status: proposed v0.1

These workflows show composition without hidden reverse calls. A module invokes only an allowed dependency. Application adapters coordinate sibling modules when no lower module should own the whole workflow.

## 1. Open a Workspace

Opening an existing Workspace is a cancellable Job. It returns Opened after Source metadata, Source Identity verification at the requested policy, Revision recovery, and index compatibility are known. Opened contains the head Snapshot and an explicit IndexStatus.

~~~mermaid
sequenceDiagram
    participant APP as Host adapter
    participant WS as point-workspace
    participant SRC as point-source
    participant REV as point-revisions
    participant IDX as point-index

    APP->>WS: Engine::open(manifest, source adapter, verification policy)
    WS->>SRC: verify(Recorded(SourceRecord), policy)
    SRC-->>WS: VerifiedSource(reader, SourceRecord, level)
    WS->>REV: open_and_recover(target, Revision Source Contract)
    REV-->>WS: durable head Revision
    WS->>IDX: open_index(target, expected Source Identity)

    alt compatible Spatial Index exists
        IDX-->>WS: complete index
    else missing or incompatible index
        IDX-->>WS: IndexStatus::Missing
    end

    WS-->>APP: Opened(Workspace, head Snapshot, IndexStatus)
    opt host needs a View
        APP->>WS: prepare_index(options)
        WS-->>APP: Job of IndexReady
        APP->>WS: snapshot(head Revision)
        WS-->>APP: new index-ready Snapshot
    end
~~~

Rules:

- a changed Source is rejected according to the requested Fast or Full verification policy before affected Point values are returned;
- recovery exposes a complete prior or new Revision;
- a missing derived index is not Workspace corruption;
- Fast reopen performs no total-Source scan; Full reopen does; and
- no GPU or window is involved.

Creating a new Workspace has one additional gate: a complete content fingerprint establishes Source Identity before Opened or any Snapshot is returned. The Workspace derives a Revision Source Contract from verified Source metadata, persists Workspace Identity in its manifest, and invokes RevisionStore::create. v0.1 does not expose provisional View data or provisional Point Identity.

### Standalone use

An index tool can bypass **point-workspace**:

~~~rust
let candidate = source_las::open_candidate(path)?;
let verified = candidate
    .verify(SourceExpectation::New, VerificationPolicy::Full)
    .await?;
let index = point_index::IndexBuilder::build_or_resume(
    verified.source,
    index_target,
    index_options,
).await?;
~~~

This is the required independence proof for Source and index modules.

## 2. Build or resume a Spatial Index

~~~mermaid
sequenceDiagram
    participant APP as Host adapter
    participant IDX as point-index
    participant SRC as point-source

    APP->>IDX: build_or_resume(verified PointSource, target, options)
    IDX->>IDX: verify checkpoint frames

    loop bounded Source spans
        IDX->>SRC: read(spans, position fields, budget)
        SRC-->>IDX: Point Batches
        IDX->>IDX: partition and summarize
        IDX->>IDX: write checksummed checkpoint
        IDX-->>APP: monotonic progress
    end

    IDX->>IDX: finalize hierarchy atomically
    IDX-->>APP: Index Artifact and build report
~~~

Cancellation leaves only verified checkpoints. Resume starts at the last verified checkpoint. Source record order remains the identity authority even if the index stores a different spatial order.

v0.1 builds the same foundation index for LAS, LAZ, and COPC. Importing a native hierarchy is deferred until a second real producer proves that seam.

## 3. Run an exact Query

~~~mermaid
sequenceDiagram
    participant APP as Host adapter
    participant QRY as point-query
    participant IDX as point-index
    participant SRC as point-source
    participant REV as point-revisions

    APP->>QRY: Snapshot.query(Point Query)

    alt complete compatible index
        QRY->>IDX: exact_candidates(Region)
        IDX-->>QRY: bounded conservative Source-span stream
    else index missing or building
        QRY->>SRC: sequential all-Point scan
    end

    loop bounded candidate spans
        QRY->>SRC: read(span, requested fields)
        SRC-->>QRY: canonical Point Batch
        QRY->>REV: overlays(Snapshot, Point Identities)
        REV-->>QRY: sparse Attribute patches
        QRY->>QRY: apply overlays and exact predicates
        QRY-->>APP: ordered exact Point Batch
    end

    QRY-->>APP: Complete(ExactPointSummary)
~~~

If no complete index is available, the Query uses a slower sequential scan. It never substitutes a partial index result. Point Queries are always exact and complete in v0.1; partial Coverage belongs only to Views.

Concurrent commits do not affect the pinned Snapshot.

## 4. Prepare and render a View

View preparation is a bounded stream and GPU drawing is synchronous. A View cannot begin until Snapshot.view_input confirms a complete compatible Spatial Index.

~~~mermaid
sequenceDiagram
    participant DESK as Desktop adapter
    participant VIEW as point-view
    participant IDX as point-index
    participant QRY as point-query
    participant GPU as render-wgpu

    DESK->>QRY: Snapshot.view_input()
    QRY-->>DESK: opaque index-ready ViewInput
    DESK->>VIEW: prepare(ViewInput, frozen View specification)
    VIEW->>IDX: hierarchy(roots and children)
    IDX-->>VIEW: bounds, counts, spans, and geometric errors
    VIEW->>VIEW: apply camera, screen-error, priority, and point-budget policy
    VIEW->>QRY: materialize planned samples at Snapshot
    VIEW-->>DESK: Reset(FrameToken)
    DESK->>GPU: apply Reset

    loop progressive refinement
        QRY-->>VIEW: authoritative Point Batches
        VIEW->>VIEW: choose world origin and pack display values
        VIEW-->>DESK: Upsert or Remove delta with Coverage
        DESK->>GPU: apply(RenderDelta)
        DESK->>GPU: render(target, FrameToken)
        GPU-->>DESK: Frame report for the same generation
    end

    VIEW-->>DESK: Complete(ViewSummary)
~~~

Rules:

- **point-view** owns LOD policy and renderer-neutral display packing;
- **render-wgpu** owns only GPU residency and drawing;
- Reset, stable batch keys, and explicit replacement prevent mixed cameras or Revisions;
- render never waits for Source I/O, decompression, indexing, or terrain construction;
- a frame reports the Snapshot Revision and Coverage it represents;
- one View Batch uses one 64-bit world origin and 32-bit relative display positions; and
- deleting all GPU resources cannot alter Workspace state.

### Standalone use

**render-wgpu** can render generated render-protocol deltas in an offscreen test. **point-view** can produce deltas for the CPU RenderStateModel or a file inspector. Neither requires the other to pass its own conformance tests.

## 5. Select Points and commit an Edit

A GPU pick is a hint. The exact Point Set is resolved by **point-query** against the frozen Snapshot and View context.

~~~mermaid
sequenceDiagram
    participant DESK as Desktop adapter
    participant GPU as render-wgpu
    participant QRY as point-query
    participant SET as point-set
    participant WS as point-workspace
    participant REV as point-revisions

    DESK->>GPU: pick_candidates(screen polygon, FrameToken)
    GPU-->>DESK: provisional candidates and resident Coverage
    DESK->>QRY: exact screen-through Query(frozen camera and Snapshot)
    QRY-->>DESK: bounded exact Point stream
    DESK->>SET: materialize(exact stream, budget)
    SET->>QRY: pull bounded Point Batches to Complete
    SET-->>DESK: immutable spillable PointSetHandle
    DESK->>DESK: choose Operation Identity and retain it with Workspace Identity
    DESK->>WS: commit(Operation Identity, expected Revision, Edit Batch)
    WS->>REV: stage full payload, then compare-and-swap commit

    alt committed
        REV-->>WS: Committed(new Revision)
        WS-->>DESK: committed outcome
    else rejected
        REV-->>WS: Rejected(reason and actual head)
        WS-->>DESK: rejected; no state changed
    else acknowledgement failed near commit point
        REV-->>WS: Indeterminate(Operation Identity)
        WS-->>DESK: Indeterminate(Operation Identity)
        DESK->>WS: reopen, then resolve_operation(Operation Identity)
        WS-->>DESK: committed, rejected, or not-recorded resolution
    end
~~~

Before the logical commit point, **point-revisions** consumes the process-scoped Point Set and durably stages the complete canonical Edit payload in bounded batches. A crash during staging cannot create a Revision. NotRecorded means no operation record exists and therefore no Revision was created; the host closes that recovery record rather than trying to reconstruct an expired Point Set. Reusing a recorded identity with different content is Rejected.

The exact v0.1 screen rule is through-selection: CPU projection includes every matching Point regardless of occlusion, including the polygon boundary. Pick candidates are only immediate feedback. The Edit targets stable Point Identities, not coordinates or transient node offsets. A Point Set from a different Revision is rejected rather than silently rebased.

## 6. Derive a Terrain Surface

The host adapter composes Query and terrain modules. **terrain-model** never opens a Workspace or reaches upward into **point-query**.

~~~mermaid
sequenceDiagram
    participant APP as Host adapter
    participant WS as point-workspace
    participant QRY as point-query
    participant TER as terrain-model

    APP->>WS: snapshot(Revision)
    WS-->>APP: immutable Snapshot
    APP->>QRY: Snapshot.query(terrain Point Query)
    APP->>QRY: Snapshot.breaklines(terrain Region)
    QRY-->>APP: two bounded streams with identical Snapshot provenance
    APP->>TER: derive(TerrainInput, normalized Recipe, limits)
    TER->>TER: thin candidates
    TER->>TER: normalize constraints
    TER->>TER: deterministic constrained triangulation
    TER->>TER: validate topology
    TER-->>APP: immutable Terrain Surface and diagnostics
~~~

This composition keeps the terrain module independently useful:

~~~rust
let points = snapshot.query(terrain_query);
let breaklines = snapshot.breaklines(terrain_region);
let surface = terrain_model::derive(
    TerrainInput::snapshot(snapshot.provenance(), points, breaklines),
    recipe,
    limits,
).await?;
~~~

The terrain module verifies both terminal stream summaries before publishing the Artifact. The result records Artifact Identity, Source Identity, Revision, Coordinate Reference, normalized Recipe, algorithm version, input digest, and TerrainLimits. A later Revision makes the prior Terrain Surface stale; it does not mutate that Artifact.

## 7. Export LandXML

~~~mermaid
sequenceDiagram
    participant APP as Host adapter
    participant XML as landxml
    participant FS as Atomic file adapter

    APP->>FS: create temporary sibling
    APP->>XML: encode(Terrain Surface, options)
    XML->>XML: validate topology and export semantics
    loop bounded byte chunks
        XML-->>APP: ByteChunk
        APP->>FS: write chunk
    end
    XML-->>APP: Complete(LandXmlReport)

    alt validation and write succeed
        APP->>FS: flush, sync, and atomically replace
        FS-->>APP: committed destination
    else any failure
        APP->>FS: discard temporary sibling
        APP-->>APP: destination remains unchanged
    end
~~~

The **landxml** module emits a bounded byte stream. Path replacement belongs to the host's file adapter, so encoding can be tested in memory and reused by bindings.

## 8. Cancellation and crash recovery

| Operation | Safe cancellation point | Permitted residue | Durable state after failure |
|---|---|---|---|
| Source read | Between decode blocks or Point Batches | None | Unchanged |
| Index build | After a checksummed checkpoint | Verified checkpoint | Previous complete index or no index |
| Query | Between Point Batches | Disposable read cache | Unchanged |
| View preparation | Between View Batches | Disposable view cache | Unchanged |
| Revision commit | Before the durable commit point | Caller-retained Operation Identity plus journal-owned canonical digest and staged payload | Rejected old head, Committed new head, or Indeterminate until reconciled |
| Terrain derivation | Between deterministic phases or partitions | Verified disposable artifact blocks | Unchanged |
| LandXML export | Before atomic target replacement | Temporary sibling | Previous destination or new complete destination |
| GPU frame | At frame end or device loss | Disposable GPU allocations | Workspace unchanged |

Recovery order is:

1. verify the Source Identity;
2. recover the Revision journal;
3. open or reject the Spatial Index;
4. discard invalid derived cache entries;
5. expose the complete head Snapshot;
6. reconcile any Indeterminate Operation Identity and close caller recovery records resolved as Committed, Rejected, or NotRecorded; and
7. resume explicitly requested background Jobs.

No recovery path guesses missing identity, Coordinate Reference, or Revision information.

## 9. Staleness propagation

Artifacts are immutable. Staleness is metadata, not in-place invalidation:

~~~mermaid
flowchart LR
    S0["Snapshot at Revision 10"] --> T0["Terrain Surface at Revision 10"]
    T0 --> X0["LandXML export at Revision 10"]
    E1["Edit commit"] --> S1["Snapshot at Revision 11"]
    S1 -. "does not mutate" .-> T0
    S1 --> T1["Optional new Terrain Surface"]
~~~

Staleness is host-owned metadata in v0.1; the Workspace does not persist an Artifact catalog. An application may continue displaying the prior complete Terrain Surface while a new one is derived, but it must label the old Revision and use distinct render-protocol generations. One rendered frame never mixes mesh batches from different Revisions.
