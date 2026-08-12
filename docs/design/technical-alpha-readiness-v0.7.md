# Technical Partner-Alpha Readiness Design (v0.7)

Status: implemented on 2026-08-10 as a repository technical-readiness slice;
external design-partner/product evidence remains outstanding

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

`Created` versus `ReconciledExisting` is an observation of one ensure attempt,
not a canonical Workflow Run fact. A crash after target publication but before
the receipt is observed necessarily turns the next attempt into
`ReconciledExisting`. The workflow journal and `audit.json` therefore record
only the stable semantic outcome `ensured_exact`; attempt disposition is never
hashed into canonical run bytes.

## Private terrain-demo workflow interface

The package moves orchestration out of `main.rs` into private `journal`,
`workflow`, `report`, and `diagnostic` modules. The binary remains the sole
caller. Tests may use a package library facade, but it is not a foundation
compatibility promise.

The implemented package facade is:

```rust,ignore
pub fn start_run(
    paths: WorkflowPaths,
    intent: WorkflowRunIntent,
    limits: WorkflowLimits,
) -> WorkflowJob;

pub fn resume_run(
    paths: WorkflowPaths,
    intent: WorkflowRunIntent,
    limits: WorkflowLimits,
) -> WorkflowJob;

pub fn inspect_run(
    run_root: impl AsRef<Path>,
    limits: WorkflowLimits,
) -> Result<WorkflowStatus, WorkflowFailure>;
```

`WorkflowPaths::new(source, index, workspace, run_root)` fixes the four paths.
The exact intent constructor is
`WorkflowRunIntent::new(run, operation, baseline_revision,
correction_ordinals, non_ground_classification, recipe, check_points,
landxml_options)`. It accepts at most 1,000 correction ordinals and 256 Check
Points, sorts them canonically, requires a nonempty unique ordinal set and
unique Check Point identities, and rejects zero/invalid identities or equal
Ground/replacement classes.

`WorkflowReceipt` exposes Run, Operation, changed Revision, and report
hash/bytes. `WorkflowStatus` exposes Run, Operation, the last semantically
validated durable `WorkflowPhase`, and Complete status. Private journal frame
counts do not cross the application facade. `WorkflowLimits` composes the
public index, Workspace row/selection/commit/audit, Terrain, QA, and LandXML
limits with intent, envelope, journal, report, and aggregate working ceilings.
Builders replace each public child-limit family and expose intent-count,
envelope, journal-byte, report-byte, and aggregate-working controls for
constrained runs and evidence.

`WorkflowPaths` supplies Source, index, Workspace, and Run-root paths on every
start or resume. Paths are not reconstructed from journal bytes. The journal
stores bounded platform-tagged path-binding hashes, and resume rejects any
mismatch before mutation.

`WorkflowRunIntent` contains:

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

The binary is a thin bounded grammar and presentation layer:

```text
terrain-demo start|resume [OPTIONS] SOURCE INDEX WORKSPACE RUN_ROOT
terrain-demo inspect RUN_ROOT
```

Start/resume require `--run-id HEX32`, `--operation-id HEX32`,
`--baseline HEX64`, one or more `--exclude-ground-ordinal N` values, explicit
`--date`/`--time` LandXML values, and `--assert-unknown-crs-metric`. Detached
observations use repeated `--check-point ID,X,Y,Z`; the replacement
classification and Surface name are bounded options. Resume repeats the
identical request. Inspect opens only the Run root and reports Run, Operation,
the last semantically validated durable phase, and Complete status. It may
repair a torn final journal suffix to the last verified frame, but it never
opens Source, index, Workspace, LandXML, or report state.

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

After a torn-suffix repair, inspection revalidates the Run-root directory
identity under the lock. If the root was replaced after the repair became
durable, it returns `PWF_PUBLICATION_INDETERMINATE` at `inspect` with the
indeterminate phase `journal-checkpoint`, rather than claiming the inspected
path is stable.

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

1. Validate the complete request and existing Run root, capture its directory
   identity, and acquire the exclusive `run.lock`.
2. Open and fully verify the Source, prepare/open the Spatial Index, and
   open the existing Workspace. Start never creates a Workspace.
3. Validate Source, Workspace, classification Attribute, baseline Revision,
   explicit ordinals, and target paths.
4. Publish the journal Intent containing the already chosen Operation
   Identity and bindings.
5. Advance the shared state machine.

If start fails before Intent publication, no Workflow Run exists and no
Workspace mutation was attempted. The fixed empty `run.lock` and independently
valid rebuildable index work may exist; neither is a Workflow checkpoint or
Workspace Edit.

The caller creates the Workspace separately through the public
`point-workspace` lifecycle and supplies its current
`workspace.head().provenance().revision()` as the baseline. The classification
example demonstrates Workspace setup and reports Revision identities. The
Workspace's selected `U8` Attribute must be Source Attribute 6, the
`source-las` classification column, as exposed by
`Workspace::schema().classification()`. After retaining the current head
identity, the caller drops all Workspace/Snapshot/PointSet handles so the
Workflow can acquire the exclusive lock. An absent Workspace is
`PWF_INVALID_REQUEST` before Run creation or Workspace mutation.

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
   marks terminal progress and returns a Workflow receipt.

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
- a named source-independent `semantic_results_hash` while retaining every
  exact identity elsewhere in the report;
