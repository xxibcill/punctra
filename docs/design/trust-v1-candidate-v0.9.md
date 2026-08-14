# Repository Trust and v1 Candidate Design (v0.9)

Status: **Implemented and Complete — repository-verified version-1
compatibility candidate; external product evidence remains outstanding**

This design is authoritative for the narrow Punctra v0.9 repository slice. Its
base is commit `9a8363a0d807990209f8252d93229c7f9464c923`, the completed
`0.8.0-alpha.1` repository interoperability-qualification slice. That base
already includes strict Complete-Run binding, canonical pass/fail evidence,
full-ceiling streaming coverage, and the v0.8 local repository gates. It does
not complete the external design-partner product MVP.

v0.9 freezes and hardens that existing repository surface rather than adding a
feature family. Its version-1 fixtures, artifact policies, interface review,
support matrix, and local qualification together establish a repository v1
candidate. They do not substitute for external product evidence or constitute
`1.0.0`.

## Outcome

v0.9 makes the already implemented repository slice easier to trust. It does
not add another workflow or format family. It:

1. retains and further qualifies the private Run-bound LandXML path completed
   in v0.8;
2. classifies every supported persisted artifact as authoritative,
   rebuildable, temporary, or caller-owned published output;
3. retains the inherited Spatial Index v1 goldens, completes the remaining
   version-1 fixture corpus, and tests the compatibility promise for each class;
4. hardens failure and recovery at existing persistence seams, including
   faithful Workflow classification of filesystem and publication failures;
5. reviews existing public interfaces and publishes an exact support matrix;
   and
6. runs and records the complete applicable local verification sequence for
   the repository candidate.

The deletion test for this slice is trust-shaped: deleting the v0.9 work would
reintroduce ambiguous artifact ownership, incomplete old-byte compatibility,
misclassified recoverable I/O, and weaker qualification evidence. It
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
| **Rebuildable** | Complete Spatial Index `.pidx` and its valid resumable `.work` prefix | An accepted version may be opened or resumed exactly. A successful build retains its bounded valid `.work`; the complete artifact wins on later opens. When a version or recipe is no longer supported, the caller may explicitly delete the index family and rebuild it from the verified immutable Source. | Never expose a partial index. Preserve valid durable work frames and fail closed on a truncated header. Never delete a pre-existing or racing work path implicitly, and never rewrite an incompatible complete target. |
| **Temporary** | Point Set spills, Workspace scratch candidates, index `.samples` files and platform-specific publication stages, and recognized journal/report/LandXML/evidence stages | No cross-release readability promise. Their meaning is process- or attempt-scoped and no authoritative fact depends on their survival before publication. | Production code never unlinks a replaceable temporary pathname. Linux index, journal, report, and evidence publication stages are unnamed; LandXML uses a retained named encoding stage plus a separate unnamed Linux publication copy. Named stages and spools remain per-attempt bounded debris. An operator may remove only the owning family's private names while no related handle, job, or process is live. A missing or changed live spill fails the operation. Unknown siblings and replacements are never deleted. |
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

The frozen v0.9 matrix keeps the base formats unchanged:

| Interface or format | Candidate support | Explicit boundary |
|---|---|---|
| `SourceRecord` schema 1 / Source contract 1 | Read, write, Full reopen, and eligible Fast reopen through the official memory and LAS/LAZ adapters | Fast evidence never overrides a mismatch; unknown record versions fail explicitly. |
| Local LAS | Point-data record formats 0–10 through the existing bounded Source interface | No Source rewriting, remote object access, COPC, or inferred Coordinate Reference. |
| Local LAZ | Point-data record formats 0–8 through the existing bounded Source interface | Formats 9 and 10 remain explicitly unsupported pending exact layered WavePacket14 decoding. |
| Spatial Index disk 1 / recipe 1 | Build, open, exact candidate lookup, display reads, valid-prefix resume, and explicit rebuild | The index is not authoritative and an incompatible target is never replaced automatically. |
| Workspace disk 1 / semantic 1 | Create, open, exact selection/rows, sparse classification Revision, immediate-head Revert, Operation recovery, and Revision Audit | No migration, branch, merge, compaction, general Attribute/position Edit, or multi-writer support. |
| Workflow Run disk 1 / semantic 1 / frame 1 | Exactly the existing eight-frame headless classification-to-terrain Run, including resume and inspect | The v0.8 qualifier accepts only a strictly revalidated Complete Run and does not mutate or repair it. |
| `audit.json` schema `punctra.terrain-workflow.audit.v1` | Exact create/reconcile for the existing Run | Repository evidence only; not partner or product acceptance. |
| LandXML | Existing metric-metre LandXML 1.2 single-TIN points/faces export plus the v0.8 narrow returned-file comparison | No general import, unit conversion, CRS interpretation, Breaklines, boundaries, or multiple Surfaces. |
| Round-Trip Evidence schema `punctra.terrain-demo.landxml-round-trip-evidence.v1` | Exact create/reconcile for canonical pass and supported semantic-failure results after strict Complete-Run binding | Caller application/version/settings are declarations, not observed execution. |
| Renderer-neutral and wgpu interfaces | Existing bounded generation, planning, update, rendering, and picking behavior | GPU values remain disposable display data; caller owns device, submission, and host application policy. |

