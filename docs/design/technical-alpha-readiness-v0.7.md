# Technical Partner-Alpha Readiness Design (v0.7)

Status: accepted on 2026-08-10; implementation in progress

This design is authoritative for Punctra v0.7. It strengthens the already
implemented LAS/LAZ classification-to-terrain path so a headless caller can
resume it without guessing after process interruption. It is a repository
technical-readiness slice, not evidence that a design partner has accepted the
workflow.

Licensed production data, partner tolerances, downstream CAD round trips,
paid use, and measured human-time improvement remain outstanding. Repository
tests cannot substitute for those facts.

## Outcome

v0.7 adds four coherent capabilities without adding another public crate:

1. `foundation-runtime` lets one parent Job propagate cancellation to the
   child Job it is synchronously awaiting;
2. `point-workspace` derives a complete bounded Revision Audit and exact Edit
   Footprint from immutable Revision rows and Source positions;
3. `point-terrain` idempotently ensures one exact LandXML target, reconciling
   a byte-identical file after a lost acknowledgement and never overwriting a
   conflicting file; and
4. private modules inside the `terrain-demo` package own one durable Workflow
   Run journal, restart state machine, bounded audit report, and actionable
   recovery diagnostics while `main.rs` remains argument and presentation
   code.

The one supported run starts from one local immutable LAS/LAZ Source, one
complete Spatial Index, and one v0.5-compatible Workspace. It may exclude a
bounded explicit set of existing Ground Point ordinals through one durable
classification Edit, derive baseline and changed Terrain Surfaces, audit the
Edit, evaluate detached Check Points, ensure one metric-metre LandXML file,
and publish one canonical technical audit report.

The run does not make those effects transactional. A committed classification
Revision remains committed if later Terrain Derivation, QA, export, or report
publication fails. Recovery reports that fact and the only safe next action.

## Why no new public crate

`point-workspace`, `point-terrain`, and `foundation-runtime` retain their
existing domain jobs. Cross-module workflow sequencing has only one real
caller, so it remains a deep private module in the application package rather
than becoming a speculative reusable framework.

The allowed dependency direction remains:

```text
foundation-runtime / point-contracts
              ^
              |
 point-source / point-index
              ^
              |
       point-workspace
              ^
              |
        point-terrain
              ^
              |
        terrain-demo
```

The private workflow module may depend on every earlier public seam. None of
those crates depends back on it. Deleting the module removes the run journal,
restart reconciliation, canonical audit encoding, and cross-module recovery
policy; it does not weaken Source, Workspace, or Terrain correctness.

## Domain language

### Workflow Run

One durable execution intent for the supported classification-to-terrain
path. Its Run Identity, Workspace Operation Identity, semantic request, and
bindings are fixed by the first durable journal frame.

### Run Checkpoint

An immutable checksummed journal frame proving that one phase fact was synced.
A checkpoint is a recovery hint that must be revalidated against its owning
module; it is never an independent source of truth.

### Revision Audit

A rebuildable in-memory Artifact describing exactly one immutable Workspace
Revision. It includes its classification transitions and Edit Footprint.

### Edit Footprint

The inclusive axis-aligned world bounds of the Source Points whose effective
classification actually changed in one Revision. It is not the region where
Terrain topology changed.

### Surface Change Envelope

The application report's conservative inclusive bounds over vertices incident
to added or removed Terrain faces between the baseline and changed Surfaces.
It is not an exact change polygon and is not persisted as Workspace state.

### Recovery Action

The only safe caller action after one structured failure, such as resume,
retry after increasing a named resource ceiling, resolve the recorded
Operation Identity, remove a conflicting caller-owned target, restore the
expected Source, or stop.

The Workspace has no dirty document buffer. A successful classification Edit
already publishes an immutable synced Revision. That is autosave by invariant;
v0.7 does not add an autosave timer, draft, or UI.

## Linked child cancellation

A workflow Job starts child Jobs through existing public APIs. Parent
cancellation must be observable by the active child rather than only between
phases.

`foundation-runtime` therefore adds one narrow blocking wait operation with
the following meaning:

