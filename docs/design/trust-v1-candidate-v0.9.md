# Repository Trust and v1 Candidate Design (v0.9)

Status: **Accepted but incomplete alpha — repository implementation and
fixture closure independently reviewed; complete local candidate record
remains a v0.10 prerequisite**

This design is authoritative for the narrow Punctra v0.9 repository slice. Its
base is commit `926ba7f`, the `0.8.0-alpha.1` state. That v0.8 state is an
**incomplete alpha**, not a Complete release: it initially implemented bounded
LandXML file parsing and comparison. Fold-forward v0.10 prerequisite work now
implements Complete-Run binding, canonical evidence publication, full-ceiling
streaming, the owner-local compatibility corpus, and representative recovery
coverage. An independent Standards/Spec review completed on 2026-08-13 with no
P0–P3 findings, but the final one-commit local release record is not retained.
v0.9 folds that unfinished qualification forward rather than declaring v0.8
Complete.

The inherited v0.8 Run-bound qualification closure is a prerequisite to v0.9
readiness. Fold-forward v0.10 work implements the repository-controlled code,
fixture, recovery, support-matrix, and independent-review items, but no focused
green test run substitutes for the complete one-commit local release record.
v0.9 therefore remains incomplete and must not be described as a v1 candidate
ready for release. The accepted [v0.10 professional inspection View
design](field-inspection-view-v0.10.md) carries these gates as prerequisites
rather than relabeling them Complete.

## Outcome

v0.9 makes the already accepted repository slice easier to trust before a v1
candidate is named. It does not add another workflow or format family. It:

1. closes the inherited private Run-bound LandXML qualification path;
2. classifies every supported persisted artifact as authoritative,
   rebuildable, temporary, or caller-owned published output;
3. retains the inherited Spatial Index v1 goldens, completes the remaining
   version-1 fixture corpus, and tests the compatibility promise for each class;
4. hardens failure and recovery at existing persistence seams, beginning with
   faithful Workflow classification of Spatial Index filesystem failures;
5. reviews existing public interfaces and publishes an exact support matrix;
   and
6. runs and records the complete applicable local verification sequence before
   the repository may be called a v1 candidate.

The deletion test for this slice is trust-shaped: deleting the v0.9 work would
reintroduce ambiguous artifact ownership, incomplete old-byte compatibility,
misclassified recoverable I/O, and an unclosed v0.8 qualification claim. It
would not remove a new end-user capability, because none is accepted here.

## Scope discipline

No new public crate, general framework, or feature family is authorized.
Existing deep modules retain their jobs and dependency direction. In
particular, v0.9 does not add:

- another Source adapter, file format, Query, Edit kind, terrain recipe,
  exporter, renderer backend, or workflow;
- general LandXML import, multiple Surfaces, Breaklines, boundaries, CRS or
  vertical-reference conversion, unit inference, or topology repair;
- a persisted Terrain Surface, durable named Point Set, Workspace branch,
  merge, compaction, or automatic migration framework;
- a product UI, installer, updater, hosted service, networking, telemetry,
  licensing, downstream automation, or vendor integration; or
- a broad filesystem abstraction or public fault-injection interface.

The Run-bound verifier remains a private `terrain-demo` module. Persistence
hardening stays behind the existing owning interfaces. Test-only internal seams
may inject exact filesystem faults, but they are not public compatibility
promises. A new public seam requires a separate accepted design and real
variation at that seam.

## Persisted artifact support classes

Cargo versions, disk versions, semantic versions, deterministic recipe
versions, and JSON/XML schemas remain independent. The support class determines
recovery policy; a filename suffix alone never does.

