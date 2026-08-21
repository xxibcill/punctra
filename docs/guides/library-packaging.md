# Library Packaging and Compatibility

Punctra v0.14.0-alpha.1 carries forward the local crates.io/docs.rs packaging
path for the twelve public library crates. It does not publish them. The two
demo applications remain private workspace packages.

## Choose the narrowest crate

| Crate | Dependency role |
|---|---|
| `foundation-runtime` | Runtime-neutral jobs, cancellation, progress, and bounded diagnostics. |
| `point-contracts` | Canonical Source, Point, Attribute, coordinate, and provenance values. |
| `point-source` | Verified immutable Source capability and bounded exact reads. |
| `source-memory` | In-memory Source adapter; its `test-support` feature adds fault controls only. |
| `source-las` | Strict LAS/LAZ Source adapter with exact metadata and Point decoding. |
| `point-index` | Rebuildable persistent spatial index and bounded display samples. |
| `point-workspace` | Durable exact classification Revisions, Point Sets, and Operations. |
| `render-protocol` | Renderer-neutral update, camera, picking, and resource contracts. |
| `point-view` | Deterministic View planning over renderer-neutral contracts. |
| `render-wgpu` | wgpu execution engine for caller-owned GPU hosts. |
| `point-review` | CPU-authoritative Pick confirmation and screen-through selection. |
| `point-terrain` | Deterministic terrain, bounded-AOI Surface persistence/reopen, exact Snapshot-bound QA and comparison, and narrow LandXML export. |

Every default feature set is empty. Enabling a feature must not change Source
Identity, exactness, persisted semantics, or deterministic Artifact meaning.
`source-memory/test-support` is the only current optional library feature and
exists for conformance and fault tests, not production behavior.

## Version and compatibility policy

All libraries in the active bounded v0.14 slice use `0.14.0-alpha.1`, require
Rust 1.90, and pin inter-Punctra registry dependencies
to exactly that version while retaining local workspace paths. This local
qualification is not publication. If a later explicit decision publishes the
packages, publish the complete set as one release unit in dependency order; do
not mix alpha package versions.

Before 1.0, a later alpha minor may make a documented public Cargo/API change.
Persisted formats do not inherit that freedom from the Cargo version. A frozen
persisted version continues to reproduce its documented bytes and semantics or
fails closed; migration and rebuild rules belong to the format owner. Source
Record v1, Workspace v1, and the other frozen v1 fixtures therefore remain
compatibility evidence after the Cargo version moves to v0.14. Surface
disk/work-v1 is a separate rebuildable format: its frozen fixtures govern its
reader/rebuild behavior without changing Terrain algorithm or Workflow Run-v1.

The packages use the explicit `MIT OR Apache-2.0` license expression. Each
archive carries the root README, complete metadata, an MSRV, a docs.rs URL,
empty default features, and bounded source content. Untracked field data and
build output are excluded.

## Local package and documentation gate

Run from a clean candidate tree:

```bash
scripts/verify-packages.rb
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
```

During uncommitted local development, use
`PUNCTRA_PACKAGE_ALLOW_DIRTY=1 scripts/verify-packages.rb`; the same metadata,
content inspection, and extracted-package builds still run. The script calls
`cargo package --list` for every library and then `cargo package` for the
publishable workspace subset. It derives the publishable inventory, private
applications, shared version/license/MSRV policy, and dependency order from
Cargo metadata so release policy has one manifest source of truth. It never
uploads, tags, signs, or changes a registry.

An actual publication is a separate maintainer action. Publish in the order
used by the script's package list, wait for each registry package to become
available to dependency resolution, and stop on the first failure. Publication
requires a fresh explicit decision and is outside the v0.14 repository exit.
