# Runtime Workflows

Status: **v0.7 durable Run, v0.8 Run-bound qualification, and v0.9 trust/
compatibility hardening Complete; the v0.10 repository View implementation and
repository-verified v0.11 exact-review technical workflow preserve the same
authoritative boundaries; the v0.12 explicit spatial-reference profile now
flows through the same Source, Workspace, Terrain, QA, export, and round-trip
boundaries; v0.13: Complete and repository-verified for the bounded
persistent-terrain slice; field activation, production-scale accuracy, true
out-of-core adoption, independent adoption, partner validation, and support
qualification outstanding. The explicit-AOI persistent Surface preparation
preserves those authority boundaries and frozen Run-v1; v0.14 bounded exact
Terrain QA and correction-loop slice Complete and repository-verified; v0.15
bounded local WebAssembly/WebGPU browser-host workflow Complete and repository-
verified; v0.16 bounded immutable-LAS Range/cache/Worker workflow Complete and
repository-verified; v0.17 bounded viewer API/exact-Point, v0.18 packed SDK/React
lifecycle, v0.19 exact local qualification, and v0.20 clean packed-consumer
integration workflows Complete and repository-verified; v0.21 private visual-
trial/capture/evidence workflow Complete and repository-verified for the exact
local attended lane, while broader browser/device support, physical-display
presentation, independent-human/adopter evidence, improved or final visual
quality, API stability, support qualification, beta, v1, and release-candidate
status remain outstanding; arbitrary remote browser delivery and broader
workflows remain outstanding**

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

When the Source has a structured v0.12 Coordinate Reference, its horizontal and
vertical identities, easting/northing/elevation axes, units, and provenance are
part of the verified Source and downstream Workspace binding. A different
reference cannot reopen the same Workspace even when Point rows are otherwise
equal.

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
    WS->>DISK: ignore recognized retained scratch; preserve unknown children
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

## 7a. Prepare or reopen one persistent bounded-AOI Surface

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant TER as point-terrain
    participant SNAP as Snapshot
    participant WORK as Surface work/stage
    participant ART as Surface disk-v1 target

    HOST->>TER: prepare(Snapshot, target, explicit-AOI Recipe, limits)
    alt compatible complete target
        TER->>ART: bounded full validation
        TER-->>HOST: PreparedTerrainSurface + Opened report
    else compatible final stage
        TER->>WORK: validate complete staged Surface
        TER->>ART: no-replace publication + sync + revalidation
        TER-->>HOST: PreparedTerrainSurface + ResumedPublication report
    else compatible input work
        TER->>WORK: validate complete Ground Input checkpoint
        TER->>TER: resume topology and final staging
        TER->>ART: no-replace publication + sync + revalidation
        TER-->>HOST: PreparedTerrainSurface + ResumedInput report
    else target family absent
        TER->>SNAP: stream exact AOI Ground Input
        TER->>WORK: sync complete verified input checkpoint
        TER->>TER: existing single-worker full-AOI triangulation
        TER->>WORK: sync and verify complete Surface stage
        TER->>ART: descriptor-bound no-replace publish + sync
        TER-->>HOST: PreparedTerrainSurface + Built report
    end
    HOST->>TER: bounded vertex_batches / face_batches
    TER-->>HOST: verified canonical record batches
~~~

The semantic Surface is identical to the legacy explicit-AOI `derive` result.
Only execution and storage differ. After the verified input checkpoint, resume
does not reread Snapshot rows, although sorting and topology may rerun. A warm
open reads no Snapshot rows. The prepared handle retains bounded metadata and
file access rather than complete vertex/face arrays.

Acknowledged publication retains the verified final-stage pathname and any
input-work sibling. A `ResumedPublication` attempt does not inspect or trust an
arbitrary work sibling. No portable unlink can be conditioned on the still-open
owned inode, while a check-then-unlink could delete a racing replacement. A
warm open gives the complete target precedence and ignores siblings; optional
cleanup is caller-controlled offline maintenance only when no related handle,
job, or process is live.

