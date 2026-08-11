# Durable document core v0.5

Status: implemented and locally verified

Punctra v0.5 adds one headless Workspace that makes exact classification
selections and reversible classification Edits durable without changing Source
bytes. The Workspace consumes one complete `PreparedIndex`, uses the verified
`Source` retained by that capability, and publishes immutable Snapshots and
linear Revisions under hard memory, temporary-storage, and durable-storage
limits.

This is a deliberately narrow document core. Query execution, Point Set spill,
classification overlays, revision persistence, and recovery remain private
parts of one deep `point-workspace` crate. The older four-crate document
proposal was not implemented: v0.5 has one caller, and separate public Query,
Point Set, and Revision persistence seams would expose construction details
without adding independent leverage.

## Outcome and boundaries

The implemented caller path is:

```text
verified Source -> complete Spatial Index -> Workspace -> Snapshot
    -> exact selection -> immutable Point Set
    -> classification Revision -> Revert Revision -> reopen
```

The release owns:

- one Source and one explicit `U8` classification Attribute per Workspace;
- exact All, inclusive world-box, and bounded explicit-Point-ID selection;
- an optional effective-classification equality predicate;
- canonical process-scoped Point Sets with bounded automatic spill;
- sparse uniform classification assignment;
- immediate-head Revert as a new inverse Revision;
- caller-retained durable Operation Identity and reconciliation;
- immutable checksummed persistence and fail-closed recovery; and
- immutable historical Snapshots across later commits and reopen.

The classification Attribute identity is chosen explicitly at Workspace
creation and validated against the Source schema. Attribute identities belong
to each Source; v0.5 does not incorrectly make LAS Attribute ID 6 a universal
contract.

An explicit Point-ID Query can confirm that named provisional display hints are
real Points and evaluate their exact effective classification. It does not
claim complete screen-through, brush, visible-only, or occlusion selection.
Those operations require a separately accepted, versioned f64 CPU projection
contract and are deferred.

## Domain rules

- A **Pick Hint** is a provisional Point Identity obtained from partial View
  residency. It is never a completeness witness.
- An **Effective Attribute Value** is the immutable Source value after all
  classification overlays through one pinned Revision have been applied.
- A **Revert Edit** creates a new Revision containing a recorded inverse. It
  never moves the head backward or deletes history. “Undo” remains a caller
  action.
- Runtime `JobId` is process-local. Durable `OperationId` is caller-retained
  and must never be derived from or confused with it.

## Public interface

The implemented public capabilities and authority boundaries are:

```rust
pub fn create(
    root: impl AsRef<Path>,
    index: PreparedIndex,
    schema: WorkspaceSchema,
    limits: OpenLimits,
) -> WorkspaceJob;

pub fn open(
    root: impl AsRef<Path>,
    index: PreparedIndex,
    limits: OpenLimits,
) -> WorkspaceJob;

#[derive(Clone)]
pub struct Workspace { /* locked durable store + complete index */ }

#[derive(Clone)]
pub struct Snapshot { /* pinned immutable revision chain */ }

#[derive(Clone)]
pub struct PointSet { /* sealed memory or checked spill */ }

impl Workspace {
    pub fn identity(&self) -> WorkspaceId;
    pub fn source(&self) -> SourceId;
    pub fn head(&self) -> Snapshot;
    pub fn snapshot(&self, revision: RevisionId)
        -> Result<Snapshot, WorkspaceError>;
    pub fn revision_info(&self, revision: RevisionId)
        -> Result<RevisionInfo, WorkspaceError>;
    pub fn commit(&self, request: CommitRequest, limits: CommitLimits)
        -> CommitJob;
    pub fn retry_operation(&self, operation: OperationId, limits: CommitLimits)
        -> CommitJob;
    pub fn resolve_operation(&self, operation: OperationId)
        -> Result<OperationResolution, WorkspaceError>;
}

impl Snapshot {
    pub fn provenance(&self) -> &SnapshotProvenance;
    pub fn select(&self, query: PointQuery, limits: PointSetLimits)
        -> PointSetJob;
    pub fn select_point_ids(
        &self,
        ids: impl IntoIterator<Item = PointId>,
        limits: PointSetLimits,
    ) -> PointSetJob;
}

impl PointQuery {
    pub fn all() -> Self;
    pub fn within(bounds: WorldBounds) -> Self;
    pub fn classification_is(self, value: u8) -> Self;
}

impl PointSet {
    pub fn metadata(&self) -> &PointSetMetadata;
    pub fn ids(&self, limits: PointIdReadLimits)
        -> Result<PointIdBatches, WorkspaceError>;
}
```

