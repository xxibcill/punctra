# Punctra

Punctra is an embeddable Rust and wgpu point-cloud rendering engine. It is for
applications that need progressive display of very large point sets, precise
large-world coordinates, bounded logical point residency, generation-safe
streaming updates, and point picking without adopting a complete editor or data
model.

Version 0.2.0 adds the adaptive planning scope and verification gates recorded
in [the v0.2 design](docs/design/adaptive-view-planning-v0.2.md) on top of
[the v0.1 renderer](docs/design/render-engine-v0.1.md).

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

- `render-protocol` defines and validates renderer-neutral View updates.
- `point-view` plans deterministic, budgeted hierarchy requests and retirement.
- `render-wgpu` owns GPU resources, pipelines, drawing, and picking.
- `renderer-demo` exercises the engine with generated point batches.

File decoding, index construction and persistence, networking, editing, terrain
construction, and application UI remain outside the workspace scope.

## Demo

Run the deterministic adaptive-LOD demo with:

```bash
cargo run --release -p renderer-demo
```

- Left-drag orbits and the mouse wheel zooms.
- `R` resets the camera, `H` toggles stable-ID highlights, and Space pauses or
  resumes node materialization while planning and safe retirement continue.
- Escape exits.

The window title reports resident points and bytes, cumulative upload bytes,
draw calls, FPS, frame time, encoding time, upload time, planner requests, and
streaming progress. Its hierarchy represents more than 10 million logical
Points while renderer residency stays at fixed point, byte, and batch limits.

## Development

Install the pinned Rust toolchain, then run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo bench -p point-view --bench planner
cargo run -p renderer-demo
```

Punctra currently targets Rust 1.90 and wgpu 30. The demo requires a graphics
adapter supported by wgpu; renderer-neutral protocol tests do not.

GPU-backed tests are separated from renderer-neutral contract tests so the
protocol remains testable on machines without a graphics adapter. Set
`PUNCTRA_REQUIRE_GPU=1` to make a missing headless adapter a local test failure.
All repository verification is run locally; no hosted CI workflow is configured.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)); or
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
