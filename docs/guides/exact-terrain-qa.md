# Exact terrain QA and correction loop

Punctra v0.14 adds bounded CPU-authoritative QA for one exact immutable
Workspace `Snapshot` and Terrain `Surface` pair. The operation can combine:

- Source-Point residuals from one exact `PointQuery`;
- detached Check Point residuals; and
- one evenly stationed profile.

Every numeric result is bound to the Snapshot provenance and the Surface's
Recipe, input, geometry, topology, and Artifact hashes. Renderer colors remain
presentation only.

## Run the generated correction loop

Run the public example with its temporary output:

```bash
cargo run -p point-terrain --example exact_terrain_qa
```

The printed paths are intentionally removed when the process exits. To retain
the artifacts, provide a path that does not yet exist:

```bash
PUNCTRA_QA_EXAMPLE_OUTPUT_DIR=path/to/new-exact-qa-example \
  cargo run -p point-terrain --example exact_terrain_qa
```

The new directory contains:

- `exact-terrain-qa-evidence.json`, the complete machine-readable evidence;
- `exact-terrain-qa-profile.svg`, a bounded visualization of the exact profile
  stations;
- the generated Spatial Index; and
- the generated Workspace with the correction and Revert Revisions.

The output directory uses create-new behavior. The example does not reuse,
replace, or clean an existing caller path.

## What the example proves

The generated Source is a three-by-three planar grid with one center Point ten
metres above the intended plane. All Points initially have Ground
classification `2`.

The example performs this exact sequence:

1. derive the baseline Surface;
2. evaluate all Source Points, one detached Check Point, and a three-station
   profile under `[-0.05 m, +0.05 m]` inclusive tolerance;
3. select authoritative Source ordinal `4` and commit classification `1` under
   caller-owned Operation Identity `2929…2929`;
4. classify the baseline QA and Surface as stale against the changed Snapshot;
5. derive and evaluate the corrected Surface;
6. compare added and removed semantic faces and their conservative incident-
   vertex bounds;
7. commit immediate-head Revert under Operation Identity `2a2a…2a2a`; and
8. prove the reverted Surface has zero semantic face changes from baseline.

The corrected Ground Surface lies on the intended plane. Because the Source
Query deliberately inspects every Point, the reclassified center Point remains
an `above` Source residual with effective classification `1`; it is no longer a
Ground input, and that explicit residual is expected rather than suppressed.

These generated facts prove only the repository contracts. They are not an
observed survey workflow, field accuracy result, independent adoption event,
partner acceptance, or support qualification.

## Trace every displayed station

Each SVG circle has both a stable element identity and a JSON Pointer-like
attribute. For example:

```xml
<circle
  id="baseline-station-1"
  data-evidence-pointer="/qa/baseline/profile/stations/1"
  ...>
```

The corresponding JSON object records:

- `id` and zero-based station index;
- `station_metres` from the declared profile start;
- exact world XY in metres;
- sampled canonical Surface face identity;
- interpolated Surface elevation in metres; or
- the explicit outcome `{"kind":"gap"}`.

The SVG line connects only these authoritative stations. It is not a promise
that unreported terrain behavior between stations was measured, and it is not
a continuous plane/TIN intersection.

## Trace residuals and tolerance

Source residuals live under
`/qa/{baseline|corrected}/source_points/{index}`. Each contains the Source
identity, ordinal, exact position ticks, transformed world position, effective
classification at the frozen Revision, selected face, Surface elevation,
signed residual, and tolerance disposition.

Detached results live under
`/qa/{baseline|corrected}/check_points/{index}` and retain the caller's nonzero
identity and supplied position. Residual always means:

```text
observed world z - interpolated Surface world z
```

`below` means the residual is less than `-below_metres`; `above` means it is
greater than `above_metres`; both boundaries are `within`. A gap has no
invented residual and is counted separately.

## Trace Snapshot and Surface authority

Every QA section contains a `binding` object with:

- Workspace, Source, and Revision identities;
- Terrain algorithm version;
- Recipe and complete Ground-Input hashes;
- geometry and topology hashes;
- provenance-sensitive Surface Artifact hash; and
- declared horizontal and vertical EPSG identities.

`input_hash` binds the tolerance, completed Source-row facts, detached Check
Points, and profile definition. `result_hash` additionally binds every
authoritative outcome. These are semantic evidence hashes; they are independent
of package version, JSON formatting, and SVG styling.

After classification correction, the exact old QA bytes remain valid
historical evidence for the baseline pair. The example reports
`stale_snapshot_and_surface` because they must not be presented as current for
the corrected Snapshot. `TerrainQaCurrentState::snapshot` checks only Snapshot
freshness and reports `SnapshotOnlyCurrent` when that Snapshot matches without
claiming that a current Surface was checked. The in-memory and prepared
constructors additionally compare the declared Surface and reject a Surface
that is not derived from the declared current Snapshot as stale.

## Use the public API

Construct a validated request and run it against either an in-memory or
prepared Surface:

```rust,no_run
# fn inspect(
#     snapshot: point_workspace::Snapshot,
#     surface: point_terrain::TerrainSurface,
# ) -> Result<(), point_terrain::TerrainError> {
use point_terrain::{
    ExactTerrainQaRequest, StationProfile, TerrainQaLimits, VerticalTolerance,
};
use point_workspace::PointQuery;

let tolerance = VerticalTolerance::new(0.025, 0.040)?;
let profile = StationProfile::new(
    [500_000.0, 4_600_000.0],
    [500_100.0, 4_600_000.0],
    100,
)?;
let request = ExactTerrainQaRequest::new(tolerance)
    .source_points(PointQuery::all())
    .profile(profile);
let report = surface
    .exact_qa(snapshot, request, TerrainQaLimits::default())
    .blocking_wait()?;
assert_eq!(report.profile_stations().len(), 101);
# Ok(())
# }
```

`PreparedTerrainSurface::exact_qa` has the same result semantics. It first
materializes verified disk-v1 records under
`max_materialized_surface_bytes`, `SurfaceReadLimits`, and the combined QA
working-byte ceiling. It never performs an unbounded load.

Compare two compatible in-memory Surfaces with `compare_surfaces`. Faces are
matched by their three authoritative Point Identities, not by Surface-local
vertex numbers. `changed_bounds` encloses vertices incident to changed faces;
it is deliberately conservative and is not an exact change polygon.

Correction and Revert remain `point-workspace` operations. Record a nonzero
Operation Identity before each commit, reconcile any indeterminate outcome by
that same identity, derive or prepare a fresh Surface for the resulting
Snapshot, and never overwrite a stale persistent target.

## Resource and failure behavior

`TerrainQaLimits` independently bounds Point-row reads, Source results,
detached Check Points, profile stations, their combined observation count,
retained results, prepared-Surface materialization, face tests, and combined
working bytes. The combined ledger conservatively includes the prepared
Surface materialization and the configured Point-row stream working ceiling
while Source inputs are collected.

`SurfaceComparisonLimits` bounds combined faces, working bytes, and sort/merge
work. A limit failure, cancellation, stale input pair, unsupported spatial
reference, corrupt prepared Surface, or Source failure returns no partial QA or
comparison report.
