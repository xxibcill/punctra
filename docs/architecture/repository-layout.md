# Repository and Dependency Layout

Status: current through v0.5; later crates are created only with accepted
behavior and a caller

The repository is one Cargo workspace. Each current crate is independently
buildable and exposes a smaller public interface than its private
implementation. No empty future crates are scaffolded.

## Current layout

~~~text
Cargo.toml
CONTEXT.md
README.md
ROADMAP.md

apps/
  renderer-demo/
    src/
      main.rs
      orbit_camera.rs
      real_cloud.rs
      scene.rs
      synthetic.rs
    tests/
      headless_smoke.rs
      planner.rs

crates/
  foundation-runtime/
    src/lib.rs
    tests/contracts.rs

  point-contracts/
    src/lib.rs
    tests/contracts.rs

  point-source/
    src/
      lib.rs
      adapter.rs
      error.rs
      stream.rs
    tests/interface.rs

  source-memory/
    src/lib.rs
    examples/memory_source.rs
    benches/read.rs
    tests/interface.rs

  source-las/
    src/
      lib.rs
      decode.rs
      format.rs
    examples/inspect.rs
    benches/read.rs
    tests/
      conformance.rs
      point_formats.rs

  point-index/
    src/
      lib.rs
      error.rs
      limits.rs
      model.rs
      persistence.rs
      prepare.rs
      read.rs
      tree.rs
    examples/direct_use.rs
    benches/index.rs
    tests/
      candidates.rs
      interface.rs
      persistence.rs

  point-workspace/
    src/
      lib.rs
      error.rs
      limits.rs
      model.rs
      persistence.rs
      point_set.rs
      selection.rs
      workspace.rs
    examples/classify.rs
    benches/document.rs
    tests/
      interface.rs
      selection.rs
      persistence.rs

  render-protocol/
    src/
      lib.rs
      camera.rs
    tests/
      contracts.rs
      state_model.rs

  point-view/
    src/
      lib.rs
      planning.rs
    benches/planner.rs
    tests/planner.rs

  render-wgpu/
    src/
      lib.rs
      frame.rs
      gpu.rs
      pick.rs
      pipeline.rs
      renderer.rs
      targets.rs
      point.wgsl
    tests/
      contracts.rs
      offscreen.rs

docs/
  architecture/
  design/
  adr/
  research/
~~~

Files inside a crate are private locality, not additional public modules. In
particular, `point-workspace/selection.rs`, `point_set.rs`, and
`persistence.rs` implement one deep caller-facing Workspace contract. They are
not separate public crate seams.

## Cargo dependency direction

The current graph is:

~~~text
point-contracts
foundation-runtime

point-source -> point-contracts + foundation-runtime
source-memory -> point-source + point-contracts + foundation-runtime
source-las -> point-source + point-contracts + foundation-runtime
point-index -> point-source + point-contracts + foundation-runtime
point-workspace -> point-index + point-source + point-contracts + foundation-runtime
render-protocol -> point-contracts
point-view -> render-protocol
render-wgpu -> render-protocol + point-contracts
renderer-demo -> source-las + point-index + point-view + render-protocol + render-wgpu
~~~

Development-only edges may add fixture adapters, `criterion`, LAS writers, or
allocation instrumentation. They do not change the production authority graph.

Rules:

- root dependencies use explicit versions; wildcard versions are forbidden;
- a lower crate cannot depend on `point-workspace` or `renderer-demo`;
- a Source adapter cannot depend on an index, Workspace, or renderer;
- only `render-wgpu` and the application that directly composes it may depend
  on wgpu;
- no headless foundation crate depends on a windowing stack; and
- a feature flag cannot change identity, exactness, Revision, or persistence
  semantics.

`cargo metadata --format-version 1` is the inspection source when checking the
actual graph. No hosted CI is required or configured; checks run locally.

## Independent build and use

Representative direct commands are:

~~~bash
cargo test -p point-source --all-features
cargo run -p source-memory --example memory_source
cargo run --release -p source-las --example inspect -- survey.laz

cargo test -p point-index --all-features
cargo run -p point-index --example direct_use
cargo bench -p point-index --bench index

