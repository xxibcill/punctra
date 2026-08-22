# Exact Terrain QA and Correction Loop Design (v0.14)

Status: **Complete and repository-verified for the bounded exact Terrain QA and
correction-loop slice; field activation, observed workflow timing, independent
adoption, partner validation, and support qualification outstanding**

This design is authoritative for the bounded Punctra v0.14 repository slice.
The maintainer's request to continue through v0.14 activates this technical
scope. It does not manufacture the roadmap's still-missing observed acceptance
work, production corpus, independent-adopter publication, or partner evidence.
Repository completion and every external exit remain reported separately.

## Outcome

Punctra v0.14 adds one exact, CPU-authoritative Terrain QA operation and one
Surface comparison operation. Together with the existing Workspace commit and
immediate-head Revert contracts, they let a caller:

1. inspect one frozen Snapshot/Surface pair;
2. evaluate explicit profile stations, Source-Point residuals, and detached
   Check Points under one declared vertical tolerance;
3. commit an exact classification correction through `point-workspace`;
4. prove the earlier QA and Surface are stale against the changed Snapshot;
5. re-derive, re-run QA, and obtain a conservative changed-region envelope;
6. compare the before/after evidence; and
7. retain the correction or Revert it through the existing durable operation
   contract.

Rendered colors remain disposable presentation. No GPU value, color, resident
sample, depth treatment, or screenshot becomes a measurement.

## Evidence boundary

Repository completion may prove analytic values, deterministic hashes,
provenance and staleness behavior, exact fixture topology comparisons, hard
limits, cancellation, examples, and local verification. It does not prove:

- that the selected profile stations, tolerance, or report fit a firm's
  acceptance workflow;
- reduced time to find, explain, or correct a production defect;
- a field accuracy baseline or accepted deliverable;
- independent use of the example or crates.io publication;
- partner acceptance or a supported workstation envelope; or
- v0.13's still-outstanding production-scale or true out-of-core exits.

## One deep public seam

`point-terrain` remains the only public Terrain and QA module. It gains:

- exact QA behavior on an in-memory `TerrainSurface`;
- the same behavior on a `PreparedTerrainSurface`, which materializes its
  checksummed disk-v1 records only under explicit QA Surface-read and
  materialization-byte ceilings;
- immutable request, tolerance, profile, result, binding, freshness, and
  report values; and
- `compare_surfaces(before, after, limits)` for an exact semantic topology
  difference and conservative changed-region envelope.

No new crate, Workspace mutation facade, UI framework, QA database, generic
report serializer, or renderer protocol is added. Correction and Revert remain
owned by `point-workspace`; the public example composes those existing seams.

## Exact QA request

One request is bound to exactly one immutable Snapshot and one Surface. It may
contain any nonempty combination of:

- one exact `PointQuery` evaluated through `Snapshot::point_rows`;
- caller-owned detached Check Points with unique nonzero identities; and
- one station profile defined by finite distinct world-XY endpoints whose
  planar metre length is finite and representable as `f64`, plus a nonzero
  interval count.

A station profile contains both endpoints and every evenly spaced station.
Every station is evaluated exactly against the CPU Terrain triangles at its
declared XY. The resulting polyline is a visualization of those authoritative
stations; it is not represented as the continuous intersection of an
arbitrary plane with every triangle, and callers must not infer unreported
features between stations.

Source-Point residual is:

```text
exact Source world z - interpolated Surface world z
```

Detached Check Point residual retains the same v0.6 meaning. A profile station
has no observed elevation and therefore reports only Surface elevation or an
explicit gap. Closed triangle edges and vertices are covered. Shared-boundary
ties select the lowest canonical face identity. A point outside the Surface
domain is a successful explicit gap, never a fabricated elevation.

## Tolerance and outcomes

The request declares finite, nonnegative lower and upper vertical tolerances in
metres. A numeric residual is classified as:

- below tolerance when it is less than negative lower tolerance;
- within tolerance when both inclusive limits contain it; or
- above tolerance when it exceeds upper tolerance.

Gaps are separate from tolerance results. The report preserves exact input
order, signed residuals, classifications, ticks or detached coordinates,
selected face identities, profile station metres, and aggregate residual and
tolerance counts. No color names or presentation palette is part of the
authoritative API.

## Provenance, hashes, and freshness

Every successful report binds:

- Workspace, Source, and Revision identity;
- Terrain algorithm, Recipe, input, geometry, topology, and Artifact hashes;
- the complete structured easting/northing/elevation metre profile;
- the normalized tolerance;
- the completed Source-row summary when a Query is present;
- canonical input and result hashes;
- explicit profile, Source-Point, Check Point, gap, and tolerance facts; and
- exact algorithm-accounted face tests and retained/working bytes.

QA starts only when the supplied Snapshot provenance exactly equals the
Surface Snapshot provenance. An immutable report remains valid historical
evidence for that pair. A freshness check against a caller-declared current
Snapshot alone distinguishes Snapshot-only current from stale Snapshot without
claiming that a current Surface was checked. A check that also declares the
current Surface distinguishes current, stale Snapshot, stale Surface, and both
stale. These five explicit outcomes prevent callers from presenting historical
evidence as current merely because its bytes remain valid.