| Class | Included persisted state | v0.9 promise | Failure and recovery rule |
|---|---|---|---|
| **Authoritative** | Workspace manifest, immutable Operation records and Revisions; Workflow `run.pwf`; serialized `SourceRecord` | Supported version-1 bytes remain readable and retain identical identity and semantic facts. They are never silently rebuilt, replaced, downgraded, or deleted. | Fail closed on unknown version, corruption, lineage mismatch, or indeterminate durability. Preserve the bytes and report the one safe reconciliation action. |
| **Rebuildable** | Complete Spatial Index `.pidx` and its valid resumable `.work` prefix | An accepted version may be opened or resumed exactly. When a version or recipe is no longer supported, the caller may explicitly delete the index family and rebuild it from the verified immutable Source. | Never expose a partial index. Preserve valid durable work frames and fail closed on a truncated header. Never delete a pre-existing or racing work path implicitly, and never rewrite an incompatible complete target. |
| **Temporary** | Point Set spills, Workspace scratch candidates, index `.samples`/`.tmp`, and recognized journal/report/LandXML/evidence stages | No cross-release readability promise. Their meaning is process- or attempt-scoped and no authoritative fact depends on their survival before publication. | Remove only when the owning module can bind the current name to the captured file identity atomically. Otherwise retain the temporary name and clear unpublished payload only through an already-owned handle. Published hard-link aliases share, rather than duplicate, payload blocks. A missing or changed live spill fails the operation. Unknown siblings and replacement files are never deleted. |
| **Caller-owned published output** | `terrain.xml`, `audit.json`, and the separate v0.8 Round-Trip Evidence target | Exact existing bytes may be reconciled; different existing bytes are a conflict. These outputs are not Workspace state and are not silently migrated. | Use synced staging, no-replace publication, read-back, directory sync, and conservative post-publication certainty. Never overwrite the caller's target. |

`SourceRecord` is authoritative persisted evidence of which immutable Source was
accepted; the Source bytes remain authoritative for Point values. Reopening a
record never adopts changed bytes or assigns a replacement Source Identity.
The Spatial Index may copy bounded exact ticks for display samples, but it
remains an accelerator and never becomes Source authority.

The Run journal is authoritative for durable Workflow facts only after each
frame has been revalidated against its owning module. A checkpoint is not a
second Workspace, Source, LandXML, or report authority. Qualification reads a
Complete Run without repairing it and never writes another checkpoint.

## Version-1 support matrix

The initial v0.9 matrix keeps the base formats unchanged:

| Interface or format | Candidate support | Explicit boundary |
|---|---|---|
| `SourceRecord` schema 1 / Source contract 1 | Read, write, Full reopen, and eligible Fast reopen through the official memory and LAS/LAZ adapters | Fast evidence never overrides a mismatch; unknown record versions fail explicitly. |
| Local LAS | Point-data record formats 0–10 through the existing bounded Source interface | No Source rewriting, remote object access, COPC, or inferred Coordinate Reference. |
| Local LAZ | Point-data record formats 0–8 through the existing bounded Source interface | Formats 9 and 10 remain explicitly unsupported pending exact layered WavePacket14 decoding. |
| Spatial Index disk 1 / recipe 1 | Build, open, exact candidate lookup, display reads, valid-prefix resume, and explicit rebuild | The index is not authoritative and an incompatible target is never replaced automatically. |
| Spatial Index disk 2 / recipe 2 | v0.10 build, open, valid-prefix resume, and bounded display reads retaining exact raw intensity/classification and optional all-channel RGB samples | Rebuildable inspection cache only. It is deliberately incompatible with disk 1 at the same path, never becomes Source/Query authority, and is not a v0.9 format. |
| Workspace disk 1 / semantic 1 | Create, open, exact selection/rows, sparse classification Revision, immediate-head Revert, Operation recovery, and Revision Audit | No migration, branch, merge, compaction, general Attribute/position Edit, or multi-writer support. |
| Workflow Run disk 1 / semantic 1 / frame 1 | Exactly the existing eight-frame headless classification-to-terrain Run, including resume and inspect | The v0.8 qualifier accepts only a strictly revalidated Complete Run and does not mutate or repair it. |
| `audit.json` schema `punctra.terrain-workflow.audit.v1` | Exact create/reconcile for the existing Run | Repository evidence only; not partner or product acceptance. |
| LandXML | Existing metric-metre LandXML 1.2 single-TIN points/faces export plus the v0.8 narrow returned-file comparison | No general import, unit conversion, CRS interpretation, Breaklines, boundaries, or multiple Surfaces. |
| Round-Trip Evidence schema `punctra.terrain-demo.landxml-round-trip-evidence.v1` | Strict Complete-Run qualification plus canonical pass/fail create-or-exactly-reconcile publication | Caller application/version/settings are declarations, not observed execution. |
| Renderer-neutral and wgpu interfaces | Existing bounded generation, planning, update, rendering, and picking behavior | GPU values remain disposable display data; caller owns device, submission, and host application policy. |

