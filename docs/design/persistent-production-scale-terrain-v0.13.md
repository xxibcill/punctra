# Persistent Bounded-AOI Terrain Design (v0.13)

Status: **Accepted for repository implementation on 2026-08-19**

This design is authoritative for the bounded Punctra v0.13 repository slice.
The maintainer's request to continue with v0.13 activates this technical scope;
it does not satisfy the roadmap's field activation gate. No permitted field
measurements currently establish a production AOI size, Ground-Point count,
latency, memory, temporary-storage, accuracy, or workstation envelope.

The release therefore deepens the existing `point-terrain` module without
claiming that a generated benchmark or a large containing Source proves
production-scale Terrain Derivation. The accepted algorithm remains the v0.6
canonical full-AOI triangulation under caller-selected hard memory limits. A
true external-memory triangulator is not silently substituted, and independent
tiles are not presented as one equivalent Delaunay Surface.

## Outcome

Punctra v0.13 adds one durable preparation path for an explicit inclusive AOI.
It can:

- build the existing exact unconstrained Surface for one pinned Snapshot and
  bounded AOI;
- checkpoint the complete verified Ground Input before topology work;
- resume topology work from that checkpoint without rereading the Source;
- stage, sync, verify, and publish one immutable Surface Artifact without
  replacing an existing target;
- resume publication from a complete verified stage;
- reopen a compatible complete Artifact without rereading Snapshot rows; and
- expose its descriptor plus bounded ordered vertex and face streams without
  retaining the complete Surface in the returned handle.

The legacy in-memory `derive` interface remains available during the pre-v1
period. Small fixtures from both paths must reproduce the same canonical
vertices, faces, geometry hash, topology hash, and semantic Artifact identity.

## Evidence boundary

Repository completion may prove only the checked-in format, generated
fixtures, fault behavior, deterministic topology, hard limits, local examples,
and measured local benchmark scales. It does not prove:

- that the selected limits fit any firm's recurring production AOI;
- completion of either required above-500-million-Point project;
- a field accuracy baseline, accepted deliverable, or supported workstation;
- lower attended time, lower rework, partner acceptance, or production
  support;
- a true out-of-core or distributed triangulation algorithm;
- crates.io publication, an independently run example, or a published
  production resource report; or
- the roadmap's Field-qualified, Partner-validated, or Support-qualified
  evidence levels.

The containing Source may be larger than an AOI because Workspace Point-row
planning is spatially bounded. That fact must not be described as proof that
the selected AOI triangulates out of core or that a complete large Source fits
the same resource envelope.

## One deep public seam

`point-terrain` remains the only public terrain module. It adds one preparation
operation conceptually equivalent to:

```rust,ignore
prepare(snapshot, target, recipe, limits) -> TerrainPrepareJob
```

`recipe` uses the existing ground-classification meaning and must contain an
explicit finite inclusive `WorldBounds` AOI. The operation has exactly four
successful dispositions:

1. open a compatible complete target;
2. publish a compatible verified final stage;
3. resume topology from compatible verified input work; or
4. build when the complete target, stage, and work paths are all absent.

The result is a file-backed prepared Surface. Its semantic descriptor is
separate from the attempt report. Cold, resumed, and warm attempts may have
different elapsed time, temporary bytes, reused work, or disposition without
changing the Surface identity.

The handle exposes bounded pull streams of canonical vertices and faces.
Callers select explicit per-stream record, payload, read-buffer, and work
limits. No method returns a complete file-backed Surface as an unbounded slice.
The existing in-memory Surface continues to own slices only for the legacy
interface.

The public seam does not expose filesystem backends, page-store traits,
triangulator plugins, checkpoint grammars, or publication hooks. Those are
private implementation details exercised through real owner-local files.

## Preserved terrain semantics

The durable path preserves the accepted v0.6 and v0.12 meanings:

- Ground Input is every Point in the inclusive AOI whose effective
  classification at the pinned Snapshot equals the Recipe value;
- Point Identity, exact Source ticks, and Source order remain authoritative;
- duplicate XY, conflicting elevations, insufficient input, collinearity, and
  unsupported numeric ranges fail explicitly;
- the unconstrained 2.5D Delaunay topology, robust predicates, canonical
  cocircular tie rule, counter-clockwise faces, and canonical ordering do not
  change;