Stale, corrupt, incompatible, or conflicting targets are preserved. A later
Workspace head does not invalidate an Artifact for its historical Snapshot, but
the same path is stale for a different requested binding. Publication never
replaces a target and reports conservative indeterminate certainty after its
commit boundary. The full-AOI triangulator still retains the complete AOI in
memory and supports one worker; this workflow is persistence, not true
out-of-core topology.

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
        TER->>DISK: independent descriptor-bound no-replace publish + sync parent
        Note over DISK: retain the per-attempt bounded private stage
        TER-->>HOST: Created receipt after durable completion
    else regular target exists
        TER->>DISK: bounded exact length/hash verification
        TER-->>HOST: ReconciledExisting or conflict
    end
~~~

Residual is observed Z minus Surface Z. Outside-hull positions are explicit
gaps. For a structured v0.12 profile, QA and LandXML require metre/metre,
easting/northing/elevation coordinates and LandXML emits one matching
`CoordinateSystem`; the frozen legacy boolean is readable only by the private
legacy reconciliation verifier. LandXML point text is northing, easting,
elevation.
No transformation or clock read occurs. Once target publication starts, any inability to prove final
verification, durability, target binding, or terminal acknowledgement is reported as
indeterminate rather than success.

An exact existing regular target reconciles without replacement. A different,
symlinked, or non-regular target fails closed. Create versus reconcile is an
attempt observation; the durable Workflow fact is always `ensured_exact`.

## 8.5. Inspect, correct, re-derive, compare, and Revert

The v0.14 public example composes existing owners without adding a workflow
facade. Exact QA and Surface comparison belong to `point-terrain`; durable
classification mutation and Revert remain `point-workspace` operations.

~~~mermaid
sequenceDiagram
    participant HOST as Host
    participant WS as Workspace
    participant TER as point-terrain

    HOST->>TER: exact_qa(baseline Snapshot/Surface, request, limits)
    TER-->>HOST: bound profile/residual/tolerance/gap evidence
    HOST->>WS: select exact Point Identities
    HOST->>WS: set_classification(recorded Operation Identity)
    WS-->>HOST: changed Revision
    HOST->>TER: freshness(current Snapshot, old Surface)
    TER-->>HOST: stale Snapshot/Surface
    HOST->>TER: derive or prepare changed Surface
    HOST->>TER: compare_surfaces(before, after, limits)
    TER-->>HOST: exact face changes + conservative bounds
    HOST->>TER: exact_qa(changed Snapshot/Surface, same intent)
    TER-->>HOST: rechecked evidence
    opt reject the correction
        HOST->>WS: revert_head(second recorded Operation Identity)
        WS-->>HOST: Revert Revision
        HOST->>TER: re-derive and compare with baseline
        TER-->>HOST: zero semantic face changes when restored
    end
~~~

Every report remains valid historical evidence only for its frozen pair.
Display colors and connecting SVG lines are not measurements. A caller must
reconcile indeterminate commits by the same Operation Identity and must use a
fresh persistent Surface target after Revision change.

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

## 10. Qualify one Complete Run round trip

~~~mermaid
sequenceDiagram
    participant CALLER as Caller
    participant QUAL as terrain-demo qualifier
    participant RUN as Complete Run root
    participant RET as Returned LandXML
    participant EVID as Evidence parent

    CALLER->>QUAL: verify-round-trip(Run, returned, declaration, tolerances, target)
    QUAL->>RUN: open and lock run.pwf strictly read-only
    QUAL->>RUN: bind Complete journal, terrain.xml, and audit.json
    QUAL->>RET: retain regular-file witness; stream complete bounded input
    QUAL->>QUAL: evaluate XML/subset/CRS, units, Point count, mapping, tolerance, topology
    QUAL->>RUN: revalidate all Run and input witnesses
    QUAL->>EVID: create or exactly reconcile canonical pass/fail evidence
    QUAL->>RUN: final unchanged revalidation
    QUAL-->>CALLER: evidence receipt or conservative failure
~~~

