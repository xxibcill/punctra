# Verification Strategy

Status: **Complete through the v0.9 repository trust and version-1
compatibility candidate, the v0.10 professional inspection View repository
implementation, and the repository-verified v0.11 exact-review technical
slice, plus the v0.12 explicit spatial-reference and package-publication
repository slice; v0.13: Complete and repository-verified for the
bounded persistent-terrain slice; field activation, production-scale accuracy,
true out-of-core adoption, independent adoption, partner validation, and
support qualification outstanding; v0.14 bounded exact Terrain QA and
correction-loop slice Complete and repository-verified; v0.15 bounded local
WebAssembly/WebGPU browser-foundation slice Complete and repository-verified;
v0.16 bounded HTTP Range/cache/worker slice Complete and repository-verified;
v0.17 bounded viewer API/exact-Point, v0.18 packed SDK/React lifecycle, and
v0.19 exact local browser/device qualification and v0.20 clean packed-consumer
integration-baseline slices Complete and repository-verified;
v0.21 bounded visual-baseline implementation Accepted and in progress, with
attended evidence and release verification outstanding; all gates run locally**

Verification follows public contracts first. Private tests are used for fault
injection and measured implementation boundaries that cannot be triggered
safely through the public API. Hosted CI is not configured.

## Test layers

1. **Value tests** validate canonical identity, coordinates, schemas, hashes,
   limits, serialization, and error taxonomy.
2. **Crate interface tests** exercise each independently usable public seam.
3. **Adapter conformance** runs one Source contract across memory, LAS, and LAZ.
4. **Persistence tests** inject interruption, corruption, cancellation, panic,
   and lost acknowledgement at publication boundaries.
5. **Composition tests** exercise Source-to-index, index-to-View, Workspace-to-
   Terrain, terrain-to-LandXML, and planner-to-renderer behavior without private
   cross-crate access.
6. **Frozen compatibility fixtures** reopen committed version-1 bytes and
   exercise format, checksum, lineage, binding, and semantic mutations without
   regenerating the expected side.
7. **Benchmarks and resource gates** measure Source-scale time, heap,
   temporary bytes, durable growth, and GPU residency.
8. **Required local GPU acceptance** proves the native wgpu path when an
   adapter is expected.
9. **Local browser acceptance** proves one exact WebAssembly/WebGPU host,
   canvas lifecycle, resize/visibility behavior, provisional pick, resource
   diagnostics, shutdown, and recreation without generalizing to other
   browsers.
10. **Packed-consumer baseline verification** installs only local tarballs into
    clean TypeScript and React applications, checks supported exports and
    deployable assets, builds development/production bundles, freezes fixture
    and scene identities, and runs the deterministic browser quickstart.
11. **Visual-baseline verification** regenerates the permission-bound corpus,
    captures settled canonical renderer images through one private module, and
    recomputes decoded-pixel, temporal, Coverage, feature, authority, resource,
    rubric, archive transport/export, and evidence-pin gates for one exact
    attended lane.

Tests assert semantic results, ordering, exactness, publication certainty, and
resource failure. They avoid depending on private tree shape, batching,
allocation strategy, scratch names, or shader internals unless that detail is
itself a persisted or public contract.

## Fixture principles

- Prefer deterministic generated fixtures with explicit seeds and schemas.
- Keep exact integer ticks available for independent oracles.
- Vary Source batch size and query partitioning.
- Include empty, singleton, boundary, duplicate, signed-zero, extreme finite,
  corrupt, truncated, and over-limit cases.
- Use generated LAS and LAZ files for format-to-Workspace integration.
- Use exact planar, cocircular, duplicate-XY, collinear, boundary, and gap
  fixtures for Terrain/QA oracles.
- Freeze complete/work Surface disk-v1 bytes before claiming compatibility;
  mutate copies for truncation, checksum, offset, ordering, reference, and
  binding failures rather than regenerating expected truth.
- Compare semantic values before exact bytes unless deterministic bytes are a
  stated contract.
- Never commit licensed production data without redistribution rights.
- Bind every licensed derivative to upstream identity, deterministic recipe,
  attribution, modification notice, redistribution permission, and rendered-
  image publication permission; exact regeneration precedes browser use.
- Keep frozen expected bytes owner-local with a manifest that pins relative
  paths, lengths, hashes, versions, identities, support class, and semantic
  facts. Consumers read or copy those bytes; they do not regenerate truth.

Generated files prove implemented behavior, not production representativeness
or customer value.

## Current crate verification

### point-contracts

- identity validation and round-trip;
- Source-aware Point ordering;
- finite position transform and world-bounds rules;
- Attribute schema/data consistency;
- normalized Source spans; and
- deterministic hashes and serde values.

### foundation-runtime

- `Future` and `blocking_wait` equivalence;
- monotonic progress;
- fused cancellation;
- parent cancellation linked directly into an actively awaited child Job;
- panic-to-runtime-error mapping;
- bounded pull-stream terminal behavior; and
- no hidden async-runtime requirement.

### Source path

The shared conformance suite checks:

- stable Source and Point Identity across repeated and differently partitioned
  reads;
- exact ticks, supported Attributes, ordering, projection, and summaries;
- cancellation and every read/decode ceiling;
- corrupt, truncated, changed, and unsupported input behavior; and
- Full/Fast reopen semantics.

Generated LAS covers point-data record formats 0–10. Generated LAZ covers
formats 0–8; 9 and 10 are explicit unsupported cases. One-million-Point memory,
LAS, and LAZ benchmarks enforce adapter-specific memory ceilings.

The version-1 Source corpus freezes the generic Source Record plus the memory
and LAS adapter representations. Reopen preserves exact identities and facts;
future version, truncation, checksum/content, and Source-binding mutations fail
closed without modifying the committed fixture.

### point-index

- candidate plans have no false negatives against a sequential exact oracle;
- candidate spans are sorted, disjoint, and bounded;
- build/resume produces deterministic descriptor and artifact bytes;
- valid-prefix recovery resumes at the exact durable boundary;
- incompatible, corrupt, truncated, over-limit, racing, and checksum-valid
  non-recipe-sample targets fail without replacement;
- display samples and exact leaves preserve Source-aware identities/ticks;
- v2 cold, resumed, and warm reads preserve row-aligned raw inspection values,
  enforce Attribute availability/types and 42-byte accounting, and leave v1
  fixtures byte-identical;
