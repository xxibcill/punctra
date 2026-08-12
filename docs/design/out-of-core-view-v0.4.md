# Out-of-core View v0.4

Status: implemented and locally verified

Punctra v0.4 adds one rebuildable persistent Spatial Index and one real-cloud
application path. A verified local `Source` can be indexed once, reopened, and
materialized progressively through the existing renderer-neutral View planner
while Point, byte, batch, host-memory, temporary-storage, and renderer
residency limits remain explicit.

The release does not introduce a Workspace, Snapshot, exact Query engine,
Revision, Edit, networking layer, or index-owned renderer. Point Identity
remains `(SourceId, logical ordinal)`, and the Source remains the authority for
complete Point values.

## Designs considered

Three independent designs were compared before accepting this scope.

1. A minimal complete-artifact design used one fixed-block bounds hierarchy,
   one deep `prepare` operation, and a private application bridge. It had the
   smallest public surface and the clearest recovery proof, but sparse root
   samples would make LAZ time-to-first-visible depend on decoding most of the
   file.
2. An immutable-generation design added external Morton sorting, sealed partial
   preview generations, typed View-only versus exact-candidate capabilities,
   content-addressed segments, and paged hierarchy readers. It made partial
   Coverage exceptionally explicit, but introduced two persisted trees and a
   generation store before the first complete-index caller exists.
3. A caller-first design placed a deterministic action/acknowledgement reducer,
   parallel loaders, cancellation, host reservations, and renderer command
   sequencing in the application. It exposed an important planner fact—the
   host must know which already-requested nodes remain demanded—but its general
   actor protocol is deeper than the first synchronous bounded loader needs.

The accepted design combines the strongest narrow decisions: one complete
fixed-block BVH, persisted samples only for internal View nodes, Source-backed
leaf reads, and one private application materializer. Partial durable preview
generations, a full Morton point store, paged hierarchy planning, parallel load
reducers, and a reusable renderer-sink trait remain deferred.

## Module ownership

### `point-index`

`point-index` owns:

- deterministic division of canonical Source order into fixed contiguous
  blocks;
- exact block bounds and a deterministic binary bounds hierarchy;
- conservative axis-aligned spatial lookup to candidate Source Spans;
- deterministic bounded internal-node display samples;
- checksummed complete artifacts and resumable incomplete build records;
- artifact/source/version validation; and
- bounded node reads and hierarchy facts.

It does not own:

- Source decoding or format dependencies;
- arbitrary spatial or Attribute predicates;
- camera policy, View planning, display color, or renderer packing;
- GPU resources or eviction;
- authoritative Point Identity or Source bytes; or
- Workspace, Revision, Snapshot, Query, or Edit behavior.

The module may call the opaque `Source` interface. It never calls an adapter or
interprets LAS/LAZ bytes.

### Application bridge

The real-cloud bridge remains in the non-published demo application. It owns:

- the verified Source/index handles and one View generation;
- mapping `IndexNodeId` to planner `NodeKey` and renderer `BatchKey`;
- Missing, Requested, and Resident node status;
- a bounded prioritized materialization queue and monotonic batch versions;
- conversion from exact ticks to origin-relative display `f32` values;
- complete `RenderUpdate::Upsert` and conditional Remove effects; and
- time-to-first-visible, queue, staging, and residency measurements.

It does not create a new foundation trait. A second application must prove a
stable reusable bridge seam before one is extracted.

## Primary interface

The public index interface is intentionally concrete and small:

```rust,ignore
pub fn prepare(
    source: Source,
    target: impl AsRef<Path>,
    limits: PrepareLimits,
) -> Job<PreparedIndex, IndexError>;

impl PreparedIndex {
    pub fn descriptor(&self) -> &IndexDescriptor;
    pub fn hierarchy(&self) -> &IndexHierarchy;
    pub fn prepare_report(&self) -> &PrepareReport;

    pub fn candidates(
        &self,
        bounds: WorldBounds,
        limits: CandidateLimits,
    ) -> Result<CandidatePlan, IndexError>;

    pub fn read_node(
        &self,
        node: IndexNodeId,
        budget: NodeReadBudget,
    ) -> Result<IndexPointBatches, IndexError>;
}
```

`prepare` atomically performs the useful operation rather than exposing a
public builder, page store, journal, open status, or filesystem abstraction:

- a compatible complete artifact is verified and opened;
- a compatible incomplete build is recovered and resumed;
- a missing artifact is built; and
- an incompatible, corrupt, unsupported, or over-budget target fails without
  being silently replaced.

Deleting the disposable sidecar and calling `prepare` rebuilds it. The Source
is retained by the returned handle so leaf reads cannot accidentally use a
different Source.

`IndexHierarchy` is an immutable complete snapshot whose nodes expose only:

- stable nonzero node and optional parent identities;
- inclusive finite world bounds;
- covered Source Point count;
- display Point count;
- conservative geometric error; and
- whether display Coverage is sampled or complete.

The application supplies renderer batch identities, versions, status, and byte
costs. `point-index` does not import `point-view` or `render-protocol`.

`CandidatePlan` contains sorted, disjoint Source Spans. It is complete or an
error; it never reports partial exact-candidate Coverage. `CandidateLimits`
bounds visited nodes, retained spans, candidate Points, and working bytes before
the plan is returned.

`IndexPointBatches` implements the common bounded batch-stream contract and
emits `IndexPointBatch` values. Each batch carries the Source Identity,
transform, index node, and sorted unique `(ordinal, ticks)` display samples.
This deliberately is not the canonical contiguous `PointBatch` contract:
internal samples are sparse partial View Coverage and must not be mistaken for
an authoritative Source read. The application can reconstruct exact Point
Identity and world position, but it cannot treat a sample batch as a complete
Query result. The terminal summary states the node, emitted count, covered
Source count, Source provenance, and whether the node is a sampled internal
node or complete leaf. Failure or cancellation publishes no summary and fuses.

## Deterministic index recipe

The v0.4 recipe is fixed and versioned rather than caller-tunable:

- canonical Source order is divided into consecutive blocks of at most 65,536
  Points;
- every block records its exact tick-derived world bounds and Source Span;
- blocks are bulk-loaded into a binary BVH by deterministic
  longest-centroid-extent median splits, with Source ordinal as the final
  tie-breaker;
- node keys are assigned in deterministic root-first order;
- internal bounds are exact child-bound unions and leaf bounds are exact Point
  bounds;
- leaf geometric error is zero, while internal error is the full finite world
  diagonal of its bounds; and
- each internal node stores at most 4,096 exact `(ordinal, ticks)` samples.

Samples are selected by the lowest values of one versioned stable ordinal hash,
then ordered by Source ordinal. The top sample of a parent can be computed by
merging its two child samples, so finalization needs only bounded sample memory.
Batch partitioning, checkpoint boundaries, thread timing, paths, timestamps,
and memory limits do not change final node keys, samples, or artifact bytes.

Leaves remain contiguous Source Spans and are read from the verified Source,
then projected into the same display-only `IndexPointBatch` shape. Internal
samples are persisted in the rebuildable index because thousands of sparse
ordinals would otherwise make a root LAZ sample decode most of the file before
the first visible frame. Persisted samples contain only ordinal and exact
ticks; the opened index supplies their verified Source Identity and transform.
They are checksummed, partial View Coverage, and never an exact Query result.
Source Attributes are not copied into the index.

For LASzip compressor modes 2 and 3 with a validated fixed-size chunk table, the
LAS/LAZ adapter uses the codec's seek when a sorted Source read crosses into a
later chunk. Leaf materialization then pays for the target chunk and bounded
requested records instead of replaying every earlier Point. Movement within the
current chunk, point-wise compressor mode 1, and variable-size chunk streams
retain bounded cancellable sequential replay. The exact PDRF 5 and 8 regression
reads fewer than half the bytes of forced replay for its far-span fixture and
compares the returned record bytes and ticks exactly.

## Conservative spatial lookup

The only v0.4 spatial request is one inclusive `WorldBounds` box.

Construction assigns every Source Point to exactly one consecutive leaf block.
Each leaf stores bounds that include every Point in that block. Candidate lookup
traverses every hierarchy node whose bounds intersect the request and returns
the Source Span of every intersecting leaf. Therefore, if a Source Point lies
inside the request, its leaf bounds intersect the request and its ordinal is in
the returned plan. False positives within a block are allowed; false negatives
and duplicate Point Identities are forbidden.

The sequential oracle decodes the Source directly and applies the same
inclusive finite coordinate comparisons. Random, boundary, degenerate, and
extreme-coordinate fixtures compare every oracle match with the candidate
union.

## Persistence and recovery