```rust,ignore
impl<T, E> Job<T, E> {
    pub fn blocking_wait_cancelled_by(
        self,
        parent: &CancellationToken,
    ) -> Result<T, E>;
}
```

Before waiting, the child links directly to the supplied root token. The
child's `OperationControl::check_cancelled`, reporter checks, and stream checks
observe either its own cancellation or the linked root cancellation. Child
cancellation does not cancel the parent. Child progress remains independent;
the workflow publishes its own phase progress only after validating each
child result.

The link is established at most once, stores no polling thread or timer, and
does not introduce a runtime. Tests cover a parent cancelled before linking,
while a child is active, after child success, and after the workflow is
dropped. A child that ignores cooperative cancellation retains the existing
detached-worker limitation.

## Exact Revision Audit

The new public Workspace seam is intent-shaped:

```rust,ignore
pub type RevisionAuditJob =
    foundation_runtime::Job<RevisionAudit, WorkspaceError>;

impl Workspace {
    pub fn revision_audit(
        &self,
        revision: RevisionId,
        limits: RevisionAuditLimits,
    ) -> RevisionAuditJob;
}

impl RevisionAudit {
    pub fn provenance(&self) -> SnapshotProvenance;
    pub fn revision(&self) -> RevisionInfo;
    pub fn edit_footprint(&self) -> Option<WorldBounds>;
    pub fn transitions(&self) -> &[ClassificationTransition];
    pub fn changed_point_count(&self) -> u64;
    pub fn point_id_hash(&self) -> ContentHash;
    pub fn content_hash(&self) -> ContentHash;
    pub fn accounted_peak_working_bytes(&self) -> u64;
    pub fn retained_result_bytes(&self) -> u64;
}
```

`ClassificationTransition` contains `before`, `after`, and a nonzero Point
count. Entries are unique and sorted by `(before, after)`.

The Root Revision returns the canonical empty audit: zero changed Points, no
footprint, no transitions, and domain-separated empty hashes. Every non-root
audit reads the immutable Revision rows in strictly ascending Source ordinal,
rejects no-op or malformed rows, joins exact Source ticks under bounded Source
reads, and computes world bounds through the verified Source transform.

The Point-ID hash covers the ordered Source-aware membership. The content hash
covers Workspace/Snapshot provenance, Revision facts, every exact ordinal and
tick triple, every before/after value, transitions, and footprint bits. It is
independent of Source and Revision batch partitioning.

An immediate-head Revert has the same Edit Footprint and Point-ID hash as the
Revision it inverts, with transitions reversed. Historical audits cannot
change after later commits.

`RevisionAuditLimits` independently cap:

- Source read Points, spans, payload, and adapter working bytes;
- Revision blocks and encoded Revision bytes read;
- changed Points and transition entries;
- retained result bytes; and
- combined peak working bytes, including Revision payload, ordinal spans,
  Source batch, decoder allowance, transition accumulator, and result sealing.

Cancellation or any error publishes no `RevisionAudit`. Audit results are not
persisted, so process loss during audit is resolved by rerunning immutable
work. Existing Workspace disk and semantic version 1 remain unchanged.

## Idempotent LandXML ensure

The strict v0.6 `export_landxml` create-new operation remains unchanged. v0.7
adds a recovery-oriented sibling:

```rust,ignore
impl TerrainSurface {
    pub fn ensure_landxml(
        &self,
        target: impl AsRef<Path>,
        options: LandXmlOptions,
        limits: LandXmlLimits,
    ) -> LandXmlJob;
}

pub enum LandXmlDisposition {
    Created,
    ReconciledExisting,
}
```

`LandXmlReceipt` records the disposition. Ensure deterministically encodes and
syncs the complete expected stage under the existing limits before making a
target decision.

- If the target is absent, ensure uses the existing no-replace hard-link,
  verification, directory-sync, and cleanup protocol and returns `Created`.
- If a regular target exists, ensure streams and bounds its verification. It
  returns `ReconciledExisting` only when length and content hash exactly match
  the expected bytes.
- A different file returns a structured conflict containing expected and
  actual hashes and never modifies either file.
- Symlinks and non-regular targets fail closed.
- A race in which another process creates the target is reconciled through the
  same exact-existing path; it is never overwritten.