The repository's reference verification environment remains the documented
local Apple arm64/macOS machine with Rust 1.90.0. Unix and Windows file-identity
implementations remain in the code, but a platform is not promoted to a
verified v1-candidate support tier without a recorded complete local matrix on
that platform. Other platforms fail closed where stable file identity is
required. Cross-platform intent is not cross-platform evidence.

## Golden fixture compatibility plan

The v0.8 base already pins one complete Spatial Index disk-1 artifact and one
resumable disk-1 work file under the owning crate, and tests open or resume
those checked-in bytes. v0.9 extends that owner-local pattern to every claimed
authoritative format before any version-2 persisted format or v1 compatibility
claim is accepted. Golden bytes are captured once from fixed, generated,
non-secret inputs and committed with their exact byte lengths, BLAKE3 hashes,
format/recipe versions, and expected semantic facts. Tests read the committed
bytes; they do not regenerate the expected side during the test.

The minimum corpus contains:

- a serialized SourceRecord version 1 with fixed Source identity, metadata,
  adapter facts, content hash, and bounded Fast evidence;
- the existing complete index disk-1/recipe-1 artifact and work-file fixtures,
  extended with mutation cases or manifest facts where the support matrix needs
  them;
- a Workspace disk-1/semantic-1 root, a committed Revision, a retryable ready
  Operation, and a recorded rejection, with fixed lineage and identities;
- Run disk-1/semantic-1/frame-1 prefixes at every checkpoint boundary plus one
  Complete eight-frame Run and its exact `terrain.xml` and `audit.json`; and
- after inherited v0.8 closure, canonical passing and semantic-failure
  Round-Trip Evidence version-1 files.

For authoritative fixtures, every later supporting release must open the old
bytes without mutation and reproduce the recorded identities, ordering,
hashes, semantic results, and recovery classifications. Deterministic writers
must reproduce exact bytes when supplied the same semantic inputs. Unknown
future versions, truncation, checksum changes, lineage forks, and mismatched
Source bindings are mutation fixtures and must fail with the stable public
error family without panic or partial publication.

For the rebuildable index, golden tests prove exact open, valid-prefix resume,
and clean rebuild equivalence for every version listed as supported. A future
release may retire an old index version only by updating the support matrix,
continuing to fail it explicitly, and documenting deletion-and-rebuild from the
verified Source. It may not reinterpret old bytes.

Temporary spills and stages deliberately have no cross-release golden-open
promise. Instead, fault fixtures prove recognition, conservative retention
when current pathname identity cannot be proven, replacement protection, and
owned-handle clearing of unpublished payload bytes. Published aliases share
the immutable target inode and do not duplicate its physical payload blocks.
Temporary bytes never count as a published authoritative fact. Golden fixtures
are generated technical evidence; they are not licensed production data or
downstream-application evidence.

Workspace selection and commit temporary ceilings apply to one attempt, not to
the sum of recognized names retained from prior attempts. Destructors clear
unpublished payload through the owned handle on a best-effort basis; a
truncate or sync failure may leave the already-bounded payload for explicit
operator cleanup, but never authorizes deleting the current pathname.

## Failure and recovery hardening

Every persistence path is reviewed at create, write, flush, close/reopen,
link, target verification, parent-directory sync, and cleanup boundaries. The
owning module must preserve the original operating-system error, operation,
and path; publish conservative certainty; and choose exactly one safe recovery
action. Cancellation and resource limits never become partial success.