The repository's reference verification environment remains the documented
local Apple arm64/macOS machine with Rust 1.90.0. Unix and Windows file-identity
implementations remain in the code, but a platform is not promoted to a
verified v1-candidate support tier without a recorded complete local matrix on
that platform. Other platforms fail closed where stable file identity is
required. Cross-platform intent is not cross-platform evidence.

## Golden fixture compatibility plan

The v0.8 base pins one complete Spatial Index disk-1 artifact and one resumable
disk-1 work file under the owning crate, and tests open or resume those
checked-in bytes. v0.9 extends that owner-local pattern to every claimed
authoritative format. Golden bytes are captured once from fixed, generated,
non-secret inputs and committed with their exact byte lengths, BLAKE3 hashes,
format/recipe versions, and expected semantic facts. Tests read the committed
bytes; they do not regenerate the expected side during the test.

The completed corpus contains:

- a serialized SourceRecord version 1 with fixed Source identity, metadata,
  adapter facts, content hash, and bounded Fast evidence;
- the existing complete index disk-1/recipe-1 artifact and work-file fixtures,
  extended with mutation cases or manifest facts where the support matrix needs
  them;
- a Workspace disk-1/semantic-1 root, a committed Revision, a retryable ready
  Operation, and a recorded rejection, with fixed lineage and identities;
- Run disk-1/semantic-1/frame-1 prefixes at every checkpoint boundary plus one
  Complete eight-frame Run and its exact `terrain.xml` and `audit.json`; and
- canonical passing and semantic-failure Round-Trip Evidence version-1 files.

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
promise. Instead, fault fixtures prove recognition, ownership-safe retention
and offline cleanup policy, replacement protection, and that temporary bytes
never count as a published authoritative fact. Golden fixtures are generated
technical evidence; they are not licensed production data or downstream-
application evidence.

## Failure and recovery hardening

Every persistence path is reviewed at create, write, flush, close/reopen,
link, target verification, parent-directory sync, and cleanup boundaries. The
owning module must preserve the original operating-system error, operation,
and path; publish conservative certainty; and choose exactly one safe recovery
action. Cancellation and resource limits never become partial success.

The class rules are asymmetric by design:

- authoritative state is preserved and reconciled, never guessed away;
- rebuildable state may retain a verified prefix or be explicitly rebuilt;
- temporary state is retained when pathname cleanup cannot prove ownership at
  the unlink instant, and is removed only by explicit offline maintenance; and
- caller-owned targets are reconciled exactly or reported as conflicts.

For LandXML, the no-replace copy is not acknowledged from a pathname-only
read-back. The owner retains an open target witness, syncs that destination
before syncing its parent directory, and revalidates the open file against the
leaf name after parent sync and terminal progress. Exact reconciliation retains
the same witness through its final acknowledgement boundary. A missing,
replaced, or changed leaf yields no receipt.

No recovery code scans and cleans a broad directory, follows a symlink, removes
an unknown child, overwrites a target, or retries an uncertain Workspace
mutation with a new Operation Identity.

### Workflow persistence I/O taxonomy

`terrain-demo` maps `IndexError::Io` at the `index` stage to `PWF_IO` with
conservative `indeterminate(index-target)` certainty and the recovery action
“restore disk capacity or permissions, then resume the same Run.”

