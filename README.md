# Punctra

Punctra is an embeddable Rust point-cloud foundation with verified Source
access, adaptive View planning, and wgpu rendering. It is for applications that
need authoritative Point reads, progressive display of very large point sets,
precise large-world coordinates, bounded logical residency, generation-safe
streaming updates, and Point picking without adopting a complete editor or
document model.

Version 0.2.0 adds the adaptive planning scope and verification gates recorded
in [the v0.2 design](docs/design/adaptive-view-planning-v0.2.md) on top of
[the v0.1 renderer](docs/design/render-engine-v0.1.md).

Version 0.3.0 completes the accepted
[Real Sources design](docs/design/real-sources-v0.3.md): canonical Source and
Point contracts, runtime-neutral bounded work, one verified Source seam, and
interchangeable in-memory, LAS, and LAZ adapters. File reads preserve exact
position ticks, supported Attributes, ordered VLR/EVLR metadata, and stable
`(SourceId, ordinal)` Point Identity under explicit batch and decoder limits.
`source-las` supports LAS point-data record formats 0–10 and LAZ formats 0–8.
LAZ formats 9 and 10 are rejected explicitly until the layered WavePacket14
codec can preserve waveform values exactly.

Version 0.4.0 completes the accepted
[Out-of-core View design](docs/design/out-of-core-view-v0.4.md): a rebuildable,
resumable persistent Spatial Index, conservative Source-span lookup, bounded
display-only hierarchy samples, efficient fixed-chunk LAZ seeks, and a private
host-owned path from a verified real Source through View planning to atomic
renderer updates. It does not introduce Workspace or exact Query semantics.

Version 0.5.0 completes the accepted
[Durable document core design](docs/design/durable-document-core-v0.5.md). The
narrow slice adds one deep, headless `point-workspace` crate for exact All,
world-box, and explicit-Point-ID classification selections; bounded temporary
Point Sets; immutable classification Revisions; immediate-head Revert Edits;
and durable Operation reconciliation. Source bytes remain immutable.
Screen-through selection, general edits, named Point Sets, terrain, and Source
rewriting remain outside this scope.

Version 0.6.0 completes the implemented technical slice in the accepted
[Terrain and QA benchmark design](docs/design/terrain-qa-benchmark-v0.6.md).
It adds one exact classification-aware `Snapshot::point_rows` stream, one deep
`point-terrain` crate for a deterministic single-worker unconstrained in-memory
TIN, detached Check Point residual QA, a private durable create-new
metric-metre LandXML 1.2 points/faces encoder, and one headless `terrain-demo`
caller. It does not add Breaklines, Profiles, a classifier, terrain
persistence, general LandXML, or coordinate transformation. This repository
completion is not a claim of product, partner, licensed-data, or downstream-
application acceptance.

Later direction is described in the [living roadmap](ROADMAP.md). Its release
themes are adjustable and do not expand accepted implementation scope by
themselves.

## Embedding model

The host owns its wgpu device, queue, command encoder, target texture, data
loading, and View policy. Punctra owns validated resident state and rendering:

```rust,ignore
let limits = RenderLimits::new(512 * 1024 * 1024, 20_000_000, 4096);
let config = RendererConfig::new(surface_format, limits);
let mut renderer = WgpuRenderer::new(&device, config)?;

renderer.apply(&RenderUpdate::Reset { view_generation })?;
renderer.apply(&RenderUpdate::Upsert { batch })?;

let frame = Frame::new(view_generation, camera, [width, height])?;
let recorded_frame = renderer.render(&mut encoder, &target, &frame)?;
let report = recorded_frame.report();
queue.submit([encoder.finish()]);
```

Point positions use a finite 64-bit world origin plus finite 32-bit relative
coordinates. Upserts replace complete batches atomically. Stale View
generations and non-increasing batch versions are rejected before publication.
The byte limit covers the fixed 24-byte GPU point vertices; per-batch uniforms,
render targets, allocator padding, and transient command uploads are outside
that logical residency model.

Frame uniform uploads are part of the recorded command stream. A host may
record several Punctra frames before one submission without later camera values
changing earlier frames. Rendering returns a `RecordedFrame`; pass that exact
value back when encoding a `PickRequest` so the pick uses the batches and
identity metadata displayed by that render. Retaining a `RecordedFrame` pins
any replaced GPU resources it references until the value is dropped. Picking
otherwise follows the same host-owned flow: submit the encoder, drive normal
wgpu device polling, and poll the returned `PickTicket` without blocking.

Adaptive View planning is a separate renderer-neutral step. The host reports a
generation-stamped hierarchy snapshot, including which nodes are missing,
requested, or resident. `point-view` frustum-culls it, selects LOD by
screen-space error, reserves point/byte/batch costs, and returns prioritized
requests, required retention, and exact conditional retirements. It performs
no I/O and never mutates renderer state.

## Workspace

- `point-contracts` defines lossless Source, Point, Attribute, coordinate, and
  provenance values.
- `foundation-runtime` provides runtime-neutral Jobs, progress, cancellation,
  and bounded pull-stream control.