Hashes use fixed domain-separated little-endian encodings. Package version,
hash grammar version, Terrain algorithm version, Surface disk version, and any
example JSON schema remain independent.

## Surface comparison and changed region

The comparison operation requires the same Workspace and Source lineage,
Terrain Recipe, algorithm, transform, and spatial profile. Revisions may
differ. Faces are compared by their three authoritative Point Identities,
independent of per-Surface vertex numbering.

The report contains added and removed face counts and hashes plus optional
inclusive bounds of every vertex incident to an added or removed face. These
bounds are a conservative changed-region envelope, not an exact change polygon
and not proof that elevation changed at every enclosed location. Equal
Surfaces produce zero counts and no changed bounds.

## Resource and cancellation contract

QA limits independently bound:

- Snapshot Point-row planning, Source reads, overlays, and output;
- Source residuals, Check Points, profile stations, and total observations;
- retained result bytes and prepared-Surface materialization bytes;
- file-backed Surface stream batches, checksum buffers, and work;
- deterministic face-containment tests; and
- combined incremental working bytes.

Comparison limits independently bound total faces, retained comparison records,
sort/comparison work, and working bytes. Limit failures never sample, truncate,
decimate, loosen tolerance, suppress gaps, or publish partial reports.
Cancellation is checked during input collection, point location, sorting, and
hashing. A cancelled or failed operation returns no report.

## Correction, re-derive, compare, and Revert

v0.14 adds no second editing model. The documented loop uses:

- exact Point Identities from QA or an existing caller decision;
- `Snapshot::select_point_ids`;
- caller-recorded nonzero `OperationId` values;
- `CommitRequest::set_classification`;
- a fresh Snapshot and Terrain derivation or preparation target;
- report freshness plus Surface comparison; and
- `CommitRequest::revert_head` when the immediate head is to be undone.

Indeterminate commits remain reconciled by Operation Identity. Stale or
conflicting persistent Surface targets are preserved and require a fresh
caller-owned target. v0.14 does not overwrite an old Surface or QA artifact.

## End-to-end adopter example

The public `point-terrain` example constructs a generated supported-profile
Source with a seeded terrain defect. It emits one machine-readable QA evidence
document and one SVG profile whose station elements carry identifiers that map
back to the evidence. The example then commits the classification correction,
proves old evidence stale, re-derives and compares the Surface, reruns QA, and
Reverts to reproduce the baseline Surface.

The accompanying guide explains every field, unit, gap, tolerance, Snapshot,
Surface, and operation identity. Generated execution is repository evidence,
not an observed professional trial or independent adoption. Public publication
and an independently run trace remain external exits until observed.

## Explicit non-goals

v0.14 does not add:

- Breaklines, boundaries, holes, constrained terrain, or CAD authoring;
- arbitrary position or Attribute editing, automatic classification, or Source
  rewriting;
- continuous plane/TIN intersection profiles, arbitrary section corridors, or
  volume calculations;
- a persistent spatial QA index, hidden caching, or unbounded prepared-Surface
  loading;
- automatic correction, automatic Revert, or recovery that invents operation
  identities;
- GPU-authoritative Queries, residuals, tolerances, profiles, or geometry;
- general charting, annotation, clipping, measurement, desktop UI, or report
  templating;
- automatic CRS inference, coordinate transformation, non-metre QA, or datum
  conversion; or
- field, adoption, partner, support, or production-scale claims from generated
  fixtures.

## Verification and completion

Repository completion requires:

- analytic planar and seeded-defect fixtures for profile, Source residual,
  detached Check Point, positive/negative/boundary tolerance, and gap results;
- exact binding and stable input/result hash fixtures across batching;
- stale Snapshot, stale Surface, and both-stale checks after Edit;
- Source/Recipe/reference mismatch rejection without partial results;
- before/after and post-Revert Surface comparison, including deterministic
  changed counts, hashes, and conservative bounds;
- prepared and in-memory Surface QA equality;
- inclusive and one-under resource limits plus cancellation at bounded work
  intervals;
- the traceable JSON/SVG correction-loop example and guide;
- updated package, architecture, changelog, roadmap, and contribution docs; and
- every applicable local command in `CONTRIBUTING.md`, including package,
  rustdoc, fuzz, benchmark, example, and forced-GPU acceptance with
  `PUNCTRA_REQUIRE_GPU=1`.

The repository exit was qualified at implementation commit
`4dc502306646dfcc5876106287861ac5cf60c9d8`. Exact environment, command,
example, benchmark, and nonclaim facts are recorded in the
[v0.14 repository verification record](../releases/v0.14.0.md). External exits
remain outstanding exactly as stated above.

No hosted CI was added. Completion is recorded against the exact local
implementation commit above. The final status wording is: **Complete and repository-
verified for the bounded exact Terrain QA and correction-loop slice; field
activation, observed workflow timing, independent adoption, partner
validation, and support qualification outstanding.**