The class rules are asymmetric by design:

- authoritative state is preserved and reconciled, never guessed away;
- rebuildable state may retain a verified prefix or be explicitly rebuilt;
- temporary payload may be cleared only through a captured owned handle;
  recognizable path aliases are retained when portable identity-conditional
  pathname deletion is unavailable; and
- caller-owned targets are reconciled exactly or reported as conflicts.

No recovery code scans and cleans a broad directory, follows a symlink, removes
an unknown child, overwrites a target, or retries an uncertain Workspace
mutation with a new Operation Identity.

### First implementation slice: Workflow index I/O taxonomy

The first v0.9 implementation slice is deliberately small:

1. `terrain-demo` maps the `IndexError::Io` and allocation-free
   `IndexError::SharedPathIo` filesystem variants at the `index` stage to `PWF_IO` with
   conservative `indeterminate(index-target)` certainty and the recovery action
   “restore disk capacity or permissions, then resume the same Run.”

The mapping keeps the operation, path, and operating-system error in the
bounded diagnostic instead of collapsing the failure into `PWF_INTERNAL`; its
rendering remains subject to the existing 1,024-byte diagnostic cap. Its
focused short-path test asserts the full diagnostic as well as the stable code,
stage, certainty, and recovery action. The index filesystem variants do not expose whether
the failure happened before or after complete-target publication, so the
Workflow does not claim `pre_publication` or a durable fact; resuming reconciles
an absent target, a valid resumable work prefix, or a complete rebuildable
target. This slice does not alter `point-index` persistence, close the inherited
v0.8 qualification prerequisite, or make v0.9 ready.

The initial work-header recovery gap is closed in v0.10 prerequisite work. The
index owner writes and syncs the complete header under a uniquely owned
temporary name, publishes the final `.pidx.work` name with a no-replace hard
link, and syncs the parent before retiring and retaining its temporary alias. A write or sync
failure before publication exposes no `.work` path; a failure after publication
leaves a complete retryable header. Pre-existing empty, truncated, racing, and
caller-owned paths are preserved and fail closed rather than being claimed or
deleted. A parent-directory-sync failure after a valid header never authorizes
deletion.

Later fault coverage uses narrow private seams or owned filesystem fixtures to
exercise representative pre-publication, post-link, sync, replacement, and
cleanup failures for each artifact class. It does not claim to simulate every
kernel, filesystem, power-loss, or hardware failure.

The representative owner-local matrix is concrete rather than inferred from a
generic “I/O failed” test:

| Artifact class | Representative repository evidence |
|---|---|
| Authoritative Workspace/Revision/Operation | `point-workspace` persistence tests inject candidate-stage, ready-intent, Revision, rejection, parent-sync, cancellation, and lost-acknowledgement failures; reopen either recovers one complete known state or fails closed. Symlinked private directories/leaves and unknown children are preserved. |
| Authoritative Workflow Run | `terrain-demo` workflow/journal tests exercise every checkpoint prefix, torn suffix repair only in the owning resume/inspect path, corrupt frames, lock and Run-root replacement, aggregate limits, cancellation, and exact report/LandXML reconciliation. Read-only qualification never invokes repair. |
| Rebuildable Spatial Index | `point-index` persistence/interface tests cover create/write/sync/publication failures, ownership-safe initial-header retry, valid-prefix resume, complete open, checksum/schema/source/recipe mismatch, v1/v2 separation, no-replace races, and exact temporary-disk accounting. |
| Temporary spills and stages | Workspace Point Set tests reject missing or replaced live spills, conservatively retain Workspace scratch names, and clear unpublished payload only through the owned handle; published aliases share immutable payload blocks. Index, LandXML, report, and evidence stage guards verify and retain their unique aliases without mutating replacement paths. No temporary file is reported as a durable result. |
| Caller-owned `terrain.xml`, `audit.json`, and Round-Trip Evidence | LandXML, report, and evidence publishers inject pre-link and every post-link acknowledgement boundary, exact/conflicting create races, parent/target replacement, cancellation, cleanup, and lost acknowledgement. Existing exact bytes reconcile; different or non-regular targets are never replaced. |

