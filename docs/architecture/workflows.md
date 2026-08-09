# Runtime Workflows

Status: deferred platform proposal; the v0.1 renderer, v0.2 adaptive View,
v0.3 Real Sources, and narrow v0.4 Spatial Index/View composition are
implemented under the accepted designs in [`docs/design`](../design)

These workflows show composition without hidden reverse calls. A module invokes only an allowed dependency. Application adapters coordinate sibling modules when no lower module should own the whole workflow.

## 1. Open a Workspace

Opening an existing Workspace remains proposed. The future cancellable Job
would return only after Source metadata, Source Identity verification at the
requested policy, and Revision recovery are known. Index preparation remains an
explicit host request through the implemented `point_index::prepare` operation.

~~~mermaid
sequenceDiagram
    participant APP as Host adapter
    participant WS as point-workspace
    participant SRC as point-source
    participant REV as point-revisions
    participant IDX as point-index

    APP->>WS: Engine::open(manifest, source adapter, verification policy)
    WS->>SRC: candidate.open(match_record(SourceRecord, policy))
    SRC-->>WS: opaque verified Source
    WS->>REV: open_and_recover(target, Revision Source Contract)
    REV-->>WS: durable head Revision
    WS-->>APP: Opened(Workspace, head Snapshot)
    opt host needs a View
        APP->>WS: prepare_index(options)
        WS->>IDX: prepare(verified Source, target, PrepareLimits)
        IDX-->>WS: complete PreparedIndex
        WS-->>APP: IndexReady
        APP->>WS: snapshot(head Revision)
        WS-->>APP: new index-ready Snapshot
    end
~~~

Rules:

- a changed Source is rejected according to the requested Fast or Full verification policy before affected Point values are returned;
- recovery exposes a complete prior or new Revision;
- a missing derived index is not Workspace corruption and is prepared only on
  an explicit host request;
- Fast reopen performs no total-Source scan; Full reopen does; and
- no GPU or window is involved.

Creating a new Workspace has one additional gate: a complete content fingerprint establishes Source Identity before Opened or any Snapshot is returned. The Workspace derives a Revision Source Contract from verified Source metadata, persists Workspace Identity in its manifest, and invokes RevisionStore::create. v0.1 does not expose provisional View data or provisional Point Identity.

### Standalone use

An index tool can bypass **point-workspace**:

~~~rust
let source = source_las::open(path).await?;
let index = point_index::prepare(
    source,
    index_target,
    PrepareLimits::default(),
).await?;
~~~

This is the required independence proof for Source and index modules.

## 2. Build or resume a Spatial Index

~~~mermaid
sequenceDiagram
    participant APP as Host adapter
    participant IDX as point-index
    participant SRC as point-source

    APP->>IDX: prepare(verified Source, target, PrepareLimits)

    alt compatible complete target exists
        IDX->>IDX: verify Source/version binding, layout, topology, and checksums
        IDX-->>APP: PreparedIndex(Opened, zero Source Points read)
    else target is absent
        IDX->>IDX: verify/create append-only work and recover valid frame prefix

        loop missing 65,536-Point Source blocks
            IDX->>SRC: read(one span, positions only, bounded budget)
            SRC-->>IDX: exact Point Batches and terminal summary
            IDX->>IDX: bounds + bounded hash-selected samples
            IDX->>IDX: append and sync checksummed frame
            IDX-->>APP: monotonic durable-Point progress
        end

        IDX->>IDX: deterministic BVH + bounded child-sample merges
        IDX->>IDX: write and sync complete temporary artifact
        IDX->>IDX: no-replace hard-link target + sync parent
        IDX->>IDX: remove disposable siblings + sync parent
        IDX-->>APP: PreparedIndex(Built or Resumed, report)
    end
~~~

Cancellation leaves only verified work frames and never a partial complete
target. Resume starts at the last verified ordinal-contiguous frame. Existing,
incompatible, corrupt, or racing targets fail without replacement. Source
record order remains the identity authority even though the hierarchy groups
Source blocks spatially.