- `point-source` verifies immutable Sources and exposes normalized bounded
  reads through one caller-facing interface.
- `source-memory` supplies deterministic in-memory Sources; its opt-in
  `test-support` feature adds conformance faults.
- `source-las` opens local LAS formats 0–10 and LAZ formats 0–8 through the same
  verified, bounded Source interface; LAZ formats 9 and 10 are explicitly
  unsupported pending exact WavePacket14 codec support.
- `point-index` prepares one deterministic checksummed fixed-block BVH, returns
  conservative Source spans, and streams bounded display samples or complete
  Source-backed leaves.
- `point-workspace` owns exact revision-pinned classification selection,
  process-scoped spillable Point Sets, immutable sparse classification
  Revisions, immediate-head Revert, Operation-ID recovery, and the narrow exact
  classification-aware `Snapshot::point_rows` pull stream behind one deep
  caller interface.
- `point-terrain` derives one immutable `TerrainSurface` with canonical
  `SurfaceVertex` and `SurfaceFace` values, evaluates detached Check Points,
  and durably creates the supported LandXML 1.2 subset.
- `render-protocol` defines and validates renderer-neutral View updates.
- `point-view` plans deterministic, budgeted hierarchy requests and retirement.
- `render-wgpu` owns GPU resources, pipelines, drawing, and picking.
- `renderer-demo` exercises the engine with either generated point batches or
  one Full-verified indexed LAS/LAZ Source.
- `terrain-demo` exercises the GPU-free LAS/LAZ-to-index-to-Workspace-to-
  terrain-to-QA-to-LandXML composition.

Networking, screen selection, general editing, Source rewriting, persistent or
constrained terrain, general export, and general application UI remain outside
the accepted scope.

## Examples

Exercise the in-memory Source headlessly with the
[in-memory Source example](crates/source-memory/examples/memory_source.rs):

```bash
cargo run -p source-memory --example memory_source
```

Inspect a real LAS or LAZ Source, including its verified identity, schema,
metadata, bounds, and exact read throughput, with the
[file inspection example](crates/source-las/examples/inspect.rs):

```bash
cargo run --release -p source-las --example inspect -- survey.laz
```

Build, query, and read an index directly over an in-memory Source with:

```bash
cargo run -p point-index --example direct_use
```

Run the deterministic adaptive-LOD demo with:

```bash
cargo run --release -p renderer-demo
```

Create a Workspace over one LAS/LAZ Source, select exact class-2 Points,
classify them, append an immediate-head Revert, and reopen the durable result:

```bash
cargo run --release -p point-workspace --example classify -- \
  survey.laz survey.laz.pidx survey.pcw CLASSIFICATION_ATTRIBUTE_ID
```

Run the complete generated in-memory Source-to-LandXML terrain composition:

```bash
cargo run -p point-terrain --example derive
```

Run the headless real LAS/LAZ terrain path. The Source must already use metric
metres; `--assert-unknown-crs-metric` is an explicit caller assertion, not CRS
inference:

```bash
cargo run --release -p terrain-demo -- \
  --date 2026-08-10 --time 00:00:00Z --qa-sample \
  --exercise-correction-revert 4 \
  survey.laz survey.laz.pidx survey.pcw existing-ground.xml
```

The correction/Revert option selects one exact Ground Point by Source ordinal,
sets it to class 1, derives the changed Ground Input, appends an immediate-head
Revert, and requires exact restoration of geometry, topology, vertices, and
faces before export.

Pass a real Source to Full-verify it, build or open its index, and render it.
The optional target defaults to `SOURCE.pidx`:

```bash
cargo run --release -p renderer-demo -- survey.laz survey.laz.pidx
```

The same Source/index/planner/materializer path has a GPU-free process smoke
mode that accepts one atomic CPU-model Upsert:

```bash
cargo run --release -p renderer-demo -- --smoke survey.laz survey.laz.pidx
```

- Left-drag orbits and the mouse wheel zooms.
- `R` resets the camera, `H` toggles stable-ID highlights, and Space pauses or
  resumes node materialization while planning and safe retirement continue.
- Escape exits.

The window title reports resident points and bytes, cumulative upload bytes,
draw calls, FPS, frame time, encoding time, upload time, planner requests, and
streaming progress. The synthetic hierarchy represents more than 10 million
logical Points; both paths keep renderer residency at fixed point, byte, and
batch limits. The real path additionally reports Full verification, index
disposition and reuse, first accepted batch latency, queue depth, and staging
peaks.

## v0.4 benchmark evidence

The checked-in `point-index` Criterion benchmark uses a deterministic
one-million-Point in-memory Source. On the local Apple M5 Pro, 24 GiB,
macOS 26.5.2 reference run with Rust 1.90.0, it produced a 1,971,528-byte
artifact. Median times were 330.515 ms for a cold build, 20.567 ms for a warm
verified open, 606 ns for whole-bounds candidate planning, 1.249 ms for the
4,096-Point internal-root display read, and 122.100 µs for one complete
65,536-Point memory-backed leaf read. The combined candidate/root/leaf
synchronous path peaked at 3,671,504 measured heap bytes under its 32 MiB gate.