After a successful new target link, failures remain
`ExportIndeterminate { expected_hash }`. A process that dies after that link
can call ensure again; it does not need to have received or persisted the
lost receipt.

## Private terrain-demo workflow interface

The package moves orchestration out of `main.rs` into private `journal`,
`workflow`, `report`, and `diagnostic` modules. The binary remains the sole
caller. Tests may use a package library facade, but it is not a foundation
compatibility promise.

The conceptual interface is:

```rust,ignore
fn start_run(
    paths: WorkflowPaths,
    intent: WorkflowIntent,
    limits: WorkflowLimits,
) -> WorkflowJob;

fn resume_run(
    paths: WorkflowPaths,
    intent: WorkflowIntent,
    limits: WorkflowLimits,
) -> WorkflowJob;

fn inspect_run(
    run_root: impl AsRef<Path>,
    limits: JournalLimits,
) -> Result<WorkflowStatus, WorkflowFailure>;
```

`WorkflowPaths` supplies Source, index, Workspace, and Run-root paths on every
start or resume. Paths are not reconstructed from journal bytes. The journal
stores bounded platform-tagged path-binding hashes, and resume rejects any
mismatch before mutation.

`WorkflowIntent` contains:

- caller-supplied nonzero Run and Workspace Operation identities;
- a bounded sorted unique set of explicit Source ordinals to change from the
  Recipe's Ground class to one explicit non-Ground classification;
- the baseline head expected by the caller;
- the normalized Terrain Recipe;
- bounded detached Check Points;
- LandXML options, including the explicit metric-metre assertion; and
- canonical request and input hashes.

The Operation Identity is never generated after mutation begins. The complete
Intent and Workspace binding are durably published before Point selection or
commit.

The Run root is caller-owned and contains fixed recognized children:

```text
run.pwf       append-only journal
run.lock      exclusive process lock
terrain.xml   ensured LandXML target
audit.json    canonical complete report
```

Temporary journal, XML, and report stages are sibling names with a private
fixed prefix. Unknown children are never deleted.

## Journal format

The journal header is fixed-width and checksummed:

```text
magic                 [u8; 8] = "PTWFJ001"
disk_version          u32 = 1
semantic_version      u32 = 1
header_bytes          u32
reserved              u32 = 0
run_id                 [u8; 16]
reserved              [u8; 8] = 0
header_hash            [u8; 32]
```

Frames are append-only:

```text
magic                 [u8; 4] = "PWF1"
frame_version         u16 = 1
kind                  u16
sequence              u64
payload_bytes         u32
reserved              u32 = 0
previous_hash         [u8; 32]
payload               [u8; payload_bytes]
frame_hash            [u8; 32]
```

The first frame chains from the header hash. Sequence begins at zero and is
contiguous. A reader consumes one bounded header and payload at a time and
never deserializes the complete file before enforcing limits.

Exactly eight monotonic frame kinds exist:

1. `Intent`;
2. `RevisionResolved`;
3. `AuditObserved`;
4. `SurfaceObserved`;
5. `QaObserved`;
6. `ExportEnsured`;
7. `ReportEnsured`; and
8. `Complete`.

Cancellation and failures are not appended, so repeated retry cannot grow the
journal without bound. Observation frames contain identities, counts, and
canonical hashes, not complete Terrain, Point, XML, QA, or report payloads.

Start atomically publishes the header plus Intent through a synced sibling
stage, read-back validation, a no-replace hard link, parent-directory sync,
stage removal, and cleanup sync. Any failure after the journal link is an
indeterminate journal publication that resume resolves by validating the
recognized file.

Each later checkpoint is encoded in one bounded frame, appended, flushed, and
synced before acknowledgement. Recovery accepts a torn final suffix only by
truncating to the last fully verified frame and syncing the repair. A complete
frame with a bad hash, sequence, previous hash, reserved field, kind order, or
semantic fact is corruption, not a disposable suffix. Unknown versions are
incompatible.

Every checkpoint is independently revalidated against Workspace state, a
recomputed Revision Audit, a rederived Terrain descriptor, a reevaluated QA
report, the ensured LandXML target, or the canonical report. Journal claims
never override their owning module.

