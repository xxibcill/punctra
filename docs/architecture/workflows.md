# Runtime Workflows

Status: v0.7 durable Run plus v0.8 bounded comparison and strict Run-bound
evidence plus full-ceiling streaming implemented; v0.9 independent review is
complete and its local candidate record remains outstanding; v0.10 repository
View implementation is complete and preserves the same authoritative workflow
boundaries; the v0.11 exact-review technical workflow is repository-verified
with field evidence outstanding; broader workflows deferred

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
    HOST->>IDX: prepare(Source, v1 target, limits)
    Note over HOST,IDX: or prepare_with_recipe(Source, v2 target, InspectionV1, limits)
    Note over HOST,IDX: cold measurement uses prepare_fresh_with_recipe and absent paths

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

Cancellation leaves only a verified work prefix and recognized owned
disposable temporaries. Existing incompatible, corrupt, or racing targets fail
without replacement. A v1 target cannot be opened as v2 or vice versa; the
caller moves/deletes the rebuildable family or chooses a new target to migrate.
`PreparedIndex` retains the exact verified Source capability used to build or
open it.

For a claimed cold-build measurement, `prepare_fresh_with_recipe` rejects and
preserves any existing complete/work family before it can be opened or resumed;
the point-index and viewing benchmarks and corpus runner use this stricter
operation. A later ordinary prepare proves the distinct warm-open path.

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
    WS->>DISK: validate recognized retained scratch aliases
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

## 7. Stream exact Ground Input and derive a Terrain Surface

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant SNAP as Snapshot
    participant ROWS as SnapshotPointBatches
    participant TER as point-terrain

    HOST->>TER: derive(Snapshot, TerrainRecipe, TerrainLimits)
    TER->>SNAP: point_rows(ground Query, PointRowLimits)
    SNAP-->>TER: pull-based exact row stream
    loop bounded nonempty batches
        TER->>ROWS: next()
        ROWS-->>TER: Point Identity + ticks + effective class
        TER->>TER: account, normalize, hash, and stage Ground Input
    end
    TER->>ROWS: terminal next() + summary()
    TER->>TER: robust deterministic triangulation and final validation
    TER-->>HOST: immutable TerrainSurface
~~~

Rows are provisional until the stream publishes its complete terminal summary.
The single worker canonicalizes exact ticks and Point Identity, rejects
unsupported degeneracy, charges overlapping working/result allocations, and
publishes no partial Surface after error or cancellation. Classification
correction is a separate existing Workspace commit; immediate-head Revert and
a later Derivation can restore equal geometry while retaining distinct Revision
provenance. Source bytes remain unchanged.

## 8. Evaluate detached QA and ensure LandXML

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant TER as TerrainSurface
    participant DISK as Caller export directory

    HOST->>TER: check_points(detached observations, limits)
    TER->>TER: validate identities; locate closed faces; interpolate
    TER-->>HOST: ordered samples/gaps + compensated residual statistics

    HOST->>TER: ensure_landxml(target, explicit options, limits)
    TER->>DISK: create/sync/reopen/verify bounded sibling stage
    alt target absent
        TER->>DISK: no-replace publish target + sync parent
        TER->>DISK: verify and retain unique stage alias + sync cleanup
        TER-->>HOST: Created receipt after durable completion
    else regular target exists
        TER->>DISK: bounded exact length/hash verification
        TER-->>HOST: ReconciledExisting or conflict
    end
~~~

Residual is observed Z minus Surface Z. Outside-hull positions are explicit
gaps. LandXML coordinates are northing, easting, elevation and require caller-
established metric-metre Source coordinates; no transformation or clock read
occurs. Once target publication starts, any inability to prove final
verification, durability, cleanup, or terminal acknowledgement is reported as
indeterminate rather than success.

An exact existing regular target reconciles without replacement. A different,
symlinked, or non-regular target fails closed. Create versus reconcile is an
attempt observation; the durable Workflow fact is always `ensured_exact`.

## 9. Start, resume, or inspect one durable terrain Run

The application facade composes the earlier workflows without adding a public
foundation crate. The caller supplies the same complete paths, identities,
baseline, correction ordinals, Terrain Recipe, detached Check Points, LandXML
options, and limits on every start or resume.

