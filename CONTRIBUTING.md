# Contributing

Punctra is intentionally a small render-engine workspace. Before adding a new
public seam, check it against the accepted
[v0.1 renderer scope](docs/design/render-engine-v0.1.md) and
[v0.2 planning scope](docs/design/adaptive-view-planning-v0.2.md). File
decoding, index construction, networking, editing, terrain, and host UI belong
in callers or future projects unless the scope is explicitly revised.

## Local verification

Install the pinned Rust toolchain and run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo bench -p point-view --bench planner
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen --test planner
```

GPU acceptance tests use any available headless wgpu adapter. They skip when no
adapter is present unless `PUNCTRA_REQUIRE_GPU=1`. Run all verification locally;
the repository does not use hosted CI.

Keep public behavior documented, add interface-level tests for changes, avoid
unsafe code, and preserve caller-owned wgpu submission.