The target is one immutable complete artifact. A deterministic sibling work
file is an append-only sequence of Source-block frames. Each complete frame
contains its Source Span, exact bounds, bounded leaf sample, and BLAKE3 checksum.
Disk version 1 stores all multibyte integer fields and the raw bit patterns of
all `f64` fields in little-endian order. Fixed magic bytes, Source identities,
and BLAKE3 checksum bytes retain their declared byte order.

After interruption, `prepare`:

1. verifies the work header's Source Identity, Point count, transform, recipe,
   and disk-contract versions;
2. scans frames to the last complete valid checksum and ordinal-contiguous
   boundary;
3. discards only the invalid or incomplete disposable suffix;
4. resumes at the next canonical Source ordinal; and
5. keeps reporting progress from the durable completed count.

Finalization loads only bounded leaf/node metadata, builds the deterministic
BVH, merges child samples with fixed working buffers, and writes a new complete
artifact. The artifact header records Source Identity, Source count, exact
transform bits, recipe and disk versions, counts, offsets, and lengths. A final
BLAKE3 checksum covers every prior byte. The temporary artifact is written and
`sync_all` completes before publication. Publication is atomic and no-replace:
an existing or racing target is rejected and is never overwritten. The v0.9
hardening publishes from the owned open descriptor (`fclonefileat` on the
verified macOS filesystem; an unnamed `O_TMPFILE` linked once on Linux), then
syncs the parent directory. It retains the valid work prefix and any named
sample spool or platform-specific stage instead of deleting a pathname that a
racer could have replaced. This is deliberately not rename-and-replace
behavior.

Opening validates lengths, counts, versions, Source binding, checksum, node
topology, nested bounds, Source-span coverage, exact bottom-k sample ordinals
from descendant spans, and the caller's hierarchy and resident-byte limits
before returning `PreparedIndex`. No incomplete artifact can answer candidates
or View reads.

Complete artifacts are trusted local rebuildable caches. Their unkeyed BLAKE3
checksums detect accidental corruption and concurrent mutation; they do not
authenticate bytes deliberately rewritten by an adversary. Warm opening does
not reread Source Points, so an artifact obtained from untrusted storage must be
discarded and rebuilt from the verified Source rather than opened in place.

## Progressive View bridge

The existing `point-view` planner remains the only owner of frustum culling,
screen-error policy, priority, refinement, Coverage retention, and retirement.
One additive planner fact is required: `ViewPlan::demanded_nodes()` returns the
sorted set of nonresident target nodes, including nodes already marked
Requested. `requests()` remains the new-load delta. The application can then
drop camera-stale queued work without inferring demand from absence.

For each pump, the private bridge:

1. supplies a complete hierarchy snapshot plus host-owned statuses to
   `ViewPlanner`;
2. removes queued Requested nodes absent from `demanded_nodes()` and marks them
   Missing;
3. enqueues new `requests()` in planner priority order within queue and host
   reservation limits;
4. materializes at most the caller's bounded Point/byte/action allowance;
5. waits for the exact node-read summary before constructing one complete
   renderer Point Batch;
6. applies one atomic Upsert and marks Resident only after renderer acceptance;
   and
7. applies the planner's conditional retirements only after replacement
   Coverage is resident.

The first implementation performs at most one bounded node read per pump. This
keeps ordering, reservations, and tests simple; a host may run the same concrete
read off-thread. Parallel loaders and a general action/acknowledgement reducer
remain deferred until measurements show the synchronous bounded pump is
insufficient.

Generation reset, shutdown, Source failure, stale batch results, renderer
rejection, and budget failure never publish a partial node. Existing resident
Coverage remains until a replacement Upsert is accepted.

## Resource limits

Limits remain separate so one subsystem cannot hide another's allocation:

- `PrepareLimits` caps Source batch Points/payload, adapter working bytes,
  build working bytes, incomplete bytes, complete artifact bytes, hierarchy
  nodes, and resident index metadata bytes.
- `CandidateLimits` caps visited nodes, output spans, candidate Points, and
  working bytes.
- `NodeReadBudget` caps emitted Points, Source spans, Source-read batch Points
  and payload, display-batch bytes, index buffers, and adapter working bytes.
- the application staging budget caps queued nodes, one materialized render
  batch, and total host-owned staging bytes;
- `PlanningBudget` caps planned Points, estimated GPU bytes, and batches; and
- `RenderLimits` independently enforces actual logical renderer residency.