Qualification never invokes journal repair and never writes inside the Run
root. A torn or non-Complete Run, changed input, cancellation, or resource
failure is operational failure. After complete stable reads, supported semantic
non-conformance produces canonical failed evidence with a stable reason. Exact
existing evidence reconciles; different bytes are never overwritten. The
opaque downstream declaration is not evidence that the named application ran.
Both reference and returned v0.12 LandXML must carry the same complete
supported `CoordinateSystem`; reference drift fails before coordinate
tolerances are evaluated. Legacy generated files remain comparable only when
both omit it.

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

## 11b. Run the private browser acceptance host

The v0.15 private host preserves the same renderer ownership boundary while
moving the application composition into a browser. It uses a deterministic
generated batch; browser Source delivery begins in a later accepted scope.

~~~mermaid
sequenceDiagram
    participant JS as JavaScript host
    participant WASM as private browser-demo adapter
    participant VIEW as point-view
    participant GPU as render-wgpu / WebGPU

    JS->>WASM: createViewer(canvas, CSS size, DPR)
    WASM->>GPU: request compatible adapter/device/surface
    WASM->>VIEW: plan missing generated root
    VIEW-->>WASM: one request
    WASM->>GPU: Reset + complete generated Upsert
    WASM->>VIEW: plan resident root
    VIEW-->>WASM: retain root; no request/retirement
    WASM-->>JS: ready diagnostics
    JS->>WASM: resize / visibility / render
    WASM->>GPU: configure / record / submit / present
    JS->>WASM: centre provisional pick
    WASM->>GPU: asynchronous readback
    GPU-->>WASM: generation-safe provisional Point identity
    WASM-->>JS: bounded non-authoritative diagnostics
    JS->>WASM: shutdown
    WASM-->>JS: GPU resources dropped; later work rejected
~~~

JavaScript owns the canvas, CSS placement, DPR, visibility, scheduling, error
presentation, and retry decision. The Rust adapter owns WebGPU resources on its
behalf. Device loss, surface loss, or surface validation failure instructs the
caller to destroy and explicitly recreate the viewer. A timeout or occlusion
keeps the last frame and waits for a caller-requested frame; an outdated surface
requires a caller-requested bounded resize before another frame. Neither layer
silently retries. The pick is progressive display evidence, never an exact
empty or selected Source Query.

## 11c. Stream one private immutable browser deployment

The v0.16 path keeps networking and storage policy in the private JavaScript
host. The deployment manifest is trusted configuration derived from a complete
native `source-las` verification and compatible `point-index` build; browser
display does not reopen a public Source capability.

~~~mermaid
sequenceDiagram
    participant JS as JavaScript host
    participant WK as module Worker
    participant HTTP as immutable HTTP server
    participant CACHE as host-selected cache
    participant WASM as private browser-demo adapter
    participant GPU as render-wgpu / WebGPU

    JS->>WK: start delayed acceptance probe, then cancel
    WK-->>JS: cancelled within 1,000 ms; publish no completion
    JS->>WK: start(operation, manifest URL, cache policy)
    WK->>HTTP: GET bounded deployment manifest
    WK->>CACHE: invalidate exact namespace when requested
    WK->>HTTP: Range Source probe (validator + digest)
    WK->>HTTP: Range index header and root record
    WK->>WK: validate disk-v2 binding and Sampled Coverage
    WK->>HTTP: Range root sample block
    WK->>WK: decode bounded attributed samples
    WK-->>JS: deployment identity and transferable batches
    JS->>WASM: beginStream + bounded publishStreamBatch
    WASM->>GPU: Reset v0.16 generation + validated Upserts
    JS->>WASM: render progressive Sampled Coverage
    WK->>CACHE: retain verified identity-versioned ranges
    JS->>JS: destroy and recreate viewer and Worker
    WK->>CACHE: revalidate exact warm entries
    WK-->>JS: same Source identity; zero binary network requests
~~~

The worker permits one operation and one request at a time. Cancellation aborts
the current Fetch, publishes no completion, and is acknowledged within the
fixed 1,000-millisecond acceptance limit. Every transferred batch is
strictly Source-ordinal ordered and detached from worker memory. A cache entry
is reachable only through the deployment schema, Source identity, strong
validator, index digest, resource kind, and exact range; quota or Cache API
failure cannot silently change the caller's policy. A versioned fixed-size
ledger enforces both the response-body and 64-entry namespace ceilings without
enumerating Cache API keys. The main thread yields
between at-most-1,024-Point publications. All output remains non-authoritative
Sampled Coverage.