The matrix asserts the stable owning error family, publication certainty, and
retry action wherever that layer exposes them. An operating-system failure is
not reclassified as semantic failure, and an indeterminate acknowledgement is
never reported as a definitive absence.

## Inherited Run-bound qualification closure

Before v0.9 can reach readiness, the two pending v0.8 delivery slices are
implemented without changing the v0.7 Run:

1. `verify-round-trip` strictly opens a non-mutating Complete Run, revalidates
   the journal, request, Source/Workspace/Revision identities, `terrain.xml`,
   and `audit.json`, compares the caller-returned narrow LandXML under the
   declared policy, and creates or exactly reconciles canonical pass/fail
   evidence outside the Run root; and
2. the end-to-end generated matrix, streaming input coverage through the full
   accepted v0.7 LandXML export ceiling, canonical fixture coverage,
   documentation, independent review, and complete local release gates pass.

A torn or non-Complete Run is rejected and is never repaired by qualification.
Operational failure publishes no final pass or fail evidence. Semantic failure
may publish canonical failed evidence only after every prerequisite fact was
successfully evaluated. The v0.7 journal remains disk/semantic/frame version 1
with exactly eight frames, and `audit.json` remains byte-compatible schema v1.

The first item and the repository-controlled implementation, matrix, streaming,
canonical-fixture, and independent-review portions of the second item are
implemented in the v0.10 prerequisite work. The complete one-commit local
release record still has to close before v0.8/v0.9 may be described as
complete.

### Current repository prerequisite ledger

This ledger separates implemented repository evidence from the one remaining
local release-process gate:

| Gate | Current state | Exact remaining repository work |
|---|---|---|
| Strict Complete-Run qualification and canonical evidence | Implemented with shared existing Run and journal locks, retained stable artifact witnesses, pass/fail evidence, no-replace publication, exact-existing reconciliation, every application-defined new/existing-target acknowledgement boundary, create races, replacement preservation, and retry reconciliation | None. |
| Accepted v0.7 LandXML ceiling | Implemented by the bounded local XML stream/parser at 4 GiB, 10 million Points, and 20 million faces, with borrowed `quick-xml` token views, XML-token, parser-working, retained-working, and comparison ceilings plus accounted peaks | None. Generated boundary, over-boundary, and sparse 4-GiB rejection coverage exercises the same CLI path. |
| Generated end-to-end matrix | Run-bound process coverage includes exact/presentation-only pass, inclusive tolerance boundary, unit/topology/ambiguity failure evidence, malformed/unsupported/resource operational rejection, non-Complete and changed Runs, exact/conflicting reconciliation, and Run non-mutation. Private publisher tests inject create races and every acknowledgement fault. | None. Fault injection remains owner-private rather than adding a public process backdoor. |
| Golden persisted compatibility corpus | Owner-local immutable fixtures cover SourceRecord v1; Workspace root/committed/ready/rejected states; Spatial Index v1/v2 complete/work artifacts; every Workflow Run checkpoint prefix plus exact artifacts; and canonical passing/topology-failed evidence. Tests pin manifests/bytes and future-version, truncation, checksum, lineage, path/Source-binding, and exact-reopen behavior. | None. Generated fixtures are technical evidence only. |
| Artifact-class recovery matrix | Owner-local fault tests cover authoritative Workspace/Run state, serialized SourceRecord mismatch rules, rebuildable index state including all six initial-header boundaries, temporary spill/stage identity, and no-replace LandXML/report/evidence outputs. Replacement paths are preserved and uncertain publication is never acknowledged as absent or successful. | None. |
| Support matrix and interface review | The version-1 table is reconciled with the separate v0.10 index-v2 promise; every foundation crate publishes its exported-surface class in crate rustdoc and has direct interface tests. Independent Standards/Spec review of `3dc4cb1` through the v0.10 working tree completed on 2026-08-13 with zero P0–P3 findings on both axes. | None. |
| Local candidate record | Focused `terrain-demo` formatting, Clippy, tests, and rustdoc pass for the Run-bound slice | Run the entire `CONTRIBUTING.md` sequence at one recorded commit, including fuzz manifests, examples, all benchmarks, process tests, and both `PUNCTRA_REQUIRE_GPU=1` acceptance gates; record machine, OS, filesystem, toolchain, adapter/backend, scales, and outcomes. |