cargo test -p point-workspace --all-features
cargo run --release -p point-workspace --example classify -- \
  survey.laz survey.laz.pidx survey.pcw CLASSIFICATION_ATTRIBUTE_ID
cargo bench -p point-workspace --bench document

cargo bench -p point-view --bench planner
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
~~~

The Workspace direct example proves the one-deep-crate lifecycle without a GUI:
LAS/LAZ open, index prepare/open, Workspace create, exact classification
selection, classification commit, immediate-head Revert, and reopen.

## Private depth inside point-workspace

The public interface is compact:

~~~text
create/open -> Workspace
Workspace -> head/snapshot/revision_info
Snapshot -> select/select_point_ids -> PointSet
PointSet -> metadata/ids
Workspace -> commit/retry_operation/resolve_operation
~~~

Private behavior hidden behind it includes:

- complete candidate planning and exact Source predicates;
- bounded Source reads and cumulative overlay joins;
- Point-ID validation, sorting, deduplication, and span normalization;
- in-memory Point Set growth, checked spill, repeated bounded reads, and
  cleanup;
- request/delta hashing and sparse before/after rows;
- immutable manifest, intent, rejection, and Revision encodings;
- no-replace publication, directory durability, lock ownership, and recovery;
- cancellation/publication phase tracking; and
- operation-specific memory, temporary, and durable ledgers.

Do not expose private index pages, Source decoder buffers, Point Set frames,
overlay blocks, Revision frames, scratch paths, hard-link details, GPU buffers,
or scheduler internals.

## Persisted directories

Index and Workspace storage are separate:

~~~text
survey.laz                    # immutable Source, host-owned
survey.laz.pidx               # complete rebuildable point-index artifact
survey.laz.pidx.work          # disposable/resumable index sidecar when present
survey.laz.pidx.samples       # disposable index construction sidecar when present

survey.pcw/
  manifest.pwm               # point-workspace schema and lineage
  workspace.lock             # exclusive session lock
  operations/
    <operation-id>.ready      # complete retryable intent
    <operation-id>.reject     # definitive rejection
  revisions/
    <sequence>-<revision-id>.pwr
  scratch/                    # recognized disposable stages and live Point Sets
~~~

Ownership rules:

- only `point-index` interprets `.pidx` and its sidecars;
- only `point-workspace` interprets the Workspace directory;
- the Source remains outside the Workspace and is never rewritten;
- a Point Set spill is temporary and retained only by live handles; and
- View and GPU state are not persisted as authoritative document data.

## Versioning

Cargo semantic versions, persisted schema versions, and deterministic algorithm
versions are separate axes. A Cargo `0.5` version does not imply disk schema
version 5.

- Unknown persisted major versions fail explicitly.
- Identity and persisted schema values remain opaque outside their owner.
- A future migration must leave the prior representation recoverable until the
  new representation is complete and durable.
- Algorithm versions change when deterministic artifact meaning changes.
- Golden fixtures are required when more than one persisted version is
  supported.

v0.5 creates one disk/semantic contract and does not claim migration or
compaction.

## Implementation order

Completed vertical slices are:

1. render protocol and wgpu engine;
2. adaptive View planning;
3. verified Source contracts with memory/LAS/LAZ adapters;
4. complete persistent Spatial Index and real-cloud View composition; and
5. one deep durable classification Workspace.

The next accepted slice may add terrain behavior and the exact edited Point-row
seam that its real caller needs. COPC, LandXML, UI, bindings, and other adapters
remain deferred until their own evidence and designs exist.

## Definition of ready for a crate

A crate is ready for another release to build on when:

- its one-job sentence describes every public operation;
- invariants, ordering, limits, effects, and error certainty are documented;
- direct interface tests exercise the public seam;
- at least one real caller uses it without private access;
- persisted formats have corruption, interruption, and recovery coverage;
- Source-scale work has benchmark and memory evidence;
- promised determinism is tested across repeat runs and partitioning; and
- the full relevant local verification sequence passes.

## Licensing

All new crates and fixtures must be compatible with the repository's dual MIT
or Apache-2.0 license. Production datasets, vendor SDKs, and third-party sample
files require explicit redistribution rights; generated fixtures are the
default repository evidence.
