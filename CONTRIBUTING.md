# Contributing

Punctra is intentionally a small set of independently usable modules. Before
adding a new public seam, check it against the accepted
[v0.1 renderer scope](docs/design/render-engine-v0.1.md) and
[v0.2 planning scope](docs/design/adaptive-view-planning-v0.2.md), plus the
completed [v0.3 Real Sources scope](docs/design/real-sources-v0.3.md) and active
[v0.4 Out-of-core View scope](docs/design/out-of-core-view-v0.4.md). Format
decoding belongs only in accepted Source adapter crates. v0.4 permits one
rebuildable Spatial Index crate and one private demo materializer; networking,
editing, terrain, Workspace behavior, and general host UI remain in callers or
future projects unless the scope is explicitly revised.

## Local verification

Install the pinned Rust toolchain and run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo bench -p point-view --bench planner
cargo bench -p source-memory --bench read
cargo bench -p source-las --bench read
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
```

GPU acceptance tests use any available headless wgpu adapter. They skip when no
adapter is present unless `PUNCTRA_REQUIRE_GPU=1`. Run all verification locally;
the repository does not use hosted CI.

Keep public behavior documented, add interface-level tests for changes, avoid
unsafe code, and preserve caller-owned wgpu submission.