- LAZ fixed-chunk seeks cross chunk boundaries exactly; and
- process smoke covers Full verification, Built then Opened index paths, and
  one complete CPU-model renderer Upsert.

The rebuildable index corpus pins one complete disk-1/recipe-1 artifact and one
valid resumable work prefix. Tests open, resume, and rebuild equivalently while
preserving incompatible or corrupt artifacts for diagnosis.

### point-workspace

The current public, persistence, fixture, fault-injection, and allocation suites
retain the v0.6/v0.7 lifecycle, selection, row-stream, persistence, and Revision
Audit coverage. They prove:

- create, root identity, exclusive lock, complete-handle lifetime, and reopen;
- schema rejection when the chosen classification Attribute is absent or not
  `U8`;
- All, inclusive world-box, optional effective-classification, and explicit
  Point-ID selection against a brute-force Source-plus-overlay oracle;
- 4,099 seeded randomized Points across 32 box/classification Queries and
  varied Source batching;
- Point-ID Source validation, bounds, sorting, and deduplication;
- identical resident/forced-spill membership, order, hashes, repeated reads,
  corruption detection, live path-replacement rejection, and retained private
  spill behavior;
- mixed before-values, sparse classification rows, no-op rejection, immutable
  historical Snapshots, immediate-head Revert, and redo-by-Revert;
- Point Identity through Source, index, Point Set, commit, Revert, and reopen;
- byte-for-byte unchanged generated LAS/LAZ Source files and unchanged
  non-classification values;
- prepublication failure for selection, ID-read, open, and commit hard limits;
- committed, rejected, retryable, not-recorded, conflict, and indeterminate
  Operation reconciliation;
- idempotent retry with at most one Revision;
- fail-closed corruption plus validated, conservatively retained recognized scratch;
- exact ordered `Snapshot::point_rows` values at root, edited, historical, and
  Revert Snapshots;
- partition-independent row membership/content hashes matching Point Set
  membership and identical generated LAS/LAZ row values;
- cumulative row limits, complete no-match behavior, fused error/cancellation,
  and absence of a terminal summary after failure; and
- exact root/classification/Revert Revision Audit transitions, membership and
  content hashes, Edit Footprints, historical immutability, Source partition
  independence, every audit resource family, cancellation, and corruption
  rejection.

The Workspace version-1 corpus freezes a complete root/Revision lineage,
retryable ready Operation, definitive rejection, and absence of a live lock.
It reopens without mutation and fails closed on future versions, truncation,
checksum changes, lineage forks, and Source/index binding drift.

Private persistence tests inject error, cancellation, panic, and lost
acknowledgement at candidate stage/file-sync/close/read-only/revalidation,
ready link and operations-directory sync, Revision link and directory sync,
rejection stage/file-sync/read-only/revalidation/link/directory-sync/cleanup,
and recovery sync boundaries. Reopen exposes only a complete old head or a
complete new head; it never fabricates a partial Revision.

The selection unit gate measures worker-equivalent allocation around the same
private worker function used by the Job. Its observed peak was 6,292,224 bytes
under a 64 MiB ceiling, with zero retained measured allocations.

### point-review

Public interface fixtures independently exercise perspective and orthographic
CPU projection, inclusive clip/rectangle edges, top-left screen coordinates,
optional effective-classification filtering, normalized and invalid rectangle
input, exact pick confirmation, foreign/out-of-range identities, Snapshot
provenance, resident/forced-spill Point Sets, hard match/working limits, and
cancellation without terminal publication. Confirmed values come from exact
ticks and overlays rather than renderer payloads.

The generated Criterion benchmark measures the complete screen-through scan at
declared fixture sizes and reports resident versus forced-spill disposition,
generated Source/match counts, and every configured review/Point Set resource
ceiling. It additionally reports the terminal review's conservative
algorithm-accounted working high-water and stable owned-fixture
file-count/logical-file-length delta while each verified Point Set remains
alive. Those are not measured heap, allocated filesystem blocks, or
process-wide disk observations. It is repository microbenchmark evidence, not
a production latency or attended-time claim.

### point-terrain and terrain-demo

The `point-terrain` package and documentation suites cover:

- unit/private tests for canonical input keys, deterministic robust
  predicates, cocircular tie-breaking, the pinned `delaunator` oracle, bounded
  topology work, large-world and extreme finite numeric behavior, cancellation,
  diagnostics, allocation preflight, and injected post-publication LandXML
  certainty boundaries;
- public interface tests for canonical facts, inclusive Ground Input,
  partition-independent hashes, and historical/Edit/Revert Snapshots;
- topology tests proving counter-clockwise canonical manifold Delaunay disks
  and the canonical cocircular diagonal;
- resource tests for insufficient, duplicate/conflicting, collinear,
  unsupported numeric, input/output/face/work/working/retained limits, and
  cancellation without publication;
- QA tests for closed boundaries, stable face selection, rounded and
  large-world sampling, ordered
  positive/negative/zero residuals, explicit gaps, compensated statistics,
  overflow-safe large finite statistics, duplicate identities, every QA
  resource family, bounded input consumption, cancellation, and result-sealing
  overlap; and
- LandXML tests for deterministic independent semantic parsing, explicit
  coordinate/date/time and metric-unit assumptions, every XML resource family,
  target conflict, cancellation, and durable publication certainty.

The `point-terrain` doctest and direct example compose the public Source/index/
Workspace/Terrain/QA/LandXML APIs. v0.7 adds exact-existing LandXML ensure tests
for create, reconcile, conflict, races, symlink/non-regular rejection,
publication faults, post-link cancellation certainty, and lost
acknowledgement.

The completed v0.13 persistence suites additionally cover public-interface
coverage for `TerrainPrepareDisposition::{Built, ResumedInput,
ResumedPublication, Opened}`; exact equivalence with the legacy
explicit-AOI topology oracle; no Snapshot row consumption on warm open;
Snapshot/Recipe/AOI/algorithm/transform/spatial-reference stale rejection;
bounded vertex/face stream ordering, complete exhaustion, and touched-block
revalidation; complete/work-v1 goldens; and no-replace publication certainty.
Fault tests cover torn final work suffixes, interior corruption, truncation,
invalid counts/offsets/order/face references, disk exhaustion, cancellation,
create/write/sync/link/readback failure, races, and changed paths while
preserving unproven files.