- the exact position transform and complete metre/metre spatial profile remain
  required and are never inferred; and
- no limit failure may decimate Points, alter membership, relax a predicate,
  change topology, or publish a partial result.

Terrain algorithm version, Surface disk version, work version, package
version, and any private report schema are independent numbers. Adding disk-v1
does not change the existing terrain algorithm version when semantic topology
and hashes remain identical.

## Artifact identity and format

The Surface Artifact is immutable, checksummed, and rebuildable. It is not
Workspace authority and is not inserted into the frozen Workflow Run-v1 wire
format. Its private fixed-endian disk-v1 encoding binds:

- disk and terrain algorithm versions;
- Workspace, Source, and Revision identity from `SnapshotProvenance`;
- normalized Recipe and AOI bytes plus the Recipe hash;
- exact Source position-transform bits;
- the complete structured spatial-reference bytes;
- complete Ground-Input content hash;
- canonical geometry and topology hashes;
- semantic Artifact hash;
- exact input, vertex, face, and hull counts;
- exact Surface bounds; and
- section record widths, lengths, offsets, and checksums.

Vertex and face records retain the existing identities and canonical order.
Checksummed blocks permit bounded stream verification, and a checksum stored in
the footer covers every byte preceding that footer. Open rejects unknown
versions, truncation, checksum mismatch, impossible counts or offsets, invalid
ordering, invalid face references, and disagreement between descriptor and
body.

A complete Artifact is owner-local rebuildable data. Frozen disk-v1 fixtures
therefore promise exact read compatibility or an explicit unsupported-version
failure; temporary or incomplete private stages do not become authoritative
deliverables.

## Work, resume, and publication

The sibling work path has an ownership-safe checksummed header bound to the
same Snapshot, Recipe, algorithm, transform, and spatial reference as the
requested target. The accepted durable checkpoints are:

1. complete verified AOI Ground Input with terminal Snapshot row facts; and
2. a complete staged Surface encoding ready for no-replace publication.

Resume from checkpoint 1 may rerun sorting and triangulation, but it must not
reread the Source. Cancellation or failure during topology work publishes no
Surface and retains only verified owner-local work. Resume from checkpoint 2
revalidates the entire staged encoding before attempting publication.

A torn or truncated work file, interior corruption, an incompatible header, an
unknown version, or an unproven path is preserved and rejected. The
implementation never guesses that an arbitrary sibling file belongs to the
current request or uses a check-then-unlink cleanup that could delete a racing
replacement.

Publication uses an owned stage, file synchronization, descriptor-bound
no-replace target creation, parent-directory synchronization, reopen, complete
verification, and path/identity revalidation before success is acknowledged.
Pre-publication failure proves that this attempt published no target. Recovery
is state-specific: an absent or complete verified checkpoint permits retry of
the same request, while a torn or truncated checkpoint requires a fresh target
family or explicit offline owner-controlled cleanup. A failure after the
no-replace commit boundary is reported as indeterminate with the expected
complete-payload/footer checksum so a retry can reconcile it. Existing regular
files, symlinks, directories, devices, FIFOs, racing paths, and changed paths
are never replaced or deleted.

After acknowledged publication, the verified stage and any work sibling remain
recognized owner-local pathnames. `ResumedPublication` does not inspect or
trust an arbitrary work sibling. There is no portable unlink operation
conditioned on the verified open inode, so preparation retains the stage and
any work pathname instead of risking deletion of a racing replacement. The
complete target takes precedence on later warm opens; optional removal is an
explicit offline caller action only when no related handle, job, or process is
live.

## Staleness and rebuild decisions

A valid Artifact is current only for its exact Snapshot provenance, Recipe,
AOI, algorithm, transform, and spatial reference. A later Workspace head does
not invalidate a historical Artifact for its recorded Snapshot; it is stale
only when a caller requests a different binding at the same target.

Stale, incompatible, or conflicting complete targets are preserved. The
library reports the binding mismatch and requires the caller to choose a fresh
target or explicitly remove/move owner-controlled rebuildable data. Automatic
overwrite, implicit migration, and rebuild-in-place are not accepted.

## Resource contract

Preparation limits independently bound:

- complete Snapshot Point-row planning, decoding, overlays, and output;
- AOI Ground-Input count;
- canonical vertices and faces;
- retained full-AOI triangulation memory;
- deterministic topology work units;
- work-header, checkpoint, and staged bytes;
- final Artifact bytes;
- checksum and stream read buffers;
- path/header metadata retained by the prepared handle; and
- cumulative temporary bytes owned by the attempt.

The implementation supports exactly one topology worker in v0.13. Parallel
worker counts, scheduling policy, and distributed execution are not Recipe
semantics and are not accepted in this release. A later design may add a worker
count only after every supported count reproduces canonical bytes.

Attempt reports distinguish algorithm accounting from observed process or
filesystem measurements. A missing observation remains absent; it is never
filled with an estimate. Source verification, Spatial Index preparation,
Terrain preparation, warm reopen, QA/export, and View measurements are
reported as separate phases.

## Errors and certainty

Public errors distinguish invalid AOI/arguments, resource exhaustion,
unsupported spatial or numeric input, terrain degeneracy, Workspace failure,
I/O, corrupt work, corrupt Artifact, unsupported version, stale binding,
existing/conflicting target, cancellation, and indeterminate publication.
Diagnostics remain bounded UTF-8 and retain structured underlying errors where
the existing taxonomy does so.

No error returns a partial prepared handle, partial descriptor, provisional
stream, or guessed recovery action. Open and resume validate before exposing
semantic facts.

## Reproducible example and reporting

The repository adds a public `point-terrain` example that builds a generated
verified Source, Spatial Index, Workspace, explicit AOI Surface, reopens it,
and consumes bounded vertex/face streams. Its machine-readable report states
the generated Point count and exact limits, separates Source verification,
indexing, Terrain cold/resumed/warm work, and records only observations that
were actually measured.

Because this accepted slice uses full-AOI resident triangulation, the example
is a persistent bounded-memory example, not the roadmap's still-outstanding
true out-of-core adoption exit. Generated 10,000, 100,000, or 1,000,000-Point
results are not extrapolated to 500 million Points or production data.

## Explicit non-goals

v0.13 does not add:

- independent tile triangulation, arbitrary halos, seam stitching, or a new
  tiled-Surface grammar;
- external-memory, memory-mapped, parallel, distributed, GPU, or approximate
  triangulation;
- Breaklines, boundaries, holes, constrained TINs, multiple Sources, or
  automatic AOI selection;
- new classification, position, Attribute, or Source edits;
- general persistent QA indexes, profiles, cross-sections, or the v0.14
  correction loop;
- a new Workflow Run wire version or automatic migration of frozen Run-v1;
- target replacement, background cleanup of unproven files, network storage,
  cloud execution, or a storage plugin interface; or
- field qualification, partner acceptance, independent adoption, production
  support, registry publication, hosted CI, or release tagging.

## Repository verification gates

Repository completion requires:

- an explicit-AOI success fixture whose prepared vertices, faces, descriptor,
  and hashes equal the legacy in-memory oracle;
- repeated and differently batched cold builds with identical bytes;
- resume from verified input and complete publication checkpoints with bytes
  identical to an uninterrupted build;
- warm reopen without Snapshot row consumption;
- stale Snapshot, Recipe/AOI, algorithm, transform, and spatial-reference
  rejection without modification;
- frozen complete/work disk-v1 fixtures and manifest checks;
- truncation, interior corruption, torn suffix, unknown version, invalid
  counts/offsets/order/references, and complete-payload/footer checksum faults;
- cancellation and injected create/write/sync/link/readback/disk-exhaustion
  faults at every durable boundary, including publication certainty;
- inclusive and just-over coverage for every independent resource ceiling;
- bounded vertex/face stream partitioning and mutation/corruption detection;
- the reproducible generated example, benchmark, and non-extrapolating report;
- unchanged Source, Spatial Index, Workspace, Run-v1, LandXML, and round-trip
  fixtures; and
- every applicable local command in `CONTRIBUTING.md`, including package,
  rustdoc, fuzz, benchmark, example, and required GPU acceptance with
  `PUNCTRA_REQUIRE_GPU=1`.

No hosted CI is added. Completion is recorded only against an exact local
implementation commit. The final status wording is: **Complete and
repository-verified for the bounded persistent-terrain slice; field
activation, production-scale accuracy, true out-of-core adoption,
independent adoption, partner validation, and support qualification
outstanding.**