`PreparedIndex::source()` exposes the already retained verified `Source` by
shared reference. Workspace construction therefore accepts one coherent
Source/index capability instead of two independently supplied values that
could disagree.

`WorkspaceSchema` names exactly one classification `AttributeId`. Creation
requires that definition to exist with exact type `U8`; open rechecks the
persisted definition and complete Source contract before publishing a
Workspace.

The normal edit interface is intent-shaped rather than a generic future-facing
Edit Batch:

```rust
impl CommitRequest {
    pub fn set_classification(
        operation: OperationId,
        points: PointSet,
        value: u8,
    ) -> Self;

    pub fn revert_head(
        operation: OperationId,
        expected_head: RevisionId,
    ) -> Self;
}

pub enum CommitOutcome {
    Committed(CommitReceipt),
    Rejected(CommitRejection),
    Indeterminate(CommitUncertainty),
}

pub enum OperationResolution {
    Committed(CommitReceipt),
    Rejected(RecordedRejection),
    Retryable(Box<RecordedIntent>),
    NotRecorded,
    Indeterminate(CommitUncertainty),
}
```

`set_classification` derives its expected Revision from Point Set provenance,
so callers cannot supply contradictory Point Set and compare-and-swap facts.
`revert_head` names an expected head because it has no Point Set.

`CommitJob` wraps the runtime Job with operation-record and Revision-publication
phase witnesses. A spawn failure is definitely pre-publication. A worker panic
or lost result after either durable publication begins is mapped conservatively
to `Indeterminate`, never to an error that falsely claims no operation record or
no commit.

## Caller flow

```rust,ignore
let source = source_las::open("survey.laz").blocking_wait()?;
let index = point_index::prepare(
    source,
    "survey.laz.pidx",
    PrepareLimits::default(),
).blocking_wait()?;

let schema = WorkspaceSchema::new(classification_attribute);
let workspace = point_workspace::create(
    "survey.pcw",
    index,
    schema,
    OpenLimits::default(),
).blocking_wait()?;

let r0 = workspace.head();
let selected = r0.select(
    PointQuery::within(bounds).classification_is(2),
    PointSetLimits::default(),
).blocking_wait()?;

let classify = OperationId::generate()?;
host_recovery_record.save(workspace.identity(), classify)?;
let r1 = match workspace.commit(
    CommitRequest::set_classification(classify, selected, 1),
    CommitLimits::default(),
).blocking_wait()? {
    CommitOutcome::Committed(receipt) => receipt.revision(),
    CommitOutcome::Rejected(reason) => return Err(reason.into()),
    CommitOutcome::Indeterminate(_) => {
        // The recovery branch below must own and drop the whole session graph.
        drop(r0);
        drop(workspace);
        let reopened = point_workspace::open(
            "survey.pcw",
            reopen_index()?,
            OpenLimits::default(),
        ).blocking_wait()?;
        return resolve_or_retry(reopened, classify);
    }
};

let revert = OperationId::generate()?;
host_recovery_record.save(workspace.identity(), revert)?;
let r2 = match workspace.commit(
    CommitRequest::revert_head(revert, r1),
    CommitLimits::default(),
).blocking_wait()? {
    CommitOutcome::Committed(receipt) => receipt.revision(),
    CommitOutcome::Rejected(reason) => return Err(reason.into()),
    CommitOutcome::Indeterminate(_) => {
        drop(r0);
        drop(workspace);
        let reopened = point_workspace::open(
            "survey.pcw",
            reopen_index()?,
            OpenLimits::default(),
        ).blocking_wait()?;
        return resolve_or_retry(reopened, revert);
    }
};

drop(r0);
drop(workspace);
let reopened = point_workspace::open(
    "survey.pcw",
    reopened_index,
    OpenLimits::default(),
).blocking_wait()?;
assert_eq!(reopened.head().provenance().revision(), r2);
```

