# Verification Strategy

Status: implemented verification through v0.5; all gates run locally

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
5. **Composition tests** exercise Source-to-index, index-to-View, and
   planner-to-renderer behavior without private cross-crate access.
6. **Benchmarks and resource gates** measure Source-scale time, heap,
   temporary bytes, durable growth, and GPU residency.
7. **Required local GPU acceptance** proves the wgpu path when an adapter is
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
- Compare semantic values before exact bytes unless deterministic bytes are a
  stated contract.
- Never commit licensed production data without redistribution rights.

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

### point-index

- candidate plans have no false negatives against a sequential exact oracle;
- candidate spans are sorted, disjoint, and bounded;
- build/resume produces deterministic descriptor and artifact bytes;
- valid-prefix recovery resumes at the exact durable boundary;
- incompatible, corrupt, truncated, over-limit, and racing targets fail
  without replacement;
- display samples and exact leaves preserve Source-aware identities/ticks;
- LAZ fixed-chunk seeks cross chunk boundaries exactly; and
- process smoke covers Full verification, Built then Opened index paths, and
  one complete CPU-model renderer Upsert.

### point-workspace

The v0.5 package has 61 tests: 19 integration tests through the public
interface and 42 unit, fault-injection, and allocation gates. The public suites
prove:

- create, root identity, exclusive lock, complete-handle lifetime, and reopen;
- schema rejection when the chosen classification Attribute is absent or not
  `U8`;
- All, inclusive world-box, optional effective-classification, and explicit
  Point-ID selection against a brute-force Source-plus-overlay oracle;
- 4,099 seeded randomized Points across 32 box/classification Queries and
  varied Source batching;
- Point-ID Source validation, bounds, sorting, and deduplication;
- identical resident/forced-spill membership, order, hashes, repeated reads,
  corruption detection, and final-handle cleanup;
- mixed before-values, sparse classification rows, no-op rejection, immutable
  historical Snapshots, immediate-head Revert, and redo-by-Revert;
- Point Identity through Source, index, Point Set, commit, Revert, and reopen;
- byte-for-byte unchanged generated LAS/LAZ Source files and unchanged
  non-classification values;
- prepublication failure for selection, ID-read, open, and commit hard limits;
- committed, rejected, retryable, not-recorded, conflict, and indeterminate
  Operation reconciliation;
- idempotent retry with at most one Revision; and
- fail-closed corruption plus recoverable recognized scratch.

Private persistence tests inject error, cancellation, panic, and lost
acknowledgement at candidate stage/file-sync/close/read-only/revalidation,
ready link and operations-directory sync, Revision link and directory sync,
rejection stage/file-sync/read-only/revalidation/link/directory-sync/cleanup,
and recovery sync boundaries. Reopen exposes only a complete old head or a
complete new head; it never fabricates a partial Revision.

The selection unit gate measures worker-equivalent allocation around the same
private worker function used by the Job. Its observed peak was 6,292,224 bytes
under a 64 MiB ceiling, with zero retained measured allocations.

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
- host-owned submission and polling; and
- planner-to-renderer parent Coverage retirement after replacements render.

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

## Local verification lanes

### Change qualification

Run the authoritative sequence from [CONTRIBUTING.md](../../CONTRIBUTING.md):

~~~bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps

cargo bench -p point-view --bench planner
cargo bench -p source-memory --bench read
cargo bench -p source-las --bench read
cargo bench -p point-index --bench index
cargo bench -p point-workspace --bench document

cargo run -p source-memory --example memory_source
cargo run -p point-index --example direct_use
cargo run --release -p point-workspace --example classify -- \
  survey.laz survey.laz.pidx survey.pcw CLASSIFICATION_ATTRIBUTE_ID

cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
~~~

Opt-in larger generated Workspace runs use:

~~~bash
PUNCTRA_POINT_WORKSPACE_BENCH_POINTS=10000000 \
  cargo bench -p point-workspace --bench document
~~~

### Release qualification

In addition to the full local sequence:

- run generated LAS and LAZ process paths;
- retain the named machine/toolchain with benchmark output;
- inspect every public doc link and example signature;
- review persisted-format and fault-injection coverage;
- set `PUNCTRA_REQUIRE_GPU=1` on the expected local adapter; and
- record external evidence as outstanding unless it was actually obtained.

## Definition of verified

A slice is verified only when public behavior, corruption and interruption,
hard limits, examples, docs, benchmarks, and applicable local GPU gates agree
with its accepted design. Passing generated tests does not prove customer fit,
licensed-data behavior, or interoperability with a downstream product.
