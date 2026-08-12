# Contributing

Punctra is intentionally a small set of independently usable modules. Before
adding a new public seam, check it against the accepted
[v0.1 renderer scope](docs/design/render-engine-v0.1.md) and
[v0.2 planning scope](docs/design/adaptive-view-planning-v0.2.md), plus the
completed [v0.3 Real Sources scope](docs/design/real-sources-v0.3.md) and
completed [v0.4 Out-of-core View scope](docs/design/out-of-core-view-v0.4.md).
The completed [v0.5 Durable document core
scope](docs/design/durable-document-core-v0.5.md) permits one deep
`point-workspace` crate for exact classification selection, temporary Point
Sets, sparse classification Revisions, and Operation recovery. The completed
[v0.6 Terrain and QA benchmark
scope](docs/design/terrain-qa-benchmark-v0.6.md) additionally permits the exact
`Snapshot::point_rows` stream, one deep `point-terrain` crate, and the private
headless `terrain-demo` composition. The completed [v0.7 technical-readiness
scope](docs/design/technical-alpha-readiness-v0.7.md) permits linked child
cancellation, exact Revision Audit and Edit Footprint facts, exact LandXML
ensure/reconciliation, and the private durable `terrain-demo` Workflow Run,
canonical report, and structured recovery diagnostics. Format decoding belongs
only in accepted Source adapter crates. Networking, screen selection, general
editing, constrained or persistent terrain, general export, Source rewriting,
and general host UI remain in callers or future projects unless the scope is
explicitly revised.

## Local verification

Install the pinned Rust toolchain and run:

```bash
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
cargo run -p source-memory --example memory_source
cargo run -p point-index --example direct_use
cargo test -p point-workspace --all-features
cargo run -p point-terrain --example derive
cargo test -p point-terrain --all-features
cargo test -p terrain-demo --test workflow
cargo test -p terrain-demo --test process
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
```

The default `point-index` benchmark generates one million Points. Use only the
documented scale values when a larger local run is intended, for example:

```bash
PUNCTRA_POINT_INDEX_BENCH_POINTS=10000000 cargo bench -p point-index --bench index
PUNCTRA_POINT_WORKSPACE_BENCH_POINTS=10000000 \
  cargo bench -p point-workspace --bench document
PUNCTRA_TERRAIN_BENCH_POINTS=100000 \
  cargo bench -p point-terrain --bench terrain
PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS=100000 \
  cargo bench -p terrain-demo --bench journal
```

The terrain benchmark accepts positive generated sizes through one million
Points; its intended scales are 10,000, 100,000, and 1,000,000. The default is
10,000. Its descriptor and QA byte facts are algorithm accounting. A null
`worker_heap_measurement` means that no observed worker-heap measurement is
claimed.

The Workflow benchmark accepts exactly 10,000, 100,000, or 1,000,000 generated
Points. It measures cold start, committed-Edit resume, Retryable-intent resume,
LandXML/report reconciliation, and Complete revalidation. Its journal/report
bytes and semantic limit facts are deterministic generated evidence. It does
not measure worker peak heap or establish production, partner, downstream, or
human-workflow acceptance.

Exercise the complete real-cloud process path without requiring a GPU:

```bash
cargo run -p source-las --example inspect -- path/to/source.laz
cargo run --release -p renderer-demo -- --smoke path/to/source.laz path/to/source.pidx
cargo run --release -p point-workspace --example classify -- \
  path/to/source.laz path/to/source.pidx path/to/workspace.pcw \
  6
cargo run --release -p terrain-demo -- start \
  --run-id "$RUN_ID_HEX" --operation-id "$OPERATION_ID_HEX" \
  --baseline "$BASELINE_REVISION_HEX" --exclude-ground-ordinal 4 \
  --date 2026-08-10 --time 00:00:00Z \
  --assert-unknown-crs-metric \
  path/to/source.laz path/to/source.pidx path/to/workspace.pcw path/to/run-root
cargo run --release -p terrain-demo -- inspect path/to/run-root
```

`RUN_ID_HEX` and `OPERATION_ID_HEX` are caller-owned nonzero 32-character hex
identities; `BASELINE_REVISION_HEX` is the exact expected 64-character
Workspace head identity. Create the Workspace separately through
`point-workspace`; its classification example demonstrates setup and prints
Revision identities. Its selected `U8` Attribute must be Source Attribute 6,
the `source-las` classification column. `terrain-demo` opens but never creates
it, and an absent Workspace is `PWF_INVALID_REQUEST` before Run creation or
Workspace mutation. Retain the current head identity, then drop all
Workspace/Snapshot/PointSet handles before starting so the app can acquire the
exclusive Workspace lock.
`path/to/run-root` must already be a directory. Resume repeats the identical
command and request with `resume` in place of `start`. At least one repeated
`--exclude-ground-ordinal ORDINAL` is required, and every listed ordinal must
be in the baseline class-2 Ground Input. Optional detached observations use
repeated `--check-point ID,X,Y,Z` arguments.

`terrain-demo` requires metric-metre coordinates and performs no
transformation. The caller must pass `--assert-unknown-crs-metric` only when
that unit assertion is independently known to be true; the application cannot
infer units from the opaque Coordinate Reference.

The durable v0.7 command replaces the v0.6 one-shot
`--exercise-correction-revert` grammar. It still validates the correction
ordinals against the baseline class-2 Ground Input, commits the requested
non-Ground classification, and derives both baseline and changed Surfaces.
The retained v0.6 regressions prove exact immediate-head Revert restoration of
geometry, topology, vertices, and faces and keep the Source bytes unchanged.
The v0.7 Workflow does not automatically Revert a committed classification
Revision when a later Derivation, QA, export, or report phase fails.

GPU acceptance tests use any available headless wgpu adapter. They skip when no
adapter is present unless `PUNCTRA_REQUIRE_GPU=1`. Run all verification locally;
the repository does not use hosted CI.

The stable fuzz-crate test runs the checked-in short corpus through the same
bounded harness as libFuzzer. Longer local campaigns may use `cargo-fuzz` and a
nightly toolchain:

```bash
cargo +nightly fuzz run index_persistence fuzz/corpus/index_persistence -- \
    -max_len=262144 -timeout=5
```

Keep public behavior documented, add interface-level tests for changes, avoid
unsafe code, and preserve caller-owned wgpu submission.