The host records `(WorkspaceId, OperationId)` before starting a commit. After
an ambiguous acknowledgement it drops every Workspace, Snapshot, and Point Set
handle in the session, reopens, and calls `resolve_operation`; it does not
invent a replacement identity. A `Retryable` resolution always refers to a
complete sealed operation payload, so `retry_operation` needs only the retained
Operation ID.

## Exact selection and Point Sets

For All or world-box selection, the Workspace asks the complete index for a
complete conservative candidate plan before reading Source data.
`select_point_ids` consumes, Source-validates, bounds, sorts, and deduplicates
its input only after the same `PointSetLimits` ledger is available, then
converts it to normalized Source spans. Selection then:

1. reads exact positions and the classification column from the verified
   Source in ascending ordinal order;
2. applies all classification overlays through the pinned Revision;
3. applies the closed inclusive world-box predicate when present;
4. applies the effective-classification predicate when present; and
5. seals ordered unique `(ordinal, effective_classification)` records.

Index false positives are removed by the exact predicate. Index false
negatives remain forbidden. View samples, GPU pick results, visibility, and
depth never exclude a Point.

`Snapshot::select` owns one cumulative candidate/read/overlay/Point-Set ledger.
Cancellation, Source/index failure, corruption, or any limit breach publishes
no Point Set. The caller never has to combine a physically observed Query
prefix with a separate terminal summary.

Point Set public meaning is ordered unique Point Identity. Its metadata records
Workspace, Source, materialization Revision, exact count, a Point-ID digest
that is stable for equal membership across Revisions, and a provenance-bound
content hash. The private effective classification beside each ordinal is the
authoritative before-value used by the first commit.

A Point Set begins in bounded memory and automatically spills to the locked
Workspace scratch directory when the resident threshold is crossed. Spill
frames and the sealed footer are checksummed. Iteration is repeatable and
bounded. The Point Set retains the Workspace lock, detects missing or modified
spill data, and removes its spill after the final handle is dropped. It is not
a durable named selection or a recovery record.

## Revisions and Revert

The root Revision represents unmodified Source classifications. A classification
Revision stores only changed rows in ascending ordinal order:

```text
Source ordinal | before u8 | after u8
```

No-op rows are omitted. If every selected Point already has the requested
value, the operation is recorded as `Rejected(NoChanges)` and no Revision is
created. Point position, flags, intensity, color, and every non-classification
Attribute remain Source-authoritative.

A Revert is accepted only for the current non-root head. It swaps that
Revision's before/after rows and appends a new child Revision. Reverting the
inverse acts as redo. The target Revision and all earlier Snapshots remain
immutable and addressable.

Revision IDs are opaque tagged BLAKE3 values over Workspace lineage, parent,
sequence, Operation Identity, canonical request digest, and delta digest.
Sequence and parent metadata—not Revision ID byte order—define history.
Independently created Workspaces over the same Source intentionally have
different lineage and Revision identities.

## Persistent layout and commit point

The private v0.5 directory layout is:

```text
survey.pcw/
  manifest.pwm
  workspace.lock
  operations/
    <operation-id>.ready        # sealed complete candidate + intent
    <operation-id>.reject       # only for a recorded rejection
  revisions/
    00000000000000000001-<revision-id>.pwr
  scratch/
    ...                         # disposable stages and Point Sets
```

The manifest records separate disk and semantic contract versions, Workspace
identity, Source identity and count, exact position-transform bits, the chosen
classification definition, and root Revision identity. It is checksummed and
published from a synced temporary file without replacing an existing
manifest.

Operation-ready publication binds an Operation ID to both the canonical request
digest and one complete sealed candidate Revision. The digest covers
Workspace/Source lineage, expected Revision, change kind, Point Set
provenance/count/digests, and the requested classification. It excludes paths,
timestamps, budgets, batch partitioning, and spill layout. A ready record is
sufficient for retry after process loss; a partial scratch file is not an
Operation record and resolves `NotRecorded`.

Each immutable ready/Revision file contains a header, parent and operation
facts, bounded block directory, sorted reversible rows, per-block checksums,
and a final file digest. The commit protocol is:

1. hold the cross-process exclusive Workspace lock and serialize the commit;
2. validate health, limits, head, Point Set provenance, and any prior operation;
3. stream one complete candidate into `scratch/`, `sync_all`, close every
   writable handle, mark the inode read-only, reopen it read-only, and validate
   every fact and checksum;