## 11d. Drive the browser through one coherent viewer API

The v0.17 façade owns composition, not application policy. The host supplies a
canvas, camera/navigation choices, cache/credential policy, and a separate
exact bridge; it does not coordinate raw worker messages or Wasm publication.

~~~mermaid
sequenceDiagram
    participant HOST as plain browser host
    participant API as viewer-api.js
    participant WK as streaming Worker
    participant WASM as browser-demo Wasm
    participant QUERY as exact-Point bridge

    HOST->>API: create / subscribe / loadSource
    API->>WK: one bounded operation
    WK-->>API: deployment + transfer-v2 batches
    API->>WASM: begin/publish active generation
    HOST->>API: camera / display / render
    API->>WASM: validated complete presentation changes
    HOST->>API: pick physical pixel
    WASM-->>API: provisional Source identity + ordinal + generation
    HOST->>API: setHighlights(complete set)
    API->>WASM: presentation-only replacement
    HOST->>API: confirmPoint(provisional)
    API->>QUERY: exact identity request + AbortSignal
    QUERY-->>API: exact immutable LAS record
    API-->>HOST: exact result only if generation is still active
    HOST->>API: clear / destroy
~~~

A new Source generation clears recorded-pick and highlight presentation. Exact
completion rechecks generation and Source identity after the asynchronous
bridge returns. Cancellation preserves the last complete frame; fused device,
surface, or partial-publication failures destroy the viewer before returning a
bounded structured error.

## 11e. Install and own the packaged SDK lifecycle

The v0.18 package adds no second viewer state model. It resolves deployable
assets and then enters the same v0.17 façade. The framework adapter translates
only its host lifecycle:

~~~mermaid
sequenceDiagram
    participant APP as TypeScript or React host
    participant SDK as @punctra/viewer
    participant ASSET as Wasm / module Worker assets
    participant API as BrowserViewer
    participant RA as @punctra/react

    APP->>SDK: import packed artifact
    SDK->>ASSET: resolve import.meta.url or explicit URLs
    APP->>SDK: createViewer(canvas, viewport)
    SDK->>ASSET: initialize one matching Wasm module
    SDK-->>APP: independent disposable BrowserViewer
    APP->>API: resize / pause / resume / render
    APP->>API: dispose
    RA->>SDK: create after caller canvas mounts
    RA->>API: resize / pause / resume
    RA->>API: unsubscribe then dispose on cleanup
    SDK-->>RA: late async viewer after cleanup
    RA->>API: dispose without publication
~~~

Bundler-owned Worker construction uses the static module-Worker form so the
qualified build can include its private dependency graph and content hash.
Explicit Worker URLs opt out of that build behavior and make co-located asset
deployment a host obligation. Neither path changes Source URL, credentials,
cache consent, interaction policy, exact authority, or recovery ownership.

## 11f. Qualify one exact browser/device lane

The v0.19 qualification runner observes the existing SDK without taking over
Browser Host policy:

~~~mermaid
sequenceDiagram
    participant HOST as qualification host
    participant SDK as @punctra/viewer
    participant WORKER as module Worker
    participant SERVER as strict Range server
    participant MATRIX as local evidence matrix

    HOST->>SDK: create packed viewer
    HOST->>SDK: invalid resize, DPR change, hide/resume
    HOST->>WORKER: deliberate pre-publication crash
    WORKER-->>HOST: worker_failed, retain viewer
    HOST->>SERVER: disconnected manifest request
    SERVER-->>HOST: offline, retain viewer
    HOST->>SDK: cold load, settle, dispose, recreate
    SDK->>SERVER: three bounded 206 requests
    HOST->>SDK: warm load and 30 settled frames
    SDK-->>HOST: timings, state, resource facts
    HOST->>MATRIX: evaluate fixed ceilings and record exact lane
~~~