- Edit Point count, Revision transitions, Edit Footprint, and audit hashes;
- baseline and changed Terrain descriptor counts and hashes;
- Surface Change Envelope counts, bounds, and hashes;
- ordered Check Point outcomes and residual statistics;
- the stable `ensured_exact` LandXML outcome, content hash, bytes, vertex count,
  and face count;
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

The package facade exposes stable getters for `code`, `stage`, `certainty`, an
optional indeterminate `publication_phase`, `recovery_action`, and any known
Run, Source, Workspace, Operation, or Revision identity. The CLI renders that
same bounded information.

The stable codes are `PWF_INVALID_REQUEST`, `PWF_RESOURCE_LIMIT`,
`PWF_CANCELLED`, `PWF_SOURCE_MISMATCH`, `PWF_WORKSPACE_MISMATCH`,
`PWF_STALE_BASELINE`, `PWF_OPERATION_REJECTED`,
`PWF_OPERATION_INDETERMINATE`, `PWF_JOURNAL_CONFLICT`,
`PWF_JOURNAL_CORRUPT`, `PWF_OUTPUT_CONFLICT`,
`PWF_PUBLICATION_INDETERMINATE`, `PWF_IO`, and `PWF_INTERNAL`. Stage names are
`validate`, `lock`, `source`, `index`, `workspace`, `intent-publication`,
`operation-resolution`, `exact-selection`, `commit`, `revision-audit`,
`terrain-derivation`, `surface-change-envelope`, `check-point-qa`,
`landxml-ensure`, `report-ensure`, `complete-checkpoint`, and `inspect`.
Certainty is `pre_publication`, `durable_fact`, or `indeterminate`; the last may
name its exact publication phase.

## Verification and fault matrix

All verification remains local. The implemented component suites cover linked
cancellation; exact Root/classification/historical/Revert Revision Audits;
Revision and LandXML corruption, limits, races, publication faults, and lost
acknowledgements; and the existing Terrain/QA resource and semantic contracts.
Private journal tests exhaust the application-defined Intent-publication
boundaries and the append-before-write, before-sync, and after-sync lost-
acknowledgement boundaries using `Complete`. Private report tests exhaust the
application-defined post-link boundaries. Representative report cases cover
pre-link cancellation/failure, exact and conflicting `AlreadyExists` races,
post-link replacement, target kind, staging/working limits, and stage/parent
directory identity.

The `terrain-demo` package has 35 tests: 18 unit/private tests, 14 public
workflow-facade tests, and three process tests. The 17 public/process tests
prove:

- every prefix of the eight checkpoints resumes to the same final report and
  at most one Workspace Revision for the Operation Identity;
- exact report reconciliation/conflict, LandXML/report recovery, the exclusive
  Run lock, path binding, and representative torn/corrupt journal behavior;
- generated LAS and LAZ runs have matching explicitly source-independent
  `semantic_results_hash` projections where Point meaning is identical, while
  full reports retain and honestly differ on exact Source, Workspace, Run,
  Operation, and Revision identities;
- immediate parent cancellation and an active dropped Workflow publish no
  false Complete checkpoint, remain resumable, and leave Source bytes
  unchanged;
- 12 public resource-limit families stop with `PWF_RESOURCE_LIMIT` and can be
  retried or resumed as their durable prefix permits;
- stale head, differently bound recorded rejection, changed Source, changed
  Workspace identity, and deterministic Retryable Workspace intent fail or
  recover without inventing another Operation identity;
- `Workspace::schema().classification()` rejects any Attribute other than
  Source Attribute 6 before Run or Workspace mutation;
- Run-root validation failures retain the already-known Run, Operation, and
  baseline Revision identities; and
- `start`, `resume`, and `inspect` expose bounded structured CLI diagnostics.

These exhaustive labels apply only to the named application-defined boundary
sets, not every possible operating-system fault. The named report cases,
active-child cancellation, and corrupt frame topology are representative. The
reader's fail-closed validation contract is exercised by torn, version,
reserved-field, sequence, and semantic cases; v0.7 does not claim every
possible corrupt journal topology or OS fault.

The private workflow regression rederives the immediate-head Revert and proves
an empty baseline-to-restored Surface Change Envelope. The LandXML suite
additionally covers cancellation after target link with conservative
indeterminate certainty and exact reconciliation on retry.

The checked-in Criterion benchmark uses generated public APIs and accepts only
the documented 10,000, 100,000, and 1,000,000-Point modes. The completed local
10,000-Point smoke used ten samples:

| Mode | Lower | Estimate | Upper |
|---|---:|---:|---:|
| Cold start | 153.38 ms | 157.84 ms | 161.25 ms |
| Resume after committed Edit | 113.23 ms | 114.88 ms | 117.08 ms |
| Resume from Retryable Workspace intent | 123.76 ms | 126.67 ms | 129.66 ms |
| LandXML and report reconciliation | 96.871 ms | 97.629 ms | 98.365 ms |
| Complete revalidation | 87.233 ms | 88.181 ms | 89.112 ms |

The resulting journal was 2,804 bytes, the canonical report was 11,490 bytes,
and the report named 115 semantic limit facts across eight frames. Worker peak
heap is explicitly unmeasured. These observations use generated local data and
are not labeled partner, production, downstream round-trip, or human-time
evidence.

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

Implementation completed in four reviewable slices:

1. linked child cancellation plus exact Revision Audit and public tests;
2. idempotent LandXML ensure and publication/reconciliation fault tests;
3. private run journal, workflow state machine, report/diagnostic modules, and
   generated LAS/LAZ restart matrix; and
4. benchmark, examples, documentation, independent review, and complete local
   release gates.