Default journal limits cap the file at 1 MiB, eight frames, each payload at
16 KiB, each supplied path binding at 4 KiB, and journal working memory at
64 KiB. Workflow limits additionally carry the existing Source, index,
Workspace selection/commit/audit, Terrain, QA, LandXML, and report ceilings.
All old-plus-new allocation overlap is charged before allocation.

## Start and resume algorithm

Both entrypoints run one state machine.

### Start

1. Validate the complete request and Run root without mutation.
2. Open and fully verify the Source, prepare/open the Spatial Index, and
   open/create the Workspace.
3. Validate Source, Workspace, classification Attribute, baseline Revision,
   explicit ordinals, and target paths.
4. Publish the journal Intent containing the already chosen Operation
   Identity and bindings.
5. Advance the shared state machine.

If start fails before Intent publication, no Workflow Run exists and no
Workspace mutation was attempted.

### Resume

1. Acquire the exclusive Run lock.
2. Validate and recover the journal under limits.
3. Recompute the complete Intent and path bindings supplied by the caller.
4. Open Source/index/Workspace and fail closed if their identities or schema
   differ.
5. Advance the same state machine.

### Advance

For the optional classification Edit, resolve the recorded Operation Identity
before constructing new mutation state:

- `Committed`: validate parent, request meaning, and Revision;
- `Retryable`: retry the sealed Workspace intent with the same identity;
- `NotRecorded`: proceed only when head still equals the journal baseline,
  rematerialize the exact explicit Point Set, and commit with the same identity;
- `Rejected`: publish a terminal rejected status and safe action; and
- `Indeterminate`: stop and require a later resume without inventing another
  identity.

A commit that returns indeterminate produces no `RevisionResolved` checkpoint.
Resume uses Workspace recovery and the same Operation Identity to establish
the fact.

After Revision resolution, the workflow:

1. appends or validates `RevisionResolved`;
2. computes and validates the Revision Audit and appends `AuditObserved`;
3. derives the baseline and changed Surfaces, computes the private Surface
   Change Envelope, and appends `SurfaceObserved`;
4. evaluates detached Check Points and appends `QaObserved`;
5. calls `ensure_landxml` and appends `ExportEnsured`;
6. deterministically encodes and no-replace ensures `audit.json`, then appends
   `ReportEnsured`;
7. revalidates every final identity and hash, appends `Complete`, and only then
   returns a Workflow receipt.

Terrain, Revision Audit, QA, and the change envelope are recomputed on resume.
Their observation frames detect nondeterminism; they are not persistence or
cache claims.

Crash states are unambiguous:

- before Intent: no Run and no Edit;
- after Intent: resume uses the same Operation Identity;
- after Workspace publication but before its checkpoint: Workspace resolution
  discovers the complete Revision or retryable intent;
- during Audit, Derivation, or QA: rerun immutable work;
- after LandXML publication but before its checkpoint: ensure reconciles the
  exact file;
- after report publication but before its checkpoint: exact report bytes are
  reconciled; and
- after Complete sync but before the caller receives it: resume revalidates
  Complete and returns the same receipt.

## Surface Change Envelope

The workflow compares the baseline and changed immutable Surfaces without
adding a public Terrain comparison seam. It canonicalizes each face by its
three Source-aware Point identities, merge-compares bounded sorted face keys,
and records added/removed face counts and hashes. Envelope bounds include exact
world positions of vertices incident to every added or removed face.

The envelope is conservative and may include unchanged Terrain. It must not be
called an exact change polygon. A Revert test rederives the restored Surface
and proves an empty baseline-to-restored delta.

## Canonical audit report

`audit.json` is deterministic UTF-8 with one fixed schema and key order. It
contains:

- Run, Source, Workspace, baseline/changed Revision, and Operation identities;
- normalized request and binding hashes;
- Edit Point count, Revision transitions, Edit Footprint, and audit hashes;
- baseline and changed Terrain descriptor counts and hashes;
- Surface Change Envelope counts, bounds, and hashes;
- ordered Check Point outcomes and residual statistics;
- LandXML disposition, content hash, bytes, vertex count, and face count;
- every semantic resource ceiling; and
- an explicit statement that partner, downstream, and human-workflow
  acceptance was not evaluated.