The mapping keeps the operation, path, and operating-system error in the
bounded diagnostic instead of collapsing the failure into `PWF_INTERNAL`; its
rendering remains subject to the existing 1,024-byte diagnostic cap. Its
focused short-path test asserts the full diagnostic as well as the stable code,
stage, certainty, and recovery action. `IndexError::Io` does not expose whether
the failure happened before or after complete-target publication, so the
Workflow does not claim `pre_publication` or a durable fact; resuming reconciles
an absent target, a valid resumable work prefix, or a complete rebuildable
target. The broader v0.9 hardening applies the same principle at journal,
Workspace, terrain, report, evidence, and qualification seams: preserve the
typed cause, distinguish prepublication from indeterminate publication when
known, and give exactly one safe recovery action.

The index-owner recovery slice closes the initial work-header gap. It writes
and syncs the complete fixed-size header to a private file, then publishes that
open file descriptor to `.pidx.work` with one atomic no-replace operation. A
write or file-sync failure therefore leaves the canonical work path absent. A
pre-existing or racing canonical path wins unchanged. A
parent-directory-sync failure after publication reports the original I/O error
and retains the valid header; it never authorizes deletion.

On the verified local macOS filesystem, descriptor publication uses atomic
`fclonefileat`: the already immutable, synced header or complete artifact is
cloned into a destination name that must not exist. The destination has
independent copy-on-write bytes, so all artifact writes and file syncs finish
before cloning. The owner then opens the canonical destination without
following symlinks, proves that it has the platform-expected identity relation
to the stage, flushes that destination inode, syncs the parent directory, and
revalidates the same canonical leaf and exact header or artifact checksum
before acknowledging publication. Complete-artifact opening likewise retains
one descriptor witness through decoding and final hashing, so even a same-size
leaf replacement is an indeterminate I/O failure rather than a returned index.
Linux creates the stage as an unnamed `O_TMPFILE` in the target directory and
uses `linkat` through its `/proc/self/fd` descriptor path with
`AT_SYMLINK_FOLLOW` to assign exactly one no-replace canonical name. If the
filesystem lacks `O_TMPFILE`, procfs descriptor linking is unavailable, or the
kernel rejects either operation, preparation fails closed with no canonical
publication. Other platforms fail closed. In both implementations the target
leaf is resolved relative to a directory opened from the caller-provided
parent at publication time. The verified scope assumes that caller-controlled
parent namespace remains stable for the operation; concurrent parent-directory
replacement is not claimed as a supported workflow.

Linux initial-header and artifact stages have no pathname before publication,
so a failed stage disappears when its descriptor closes and a successful stage
has exactly the canonical name. Named stages on other platforms and named
sample spools remain per-attempt bounded recognized temporary debris because no
portable conditional-unlink primitive can prove that a pathname still names
the owned open file; retry ignores those private names. Publication is bound to
the owned open file rather than its replaceable pathname. Completed valid
`.work` files likewise remain beside `.pidx`, which wins on later opens without
inspecting, mutating, or deleting the work path. A metadata check followed by
pathname deletion is never used because a racing replacement can arrive
between those operations.

Fault coverage uses narrow private seams or owned filesystem fixtures to
exercise representative pre-publication, post-link, sync, replacement, and
cleanup failures for each artifact class. It does not claim to simulate every
kernel, filesystem, power-loss, or hardware failure.

## Run-bound qualification retained from v0.8

The completed v0.8 delivery is retained without changing the v0.7 Run:

1. `verify-round-trip` strictly opens a non-mutating Complete Run, revalidates
   the journal, request, Source/Workspace/Revision identities, `terrain.xml`,
   and `audit.json`, compares the caller-returned narrow LandXML under the
   declared policy, and creates or exactly reconciles canonical pass/fail
   evidence outside the Run root; and
2. the end-to-end generated matrix, streaming input coverage through the full
   accepted v0.7 LandXML export ceiling, canonical fixture coverage,
   documentation, independent review, and local release gates remain covered.

A torn or non-Complete Run is rejected and is never repaired by qualification.
Operational failure publishes no final pass or fail evidence. Semantic failure
may publish canonical failed evidence only after every prerequisite fact was
successfully evaluated. The v0.7 journal remains disk/semantic/frame version 1
with exactly eight frames, and `audit.json` remains byte-compatible schema v1.

The checked-in qualification corpus pins every journal checkpoint prefix and
the Complete journal, report, LandXML, and passing/failing evidence bytes. Its
Run-root path binding intentionally prevents relocating those bytes and then
calling the public verifier as though the copy were the original Run. The
corpus consumer therefore uses the owning strict journal, report, parser, and
evidence interfaces to verify the committed bytes without regenerating the
expected side; generated process tests exercise the complete public command at
the Run path captured in its request.