The v0.13 resource gates independently cover Point rows, Ground Input,
full-AOI triangulation memory, topology work, work/checkpoint/stage/Artifact
bytes, handle metadata, checksum/read buffers, stream records/payload/work, and
cumulative temporary bytes. A passing disk-persistence test is not an
out-of-core-memory claim: the complete AOI remains resident during topology.

The v0.14 exact-QA suite additionally covers analytic station profiles,
Source-Point and detached residuals, positive/negative/inclusive-boundary
tolerance, gaps, stable hashes across Source-row batching, exact
Snapshot/Surface provenance, stale state after Edit, prepared/in-memory result
equality, semantic face comparison, conservative changed bounds, post-Revert
topology restoration, independent one-under limits, and cancellation without a
partial result. The public `exact_terrain_qa` example emits traceable JSON/SVG
evidence while explicitly denying field, timing, adoption, partner, and support
claims.

The `terrain-demo` unit/private, frozen-corpus, workflow-facade, and process
suites cover:

- every prefix of the eight-frame journal resuming to the same receipt and one
  Operation Revision;
- torn-final-suffix recovery plus representative complete-frame corruption,
  lock, limit, and path-binding failures;
- exact report reconciliation/conflict and LandXML/report recovery;
- generated LAS/LAZ equality of the named source-independent semantic
  projection while full identity-bearing reports honestly differ;
- immediate parent cancellation and an active dropped Workflow without a false
  `Complete` checkpoint, with resumability and unchanged Source bytes;
- 12 public limit families, stale baseline, differently bound recorded
  rejection, changed Source, changed Workspace identity, and deterministic
  Retryable intent;
- rejection of an existing Workspace whose public
  `schema().classification()` is not Source Attribute 6, before Run or
  Workspace mutation;
- Run-root validation failures retaining the already-known Run, Operation, and
  baseline Revision identities; and
- bounded `start`, `resume`, `inspect`, `compare-landxml`, and
  `verify-round-trip` CLI output and structured failures;
- every committed journal prefix and one Complete Run with exact report,
  LandXML, returned pass/fail files, and canonical pass/fail evidence;
- full-export-ceiling streaming, XML/subset/Coordinate-Reference/unit/
  Point-count/vertex-mapping/tolerance/topology reason families, adversarial
  token and retained-memory bounds, and presentation-only rewrites; and
- strict immutable input witnesses, no Run repair or mutation, exact evidence
  reconciliation, conflicting targets, and post-publication uncertainty.

The v0.8 fold-forward coverage additionally proves a strictly read-only
Complete-Run snapshot under an existing shared Run lock; rejection of missing
locks, non-Complete/torn journals without repair, and changed bound artifacts;
canonical Run-bound pass and semantic-failure evidence; exact reconciliation
and caller-owned conflict preservation; nonzero stable semantic diagnostics;
and no evidence for malformed or operational failures. Private evidence tests
also exhaust every application-defined post-link acknowledgement boundary,
prove exact/conflicting no-replace create races, preserve post-link replacement,
and reconcile an exact retry. Checked-in v1 pass and topology-failure evidence
fixtures pin exact lengths, BLAKE3 hashes, semantic reason facts, and canonical
bytes. The bounded local XML verifier streams the exact captured bytes under the v0.7
exporter's 4-GiB, 10-million-vertex, and 20-million-face ceilings. Focused
tests cover inclusive and just-over file/node/text/token/parser/Point/face/
comparison/retained limits; comments, CDATA, processing instructions, and DTD
rejection; partial `BufRead` consumption; deep namespace nesting; same-inode
token mutation; append past the captured length; and measured small-allocation
oversized-token rejection.

The evidence `accounted_*_peak_bytes` values are deterministic algorithm
charges: fixed read/scanner buffers, exact requested token/event/stack/text
payloads, semantic collections, comparison mappings/topology samples, and the
concurrent exact-compare buffers. They deliberately exclude allocator
metadata/slack, process RSS, and measured heap. Every modeled growth is checked
before its fallible reserve; the bounded reader prevents more than one lexical
token or bytes past the captured length from reaching semantic parsing.

The retained v0.6 process/regression coverage also proves deterministic
generated LAS and LAZ Terrain semantics, exact changed Ground Input,
byte-identical Source data, exact immediate-head Revert restoration of
geometry/topology/vertices/faces, and explicit Revert behavior independently
of the durable workflow facade. The superseded one-shot CLI grammar is not
retained, and the v0.7 Workflow leaves a committed classification Revision in
place when a later phase fails.

This is representative public recovery evidence, not an exhaustive injected
hook at every OS fault or active-child cancellation boundary. Private journal
tests exhaust the application-defined Intent-publication and
append-before-write, before-sync, and after-sync lost-acknowledgement boundaries
using `Complete`. Private report tests exhaust the application-defined
post-link boundaries. Representative report cases cover pre-link cancellation/
failure, exact and conflicting `AlreadyExists` races, post-link replacement,
target kind, staging/working limits, and stage/parent directory identity. These
labels do not claim every possible OS fault, active-child cancellation, or
corrupt journal topology.

The private workflow suite also rederives the immediate-head Revert and proves
an empty baseline-to-restored Surface Change Envelope.

### render-protocol and point-view

- generation reset and stale update rejection;
- atomic Upsert and conditional Remove;
- atomic complete-highlight rejection before duplicate removal under its
  independent input ceiling;
- frustum and screen-space-error selection;
- deterministic priority and hierarchy-cut decisions;
- point, byte, and batch reservation;
- demanded Requested work, retention, and safe retirement;
- hysteresis reset across generations; and
- malformed hierarchy rejection before planner history changes.

### render-wgpu and renderer-demo

- synthetic Reset/Upsert/Remove application;
- logical point/byte/batch residency ceilings;
- large-world origins and camera-stable multi-frame recording;
- `RecordedFrame` resource and identity retention;
- asynchronous provisional picking;
- a public-only host example that owns wgpu setup, submission, polling, and a
  bounded provisional pick without importing private demo state;
- host-owned submission and polling;
- planner-to-renderer parent Coverage retirement after replacements render;
- exact neutral/elevation/RGB/intensity/classification CPU color mapping,
  Source-bound normalization, all 256 raw classification values, and identity/
  geometry invariance across modes;