4. mark operation-record publication attempted, hard-link it without
   replacement as `<operation-id>.ready`, sync the operations directory, then
   unlink its scratch name and sync scratch;
5. recheck the expected head and mark the Commit Job as
   revision-publication-attempted;
6. hard-link the read-only ready file without replacement at the next Revision
   path; and
7. successfully sync the `revisions` directory.

Step 7 is the logical commit point. Before step 4, cancellation or failure
cannot create a durable Operation record or Revision. Failure after step 4
begins is `Indeterminate` until recovery says ready or not recorded; it still
cannot expose a Revision before step 5. From step 5 onward the outcome is
`Committed` or `Indeterminate` until disk reconciliation proves which one
occurred. Cleanup after the commit point cannot change a committed result.

A terminal rejection is also immutable evidence. The implementation writes a
checksummed read-only rejection temporary, closes and revalidates it, publishes
`<operation-id>.reject` without replacement, and successfully syncs the
operations directory before returning `Rejected`. Failure after rejection
publication begins returns `Indeterminate`. For one Operation ID, resolution
precedence is matching Revision, recorded rejection, complete ready payload,
then no record. Recovery only unlinks recognized scratch names; it never opens
one for mutation.

Standard-library advisory file locking excludes a second process while a
Workspace, Snapshot, or Point Set handle remains alive. Multi-process readers
and writers are not claimed in v0.5.

## Recovery and Operation Identity

Open takes the lock, validates the manifest against `PreparedIndex::source()`
and descriptor facts, then verifies every published ready payload, rejection, and
contiguous immutable Revision under explicit count/byte/metadata limits. It
fails closed on a gap, fork, lineage mismatch, Source mismatch, duplicate
Operation Identity, or corruption in published state. Recognized scratch files
and incomplete unpublished stages are disposable and removed.

For a retained Operation ID:

- a matching Revision is validated and the revisions directory is successfully
  synced before it resolves `Committed`; inability to establish that durability
  resolves `Indeterminate`;
- a recorded rejection resolves `Rejected`;
- a matching durable ready payload with no Revision/rejection resolves
  `Retryable` if its expected head is still current, otherwise a recorded stale
  rejection;
- no durable record resolves `NotRecorded`; and
- reuse with a different canonical digest is `OperationConflict`.

`retry_operation` revalidates and links the complete ready payload; it never
needs an expired Point Set. Matching retry is idempotent and can create at most
one Revision. After an `Indeterminate` result the current Workspace handle is
mutation-poisoned; the caller disposes of the entire session capability graph,
reopens, and reconciles before attempting another operation. If recovery cannot
sync a visible post-crash Revision or rejection directory entry, open/resolve
reports explicit indeterminate recovery rather than publishing a false head.

## Resource contracts

Limits remain operation-specific rather than one vague byte ceiling:

- `OpenLimits`: total durable bytes, Revision/operation counts, checksum
  buffer, resident metadata, and working bytes;
- `PointSetLimits`: candidate nodes/spans/Points/bytes, Source batch and adapter
  work, overlay segments/bytes, total matches, resident records, spill bytes,
  write buffers, and peak incremental working bytes;
- `PointIdReadLimits`: IDs per batch, payload bytes, and read buffer; and
- `CommitLimits`: selected/changed Points, input frames, overlay work, staging
  bytes, Revision bytes, total durable bytes, and peak incremental work.

Capacity is charged before allocation. Temporary accounting includes old and
new buffers that overlap during a transition. Durable growth is separate from
temporary spill. An indivisible block that cannot fit fails explicitly. No
resource fallback changes completeness or silently returns partial Coverage.

## Delivery and acceptance

The implementation was delivered in four slices:

1. identities, limits, `PreparedIndex::source`, Workspace manifest/locking, and
   root Snapshot;
2. exact selection, overlay reads, Point Set memory/spill, and bounded ID
   iteration;
3. immutable operation/rejection/Revision files, classification commit,
   Revert, and recovery; and
4. direct caller example, generated LAS/LAZ integration, fault matrix,
   allocation/temp gates, benchmark, documentation, and full regression gates.

Repository evidence proves:

- indexed All/box/Point-ID selection equals a brute-force Source-plus-overlay
  oracle for boundaries, seeded randomized data, and varied Source batching;
- resident and forced-spill Point Sets have identical order, count, digests,
  repeated iteration, and cleanup;
- Point Identity survives Source, index, selection, commit, Revert, and reopen;
- mixed before classifications restore exactly and every historical Snapshot
  remains unchanged;
- Source file bytes and all non-classification values remain identical;
- tested selection input/output, spill, overlay, Point-ID read, commit, and
  recovery limit families fail without a published Point Set or partial
  Revision;
- representative injected failures spanning ready-payload and rejection
  staging, hard-link, directory-sync, cleanup, cancellation, panic, and
  lost-ack phases expose only the old head or the complete new head;
- identical Operation retry creates at most one Revision and changed reuse
  conflicts;
- corrupt published state fails closed while disposable scratch is recoverable;
- exclusive locking rejects a second open; and
- local formatting, strict lint, workspace tests, warning-free docs, examples,
  benchmarks, and required GPU regressions pass.

The package has 61 tests: 19 integration tests through the public interface and
42 unit, fault-injection, and allocation gates. Generated LAS and LAZ fixtures
cover exact selection, classification commit, Revert, reopen, effective
overlays, and byte-for-byte unchanged Source files. Private persistence fault
injection covers candidate/rejection staging, file-sync,
read-only/revalidation, no-replace link, directory-sync, cleanup, cancellation,
panic, and lost-acknowledgement boundaries.

The default benchmark uses a generated one-million-Point Source and records
0/1/50/100-percent selection, resident versus forced spill, sparse/dense
classification and Revert, reopen at increasing Revision depth, bytes per
changed Point, public Point-ID allocation, sampled process RSS, peak temporary
bytes, and durable growth. A separate 131,073-Point synchronous allocation test
measures the same selection worker path. Larger generated runs are opt-in.
Timing is a named one-machine baseline, not a universal latency or
licensed-production-data claim. On the local Apple M5 Pro, 24 GiB, arm64,
macOS 26.5.2 reference machine with Rust 1.90.0, the one-million-Point evidence
pass and all declared Criterion cases completed locally. The separate
worker-equivalent allocation
gate measured 6,292,224 peak bytes under its 64 MiB ceiling. The
one-million-Point benchmark itself does not claim worker heap. Its public
Point-ID iteration peaked at 2,621,440 caller-thread bytes with zero retained
bytes. Resident-selection process RSS was 62,668,800 bytes; forced-spill RSS
started at 62,685,184, sampled at 62,832,640, and therefore increased by
147,456 bytes. The sealed temporary payload was 9,009,182 bytes and was removed
with the final handle.

The sparse 10,000-Point classification/Revert pair measured approximately
16.442/15.818 ms and 20.100 logical bytes per changed Point; the dense
500,000-Point pair measured approximately 34.973/35.778 ms and 20.004 logical
bytes per changed Point. Reopen at depths 2, 4, and 8 measured approximately
1.231, 37.753, and 74.968 ms. Final durable storage was 40,812,316 logical
directory-entry bytes and 20,418,560 physical bytes reported by `du`.

Licensed production-data, above-500-million-Point, and design-partner evidence
remain explicitly outstanding; the generated fixtures do not satisfy those
external gates.

## Explicitly out of scope

v0.5 does not add:

- separate public Query, Point Set, or Revision-persistence crates;
- public exact edited Point-row streaming; v0.6 must earn that interface from
  its terrain caller;
- screen-through, polygon, corridor, frustum, visible-only, or occlusion
  selection;
- general predicate languages or edits to position, flags, intensity, color,
  returns, waveform, or Extra Bytes;
- Breaklines, terrain, QA, export, or Source rewrite;
- persistent named/shared Point Sets or automatic selection rebase;
- branching, merge, collaboration, arbitrary historical Revert, pruning,
  migration, or compaction;
- multiple Sources, remote storage, networking, or multiple processes; or
- Workspace-owned Source discovery, verification, index building, rendering,
  picking, application UI, or autosave policy.

Deleting the index remains safe because Source and immutable Revision files are
authoritative. Deleting GPU/View state changes no Workspace bytes. Deleting a
process-scoped Point Set only removes a temporary selection and cannot remove a
committed Revision.