v0.9 also tightens the inherited caller-output publication protocol. On macOS,
the synced open stage descriptor is published as an independent no-replace
clone; on the unverified Linux path, an unnamed `O_TMPFILE` publication copy
is linked exactly once via `/proc/self/fd`. The target never aliases the
separately encoded named stage. That named stage is intentionally retained as
bounded private debris because portable identity-conditional unlink is
unavailable; the Linux publication copy remains unnamed until its one link.
Linux fails closed if the filesystem, procfs path, privileges, or kernel reject
the required operation.
This supersedes v0.7's cleanup-step wording for new v0.9 publications: a
racing replacement is preserved even in the terminal acknowledgement window.

## Interface review

v0.9 reviewed existing interfaces; it did not assume that every public Rust
item is a v1 promise. Each module owner recorded:

- its one-job sentence and the exact caller-visible interface;
- invariants, ordering, determinism, limits, performance class, side effects,
  persisted effects, error categories, and recovery certainty;
- whether the seam has at least two real adapters or otherwise has a direct
  real caller that earns it;
- direct interface tests that use no private implementation access; and
- one classification: **v1 candidate**, **version-coupled adapter-author
  seam**, **test support**, or **private application surface**.

`point-source` adapter-author traits remain version-coupled to official
adapters. Feature-gated fault controls remain test support. Demo package
facades, CLI syntax, private journal/round-trip modules, and examples do not
become stable product interfaces merely because they are callable in repository
tests. Any later pre-v1 breaking cleanup must be explicit,
minimal, locally migrated in one slice, and reflected in documentation and
interface tests. No speculative wrapper is added to preserve a shallow seam.

The frozen review and support matrix agree, and no supported caller must learn
a private stage, filesystem layout, decoder, triangulator, or renderer
implementation detail.

## Local verification and completed readiness gate

All verification runs locally. No GitHub Actions or other hosted CI is added.
The completed qualification uses the complete command sequence in
`CONTRIBUTING.md`, including every documented example, process test, and
Criterion benchmark.
When a local GPU adapter is expected, both GPU acceptance commands run with
`PUNCTRA_REQUIRE_GPU=1` so a missing adapter is a failure rather than a skip.

The release record names the commit, pinned toolchain, machine/OS, filesystem,
GPU adapter/backend, commands, outcomes, benchmark scales, and any explicitly
unsupported platform. A generated benchmark establishes only the stated local
resource and performance observation. Documentation activation or a partial
test pass is not repository readiness. The exact completed outcomes are owned
by the [v0.9 repository verification record](../releases/v0.9.0.md).

The v0.9 repository candidate is Complete because all of these gates are bound
to the release record:

- Run-bound qualification and full-ceiling streaming coverage pass;
- every artifact has one support class and every supported version appears in
  the matrix;
- authoritative version-1 and published-evidence golden fixtures pass without
  mutation, and rebuildable/temporary fixtures obey their distinct policies;
- the representative failure/recovery matrix, including ownership-safe initial
  index-header recovery and Workflow I/O mapping, passes;
- interface classifications and documentation are independently reviewed; and
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

Implementation was divided into independently reviewable slices:

1. **Implemented:** preserve `IndexError::Io` as recoverable `PWF_IO` at the
   Workflow index stage, including bounded operation, path, and source-error
   rendering.
2. **Implemented:** retain strict Complete-Run binding, canonical pass/fail
   evidence, exact no-replace publication/reconciliation, and stable
   operational versus semantic diagnostics.
3. **Implemented:** retain the full-ceiling streaming path and generated
   end-to-end comparison, and complete the remaining version-1 fixture corpus
   and mutation cases.
4. **Implemented:** make the initial index-header failure safely retryable
   without pathname-race deletion, and complete the
   class-based representative fault matrix across authoritative
   Workspace/Run/SourceRecord state, rebuildable index state, temporary
   spills/stages, and caller-owned outputs.
5. **Implemented:** freeze the exact support matrix, resolve the public
   interface review, align architecture/user documentation, perform independent
   review, and run the complete local release gates.

Completion describes only this repository surface. No slice, fixture, generated
benchmark, local fault test, or candidate label is external product evidence.