The host recreates explicitly after device loss, another fused renderer error,
or any failure after partial Source publication. It retries in place only when
the structured failure is recoverable and the active generation was never
changed. The runner uploads nothing and cannot convert a passing unlisted
browser into a supported matrix entry without a new recorded trial.

## 11g. Complete the packed browser quickstart

The v0.20 clean consumer exercises the supported integration boundary without
repository-relative imports:

~~~mermaid
sequenceDiagram
    participant APP as TypeScript host
    participant SDK as @punctra/viewer
    participant INPUT as @punctra/viewer/input
    participant EXACT as @punctra/viewer/exact-query
    participant SERVER as strict Range server

    APP->>SDK: create viewer on caller-owned canvas
    APP->>SDK: cancel delayed load; retain viewer/frame
    APP->>SDK: load immutable manifest
    SDK->>SERVER: bounded verified ranges
    APP->>SDK: five modes, two projections, host camera policy
    INPUT-->>APP: normalized pointer/wheel/keyboard facts
    APP->>SDK: provisional pick and presentation highlight
    APP->>EXACT: confirm immutable Source record
    EXACT->>SERVER: bounded exact record range
    EXACT-->>APP: exact_source_record
    APP->>SDK: clear, pause, resume, dispose
~~~

The app owns navigation, controls, recovery messaging, and teardown. The SDK
owns generation-safe state and bounded operations. The bridge's LAS decoders
remain package-private; no step turns the fixed deployment into general Source
or Query support.

## 11h. Reproduce one private visual baseline

The completed v0.21 workflow crosses one closed private Visual Trial seam. A
trial identifier fixes its generated or permission-bound derived input,
camera, mode, projection, highlight, viewport, settling, feature, tolerance,
and resource facts; the runner exposes none of those as a configurable public
capture interface:

~~~mermaid
sequenceDiagram
    participant RUNNER as private visual runner
    participant CORPUS as closed Visual Corpus
    participant VIEW as browser viewer/harness
    participant GPU as private capture target
    participant REVIEW as attended post-capture review
    participant BUNDLE as private USTAR transport
    participant EXPORT as opt-in same-origin export
    participant REPO as pinned repository inputs
    participant VERIFY as visual verifier

    Note over RUNNER,REPO: Stage 1: attended record mode
    loop nine trials, three complete recreations each
        RUNNER->>CORPUS: resolve immutable trial identifier
        CORPUS-->>RUNNER: input, camera, features, limits, permission facts
        RUNNER->>VIEW: create at 320x240 CSS, requested DPR 2
        RUNNER->>VIEW: publish exact trial batches and presentation state
        VIEW-->>RUNNER: Settled Cut plus 30 unchanged foreground frames
        RUNNER->>GPU: render same frame and read bounded copyable target
        GPU-->>RUNNER: top-left canonical 640x480 RGBA8
        RUNNER->>VIEW: dispose viewer, mapped buffers, textures, capture state
    end
    RUNNER->>REVIEW: load exact bound images after capture
    REVIEW-->>RUNNER: submit calibration-only rubric
    RUNNER->>BUNDLE: one repository-relative record bundle
    alt standard Blob download
        BUNDLE-->>REPO: operator extracts retained baseline inputs
    else explicit transport=server fallback
        BUNDLE->>EXPORT: same-origin bounded TAR POST
        EXPORT-->>RUNNER: fixed no-replace local TAR plus non-evidence receipt
        RUNNER-->>REPO: operator extracts retained baseline inputs
    end
    REPO->>REPO: freeze qualified paths and implementation pin

    Note over RUNNER,VERIFY: Stage 2: rebuild pin, qualify, then attended verify mode
    REPO->>RUNNER: open pinned URL and use visible Run gesture
    loop nine trials, three complete recreations each
        RUNNER->>VIEW: reproduce pinned trial and Settled Cut
        VIEW-->>RUNNER: 30 unchanged foreground frames
        RUNNER->>GPU: capture canonical RGBA8
        RUNNER->>VERIFY: image, temporal, Coverage, feature, authority, resource facts
        RUNNER->>VIEW: dispose all recreation state
    end
    RUNNER->>REVIEW: load exact verify images after capture
    REVIEW-->>RUNNER: submit final non-gating maintainer rubric
    RUNNER->>BUNDLE: one repository-relative verify bundle
    alt standard Blob download
        BUNDLE-->>VERIFY: operator extracts evidence JSON and PNGs
    else explicit transport=server fallback
        BUNDLE->>EXPORT: same-origin bounded TAR POST
        EXPORT-->>RUNNER: fixed no-replace local TAR plus non-evidence receipt
        RUNNER-->>VERIFY: operator extracts evidence JSON and PNGs
    end
    VERIFY-->>RUNNER: derived pass/fail or explicit incomplete evidence