The downstream-firm, paid-pilot, conversion, and measured-savings gates below
require external evidence. They cannot be closed by more repository fixtures or
local commands and are not included in the repository-completion rows above.

## Interface review

v0.9 reviews existing interfaces; it does not assume that every public Rust
item is a v1 promise. Each module owner records:

- its one-job sentence and the exact caller-visible interface;
- invariants, ordering, determinism, limits, performance class, side effects,
  persisted effects, error categories, and recovery certainty;
- whether the seam has at least two real adapters or otherwise has a direct
  real caller that earns it;
- direct interface tests that use no private implementation access; and
- one classification: **v1 candidate**, **version-coupled adapter-author
  seam**, **test support**, or **private application surface**.

The owner review classifies the complete exported surface by owning crate or
explicit exception. Each foundation crate repeats its classification in
crate-level rustdoc so a caller does not have to infer compatibility intent
from this design:

| Owner | Exported surface classification | Direct evidence and boundary |
|---|---|---|
| `foundation-runtime` | v1 candidate | Unit and doc tests exercise cancellation, progress, owned Jobs, and fused Batch Streams. No executor or persistence policy is exported. |
| `point-contracts` | v1 candidate | Direct value/validation tests cover Source and Point identity, positions, Attributes, metadata, provenance, and bounded diagnostics. No I/O or scheduling is exported. |
| `point-source` | v1 candidate, except `adapter` | Interface tests exercise Full/Fast verification, records, requests, summaries, limits, cancellation, and change detection. `adapter` is the version-coupled official-adapter seam. |
| `source-memory` | v1-candidate official adapter; `MemoryFaultControl` is test support | Adapter and conformance tests use the same public Source path. The `test-support` feature does not become a production fault interface. |
| `source-las` | v1-candidate official adapter | LAS/LAZ interface, format, corruption, limits, sparse-read, cancellation, and fixture tests call only path-based open functions; decoder integration stays private. |
| `point-index` | v1 candidate | Interface tests call preparation, candidate lookup, hierarchy observation, and bounded node reads. Disk 1/recipe 1 and disk 2/recipe 2 are separately versioned rebuildable-cache contracts, not authority. |
| `point-workspace` | v1 candidate | Selection, row-stream, commit/recovery, reopen, audit, cancellation, and limit tests call the exported Workspace/Snapshot jobs and values. File layout and fault hooks stay private. |
| `point-terrain` | v1 candidate | Terrain, Check Point, LandXML, cancellation, resource, and benchmark tests call the exported jobs and values. Triangulation and publication stages stay private. |
| `render-protocol` | v1 candidate | Contract tests exercise cameras, viewports, generation transitions, limits, and atomic updates without a renderer. |
| `point-view` | v1 candidate | Planner interface and benchmark tests cover visibility, perspective/orthographic SSE, budgets, ordering, retirement, and cancellation-free determinism. Host loading policy stays private. |
| `render-wgpu` | v1 candidate | Required-GPU offscreen tests exercise update, render, projection, large-world positioning, depth, and picking through the exported renderer. Shader and allocation layout stay private. |
| `terrain-demo` and `renderer-demo` | private application surface | CLI/process/GPU tests prove the supported examples and evidence paths. Their syntax, journals, report helpers, corpus runner, and unpublished test facade are not foundation interfaces. |

“v1 candidate” in this table is a review classification only. It becomes a
release-readiness claim only after the complete fixture, recovery, and local
matrix below passes at one recorded commit.