The implemented Spatial Index uses the same canonical Source seam for memory
and LAS/LAZ adapters. A future COPC adapter can use that seam, but native
hierarchy import remains deferred until a second real producer proves it.

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
        QRY->>IDX: candidates(Region bounds, CandidateLimits)
        IDX-->>QRY: complete conservative CandidatePlan
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

Adaptive planning is one synchronous, renderer-neutral CPU operation. The host
owns hierarchy acquisition, node materialization, scheduling, renderer updates,
command submission, and device polling. **point-view** does not require a
Snapshot, Spatial Index, or `ViewInput`. The implemented private real-cloud
bridge derives its `AvailableNodes` snapshot from `PreparedIndex`; other hosts
may continue to supply unrelated hierarchies.

~~~mermaid
sequenceDiagram
    participant HOST as Host adapter
    participant IDX as point-index
    participant VIEW as point-view
    participant GPU as render-wgpu

    HOST->>GPU: apply(RenderUpdate::Reset)

    loop camera, viewport, or residency change
        HOST->>VIEW: plan(Camera, viewport, AvailableNodes, PlanningBudget)
        VIEW-->>HOST: ViewPlan(demanded nodes, requests, retention, retirements)

        HOST->>HOST: prune queued Requested nodes no longer demanded

        loop safe conditional retirements
            HOST->>GPU: apply(RenderUpdate::Remove)
        end

        opt one prioritized requested node fits host staging
            HOST->>IDX: read_node(IndexNodeId, NodeReadBudget)
            IDX-->>HOST: display batches + exact terminal summary
            HOST->>HOST: exact ticks to origin-relative render points
            HOST->>GPU: apply(one complete RenderUpdate::Upsert)
            GPU-->>HOST: accepted or rejected
        end

        HOST->>GPU: render(encoder, target, Frame)
        GPU-->>HOST: RecordedFrame and FrameReport
    end
~~~

Rules:

- **point-view** owns culling, screen-error LOD, hysteresis, budget planning,
  Coverage retention, and safe retirement decisions;
- `ViewPlan::demanded_nodes()` includes current nonresident targets already
  Requested, while `requests()` remains only the new-load delta;
- the host owns Point Batch materialization, origin-relative display packing,
  request execution, and application of every renderer update;
- **render-wgpu** owns bounded GPU point residency, command recording, and
  provisional picking, but not automatic eviction;
- Reset, stable batch keys, increasing versions, and conditional removal prevent
  stale generations or plans from replacing newer data;
- render never waits for Source I/O, decompression, indexing, or terrain construction;
- the host retains any Snapshot or Revision provenance associated with its View
  generation; neither `ViewPlan` nor `Frame` claims that provenance;
- one `PointBatch` uses one 64-bit world origin and 32-bit relative display
  positions; and
- the real bridge marks a node Resident only after an accepted complete Upsert,
  assigns monotonically increasing versions to retries, and retains parent
  Coverage until the planner emits its exact conditional retirement; and
- deleting all GPU resources cannot alter Workspace state.

### Standalone use

**render-wgpu** can apply generated render-protocol updates and render them in
an offscreen test. **point-view** can plan directly from generated hierarchy
metadata and inspect `ViewPlan` without creating a GPU device.
`renderer-demo --smoke SOURCE [INDEX_TARGET]` Full-verifies LAS/LAZ, prepares
the index, plans one node, and accepts one atomic CPU-model Upsert without a
GPU. The modules do not require each other at runtime to pass their own
conformance tests.

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

    DESK->>GPU: pick(encoder, RecordedFrame, PickRequest)
    GPU-->>DESK: nonblocking PickTicket
    DESK->>DESK: submit encoder, drive device polling, poll provisional PickHit
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
| Index prepare | After a synced checksummed work frame; before no-replace publication | Verified work prefix and disposable temporary/spool files | Existing target unchanged, or one complete newly linked target |
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