~~~

The generated mixed-LOD trial records its bounded nine-step parent/child
transition and then requires exact settled temporal pixels. Other decoded
comparisons use the named profile, whose caps are channel threshold 2,
unstable-pixel fraction 0.001, maximum channel delta 4, and one physical pixel
of feature displacement. Coverage, feature, authority, and each independent
renderer/canvas/capture/readback/canonical/encoded resource gate remain
separate from the image aggregate.

Record and verify are sequential rather than alternative modes. Record-mode
evidence, rubric, recreation, transition, and difference images are calibration
output; only its nine canonical baselines and commit-free baseline-input
manifest cross the implementation pin. The pinned checkout is rebuilt and must
repeat inherited quickstart and browser qualification before verify mode.
Only the later verify-mode evidence is eligible for final acceptance. Its URL
supplies the exact implementation commit, verifier byte length, and verifier
SHA-256; the runner fixes the attended-lane identity and disables the visible
Run control until those pins are valid.

In both stages the rubric begins only after capture and exact bound images have
loaded in the visible document. The uncompressed TAR preserves repository paths
for transport but is not itself evidence and is not checked in. Standard Blob
download is primary. If it does not materialize, an explicitly enabled same-
origin local-server endpoint may receive that same bounded archive and publish
it below an operator-selected export directory with no replacement; cross-
origin POST and an existing target fail closed. This path requires
`--visual-export-dir <existing-fresh-dir>` and `transport=server`; it POSTs
`application/x-tar` to `/qualification-visual-export`. The endpoint receipt is
not evidence. The capture
target observes renderer output before OS composition and display color
management. Its callback intervals include scheduling delay and do not measure
physical GPU-completion time. It is presentation-only evidence and cannot
publish Source geometry, exact Query truth, or a browser/display support claim.
The checked-in evidence workflow is Complete and repository-verified because
every fixed trial, recreation, accepted artifact, interpretation binding,
implementation pin, and verifier pin exists and passes; any reproduction
missing one remains incomplete. See the [v0.21 release
record](../releases/v0.21.0.md).

## 12. Cancellation and crash matrix

