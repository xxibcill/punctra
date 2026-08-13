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
canonical report, and structured recovery diagnostics. The accepted
[v0.8 repository interoperability qualification
scope](docs/design/design-partner-mvp-v0.8.md) implements the private, bounded
`terrain-demo` LandXML comparison reader. Its inherited Run-bound evidence
closure, full-ceiling streaming reader, and generated/golden matrix are now
implemented in v0.10 prerequisite work. Independent Standards/Spec review is
complete with no P0–P3 findings; the complete one-commit local release record
and external product evidence remain outstanding. The incomplete alpha was
inherited by the [v0.9 Trust and v1 Candidate
scope](docs/design/trust-v1-candidate-v0.9.md). That v0.9 scope permits
compatibility fixtures, recovery and failure hardening, support-matrix
documentation, interface review, and reproducible local qualification for the
existing narrow workflow. Its repository matrix is implemented and
independently reviewed, but it remains incomplete pending a complete
one-commit local candidate record; that outstanding gate is a prerequisite of
the accepted [v0.10 Field
Qualification and Professional Inspection View
scope](docs/design/field-inspection-view-v0.10.md). v0.10
permits evidence collection and a staged professional display path while
keeping sampled GPU values non-authoritative. Its five display modes,
perspective/orthographic controls, View-state diagnostics, and corpus runner
stay inside the private `renderer-demo` host. The narrow public point-index
additions are an explicit disk-v2 inspection recipe and an ownership-safe
fresh-preparation policy used by corpus and benchmark cold-build measurements;
the disk-v1 path remains byte-compatible. Apart from the explicit v0.8 reader
exception, external format decoding belongs only in accepted Source adapter
crates.
Networking, screen selection, general editing, constrained or persistent
terrain, general export, Source rewriting, and general host UI remain in
callers or future projects unless the scope is explicitly revised.

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
cargo bench -p renderer-demo --bench viewing
cargo run -p source-memory --example memory_source
cargo run -p point-index --example direct_use
cargo test -p point-workspace --all-features
cargo run -p point-terrain --example derive
cargo test -p point-terrain --all-features
cargo test -p terrain-demo --lib --all-features
cargo test -p terrain-demo --test workflow
cargo test -p terrain-demo --test process
cargo test -p terrain-demo --test run_golden_v1
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test headless_smoke \
  corpus_success_binds_trace_inputs_and_separate_resource_measurements -- --exact
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test display_gpu
test -f docs/guides/first-las-laz.md
jq -e . docs/guides/field-corpus.example.json >/dev/null
git diff --check
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
PUNCTRA_RENDERER_VIEW_BENCH_POINTS=1000000 \
  cargo bench -p renderer-demo --bench viewing
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

The renderer viewing benchmark defaults to 100,000 generated Points and
measures a warm verified position-only index open plus the first bounded root
display batch. `PUNCTRA_RENDERER_VIEW_BENCH_POINTS` accepts a positive size
through ten million. It prints generated Point/node counts plus artifact and
observed index-temporary bytes. This is a local generated-fixture
microbenchmark, not a GPU frame benchmark, production-corpus result, first-use
promise, or professional workflow observation.

Exercise the complete real-cloud process path without requiring a GPU:

```bash
cargo run -p source-las --example inspect -- path/to/source.laz
cargo run --release -p renderer-demo -- --smoke path/to/source.laz path/to/source.pidx
cargo run --release -p renderer-demo -- \
  --smoke --display classification path/to/source.laz \
  path/to/source.inspection-v2.pidx
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
cargo run --release -p terrain-demo -- verify-round-trip \
  --downstream-app "$DECLARED_APP" --downstream-version "$DECLARED_VERSION" \
  --downstream-setting "profile=$DECLARED_SETTINGS" \
  --horizontal-tolerance-metres 0.001 --vertical-tolerance-metres 0.001 \
  path/to/run-root path/to/returned.xml path/to/round-trip-evidence.json
```

Neutral and elevation use the position-only disk-v1 index recipe. RGB,
intensity, and classification use the disk-v2 inspection recipe and can share
one v2 target. Do not point both recipe families at one path: incompatible
complete/work targets are preserved and rejected. RGB requires all three LAS
`U16` channels. The [five-minute first LAS/LAZ
guide](docs/guides/first-las-laz.md) documents display mappings, projection
controls, loading/Coverage truth, and recovery behavior.

The exact private mapping contract is: neutral `[190,205,220,255]`; elevation
normalizes complete Source world Z, clamps to `[0,1]`, uses `0.5` for zero
extent, and interpolates the five stops recorded in the [v0.10
design](docs/design/field-inspection-view-v0.10.md#implemented-display-mappings-and-cli).
Each RGB/intensity `U16` value `v` becomes `(v * 255 + 32767) / 65535` by
integer division. Classification 0–18 uses the checked-in fixed table; 19–255
uses wrapping `u8` `(73c+41, 151c+97, 199c+17)`. Alpha is always 255. These
bytes are presentation only.

The local GPU corpus runner accepts only bounded manifests whose entries state
both inspection and measurement permission:

```bash
cargo run --release -p renderer-demo -- corpus \
  --manifest path/to/private-field-corpus.json \
  --report path/to/new-private-viewing-report.json
```

Start from the [example manifest](docs/guides/field-corpus.example.json), use
opaque identifiers, replace every placeholder, and give each entry a fresh
absent index target. The runner requires a genuine cold build and then records
a separate immediate warm open; existing or resumable indexes are preserved
and rejected. A report target is
published without replacement; use a new target for a new timed run. Reports
are viewing measurements with explicit nonclaims, not field qualification,
terrain capacity, partner acceptance, downstream support, or human-time
evidence. Do not publish Sources, manifests, reports, screenshots, or benchmark
facts without the required permission.

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

`verify-round-trip` is a strictly read-only consumer of a Complete Run and
requires the existing `run.lock`; it never creates or repairs Run files. Its
evidence target must have an existing parent outside the Run root. Caller
labels do not prove application execution, vendor support, partner acceptance,
paid use, conversion, or labor savings.

The verifier streams the exact captured bytes through a bounded local XML
scanner/parser and uses borrowed `quick-xml` token views; it accepts
at most 4 GiB per input, 10,000,000 Points, and 20,000,000 faces. The additional
defaults are 70,000,128 XML nodes, 4 GiB of XML text/attribute bytes, a 4-KiB
lexical token, 8 MiB of accounted parser working bytes per input, 32,000,000
candidate comparisons, and 4 GiB of accounted retained working bytes across
both inputs and comparison. Evidence reports deterministic algorithm charges,
not allocator metadata/slack, process RSS, or measured heap. Oversized tokens,
changed/extended captured files, namespace-stack growth, and every retained
collection projection fail before the corresponding algorithm-accounted
reserve is requested.
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
