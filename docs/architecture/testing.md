# Verification Strategy

Status: **Complete for the v0.9 repository trust and version-1 compatibility
candidate; all gates run locally**

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
8. **Required local GPU acceptance** proves the wgpu path when an adapter is
   expected.

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
- Compare semantic values before exact bytes unless deterministic bytes are a
  stated contract.
- Never commit licensed production data without redistribution rights.
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
canonical report was 11,490 bytes and contained 115 semantic limit facts.

The intervals are local generated observations, not universal latency claims.
Worker peak heap was not measured. No partner, production, downstream round-
trip, paid-use, or human-time acceptance is inferred from this benchmark.

## Local verification lanes

### Change qualification

Run the authoritative sequence from [CONTRIBUTING.md](../../CONTRIBUTING.md):

~~~bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo fmt --manifest-path fuzz/Cargo.toml --all --check
cargo check --manifest-path fuzz/Cargo.toml --bin index_persistence
cargo test --manifest-path fuzz/Cargo.toml --lib
cargo bench -p point-view --bench planner
cargo bench -p source-memory --bench read
cargo bench -p source-las --bench read
cargo bench -p point-index --bench index
cargo bench -p point-workspace --bench document
cargo bench -p point-terrain --bench terrain
cargo bench -p terrain-demo --bench journal
cargo bench -p renderer-demo --bench viewing

cargo run -p source-memory --example memory_source
cargo run -p point-index --example direct_use
cargo test -p point-workspace --all-features
cargo run -p point-terrain --example derive
cargo test -p point-terrain --all-features
cargo test -p terrain-demo --test workflow
cargo test -p terrain-demo --test process
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test headless_smoke \
  corpus_success_binds_trace_inputs_and_separate_resource_measurements -- --exact
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test display_gpu
test -f docs/guides/first-las-laz.md
jq -e . docs/guides/field-corpus.example.json >/dev/null
git diff --check
~~~

Opt-in larger generated Workspace runs use:

~~~bash
PUNCTRA_POINT_WORKSPACE_BENCH_POINTS=10000000 \
  cargo bench -p point-workspace --bench document
PUNCTRA_TERRAIN_BENCH_POINTS=100000 \
  cargo bench -p point-terrain --bench terrain
PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS=100000 \
  cargo bench -p terrain-demo --bench journal
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
