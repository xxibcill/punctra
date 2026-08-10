# Runtime Workflows

Status: implemented through v0.5; terrain and export workflows deferred

The host composes sibling modules explicitly. Lower crates never call back into
an application, discover a Source for a Workspace, submit a GPU queue, or infer
recovery policy.

## 1. Verify a Source and prepare an index

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant LAS as source-las
    participant IDX as point-index
    participant SRC as point-source

    HOST->>LAS: open(path)
    LAS-->>HOST: verified Source Job result
    HOST->>IDX: prepare(Source, target, PrepareLimits)

    alt compatible complete target exists
        IDX->>IDX: validate binding, versions, topology, and checksums
        IDX-->>HOST: PreparedIndex(Opened)
    else compatible valid work prefix exists
        loop missing fixed Source blocks
            IDX->>SRC: bounded exact position read
            SRC-->>IDX: Point Batches + terminal summary
            IDX->>IDX: append and sync checksummed work frame
        end
        IDX->>IDX: deterministic BVH and display samples
        IDX->>IDX: sync, revalidate, and no-replace publish artifact
        IDX-->>HOST: PreparedIndex(Resumed)
    else target is absent
        IDX->>SRC: bounded fixed-block reads
        IDX->>IDX: build, sync, revalidate, and no-replace publish
        IDX-->>HOST: PreparedIndex(Built)
    end
~~~

Cancellation leaves only a verified work prefix and recognized disposable
sidecars. Existing incompatible, corrupt, or racing targets fail without
replacement. `PreparedIndex` retains the exact verified Source capability used
to build or open it.

Standalone callers may stop here:

~~~rust,ignore
let source = source_las::open(path).blocking_wait()?;
let index = point_index::prepare(
    source,
    index_target,
    PrepareLimits::default(),
).blocking_wait()?;
~~~

## 2. Create or reopen a Workspace

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant WS as point-workspace
    participant IDX as PreparedIndex
    participant DISK as Workspace directory

    HOST->>WS: create(root, index, schema, OpenLimits)
    WS->>IDX: inspect retained Source and complete descriptor
    WS->>WS: validate selected U8 classification Attribute
    WS->>DISK: acquire exclusive lock
    WS->>DISK: stage, sync, revalidate, and publish manifest
    WS->>WS: construct deterministic root Revision
    WS-->>HOST: Workspace + root Snapshot capability

    Note over HOST,DISK: Later process/session

    HOST->>WS: open(root, reopened_index, OpenLimits)
    WS->>DISK: acquire exclusive lock
    WS->>DISK: validate manifest, operations, contiguous Revisions
    WS->>IDX: revalidate Source/schema/index binding
    WS->>DISK: clean recognized disposable scratch
    WS-->>HOST: Workspace at complete recovered head
~~~

Create and open take a complete `PreparedIndex`; the Workspace never discovers
or builds one. A second open fails while any Workspace, Snapshot, or Point Set
from the first session retains the lock. Open fails closed on corrupt, forked,
gapped, mismatched, or over-limit durable state.

## 3. Select an exact Point Set

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant SNAP as Snapshot
    participant IDX as point-index
    participant SRC as point-source
    participant WSP as private Workspace state

    HOST->>SNAP: select(PointQuery, PointSetLimits)
    SNAP->>IDX: complete conservative candidate plan
    IDX-->>SNAP: sorted disjoint Source spans

    loop bounded Source batches
        SNAP->>SRC: positions + classification
        SRC-->>SNAP: exact values
        SNAP->>WSP: apply overlays through pinned Revision
        SNAP->>SNAP: exact inclusive bounds and class predicate
        SNAP->>WSP: append bounded Point Set records; spill if needed
    end

    SNAP->>SRC: verify terminal summary
    SNAP->>WSP: seal count, hashes, and spill footer
    SNAP-->>HOST: immutable PointSet
~~~

`select_point_ids` replaces index planning with bounded input consumption,
Source validation, sorting, deduplication, and span normalization. The exact
Source/overlay/filter/seal path is otherwise the same.

No Point Set is published after cancellation, Source/index error, corruption,
or resource-limit failure. Display samples and GPU picks never act as a
negative completeness witness.

## 4. Commit a classification Edit

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant WS as Workspace
    participant SET as PointSet
    participant DISK as Workspace persistence

    HOST->>HOST: generate and retain (WorkspaceId, OperationId)
    HOST->>WS: commit(set_classification(OperationId, PointSet, value), limits)
    WS->>WS: serialize writer; validate health/head/provenance/limits
    WS->>SET: bounded records with exact before-values
    SET-->>WS: ordered records
    WS->>WS: omit no-op rows; hash request and reversible delta

    alt no changed rows or definitive stale/conflict
        WS->>DISK: sync immutable rejection without replacement
        WS-->>HOST: Rejected
    else nonempty candidate
        WS->>DISK: stage/sync/close/read-only/revalidate candidate
        WS->>DISK: no-replace link Operation ready + sync operations directory
        WS->>WS: recheck expected head
        WS->>DISK: no-replace link Revision + sync revisions directory
        WS-->>HOST: Committed(CommitReceipt)
    end
~~~