The caller creates the Workspace separately through workflow 2 and reads its
current `workspace.head().provenance().revision()` as the baseline. The terrain
Workflow requires Source Attribute 6 (`source-las` classification) as the
Workspace's selected `U8` Attribute. It opens only and never creates a
Workspace. An absent Workspace returns `PWF_INVALID_REQUEST` before Run
creation or Workspace mutation.

~~~mermaid
sequenceDiagram
    participant CALLER as Caller
    participant RUN as terrain-demo Workflow
    participant WS as Workspace
    participant TER as point-terrain
    participant DISK as Run root

    CALLER->>RUN: start_run(paths, intent, limits)
    RUN->>DISK: acquire run.lock; publish synced Intent
    RUN->>WS: resolve/select/commit same Operation Identity
    WS-->>RUN: one changed Revision or structured stop
    RUN->>DISK: RevisionResolved
    RUN->>WS: exact revision_audit
    RUN->>DISK: AuditObserved
    RUN->>TER: derive baseline + changed Surfaces
    RUN->>DISK: SurfaceObserved
    RUN->>TER: detached Check Point QA
    RUN->>DISK: QaObserved
    RUN->>TER: ensure terrain.xml
    RUN->>DISK: ExportEnsured
    RUN->>DISK: ensure canonical audit.json; ReportEnsured
    RUN->>DISK: revalidate all final facts; Complete
    RUN-->>CALLER: WorkflowReceipt

    Note over CALLER,DISK: After interruption, same paths and intent
    CALLER->>RUN: resume_run(paths, intent, limits)
    RUN->>DISK: verify/repair journal prefix and semantic links
    RUN->>WS: revalidate or resolve durable Operation fact
    RUN->>TER: recompute immutable Audit/Terrain/QA facts
    RUN-->>CALLER: same complete receipt

    CALLER->>RUN: inspect_and_repair_run(run_root, limits)
    RUN->>DISK: lock; verify journal format/hash/semantic chain
    RUN->>DISK: repair only torn suffix; revalidate root identity
    RUN-->>CALLER: Run/Operation/semantic-phase status
~~~

The exact monotonic frame order is `Intent`, `RevisionResolved`,
`AuditObserved`, `SurfaceObserved`, `QaObserved`, `ExportEnsured`,
`ReportEnsured`, `Complete`. Recomputed values must match any existing frame.
A torn final suffix repairs to the last complete frame; corruption in a
complete frame fails closed. A committed Workspace Revision remains committed
if a later phase fails, and resume never invents another Operation Identity. If
the Run root is replaced after a durable inspect repair, inspection returns
publication-indeterminate at `inspect` with the `journal-checkpoint` phase.

The fixed Run root contains `run.pwf`, `run.lock`, `terrain.xml`, and
`audit.json`. Exact XML/report targets reconcile, conflicts are not overwritten,
and unknown children are not deleted. `WorkflowFailure` names the stable code,
stage, certainty, known identities, and exactly one safe recovery action.

## 10. Qualify one returned LandXML against a Complete Run

Qualification is a private post-Run operation. It never appends a checkpoint,
repairs a torn journal, opens the Source/Workspace/index, or writes inside the
Run root.

~~~mermaid
sequenceDiagram
    participant CALLER as Caller
    participant QUAL as terrain-demo qualifier
    participant RUN as Complete Run root
    participant RETURNED as Returned LandXML
    participant OUT as Evidence parent

    CALLER->>QUAL: verify-round-trip(Run, returned, declaration, tolerances, target)
    QUAL->>RUN: open existing run.lock shared; witness root identity
    QUAL->>RUN: read-only validate exact eight-frame run.pwf
    QUAL->>RUN: hash/revalidate terrain.xml and audit.json
    QUAL->>RETURNED: capture regular file; bounded semantic compare
    alt all semantic checks pass
        QUAL->>OUT: synced stage; no-replace publish or exact reconcile
        QUAL-->>CALLER: passed evidence receipt
    else fully evaluated semantic mismatch
        QUAL->>OUT: canonical failed evidence with stable reason
        QUAL-->>CALLER: durable evidence fact plus nonzero semantic result
    else declaration, parse, resource, I/O, race, or publication uncertainty
        QUAL-->>CALLER: operational failure; no final pass/fail evidence
    end