- disk-v1 position-only versus disk-v2 attributed recipe validation and
  generated LAS/LAZ build/open process paths;
- perspective/orthographic camera grammar, target-plane-scale toggle, pan,
  reset, matching View frustum/SSE behavior, and large-world depth/picking;
- truthful demanded/candidate/issued/retained/retired and
  Sampled/Complete/paused state presentation;
- bounded stable View diagnostic rendering and phase/action mappings;
- bounded permission-gated corpus manifests, deterministic no-private-path
  report encoding/nonclaims, and no-replace create/reconcile/conflict; and
- tolerant local offscreen GPU checks for every accepted display mapping.

The mapping oracle fixes neutral at `[190,205,220,255]`; elevation at the five
accepted palette stops with clamping, midpoint for zero extent, interpolation,
and nearest-byte rounding; and RGB/intensity `U16` scaling at
`(v * 255 + 32767) / 65535`. Classification exhausts all 256 inputs: 0–18 use
the fixed table in the [v0.10
design](../design/field-inspection-view-v0.10.md#implemented-display-mappings-and-cli),
while 19–255 use wrapping `u8` `(73c+41, 151c+97, 199c+17)`. All outputs are
opaque. Identity/geometry tests compare modes before the tolerant GPU readback.

Required GPU tests set `PUNCTRA_REQUIRE_GPU=1`; a missing adapter is then a
failure rather than a skip.

## v0.5 benchmark and allocation evidence

The default `point-workspace` benchmark generates one million Points and
measures 0/1/50/100-percent exact selection, resident versus forced spill,
sparse/dense classification and Revert, increasing-depth reopen, logical bytes
per changed Point, allocation, process RSS, temporary storage, and durable
growth.

Reference environment: Apple M5 Pro (`Mac17,9`), 24 GiB, arm64, macOS 26.5.2,
Rust 1.90.0. The evidence pass and all declared Criterion cases completed
locally.

The worker-equivalent value below comes from a separate 131,073-Point
synchronous allocation test around the same private selection function used by
the Job. The one-million-Point benchmark explicitly does not claim worker heap;
it reports caller-thread Point-ID allocation and sampled process RSS instead.

| Evidence | Local value |
|---|---:|
| Companion selection worker-equivalent peak heap | 6,292,224 B (64 MiB gate) |
| Point-ID iteration peak / retained heap | 2,621,440 B / 0 B |
| Resident-selection process RSS | 62,668,800 B |
| Forced-spill baseline / sampled peak RSS | 62,685,184 B / 62,832,640 B |
| Forced-spill sampled RSS delta | 147,456 B |
| Sealed temporary payload | 9,009,182 B |
| Sparse 10k set / Revert | ~16.442 / 15.818 ms |
| Sparse logical bytes per changed Point | 20.100 B |
| Dense 500k set / Revert | ~34.973 / 35.778 ms |
| Dense logical bytes per changed Point | 20.004 B |
| Reopen at depth 2 / 4 / 8 | ~1.231 / 37.753 / 74.968 ms |
| Final logical directory-entry bytes | 40,812,316 B |
| Final physical `du` bytes | 20,418,560 B |

Logical directory-entry bytes count a shared hard-linked payload at each
directory entry. Physical `du` bytes reflect inode sharing. Process RSS is a
sample, not an allocation counter. These distinctions are retained in reports
rather than collapsed into one “memory” number.

The results are one-machine generated-fixture baselines, not universal latency
claims. Licensed production LAS/LAZ, above-500-million-Point, and design-partner
validation remain explicitly outstanding.

## v0.6 benchmark and resource evidence

The `point-terrain` Criterion benchmark composes a generated in-memory Source,
complete index, Workspace Snapshot, Terrain Derivation, three detached Check
Points, and durable LandXML export through public APIs. The default is 10,000
Points; `PUNCTRA_TERRAIN_BENCH_POINTS` permits positive generated sizes up to
1,000,000, with intended 10,000, 100,000, and 1,000,000 scales.

The completed local evidence is the 10,000-Point run on the Apple M5 Pro
(`Mac17,9`), 24 GiB, arm64, macOS 26.5.2 reference machine with Rust 1.90.0.
Criterion reported:

| Evidence | Local value |
|---|---:|
| Input / vertices / faces / hull vertices | 10,000 / 10,000 / 19,602 / 396 |
| Derivation | 11.983–12.049 ms |
| Derivation throughput | 829.97–834.53 Kpoints/s |
| Detached QA | 94.907–95.164 us |
| QA inputs / location work | 3 Check Points / 19,604 face tests |
| Durable LandXML creation | 18.020–18.311 ms |
| LandXML throughput / size | 53.650–54.518 MiB/s / 1,030,118 B |
| Descriptor accounted peak working bytes | 135,790,592 B |
| Descriptor retained Surface bytes | 1,034,176 B |
| Descriptor topology steps | 521,494 |
| QA accounted peak working bytes | 336 B |
| Evidence record machine | `jjaes-MacBook-Pro.local` (`macos`/`aarch64`) |
| One-shot Derivation / QA / LandXML | 13,371 / 125 / 14,656 us |
| Observed worker heap | unclaimed (`worker_heap_measurement: null`) |

Descriptor and QA bytes are explicit algorithm-accounting facts, not allocator
observations. The null worker-heap field is retained rather than inferred from
process RSS or an unrelated thread. One-shot times are retained separately from
the Criterion intervals. The 100,000 and 1,000,000-Point modes are available
but are not claimed as completed evidence here.

These values are one-machine generated-fixture technical baselines. Licensed
production LAS/LAZ, Sources above 500 million Points, named downstream Civil
3D/Bentley round trips, partner tolerance, paid-use, and human-workflow evidence
remain explicitly outstanding.

## v0.7 Workflow benchmark evidence

The `terrain-demo` Criterion benchmark uses only generated local LAS data and
the public `start_run`/`resume_run` facade. It accepts exactly 10,000, 100,000,
or 1,000,000 Points through `PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS`; the recorded
local smoke is the 10,000-Point, ten-sample run.

| Mode | Lower | Estimate | Upper |
|---|---:|---:|---:|
| Cold start | 153.38 ms | 157.84 ms | 161.25 ms |
| Resume after committed Edit | 113.23 ms | 114.88 ms | 117.08 ms |
| Resume from Retryable Workspace intent | 123.76 ms | 126.67 ms | 129.66 ms |
| LandXML and report reconciliation | 96.871 ms | 97.629 ms | 98.365 ms |
| Complete revalidation | 87.233 ms | 88.181 ms | 89.112 ms |

The completed Run's durable journal was 2,804 bytes across eight frames. Its
canonical report was 11,539 bytes and contained 116 semantic limit facts.

The intervals are local generated observations, not universal latency claims.
Worker peak heap was not measured. No partner, production, downstream round-
trip, paid-use, or human-time acceptance is inferred from this benchmark.

## v0.13 persistent-terrain verification result

The accepted v0.13 generated example and benchmark report exact fixture,
AOI, limits, algorithm/disk versions, cold/resumed/warm disposition, Source rows
read or reused, input/vertex/face counts, topology work, retained triangulation
memory, verified work bytes, complete Artifact bytes, stream buffers,
cumulative work-plus-stage bytes, and absent observations. The exact-commit
completion record pairs those reports with named machine and toolchain facts.
A direct stage-byte observation remains explicitly absent rather than
inferred. Source verification, Spatial Index preparation, Terrain preparation,
warm reopen, legacy QA/LandXML, and View work are separate phases; an unrun or
unmeasured phase remains explicitly absent. The exact environment, command
outcomes, fixtures, and generated observations are recorded in the
[v0.13 repository verification record](../releases/v0.13.0.md).

The default generated example run uses 10,000 Points and currently reports a
10,000-vertex, 19,602-face, 396-hull-vertex Surface, a 320,480-byte verified
input checkpoint, and three-batch vertex/five-batch face consumption. It
asserts exact-byte resume and complete bounded stream consumption.
`PUNCTRA_PERSISTENT_TERRAIN_EXAMPLE_POINTS` and
`PUNCTRA_TERRAIN_BENCH_POINTS` accept generated counts from 3 through 1,000,000;
10,000, 100,000, and 1,000,000 are the intended benchmark scales. Only an
actually completed local run may be recorded. None establishes a production
AOI, an above-500-million-Point project, a supported workstation, true out-of-
core topology, field accuracy, independent adoption, partner acceptance, or
support qualification. The exact-commit completion record does not convert any
of those external gates into repository facts.

## v0.14 exact-QA verification target

Repository qualification requires the exact-QA integration suite, the public
correction-loop example, package/rustdoc verification, existing fuzz and
benchmark lanes, and every required forced-GPU acceptance command to pass from
one exact commit. Generated results can establish only deterministic values,
hashes, freshness, comparison, resource ceilings, and artifact traceability.
Observed professional defect-correction time and independent example execution
remain external exits.

## v0.15 browser-foundation verification target

Repository qualification requires native host/scene unit tests, strict native
and `wasm32-unknown-unknown` checks, a release WebAssembly build with the exact
matching `wasm-bindgen` CLI, and the static browser harness passing in one
recorded local WebGPU environment. The harness checks initialization, planning,
generation and batch version, bounded resize, visibility suppression,
provisional centre pick, fused shutdown rejection, and explicit recreation.
Generated diagnostics are logical protocol, canvas, and renderer texture
accounting; they are not browser heap or observed GPU-allocation measurements.
Remote Sources, independent embedding, broad compatibility, SDK stability, and
support qualification remain external or later-version exits.

## v0.16 HTTP Range streaming verification lane

The private v0.16 browser lane adds three independent local layers:

1. `cargo run -p browser-demo --bin generate_stream_fixture` fully verifies
   and regenerates the deterministic LAS/SourceRecord/disk-v2 index/deployment
   family in an isolated directory, then compares the committed semantic files;
2. `node --test apps/browser-demo/web/*.test.mjs` exercises the real local
   Range/CORS server plus manifest validation, strict Range responses, retry/
   cancellation, changed-Source, truncation, corruption, cache identity/
   invalidation/quota, disk-v2 decode, transferable batching, and worker-
   lifecycle/failure mapping; and
3. the secure-context browser harness served by
   `scripts/serve-browser-demo.py` exercises real Fetch/CORS headers, one module
   Worker, Cache API persistence across worker/viewer recreation, WebAssembly,
   WebGPU publication, and cold/warm evidence.

The Rust `browser-demo` tests independently pin Source-identity parsing,
strictly increasing transferred ordinals, sampled Coverage, View generation,
renderer publication, completion, and every deterministic resource ceiling.
The browser lane must report cold network bytes below the complete Source
length and a warm run with three verified cache hits and zero binary network
requests. It must also cancel one deliberately delayed Fetch and receive the
worker's `cancelled` acknowledgement within 1,000 milliseconds. Observed
main-thread milliseconds are recorded but are not a stable
gate; one task is deterministically capped at 1,024 Points and 24,576 bytes.

This lane supplements rather than replaces the workspace, package, fuzz,
benchmark, example, and forced-native-GPU checks. One local fixture/browser pass
does not establish hostile-server authenticity, broad browser support, process
memory, cache allocation, exact browser Queries, or SDK stability.

## v0.17 browser viewer API verification lane

The v0.17 lane adds direct JavaScript contract tests for lifecycle, immutable
state snapshots, scheduled-render coalescing, camera and viewport validation,
Source-load ownership, structured errors, declaration/runtime agreement,
generation-safe pick/highlight/exact handoff, cancellation, and normalized
input disposal. Rust tests independently pin the transfer-v2 layout, all five
inherited mappings, replacement batch versions, camera diagnostics, highlight
limits, and stale Source/generation rejection.

The real browser path drives the same checked-in public façade used by the
plain host. It renders all five modes and both projections, obtains a
provisional streamed identity, publishes and clears one presentation-only
highlight, confirms the same ordinal from one exact 34-byte LAS record, and
rejects cancelled and stale-generation confirmation. It also preserves the
v0.15 lifecycle and v0.16 cold/recreation/warm evidence. A passing local Chrome
151/WebGPU run proves only that exact recorded environment; it does not prove
SDK packaging, framework compatibility, arbitrary Sources or Queries, API
stability, or a browser support matrix.

## v0.18 packed browser SDK verification lane

The v0.18 lane first tests asset resolution, cache-token bounds, lifecycle
aliases, bundler-aware Worker construction, declaration exports, and repeated
React asynchronous mount/cleanup behavior directly in Node. The generated API
reference must match the exact packed declaration sources.

`scripts/build-browser-sdk.sh` builds the release Wasm binding and creates exact
`@punctra/viewer` and `@punctra/react` npm tarballs. The verification runner
inspects their contents, copies both checked-in trials to new temporary
directories, installs the tarballs rather than repository paths, and runs
strict TypeScript. It executes Vite development and production builds, verifies
the plain SDK dynamic-import split, content-hashed Wasm and module-Worker
assets, and development-server transforms. The React lifecycle test repeats 64
abandoned async mounts and additionally proves mounted unsubscribe-before-
dispose ordering and idempotent cleanup.

The real browser host now imports the package SDK entry rather than coordinating
raw Wasm bindings. A pass qualifies only these exact local packed artifacts,
Vite/TypeScript versions, and recorded browser environment. It does not prove
npm publication, other bundlers/frameworks, independent adoption, CSP-host
compatibility, API stability, broad browser support, or a release candidate.

## v0.19 browser/device qualification lane

The v0.19 lane preserves the v0.18 packed-artifact path and adds one private
qualification layer. Node tests pin Source-load timing capture, deterministic
frame percentiles, nullable JavaScript heap facts, exact ceiling evaluation,
atomic resize retry, pre-publication Worker recovery, partial-publication
fusion, device-loss fusion, and matrix shape. The strict server adds one
bounded disconnect route so the real Worker returns the existing `offline`
outcome without a synthetic Fetch implementation.

The attended browser run rejects an over-limit viewport without mutation,
changes and restores DPR, skips a hidden frame, resumes, recreates after normal
disposal, survives a deliberate Worker crash and disconnected manifest before
publication, acknowledges cancellation, completes cold and zero-request warm
delivery, and repeats all v0.17-v0.18 functional checks. After settlement it
samples 30 foreground callbacks and viewer submissions and evaluates every
accepted latency and resource ceiling.

`node scripts/verify-browser-qualification.mjs` binds the machine-readable
matrix to the checked-in qualification constants and exact workload/recovery
facts. The recorded Codex in-app Chromium/macOS/Apple-GPU entry is one exact
repository-qualified lane, not installed-Chrome, Safari, mobile, broad support,
or independent-adopter evidence.

## v0.20 packed integration-baseline lane

The completed v0.20 lane builds the local tarballs, installs only supported
entries into the clean TypeScript quickstart, and exercises cancellation,
load, all five modes, both projections, navigation, provisional pick,
presentation highlight, exact immutable-record confirmation, recovery,
pause/resume, and disposal. Its baseline verifier binds package, fixture,
generated-scene, presentation, recovery, quickstart, and exact matrix facts.
Those v0.20 records remain immutable historical evidence after package-facing
documents move to v0.21.

## v0.21 visual-quality baseline verification target

The static lane verifies the closed corpus manifest, deterministic generated
scene and Autzen derivative, CC BY 4.0 permission/attribution, trial cameras and
features, exact 320 by 240 CSS / DPR 2 / 640 by 480 physical viewport, capture
layout, lossless PNG and deterministic USTAR codecs, baseline-input schema,
tolerance caps, independent resource and transport ceilings, fixed URL-derived
verify provenance, post-capture rubric schema, and closed nonclaims. The
derivative command compares
regenerated bytes unless explicitly invoked in its separate write mode:

~~~bash
cargo run -p browser-demo --bin generate_visual_source_fixture
node --test apps/browser-demo/web/visual-*.test.mjs \
  apps/browser-demo/web/range-server.test.mjs \
  scripts/verify-browser-visual-baseline.test.mjs
node scripts/verify-browser-visual-baseline.mjs
~~~

The attended lane is a mandatory sequential record-then-verify workflow. Each
stage runs all nine fixed trials through three complete viewer/harness
recreations. Each capture follows 30 unchanged foreground frames. The record
stage creates the nine canonical PNGs and commit-free baseline-input manifest;
after those inputs are checked in, the implementation is pinned and rebuilt.
The inherited packed quickstart and browser qualification then pass before the
verify stage compares three complete recreations per trial against those
checked-in baselines. Only verify-mode evidence is eligible for final
acceptance:

~~~bash
scripts/build-browser-sdk.sh
python3 scripts/serve-browser-demo.py --port 8000
# First open /visual.html?mode=record and retain only baseline PNGs plus
# baseline-inputs.json from its TAR bundle. Pin and rebuild that implementation.
# Then open /visual.html?mode=verify with implementation_commit,
# verifier_byte_length, and verifier_sha256 for final comparison.
node scripts/verify-browser-visual-baseline.mjs \
  --evidence docs/releases/v0.21-browser-visual-evidence.json
~~~

The page must stay visible at DPR 2 and 100% zoom. In each stage the maintainer
runs the three-recreation corpus first, waits for every exact rubric image to
load after capture, then confirms the bounded session label, records all six
outcomes, and submits the post-capture review. Downloads unlock only after
`document.body.dataset.visualBaseline` becomes `passed`. One uncompressed USTAR
bundle transports the evidence JSON, baseline-input manifest, and PNGs at their
repository-relative paths; the bundle is not itself an evidence artifact.
The standard path is one Blob download. Local tests also prove that the
separately enabled fallback accepts only the same-origin bounded archive,
exact `application/x-tar` media type, and positive decimal body length no
greater than 1,243,611,136 bytes; rejects non-loopback or mismatched
authorities, cross-origin POSTs, zero/missing/invalid lengths, oversize bodies,
and an existing target; streams through a bounded exclusive
`.part`; and fsyncs then publishes the fixed archive below its declared export
directory without replacement. The HTTP 201
`punctra-browser-visual-export-receipt-v1` response binds the filename, absolute
path, byte length, and SHA-256, but is not evidence.

Verify mode fixes the attended lane to
`codex-iab-chromium-151-macos-26-apple-m5-pro` with visible-user-gesture and
exact-observed-lane-only semantics. Its visible Run control is disabled until
the URL supplies the exact 40-hex implementation commit, positive decimal
verifier byte length, and 64-hex verifier SHA-256. Server transport cannot
supply or weaken those pins.

Elapsed-to-submitted-work-done and elapsed-to-readback-map callback intervals
start at the begin-capture monotonic origin. They include callback/scheduling
delay, do not establish callback ordering, and are not physical GPU-completion
time. The verifier independently bounds them and preserves that nonclaim.

Until that evidence file, accepted images, maintainer-labelled rubric result,
implementation/verifier pins, and release record exist and pass, the target is
implemented work rather than repository-verified v0.21 completion. Offscreen
readback is renderer evidence; it does not observe the OS compositor or panel.

## Local verification lanes

### Change qualification

Run the authoritative sequence directly from
[CONTRIBUTING.md](../../CONTRIBUTING.md). That file is the single maintained
command list; this architecture guide does not duplicate it. Required local GPU
lanes use `PUNCTRA_REQUIRE_GPU=1`, including renderer appearance, corpus,
offscreen, planner, display-mapping, and public-host acceptance.

The v0.15–v0.21 browser lane is separate from native GPU acceptance. It uses
the build and strict local-Range-host steps in `CONTRIBUTING.md`, requires the
document itself to publish `PASS`, and records exact browser/adapter facts in
the bounded v0.19 matrix rather than inferring support for an unexecuted entry.

Opt-in larger generated Workspace runs use:

~~~bash
PUNCTRA_POINT_WORKSPACE_BENCH_POINTS=10000000 \
  cargo bench -p point-workspace --bench document
PUNCTRA_TERRAIN_BENCH_POINTS=100000 \
  cargo bench -p point-terrain --bench terrain
PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS=100000 \
  cargo bench -p terrain-demo --bench journal -- \
  --save-baseline "qualification-$$-$(date +%s)"
PUNCTRA_RENDERER_VIEW_BENCH_POINTS=1000000 \
  cargo bench -p renderer-demo --bench viewing
~~~

The renderer viewing benchmark defaults to 100,000 generated Points and
accepts a positive value through ten million. It measures warm verified
position-only index open and the first bounded root display batch and prints
generated Point/node counts, artifact bytes, and observed index-temporary
bytes. It does not exercise a GPU, production corpus, or human workflow.

### v0.10 working-tree verification record

On 2026-08-13, the complete local sequence above ran from the `codex/v0.10`
working tree based on `3dc4cb1`, immediately before that exact implementation
tree was committed for review. The reference environment was an Apple M5 Pro
(`Mac17,9`), 24 GiB, arm64, macOS 26.5.2, APFS, Rust 1.90.0, and Cargo 1.90.0.
Required GPU acceptance used the built-in 16-core Apple M5 Pro through Metal 4
with `PUNCTRA_REQUIRE_GPU=1`; the corpus, planner, display, headless, and
`render-wgpu` offscreen paths all passed.

Formatting, workspace Clippy with `-D warnings`, workspace tests, rustdoc with
`-D warnings`, fuzz formatting/check/tests, documented examples, focused
package/process/golden tests, guide/JSON checks, and `git diff --check` passed.
All eight default benchmark commands exited successfully and all declared
heap/resource thresholds passed. Criterion's saved-baseline comparison was not
performance-neutral: it classified 14 cases as statistically regressed.

| Benchmark | Final local observation |
|---|---|
| `point-view` planner | Perspective 34.882–37.250 ms, +13.470–22.945%; orthographic 31.984–32.753 ms, +4.7042–7.2225% |
| `source-memory` read | 468.95–482.84 us, +3.1392–5.0059% |
| `source-las` read | LAS Points 20.802–21.289 ms within noise; five other cases +13.499–94.755%; heap facts 372,406 B LAS / 2,588,206 B LAZ under 33,554,432 B |
| `point-index` | Cold 428.96–530.05 ms, +23.264–54.250%; warm 49.854–64.286 ms, +109.85–170.05%; candidates/root regressed; leaf unchanged; heap 262,616 B under 33,554,432 B |
| `point-workspace` | Zero regressions; three resident cases improved; 0% and forced spill unchanged; resource evidence passed |
| `point-terrain` | Derivation and LandXML unchanged; QA improved; resource ceilings passed |
| `terrain-demo` | Retryable-intent 136.82–168.89 ms, +2.6833–30.264%; four other modes unchanged |
| `renderer-demo` | Warm open 1.9206–1.9972 ms, +6.0963–9.5577%; first batch unchanged |

Those percentages are Criterion comparisons against the pre-existing local
saved baseline, not a universal latency promise or a design pass/fail
threshold. They are recorded rather than hidden. This pre-commit working-tree
sweep did not by itself satisfy v0.9's stricter **one-commit** candidate-record
gate. That gate was closed later by the
[v0.9 release verification](../releases/v0.9.0.md); the historical v0.10 sweep
likewise contains no licensed production-corpus, downstream, partner,
usability, support, or publication evidence.

An independent two-axis review of `3dc4cb1` through this working tree completed
on 2026-08-13 with zero P0–P3 findings on both Standards and Spec. That closes
the review gate only; it does not turn the working-tree verification above into
a release candidate or close any external evidence gate.

### v0.11 exact-commit verification record

On 2026-08-16, the complete authoritative sequence above passed from the final
PR tree containing this record. That tree includes review fixes which close the
complete interactive session after an indeterminate mutation and verify that a
committed immediate-head Revert has an exact inverse Revision Audit before the
host reports success. The sweep used the reference environment below, required
the expected local GPU with `PUNCTRA_REQUIRE_GPU=1`, and passed every formatting,
Clippy, test, rustdoc, fuzz, benchmark, example, focused, guide, JSON, GPU, and
diff-check lane. No hosted CI was used.

The earlier implementation qualification is retained below as historical
evidence for its exact commit; it does not qualify later PR changes by itself.

On 2026-08-13, the complete authoritative sequence above passed from exact
implementation commit `f2eaadb`, based directly on canonical v0.10 commit
`30ea9ff`. The reference environment was an Apple M5 Pro (`Mac17,9`), 24 GiB,
arm64, macOS 26.5.2, APFS, Rust 1.90.0, and Cargo 1.90.0. Required GPU
acceptance used the built-in 16-core Apple M5 Pro through Metal 4 with
`PUNCTRA_REQUIRE_GPU=1`; renderer offscreen, planner, display, corpus, and the
public `third_party_host` pick example all passed.

Formatting, workspace Clippy with warnings denied, all-feature workspace tests,
rustdoc with warnings denied, fuzz formatting/check/tests, every documented
example and focused package/process/golden lane, guide and JSON validation, and
`git diff --check` passed. All nine default benchmark commands exited
successfully. The exact-review benchmark retained and fully traversed both
resident and forced-spill Point Sets before timing. Its final local generated
facts were:

| Exact-review evidence | Resident | Forced spill |
|---|---:|---:|
| 20,000-Point scan Criterion interval | 1.5851–1.6603 ms | 10.569–11.828 ms |
| Exact matches | 5,151 | 5,151 |
| Algorithm-accounted working high-water | 134,709,248 B | 134,709,248 B |
| Composition ceiling | 268,435,456 B | 268,435,456 B |
| Point Set resident ceiling | 67,108,864 B | 0 B |
| Point Set temporary ceiling | 4,294,967,296 B | 4,294,967,296 B |
| Stable owned-fixture file delta while retained | 0 files / 0 B logical length | 1 file / 46,550 B logical length |

The working high-water is conservative algorithm accounting, not measured
heap. File evidence is a stable recursive count and sum of logical file lengths
inside the benchmark-owned temporary root, not allocated filesystem blocks or
process-wide disk use. Criterion intervals are local generated-fixture
microbenchmarks, not production latency or interactive-response promises.

A 2026-08-14 follow-up investigated the terrain-workflow labels emitted by an
unnamed historical Criterion baseline. Canonical v0.10 `30ea9ff`, v0.11
`f2eaadb`, and the same unchanged v0.10 binary ran A/B/A through one shared
target on the reference machine. The estimates in milliseconds were:

| Durable workflow mode | v0.10 A1 | v0.11 B | v0.10 A2 self-check |
|---|---:|---:|---:|
| Cold start | 142.25 | 148.93 | 143.29 |
| Resume after committed Edit | 108.98 | 125.58 | 135.13 |
| Resume from Retryable intent | 146.84 | 133.63 | 163.67 |
| LandXML/report reconciliation | 92.329 | 107.21 | 130.10 |
| Complete revalidation | 79.576 | 96.452 | 139.16 |

The unchanged base self-check moved by up to 74.9% and was slower than the
v0.11 run in four modes. The original labels therefore cannot be attributed to
v0.11 code; these wall times intentionally include workstation/APFS durable
filesystem synchronization. Qualification now saves each run under a unique
baseline name, retaining its sampled intervals and resource facts without
loading comparison history. Any cross-Revision performance claim requires a
deliberate named same-machine, same-target A/B/A comparison whose unchanged
base self-check remains stable.

Independent Standards/Spec review after the final cancellation, boundary,
highlight, durability, and benchmark-evidence fixes found no remaining P0–P2
blocker. This closes only v0.11's repository technical slice. Field activation,
licensed workflow observation, professional time/rework evidence, independent
adoption, partner validation, v0.9's inherited candidate record, and broader
support qualification remain outstanding.

### v0.12 local verification record

On 2026-08-18, the complete authoritative local sequence passed against exact
local implementation commit `9b09380e0899c4d41b9c43cd075822429ed89616` on
`codex/explicit-spatial-reference-v0.12`. This record was added afterward as a
documentation-only successor, not used as a moving qualification target. The
reference environment was an Apple M5 Pro (`Mac17,9`), 24 GiB, arm64, macOS
26.5.2 on APFS, Rust 1.90.0, and Cargo 1.90.0. Required GPU acceptance used the
built-in 16-core Apple M5 Pro through Metal 4 with `PUNCTRA_REQUIRE_GPU=1`;
offscreen rendering, planner, display, corpus, and the public
`third_party_host` example passed. No hosted CI was used.

Formatting, workspace Clippy with warnings denied, all-feature workspace tests,
rustdoc with warnings denied, fuzz formatting/check/tests, documented examples,
focused package/process/golden lanes, guide and JSON checks, and
`git diff --check` passed. The frozen persisted-v1 Source Record, Spatial Index,
Workspace, Workflow Run, report, LandXML, and Round-Trip Evidence fixtures
reproduced their existing bytes and semantics.

The spatial-reference additions passed bounded value/wire/canonical-byte tests;
complete, missing, duplicate, indirect, unsupported-unit, user-defined,
unsupported-version, malformed, and WKT-conflicting GeoTIFF fixtures; exact
Source Record and Workspace reopen tests; fail-closed Workspace and Terrain
reference hashing; public Terrain metre/metre enforcement before row
consumption; structured Terrain/QA/LandXML success/rejection tests; and
DOM/streaming reference classification and comparison before numeric
tolerances. No untracked example file was assigned an inferred reference.

The package gate inspected metadata and archive content for all twelve public
libraries, rejected publishability for both applications, and built every
extracted package with exact `0.12.0-alpha.1` inter-package requirements. The
docs.rs-equivalent all-feature rustdoc lane passed with warnings denied. No
package was uploaded.

All nine default benchmark commands exited successfully and their declared
resource ceilings passed. The durable workflow run used the unique local
baseline `qualification-v012-9b09380-20260818`. Other Criterion commands may
display comparisons against workstation-local historical baselines; those
labels are not a same-machine A/B/A attribution and no v0.12 performance claim
is made from them.

This closes only the bounded v0.12 repository technical and local packaging
slice. The production-corpus activation gate, independent reference-coordinate
checks, downstream application observation, crates.io publication, independent
adoption, partner acceptance, and support qualification remain outstanding.

### Release qualification

In addition to the full local sequence:

- run generated LAS and LAZ process paths;
- retain the named machine/toolchain with benchmark output;
- inspect every public doc link and example signature;
- validate `docs/guides/field-corpus.example.json` as JSON without treating its
  placeholders as measured evidence;
- review persisted-format and fault-injection coverage;
- set `PUNCTRA_REQUIRE_GPU=1` on the expected local adapter; and
- record external evidence as outstanding unless it was actually obtained.

The authoritative v0.9 outcomes, environment, and Criterion observations are
recorded in the [repository verification record](../releases/v0.9.0.md).

## Definition of verified

A slice is verified only when public behavior, corruption and interruption,
hard limits, examples, docs, benchmarks, and applicable local GPU gates agree
with its accepted design. Passing generated tests does not prove customer fit,
licensed-data behavior, or interoperability with a downstream product.