These are one-machine generated-fixture baselines, not universal latency claims
and not licensed production-data evidence. Licensed real-cloud and
design-partner runs remain explicitly outstanding.

## v0.5 benchmark evidence

The `point-workspace` acceptance suite has 61 package tests: 19 integration
tests through the public interface and 42 unit, fault-injection, and allocation
gates. They include generated LAS and LAZ selection, commit, Revert, reopen,
Source-immutability, forced-spill, hard-limit, corruption, retry, and injected
persistence-boundary cases.

On the local Apple M5 Pro, 24 GiB, arm64, macOS 26.5.2 reference machine with
Rust 1.90.0, the default generated one-million-Point benchmark completed its
evidence pass and all declared Criterion cases completed locally. A separate
131,073-Point synchronous test
of the same selection worker path measured 6,292,224 bytes of
worker-equivalent peak heap under its 64 MiB gate. The one-million-Point
benchmark does not claim worker heap: its public Point-ID iteration peaked at
2,621,440 measured caller-thread bytes and retained zero, while selection
memory is reported as sampled process RSS. Resident-selection RSS was
62,668,800 bytes. Forced-spill RSS started at 62,685,184 bytes and sampled at
62,832,640 bytes, a 147,456-byte delta; its sealed temporary file was 9,009,182
bytes and was removed with the final Point Set handle.

A sparse 10,000-Point classification/Revert pair took approximately
16.442/15.818 ms and added 20.100 logical bytes per changed Point. A dense
500,000-Point pair took approximately 34.973/35.778 ms and added 20.004 logical
bytes per changed Point. Reopen at Revision depths 2, 4, and 8 took
approximately 1.231, 37.753, and 74.968 ms. The final Workspace contained
40,812,316 logical directory-entry bytes; shared hard links occupied
20,418,560 physical bytes according to `du`.

These values are one-machine generated-fixture evidence, not universal
performance claims and not licensed production-data or design-partner
evidence. Those external evidence gates remain outstanding.

## v0.6 benchmark evidence

The checked-in `point-terrain` benchmark composes a generated in-memory Source,
complete Spatial Index, Workspace Snapshot, Terrain Derivation, detached QA,
and durable LandXML export through public interfaces. On the local Apple M5 Pro
(`Mac17,9`), 24 GiB, arm64, macOS 26.5.2 reference machine with Rust 1.90.0,
the 10,000-Point run measured Derivation at 11.983–12.049 ms
(829.97–834.53 Kpoints/s). Detached QA took 94.907–95.164 us for three Check
Points and 19,604 face tests. Durable LandXML creation took 18.020–18.311 ms
(53.650–54.518 MiB/s) for 1,030,118 bytes.

The emitted evidence record names `jjaes-MacBook-Pro.local` (`macos`,
`aarch64`) and records one-shot Derivation, QA, and LandXML times of 13,371 us,
125 us, and 14,656 us respectively. Those one-shot values are retained
separately from the Criterion intervals. It records 10,000 Ground Input Points,
10,000 vertices, 19,602 faces, and 396 hull vertices.

The descriptor reported 135,790,592 accounted peak working bytes, 1,034,176
retained Surface bytes, and 521,494 topology steps. QA reported 336 accounted
peak working bytes. These are explicit algorithm ledgers. The benchmark's
`worker_heap_measurement` is `null`: no observed worker-heap value is claimed.
The benchmark supports generated 10,000, 100,000, and 1,000,000-Point scales
through `PUNCTRA_TERRAIN_BENCH_POINTS`; only the completed 10,000-Point run is
reported here.

These results are one-machine generated-fixture technical evidence, not
universal performance, licensed-production, above-500-million-Point, partner,
downstream Civil 3D/Bentley, paid-use, or human-time evidence. Every such
external gate remains outstanding.

## Development

Install the pinned Rust toolchain, then run the authoritative local verification
sequence from [CONTRIBUTING.md](CONTRIBUTING.md):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo bench -p point-view --bench planner
cargo bench -p source-memory --bench read
cargo bench -p source-las --bench read
cargo bench -p point-index --bench index
cargo bench -p point-workspace --bench document
cargo bench -p point-terrain --bench terrain
cargo run -p point-index --example direct_use
cargo run --release -p point-workspace --example classify -- \
  survey.laz survey.laz.pidx survey.pcw CLASSIFICATION_ATTRIBUTE_ID
cargo run -p point-terrain --example derive
cargo test -p terrain-demo --test process
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
```

Punctra currently targets Rust 1.90 and wgpu 30. The demo requires a graphics
adapter supported by wgpu; renderer-neutral protocol tests do not.

GPU-backed tests are separated from renderer-neutral contract tests so the
protocol remains testable on machines without a graphics adapter. The required
commands above set `PUNCTRA_REQUIRE_GPU=1` so a missing headless adapter is a
local test failure. All repository verification is run locally; no hosted CI
workflow is configured.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)); or
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