`point-source` adapter-author traits remain version-coupled to official
adapters. Feature-gated fault controls remain test support. Demo package
facades, CLI syntax, private journal/round-trip modules, and examples do not
become stable product interfaces merely because they are callable in repository
tests. Any pre-v1 breaking cleanup found by the review must be explicit,
minimal, locally migrated in one slice, and reflected in documentation and
interface tests. No speculative wrapper is added to preserve a shallow seam.

The review is complete only when the support matrix and rustdoc agree and no
supported caller must learn a private stage, filesystem layout, decoder,
triangulator, or renderer implementation detail.

## Local verification and readiness gate

All verification runs locally. No GitHub Actions or other hosted CI is added.
The first hardening slice runs its focused `terrain-demo` test, then the
relevant formatting, Clippy, workspace-test, and rustdoc gates. Final
v0.9 readiness requires the complete command sequence in `CONTRIBUTING.md`,
including every documented example, process test, and Criterion benchmark.
When a local GPU adapter is expected, both GPU acceptance commands run with
`PUNCTRA_REQUIRE_GPU=1` so a missing adapter is a failure rather than a skip.

The release record names the commit, pinned toolchain, machine/OS, filesystem,
GPU adapter/backend, commands, outcomes, benchmark scales, and any explicitly
unsupported platform. A generated benchmark establishes only the stated local
resource and performance observation. Documentation activation or a partial
test pass is not repository readiness.

v0.9 may be labeled a v1 candidate only when all of these are true:

- inherited Run-bound qualification and full-ceiling streaming closure pass;
- every artifact has one support class and every supported version appears in
  the matrix;
- authoritative version-1 and published-evidence golden fixtures pass without
  mutation, and rebuildable/temporary fixtures obey their distinct policies;
- the representative failure/recovery matrix, including ownership-safe initial
  index-header recovery and Workflow I/O mapping, passes;
- the implemented interface classifications and documentation are independently reviewed; and
- the complete applicable local verification sequence passes at one recorded
  commit with required GPU acceptance.

“v1 candidate” means that this repository compatibility surface is ready for
candidate evaluation. It does not by itself mean `1.0.0`, product readiness,
production support, or satisfaction of any external evidence gate.

## External evidence boundary and nonclaims

v0.9 preserves the exact separation between repository evidence and product
evidence. It does not claim that Punctra observed a named downstream
application run, that caller-declared settings were applied, or that a vendor
certified the output. It supplies no licensed production dataset,
above-500-million-Point result, firm acceptance, paid pilot, continuing paid
use, conversion, or measured labor-savings evidence.

The v0.8 external product gates remain outstanding: three distinct firms with
accepted pipeline results, three distinct paid pilots with production-use
evidence, and two distinct firms with conversion or measured labor savings.
Canonical Round-Trip Evidence, golden files, generated fixtures, local fault
injection, benchmarks, and a v1-candidate label satisfy none of those gates.

## Delivery slices

Implementation is divided into independently reviewable slices:

1. **Implemented:** preserve the `IndexError` filesystem I/O variants as
   recoverable `PWF_IO` at the
   Workflow index stage, including bounded operation, path, and source-error
   rendering.
2. **Implemented inherited qualification closure:** strict Complete-Run
   binding, canonical pass/fail evidence, exact no-replace
   publication/reconciliation, and stable operational versus semantic
   diagnostics.
3. **Implemented full-ceiling and golden coverage:** bounded streaming at the
   accepted export ceiling, generated end-to-end comparison, and the remaining
   owner-local version-1 fixture corpus and mutation cases.
4. **Implemented artifact recovery hardening:** all six initial index-header
   boundaries without pathname-race deletion and the class-based representative
   matrix across authoritative Workspace/Run/SourceRecord state, rebuildable
   index state, temporary spills/stages, and caller-owned outputs.
5. **Implemented candidate review:** the exact support matrix,
   exported-surface rustdoc, direct interface tests, and architecture/user
   documentation are aligned; independent Standards/Spec review completed
   with no P0–P3 findings. The complete one-commit local release record remains.

No slice may be described as v0.8 Complete, v0.9 ready, a v1 candidate, or
external product evidence before its corresponding gate is actually satisfied.
