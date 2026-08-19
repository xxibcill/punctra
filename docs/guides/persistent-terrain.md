# Persistent bounded-AOI Terrain

Status: **Complete and repository-verified for the bounded persistent-terrain
slice; field activation, production-scale accuracy, true out-of-core adoption,
independent adoption, partner validation, and support qualification
outstanding**

Punctra v0.13 accepts one durable preparation path for an explicit inclusive
AOI. It preserves the v0.6 exact Ground Input and canonical unconstrained
Surface while making the result resumable, publishable without replacement,
warm-reopenable, and readable through bounded file-backed streams.

This is a bounded-memory path, not a true out-of-core triangulator. Complete
AOI sorting and topology still fit within the caller's hard triangulation-memory
limit and run on one worker. A large containing Source does not prove that its
AOI fits, and generated examples do not establish production scale.

## Public shape

The accepted public operation is:

~~~rust,ignore
use point_terrain::{
    SurfaceReadLimits, TerrainPrepareLimits, TerrainRecipe, prepare,
};

let recipe = TerrainRecipe::new(2).within(explicit_inclusive_world_bounds);
let prepared = prepare(
    pinned_snapshot,
    "existing-ground.pterr",
    recipe,
    TerrainPrepareLimits::default(),
)
.blocking_wait()?;

let descriptor = prepared.descriptor();
let report = prepared.report();
for batch in prepared.vertex_batches(SurfaceReadLimits::default())? {
    consume_vertices(batch?)?;
}
for batch in prepared.face_batches(SurfaceReadLimits::default())? {
    consume_faces(batch?)?;
}
~~~

`prepare(snapshot, target, recipe, limits)` returns a `TerrainPrepareJob`; a
successful wait yields `PreparedTerrainSurface`. The attempt report has four
observable dispositions:

- `Built`: the complete target, final stage, and input work paths were absent;
- `ResumedInput`: complete verified Ground Input was reused and topology reran;
- `ResumedPublication`: a complete verified Surface stage was republished; and
- `Opened`: a compatible complete target supplied the result. It was either
  present before the attempt, with no Snapshot row consumption, or won a
  no-replace publication race after the attempt had already consumed rows.

Use `source_points_read()` and `reused_input_points()` for attempt-specific
work observations; do not infer either value from `Opened` alone.

The `PreparedTerrainSurface` retains bounded metadata and access to the complete
Artifact. It does not retain all vertices and faces or return unbounded slices.
The semantic `SurfaceArtifactDescriptor` is separate from the attempt-specific
`TerrainPrepareReport`, so cold/resumed/warm resource observations do not change
Surface identity.

The legacy `derive` API remains available for in-memory callers. Equal explicit
AOI inputs must produce the same canonical vertices, faces, geometry/topology
hashes, and semantic Artifact identity through both paths.

## Owner-local files

The caller chooses the complete target name; `.pterr` is the documented example
suffix, not part of semantic identity. `point-terrain` alone interprets:

~~~text
existing-ground.pterr                  # complete rebuildable Surface disk-v1
existing-ground.pterr.surface-work-v1  # Ground-Input checkpoint path; verified before resume
existing-ground.pterr.surface-stage-v1 # complete verified publication stage
~~~

Successful publication deliberately retains the verified stage and any work
pathname. The target and stage may name the same immutable file identity; a
work file verified by the attempt remains a separate complete checkpoint, but
an uninspected work sibling is not thereby trusted. `point-terrain` cannot
portably condition an unlink on the still-open owned inode, so a check-then-
unlink cleanup could delete a racing replacement. A later warm open gives the
complete target precedence and ignores those siblings. Leave them in place
during live use; owner-controlled offline cleanup is optional only when no
related handle, job, or process is live.

Surface disk-v1 binds Snapshot/Source/Workspace/Revision provenance, exact
Recipe and AOI, position transform, structured metre/metre spatial reference,
terrain algorithm and disk versions, counts, bounds, and canonical hashes.
Vertex and face records remain in canonical identity order. Fixed 4,096-record
checksum blocks permit bounded verification, and a checksum stored in the
footer covers every byte preceding that footer. All bindings and
structural facts are validated before a handle is returned. Each bounded read
revalidates every touched record block before yielding decoded records.

A warm open revalidates the exact Snapshot provenance, position transform, and
supported spatial profile without consuming Snapshot Point rows. A different
binding at the same pathname is stale even when its Surface bytes would
otherwise be structurally valid.

These files are rebuildable Snapshot-derived data, not Workspace authority,
Workflow Run-v1 state, or caller-owned LandXML evidence. The v0.13 path does not
change Source, index, Workspace, Run-v1, report, LandXML, or round-trip bytes.

## Reproducible generated path

The completed local gate ran the public generated example and existing Terrain
benchmark:

~~~bash
cargo run -p point-terrain --example persistent_surface
cargo bench -p point-terrain --bench terrain
~~~

The example creates a generated verified Source, complete Spatial Index,
Workspace, explicit AOI, cold prepared `.pterr` Surface, forced input-checkpoint
resume, warm reopen, and complete bounded-stream consumption. Its machine-
readable report separates:

- generated Source verification and Source Point count;
- Spatial Index preparation disposition, time, temporary, and Artifact bytes;
- Terrain cold/resumed/warm disposition and elapsed observations;
- AOI Ground Input, vertex, face, and hull counts;
- topology work and algorithm-accounted full-AOI triangulation memory;
- verified Surface work, cumulative private temporary, complete Artifact,
  retained-handle metadata, path ceiling, stream record and batch counts, and
  payload, verification-buffer, working-memory, and work-unit ceilings; and
- direct stage bytes, QA, LandXML, View, worker heap, process resident memory,
  allocated filesystem blocks, and field accuracy that were not measured.

The default invocation generates 10,000 Points. The current self-validating
output reports 10,000 Ground Input Points and vertices, 19,602 faces, 396 hull
vertices, 521,494 topology steps, a 320,480-byte verified input checkpoint, a
556,088-byte Surface, and 876,568 peak logical private temporary bytes on the
qualified macOS/APFS path. A forced
one-byte Artifact ceiling leaves the checkpoint, then `ResumedInput` reuses all
10,000 inputs with zero Snapshot Point reads and reproduces the cold Artifact
bytes exactly. The bounded streams consume the vertices in three batches and
the faces in five; the final warm `Opened` attempt reports zero Snapshot Point
reads and no triangulation observations.

Those generated-fixture facts are recorded against implementation commit
`008a0d97fdfa23547609845b71c34b40d17d1894` in the [v0.13 repository
verification record](../releases/v0.13.0.md). They are not a latency or
production claim. Per-invocation example elapsed times are intentionally not
copied. Direct stage bytes, worker heap, process peak resident memory,
allocated filesystem blocks, QA, LandXML, View, and field accuracy remain JSON
`null`; the report does not infer them from adjacent counters. Set
`PUNCTRA_PERSISTENT_TERRAIN_EXAMPLE_POINTS` to any integer from 3 through
1,000,000 for another generated example size. The Criterion benchmark uses
`PUNCTRA_TERRAIN_BENCH_POINTS` over the same range, with 10,000, 100,000, and
1,000,000 as its intended scales. Only a run that actually completed on the
named machine may appear in release evidence.

## Recovery and support matrix

| Observed state | Meaning | Safe caller action |
|---|---|---|
| Compatible complete target | Warm-openable immutable Surface | Reuse it with the same Snapshot, Recipe, AOI, algorithm, transform, and spatial reference. |
| Complete verified input checkpoint | Source rows are durable; topology or final staging did not complete | Retry `prepare` with the same request and adequate limits/capacity. |
| Complete verified final stage | Surface bytes are durable but target publication is not acknowledged | Retry the same request so publication can resume or reconcile. |
| Compatible target plus retained stage/work siblings | Normal successful owner-local state; the target takes precedence and does not prove an uninspected work sibling valid | Keep using the target. Leave siblings alone during live use; optional offline cleanup requires exclusive caller control of the exact family. |
| Truncated or torn work file | disk-v1 cannot prove a complete checkpoint | Preserve it and fail closed; choose a fresh target family or perform explicit offline owner-controlled cleanup. |
| Interior-corrupt or incompatible work | Work ownership/meaning cannot be established | Preserve it; inspect paths/capacity and choose a fresh target family or perform explicit offline owner-controlled cleanup. |
| Valid target with another binding | Historical Artifact is valid but stale for this request | Keep it for its recorded Snapshot or choose a fresh target; never rebuild over it. |
| Corrupt/unsupported complete target | No prepared Surface may be exposed | Preserve it for diagnosis; explicitly move/remove owner-controlled rebuildable data before rebuilding. |
| Existing symlink/non-regular/racing target | Caller path is not a safe no-replace destination | Choose a new regular absent target and do not delete the unknown path. |
| Pre-publication cancellation/I/O/resource failure | No complete target was acknowledged; the retained checkpoint may be absent, complete, or torn | Restore limits, capacity, or permissions. Retry the same family only when its checkpoint is absent or complete and verified; use a fresh family or explicit offline owner-controlled cleanup for a torn checkpoint. |
| Publication-indeterminate error | A complete target may exist with the reported expected complete-payload/footer checksum | Retry the same request and let `prepare` reconcile; do not overwrite or invent a different binding at that path. |

Temporary/work limits, operating-system errors, and publication certainty are
part of the public failure meaning. Automatic overwrite, migration, broad
directory cleanup, or deletion of unproven siblings is never a recovery step.
Successful preparation is not a promise that recognized work/stage siblings
were removed.

## Evidence that remains outstanding

Repository completion can establish format, topology, fault, limit, example,
and local benchmark behavior only. It cannot mark any of these complete:

- the field-measured AOI, Ground-Point, latency, memory, temporary-storage,
  accuracy, or workstation envelope;
- either required above-500-million-Point Source project from an unrelated firm;
- true external-memory/out-of-core triangulation or the roadmap's out-of-core
  open-source adoption exit;
- independent example execution, crates.io publication, or a permitted public
  production resource report; or
- Field-qualified, Partner-validated, Support-qualified, time-saving, reduced-
  rework, paid-use, or accepted-deliverable claims.

Use the exact wording from the [accepted v0.13
design](../design/persistent-production-scale-terrain-v0.13.md): repository
completion is recorded, but remains separate from every field and product
evidence level.