| Operation | Safe cancellation boundary | Permitted residue | Published truth |
|---|---|---|---|
| Source read | Between decoder blocks or Point Batches | None | No partial Source result |
| Index prepare | After synced checksummed work frame; before artifact publication | Verified work prefix and recognized sidecars | Existing target or one complete new target |
| Workspace create/open | Before manifest/session publication; recovery becomes noncancellable once durable create is visible | Recognized scratch/partial pre-manifest directory | No Workspace, or one complete reopenable Workspace |
| Exact selection | Between candidate/Source/overlay/Point Set blocks | Bounded private spill retained after completion or cancellation; ignored by recovery and removable only during offline maintenance | No Point Set, or one sealed complete Point Set |
| Snapshot Point rows | Between candidate/Source/overlay/output blocks | Private in-memory partial batch only | No summary, or one complete terminal summary |
| Exact screen review | Between Snapshot row batches, projected rows, and Point Set construction | Private retained identity vector and Job-owned Point Set spill | No review result, or one complete exact Point Set and terminal summary |
| Revision commit | Before publication; afterward certainty is conservative | Complete ready/rejection/Revision links and recognized scratch | Rejected old head, Committed new head, or Indeterminate until reopen |
| Terrain Derivation | Between rows, sort/predicate/topology blocks, and before final seal | Private in-memory working allocations | No Surface, or one complete immutable Surface |
| Persistent Terrain prepare | Between row blocks and before publication; topology cancellation restarts from complete verified input; after no-replace publication certainty is conservative | Verified input checkpoint, complete verified stage, and possibly one complete target; publication retains the verified stage and any uninspected work sibling | No target, one resumable work family, one compatible complete Artifact/handle, conflict, or publication-indeterminate result |
| Detached QA | Between inputs and bounded face-location work | Private partial results | No report, or one complete report |
| LandXML ensure | Before target publication; afterward certainty is conservative | Recognized sibling stage and possibly one complete target | No target, one exact target plus receipt, exact-existing reconciliation, conflict, or ExportIndeterminate |
| Terrain Workflow Run | Cooperative phase boundaries and directly linked active child Jobs; after publication certainty remains conservative | Fixed `run.lock`/rebuildable index work before Intent; afterward a verified journal prefix, committed Revision, exact XML/report targets, or recognized sibling stages | No Run before Intent, or one resumable Run whose frames never overstate durable facts |
| Run-bound qualification | Before evidence publication; afterward certainty is conservative | No Run mutation; retained private evidence stage and possibly one complete target | No evidence for unevaluated operational failure; otherwise one exact canonical pass/fail record, conflict, or publication-indeterminate result |
| View planning | Before returning a plan | None | Old planner history or one complete new plan |
| GPU frame | Host-controlled frame/device boundary | Disposable GPU allocations | Workspace unchanged |
| Browser acceptance host | Before viewer return and at explicit frame/pick boundaries; shutdown is fused | Disposable WebGPU resources and ignored generated bindings | No viewer, one active private viewer, or one shut-down viewer; Workspace and Sources unchanged |
| Browser streaming worker | Abortable between manifest, Source probe, index-header, and sample ranges; late operation messages are ignored | Verified identity-versioned cache entries selected by caller policy | No remote generation, bounded partial sampled renderer batches for the active identity, or one complete sampled root; never a complete or authoritative Source result |
| Browser viewer/exact bridge | Abortable before and after worker or exact-record waits; generation and Source identity rechecked before presentation | Last complete frame plus caller-selected verified cache entries | No viewer, one complete active generation with provisional presentation, or one independently confirmed exact Point; stale work is never current |
| Browser visual trial | Between trial preparation, quiet-frame sampling, capture map, comparison, and artifact staging; no partial trial is publishable | Disposable viewer/capture resources and an unaccepted private partial observation | No evidence for that repetition, or one complete immutable trial result bound to exact input and environment facts; never Source or Query authority |
| Viewing Report | Before no-replace link of a synced, read-back-verified owned stage | Recognized identity-checked owned stage, or one complete target | No report, exact-existing reconciliation, one complete new report, or conflict without replacement |

## 13. Staleness

Snapshots and Revisions are immutable. A later commit creates a new head but
does not mutate older Snapshots. Derived and prepared Surfaces remain immutable
even when a later Revision restores equal geometry. A prepared Artifact remains
valid for its bound historical Snapshot and is stale only for a different
requested binding at the same path. View generations and GPU residency are
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
    T2 --> P2["Rebuildable prepared Surface disk-v1"]
    T2 --> XML["Caller-owned LandXML Export"]
    R2 --> AUD["Rebuildable Revision Audit"]
    T2 --> REP["Canonical audit.json"]
    RUN["Durable Workflow journal"] -. "checkpoints; must revalidate" .-> R2
    RUN -. "checkpoints; must revalidate" .-> XML
    RUN -. "checkpoints; must revalidate" .-> REP
    RUN --> QUAL["Read-only qualification"]
    XML --> QUAL
    QUAL --> EVID["Separate canonical Round-Trip Evidence"]
    VIEW["Disposable View/GPU state"] -. "never mutates" .-> R2
~~~

## Deferred workflows

Breakline/constrained, tiled, true out-of-core, parallel, or distributed
terrain, general Attribute Point-row streaming, general LandXML/import,
autosave, polygon/brush/visible-only selection, continuous painting, and
product UI require later accepted designs. They are not implied by the current
Workspace, review, or Terrain vocabulary.