Checked arithmetic precedes allocation. A single node, page, decoder block, or
batch that cannot fit fails explicitly instead of exceeding a ceiling. The
implementation uses ordinary file reads and charged buffers rather than an
unaccounted memory map.

## Delivered slices

1. `point-index` contracts, deterministic block/BVH construction, conservative
   candidates, complete persistence, and public in-memory conformance.
2. append-only checkpoint recovery, persisted internal samples, bounded node
   streams, and efficient sparse LAZ leaf seeks.
3. private real-cloud bridge, existing planner demand facts, renderer/GPU
   integration, runnable file path, and source-scale benchmarks.

## Repository acceptance

The completed v0.4 repository evidence covers:

- candidate lookup has zero false negatives against a sequential oracle and
  returns sorted duplicate-free Source Spans;
- uninterrupted, cancelled, and fault-injected resumed builds produce the same
  descriptor, node/sample facts, and complete artifact bytes;
- corrupt, truncated, incompatible, changed-Source, cancelled, and over-budget
  cases are explicit and never expose a partial complete index; filesystem
  publication errors retain their operating-system error and path;
- hierarchy keys, bounds, samples, planner results, and renderer update order
  remain deterministic across Source batch sizes and restart points;
- cached internal samples and Source-backed leaves preserve exact Source-aware
  Point Identity and ticks;
- parent Coverage remains resident until complete replacement Upserts are
  accepted;
- Source, index, staging, planner, and renderer limits are each enforced by
  interface tests and measured peak-memory gates;
- a real-file demo accepts supported LAS or LAZ and reports Full verification,
  cold/resumed/warm prepare disposition, reused/read Points, artifact size,
  first accepted batch latency, queue depth, staging peaks, and renderer
  residency; and
- all local checks in `CONTRIBUTING.md`, including required GPU acceptance,
  pass.

The process-level LAS/LAZ smoke test Full-verifies a generated file, exercises
both Built and Opened paths, plans one node, and accepts one atomic Upsert in the
CPU state model without requiring a GPU. Required GPU tests separately prove
the renderer and planner-to-renderer Coverage transition on an available local
adapter.

### One-million-Point baseline

`cargo bench -p point-index --bench index` uses a deterministic in-memory
Source with 1,000,000 Points and default limits. On the 2026-08-10 local
reference run (Apple M5 Pro, 24 GiB, macOS 26.5.2, Rust 1.90.0):

| Measurement | Result |
|---|---:|
| Complete artifact | 1,971,528 bytes |
| Resumed prepare | 65,536 durable Points reused; 934,464 Source Points read |
| Cold build, Criterion median | 330.515 ms |
| Warm verified open, Criterion median | 20.567 ms |
| Whole-bounds candidate plan, Criterion median | 606 ns |
| 4,096-Point internal-root read, Criterion median | 1.249 ms |
| 65,536-Point complete memory-backed leaf read, Criterion median | 122.100 µs |
| Candidate + root + leaf measured peak heap | 3,671,504 bytes (32 MiB ceiling) |

The separate debug allocation correctness test uses 131,073 Points and measured
1,737,922 peak heap bytes for cold prepare and 132,375 bytes for warm open,
both under its 64 MiB ceiling and with zero retained measured bytes after the
operation.

These figures are reproducible generated-fixture baselines for one named local
machine, not universal performance targets. No licensed production Source or
design-partner result is checked into the repository. Such a run remains
explicitly outstanding and must be reported separately with Source Identity,
format, Point count, limits, machine, and cold/warm cache qualification; the
repository does not fabricate external field evidence.

### Direct verification commands

```bash
cargo test -p point-index --all-features
RUSTDOCFLAGS="-D warnings" cargo doc -p point-index --all-features --no-deps
cargo run -p point-index --example direct_use
cargo bench -p point-index --bench index
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
```

## Out of scope

Punctra v0.4 does not add:

- durable partial preview generations or View state recovery;
- a Morton point store, octree plugin, or native COPC hierarchy import;
- paged/partial hierarchy planning;
- arbitrary Region shapes, exact Point predicates, or Attribute filtering;
- Workspace, Snapshot, Revision, Edit, Point Set, or Query semantics;
- network range access, retries, authentication, or remote caches;
- reprojection or Coordinate Reference guessing;
- automatic renderer eviction or index-selected display styling; or
- a stable third-party index, loader, executor, or renderer plugin API.