Machine identity and elapsed time are evidence observations and never enter
canonical report bytes. The report encoder is bounded, streams through a
hashing writer, and uses the same create/reconcile/no-overwrite discipline as
LandXML.

## Structured failures and recovery actions

The private failure value records the known Source, Workspace, Run, Operation,
and Revision identities; workflow stage; publication phase or certainty when
applicable; stable failure code; bounded diagnostic; and exactly one recovery
action.

Required actions are:

- correct invalid request;
- raise the named limit or narrow the request;
- resume the same Run;
- retry after restoring disk capacity or permissions;
- resolve the recorded Operation Identity by resuming;
- remove or rename a conflicting caller-owned target;
- restore the expected immutable Source; or
- stop and preserve files for investigation.

The app never automatically retries an uncertain mutation, overwrites a
conflicting output, replaces a Source, deletes unknown files, or guesses a new
Operation Identity.

## Verification and fault matrix

All verification remains local. Public and process tests must prove:

- parent cancellation reaches the active Source/index/Workspace/Terrain/QA/
  export child Job and no phase is falsely complete;
- Root, classification, historical, and Revert Revision Audits have exact
  transitions, hashes, and Edit Footprints across Source batch partitions;
- generated LAS and LAZ audits and Workflow reports are byte-identical where
  Source meaning is identical;
- every Revision Audit, journal, report, Terrain, QA, and LandXML limit fails
  before publishing a partial result;
- start/resume at every checkpoint yields the same final report and at most one
  Workspace Revision for the Operation Identity;
- cancellation, injected failure, panic, and lost acknowledgement immediately
  before and after journal Intent, Workspace ready/Revision, checkpoint sync,
  LandXML link/sync, report link/sync, and Complete sync recover to an old or
  complete new fact;
- corrupt, truncated, gapped, forked, oversized, symlinked, or version-mismatched
  journal and output files fail closed;
- a stale Workspace head, changed Source, incompatible index, wrong path
  binding, or conflicting output never mutates the Run or overwrites files;
- the exclusive Run lock prevents concurrent mutation;
- Source bytes are unchanged; and
- all diagnostics remain bounded and name the stage, certainty, identity, and
  only safe recovery action.

The benchmark uses generated public APIs with a 10,000-Point default and
documented 100,000/1,000,000 modes. It separately records cold start, resume
after committed Edit, resume from retryable Workspace intent, LandXML and
report reconciliation, journal/report bytes, phase times, and accounted
resource ceilings. Worker heap is unclaimed unless it is actually measured on
the worker thread. No benchmark is labeled partner or production evidence.

## Explicit exclusions

v0.7 does not add:

- Breaklines, constrained triangulation, arbitrary Terrain vertices, or a
  Workspace schema migration;
- a new public workflow/orchestration crate or trait family;
- general classification queries or editing beyond the bounded explicit
  Ground exclusion used by this caller;
- persistent Terrain Surfaces, terrain cache, resumable triangulation, or
  durable Revision Audit cache;
- an autosave timer, dirty draft, background daemon, or product UI;
- screen/polygon selection, multiple Sources, COPC, remote storage, networking,
  or multi-process mutation;
- CRS transformation, vertical-reference conversion, unit inference, general
  LandXML, or overwrite/update export;
- licensed or partner production datasets, partner tolerance acceptance,
  Civil 3D/Bentley round trips, paid pilots, or human-time claims.

The product-level “design-partner alpha” milestone remains outstanding until
external evidence exists. Completing these repository guarantees only that the
narrow workflow is technically ready to be exercised safely.

## Delivery slices

Implementation proceeds in four reviewable slices:

1. linked child cancellation plus exact Revision Audit and public tests;
2. idempotent LandXML ensure and publication/reconciliation fault tests;
3. private run journal, workflow state machine, report/diagnostic modules, and
   generated LAS/LAZ restart matrix; and
4. benchmark, examples, documentation, independent review, and complete local
   release gates.