~~~

The shared Run lock remains held through final evidence acknowledgement, and
the root/journal/artifact witnesses are revalidated around comparison and
publication. A target must have an existing parent outside the Run root. Exact
existing bytes reconcile; different caller-owned bytes are never replaced.
The evidence records caller declarations and explicit external nonclaims. The
bounded local XML stream/parser consumes the captured byte length exactly and covers
the exporter's 4-GiB, 10-million-vertex, and 20-million-face ceilings. Separate
lexical-token, parser-working, retained-working, node, text, and comparison
limits fail closed; the recorded peaks are deterministic algorithm accounting,
not allocator/RSS measurements. v0.8/v0.9 remain incomplete because the
complete one-commit local candidate record has not yet been retained; their
independent Standards/Spec review completed with no P0–P3 findings.

## 11. Prepare and render a View

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
            IDX-->>HOST: display batches + raw sample values + exact terminal summary
            HOST->>HOST: map display mode and pack origin-relative display points
            HOST->>GPU: apply one complete atomic Upsert
        end
        HOST->>GPU: render(encoder, target, frame)
        GPU-->>HOST: RecordedFrame + report
    end
~~~

The host owns scheduling, staging, update ordering, queue submission, and device
polling. A node becomes Resident only after a complete accepted Upsert. Parent
Coverage remains until the planner emits its exact conditional retirement.
Neutral/elevation select disk v1; RGB/intensity/classification select disk v2.
Display mapping changes RGBA8 only and never changes Point Identity, position,
or Coverage.

Perspective and orthographic cameras cross the same protocol, planner, and
renderer boundary. The private controller orbits, pans, zooms, toggles
projection while preserving target-plane scale, and resets without silently
changing projection. Orthographic culling and SSE are depth-independent.

The host reports demand, load candidates, actually issued work, retention,
retirement, queue/staging, requested/resident nodes, and Sampled/Complete
Coverage separately. Pausing issues no new requests; it does not claim exact
completion. Failures retain one stable `PVIEW_*` code, owning phase, bounded
detail, and one safe action.

A separate `renderer-demo corpus` command loads a bounded permission-gated
manifest, Full-verifies each Source, prepares the selected index recipe,
executes an initial view plus a declared navigation trace on a local GPU, and
publishes one canonical no-replace Viewing Report. Failed entries retain their
structured failure. The report omits private paths/project/firm identifiers
and records explicit false product nonclaims. Manifest string tokens use
literal UTF-8 without JSON escapes so a zero-allocation lexical preflight can
enforce their bound before deserialization.

## 11a. Confirm and correct exact review state

Display and exact state meet only through canonical Point identity and a
caller-owned View-generation check.

~~~mermaid
sequenceDiagram
    participant HOST as Host adapter
    participant GPU as render-wgpu
    participant REV as point-review
    participant WS as point-workspace

    HOST->>GPU: pick(encoder, RecordedFrame, pixel)
    GPU-->>HOST: provisional PickHit or miss
    HOST->>HOST: reject stale View generation; a miss proves nothing exact
    HOST->>WS: pin head Snapshot
    HOST->>REV: confirm_pick(Snapshot, PointId, limits)
    REV->>WS: select_point_ids + exact point_rows
    WS-->>REV: one-Point Point Set + exact row
    REV-->>HOST: ConfirmedPoint
    opt inclusive screen-through rectangle
        HOST->>REV: screen_through(Snapshot, Camera, Viewport, rectangle, limits)
        REV->>WS: complete exact Point-row scan + select_point_ids
        REV-->>HOST: complete Point Set + terminal summary
    end
    HOST->>WS: bounded PointSet::ids read
    HOST->>GPU: one complete SetHighlights from only those IDs
    opt explicit caller-owned correction
        HOST->>WS: set_classification(OperationId, Point Set, class)
        WS-->>HOST: Committed, Rejected, or Indeterminate
        HOST->>WS: revision_audit or same-Operation reopen/resolve
        opt immediate-head Revert requested
            HOST->>WS: revert_head(new OperationId, exact head)
        end
    end
