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

Future direction is described in the [living roadmap](ROADMAP.md). Its release
themes are adjustable and do not expand the accepted implementation scope by
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
- `render-protocol` defines and validates renderer-neutral View updates.
- `point-view` plans deterministic, budgeted hierarchy requests and retirement.
- `render-wgpu` owns GPU resources, pipelines, drawing, and picking.
- `renderer-demo` exercises the engine with either generated point batches or
  one Full-verified indexed LAS/LAZ Source.

Networking, editing, terrain construction, a Workspace, exact Query behavior,
and general application UI remain outside the completed v0.4 scope.

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
cargo run -p point-index --example direct_use
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