The Revision stores only sorted `(ordinal, before, after)` rows. Source bytes,
positions, and every non-classification Attribute remain unchanged. A failure
before publication is an error. A failure after publication begins is
`Indeterminate`, because false certainty would be unsafe.

## 5. Commit an immediate-head Revert

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant WS as Workspace
    participant DISK as Workspace persistence

    HOST->>HOST: generate and retain OperationId
    HOST->>WS: commit(revert_head(OperationId, expected_head), limits)
    WS->>WS: require current non-root expected head
    WS->>DISK: bounded read of immediate-head rows
    WS->>WS: swap before and after; derive new child identity
    WS->>DISK: publish ready, then Revision, with directory syncs
    WS-->>HOST: Committed(new inverse Revision)
~~~

Revert appends history. It does not move the head backward, delete the reverted
Revision, or support arbitrary historical targets. Reverting the inverse is
redo.

## 6. Reconcile an indeterminate Operation

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant WS as point-workspace
    participant DISK as Workspace persistence

    HOST->>HOST: drop every Workspace/Snapshot/PointSet session handle
    HOST->>WS: open(root, same complete index, OpenLimits)
    WS->>DISK: recover and validate complete durable state
    WS-->>HOST: reopened Workspace
    HOST->>WS: resolve_operation(retained OperationId)

    alt matching Revision
        WS->>DISK: establish revisions-directory durability
        WS-->>HOST: Committed
    else immutable rejection
        WS->>DISK: establish operations-directory durability
        WS-->>HOST: Rejected
    else complete ready and expected head is current
        WS->>DISK: establish operations-directory durability
        WS-->>HOST: Retryable
        HOST->>WS: retry_operation(same OperationId, CommitLimits)
        WS->>DISK: revalidate and link complete payload
        WS-->>HOST: Committed or Indeterminate
    else no durable record
        WS-->>HOST: NotRecorded
    else durability cannot be proved
        WS-->>HOST: Indeterminate
    end
~~~

The host never invents a replacement identity for the same logical request.
`Retryable` contains a complete durable intent; no live Point Set is needed.

## 7. Prepare and render a View

View planning remains separate from exact Workspace selection. The current
real-cloud host bridge reads `PreparedIndex` directly.

~~~mermaid
sequenceDiagram
    participant HOST as Host adapter
    participant IDX as point-index
    participant VIEW as point-view
    participant GPU as render-wgpu

    HOST->>GPU: apply(Reset)
    loop camera, viewport, hierarchy, or residency change
        HOST->>VIEW: plan(camera, viewport, nodes, budget)
        VIEW-->>HOST: demand, new requests, retention, retirements
        HOST->>HOST: cancel queued work no longer demanded
        HOST->>GPU: apply safe conditional Removes
        opt one requested node fits staging limits
            HOST->>IDX: read_node(node, budget)
            IDX-->>HOST: display batches + exact terminal summary
            HOST->>HOST: pack exact ticks into origin-relative display points
            HOST->>GPU: apply one complete atomic Upsert
        end
        HOST->>GPU: render(encoder, target, frame)
        GPU-->>HOST: RecordedFrame + report
    end
~~~

The host owns scheduling, staging, update ordering, queue submission, and device
polling. A node becomes Resident only after a complete accepted Upsert. Parent
Coverage remains until the planner emits its exact conditional retirement.

## 8. Cancellation and crash matrix

| Operation | Safe cancellation boundary | Permitted residue | Published truth |
|---|---|---|---|
| Source read | Between decoder blocks or Point Batches | None | No partial Source result |
| Index prepare | After synced checksummed work frame; before artifact publication | Verified work prefix and recognized sidecars | Existing target or one complete new target |
| Workspace create/open | Before manifest/session publication; recovery becomes noncancellable once durable create is visible | Recognized scratch/partial pre-manifest directory | No Workspace, or one complete reopenable Workspace |
| Exact selection | Between candidate/Source/overlay/Point Set blocks | Live disposable spill owned by Job | No Point Set, or one sealed complete Point Set |
| Revision commit | Before publication; afterward certainty is conservative | Complete ready/rejection/Revision links and recognized scratch | Rejected old head, Committed new head, or Indeterminate until reopen |
| View planning | Before returning a plan | None | Old planner history or one complete new plan |
| GPU frame | Host-controlled frame/device boundary | Disposable GPU allocations | Workspace unchanged |

## 9. Staleness

Snapshots and Revisions are immutable. A later commit creates a new head but
does not mutate older Snapshots. View generations and GPU residency are
separate disposable state and may be reset independently.

~~~mermaid
flowchart LR
    SRC["Immutable Source"] --> R0["Root Revision"]
    R0 --> R1["Classification Revision"]
    R1 --> R2["Revert Revision"]
    R0 --> S0["Historical Snapshot"]
    R2 --> S2["Head Snapshot"]
    IDX["Rebuildable index"] -. "accelerates exact reads" .-> S0
    IDX -. "accelerates exact reads" .-> S2
    VIEW["Disposable View/GPU state"] -. "never mutates" .-> R2
~~~

## Deferred workflows

Terrain derivation, Breaklines, edited Point-row streaming, LandXML export,
autosave, screen selection, and product UI require later accepted designs.
They are not implied by the current Workspace vocabulary.