~~~

The Workspace must already exist and remain bound to the same prepared Source.
The host retains both mutation Operation identities before submission and
allows no second correction while one outcome is unresolved. A completed old
Point Set remains exact for its pinned Snapshot but cannot commit over a newer
head. Highlights are display overlays only: exact selected identities that are
not resident need not be visible.

## 12. Cancellation and crash matrix

| Operation | Safe cancellation boundary | Permitted residue | Published truth |
|---|---|---|---|
| Source read | Between decoder blocks or Point Batches | None | No partial Source result |
| Index prepare | After synced checksummed work frame; before artifact publication | Verified work prefix and recognized sidecars | Existing target or one complete new target |
| Workspace create/open | Before manifest/session publication; recovery becomes noncancellable once durable create is visible | Recognized scratch/partial pre-manifest directory | No Workspace, or one complete reopenable Workspace |
| Exact selection | Between candidate/Source/overlay/Point Set blocks | Live spill owned by the Job; after release, an emptied recognized scratch alias may remain | No Point Set, or one sealed complete Point Set |
| Snapshot Point rows | Between candidate/Source/overlay/output blocks | Private in-memory partial batch only | No summary, or one complete terminal summary |
| Exact screen review | Between Snapshot row batches, projected rows, and Point Set construction | Private retained identity vector and Job-owned Point Set spill | No review result, or one complete exact Point Set and terminal summary |
| Revision commit | Before publication; afterward certainty is conservative | Complete ready/rejection/Revision links and recognized scratch | Rejected old head, Committed new head, or Indeterminate until reopen |
| Terrain Derivation | Between rows, sort/predicate/topology blocks, and before final seal | Private in-memory working allocations | No Surface, or one complete immutable Surface |
| Detached QA | Between inputs and bounded face-location work | Private partial results | No report, or one complete report |
| LandXML ensure | Before target publication; afterward certainty is conservative | Recognized sibling stage and possibly one complete target | No target, one exact target plus receipt, exact-existing reconciliation, conflict, or ExportIndeterminate |
| Terrain Workflow Run | Cooperative phase boundaries and directly linked active child Jobs; after publication certainty remains conservative | Fixed `run.lock`/rebuildable index work before Intent; afterward a verified journal prefix, committed Revision, exact XML/report targets, or recognized sibling stages | No Run before Intent, or one resumable Run whose frames never overstate durable facts |
| Round-Trip qualification | Before no-replace evidence publication; afterward acknowledgement is conservative | No Run-root change; recognized evidence stage and possibly one complete caller-owned target | No pass/fail evidence for operational failure, canonical pass/fail bytes after complete acknowledgement, or publication-indeterminate for an unacknowledged complete target |
| View planning | Before returning a plan | None | Old planner history or one complete new plan |
| GPU frame | Host-controlled frame/device boundary | Disposable GPU allocations | Workspace unchanged |
| Viewing Report | Before no-replace link of a synced, read-back-verified owned stage | Recognized identity-checked owned stage, or one complete target | No report, exact-existing reconciliation, one complete new report, or conflict without replacement |

## 13. Staleness

Snapshots and Revisions are immutable. A later commit creates a new head but
does not mutate older Snapshots. Derived Surfaces remain immutable even when a
later Revision restores equal geometry. View generations and GPU residency are
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
    S0 --> T0["Immutable Terrain Surface"]
    S2 --> T2["Later immutable Terrain Surface"]
    T2 --> XML["Caller-owned LandXML Export"]
    R2 --> AUD["Rebuildable Revision Audit"]
    T2 --> REP["Canonical audit.json"]
    RUN["Durable Workflow journal"] -. "checkpoints; must revalidate" .-> R2
    RUN -. "checkpoints; must revalidate" .-> XML
    RUN -. "checkpoints; must revalidate" .-> REP
    VIEW["Disposable View/GPU state"] -. "never mutates" .-> R2
~~~

## Deferred workflows

Breakline/constrained or persisted terrain, general Attribute Point-row
streaming, general LandXML/import, autosave, polygon/brush/visible-only
selection, continuous painting, and product UI require later accepted designs.
They are not implied by the current Workspace, review, or Terrain vocabulary.
