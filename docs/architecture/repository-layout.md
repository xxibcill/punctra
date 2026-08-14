# Repository and Dependency Layout

Status: frozen for the completed v0.9 repository trust and version-1
compatibility candidate; later crates are created only with accepted behavior
and a caller

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

  terrain-demo/
    src/
      lib.rs
      main.rs
      bounded_diagnostic.rs
      cli.rs
      diagnostic.rs
      journal.rs
      publication.rs
      report.rs
      roundtrip.rs
      roundtrip_evidence.rs
      roundtrip_stream.rs
      workflow.rs
    benches/journal.rs
    tests/
      process.rs
      workflow.rs
      support/mod.rs
      fixtures/qualification-v1/

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
    tests/
      fixture_manifest.rs
      interface.rs
      fixtures/v1/
      v1_fixtures.rs

  source-memory/
    src/lib.rs
    examples/memory_source.rs
    examples/generate_v1_fixtures.rs
    benches/read.rs
    tests/
      fixture_manifest.rs
      interface.rs
      fixtures/v1/
      v1_fixtures.rs

  source-las/
    src/
      lib.rs
      decode.rs
      format.rs
    examples/inspect.rs
    examples/generate_v1_fixtures.rs
    benches/read.rs
    tests/
      conformance.rs
      fixture_manifest.rs
      fixtures/v1/
      point_formats.rs
      sequential_laz.rs
      v1_fixtures.rs

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
      fixtures/v1/
      support/mod.rs

  point-workspace/
    src/
      lib.rs
      error.rs
      hashes.rs
      limits.rs
      model.rs
      persistence.rs
      point_id_hash.rs
      point_rows.rs
      point_set.rs
      query.rs
      revision_audit.rs
      selection.rs
      util.rs
      workspace.rs
    examples/classify.rs
    benches/document.rs
    tests/
      interface.rs
      revision_audit.rs
      row_stream.rs
      selection.rs
      persistence.rs
      v1_fixtures.rs
      fixtures/v1/
      support/

  point-terrain/
    src/
      lib.rs
      derive.rs
      error.rs
      landxml.rs
      limits.rs
      model.rs
      qa.rs
      triangulation.rs
    examples/derive.rs
    benches/terrain.rs
    tests/
      interface.rs
      landxml.rs
      qa.rs
      resource.rs
      topology.rs

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
  releases/
  adr/
  research/
~~~

Files inside a crate are private locality, not additional public modules. In
particular, `point-workspace/selection.rs`, `point_rows.rs`, `point_set.rs`, and
`persistence.rs` implement one deep caller-facing Workspace contract. Likewise,
`point-terrain` keeps derivation, triangulation, QA, and LandXML encoding behind
one public terrain seam. `terrain-demo` likewise keeps journal, publication,
comparison, streaming, qualification, evidence, report, and recovery policy
behind one private application surface. These files are not separate public
crate seams.

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
point-terrain -> point-workspace + point-contracts + foundation-runtime
render-protocol -> point-contracts
point-view -> render-protocol
render-wgpu -> render-protocol + point-contracts
renderer-demo -> source-las + point-index + point-view + render-protocol + render-wgpu
terrain-demo -> source-las + point-source + point-index + point-workspace + point-terrain + point-contracts + foundation-runtime
~~~

Development-only edges may add fixture adapters, `criterion`, LAS writers, or
allocation instrumentation. They do not change the production authority graph.

Rules:

- root dependencies use explicit versions; wildcard versions are forbidden;
- a crate below the Workspace authority boundary cannot depend on
  `point-workspace` or an application;
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
  survey.laz survey.laz.pidx survey.pcw 6
cargo bench -p point-workspace --bench document

cargo test -p point-terrain --all-features
cargo run -p point-terrain --example derive
cargo bench -p point-terrain --bench terrain
cargo test -p terrain-demo --test workflow
cargo test -p terrain-demo --test process
cargo bench -p terrain-demo --bench journal

cargo bench -p point-view --bench planner
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
~~~

The Workspace direct example proves the one-deep-crate lifecycle without a GUI:
LAS/LAZ open, index prepare/open, Workspace create, exact classification
selection, classification commit, immediate-head Revert, and reopen.
The terrain example proves the public in-memory Source-to-LandXML composition.
The `terrain-demo` workflow and process tests prove the generated LAS/LAZ
durable start/resume/inspect composition; its journal benchmark measures five
generated restart modes.

## Private depth inside point-workspace

The public interface is compact:

~~~text
create/open -> Workspace
Workspace -> schema/head/snapshot/revision_info
Workspace -> revision_audit
Snapshot -> select/select_point_ids -> PointSet
Snapshot -> point_rows -> SnapshotPointBatches
PointSet -> metadata/ids
Workspace -> commit/retry_operation/resolve_operation
~~~

Private behavior hidden behind it includes:

- complete candidate planning and exact Source predicates;
- bounded Source reads and cumulative overlay joins;
- exact ordered Point-row filtering, row/content hashing, fused terminal
  summary, and row-specific ledgers;
- Point-ID validation, sorting, deduplication, and span normalization;
- in-memory Point Set growth, checked spill, repeated bounded reads, retained
  private storage, and ownership-safe offline cleanup policy;
- request/delta hashing and sparse before/after rows;
- immutable manifest, intent, rejection, and Revision encodings;
- exact Revision-row validation, Source-position joining, transitions, hashes,
  and Edit Footprint derivation;
- no-replace publication, directory durability, lock ownership, and recovery;
- cancellation/publication phase tracking; and
- operation-specific memory, temporary, and durable ledgers.

Do not expose private index pages, Source decoder buffers, Point Set frames,
overlay blocks, Revision frames, scratch paths, hard-link details, GPU buffers,
or scheduler internals.

## Private depth inside point-terrain

The public interface is also compact:

~~~text
derive(Snapshot, TerrainRecipe, TerrainLimits) -> TerrainSurface
TerrainSurface -> descriptor/vertices/faces
TerrainSurface -> check_points -> CheckPointReport
TerrainSurface -> export_landxml/ensure_landxml -> LandXmlReceipt
~~~

Private behavior includes exact row ingestion, normalized predicate inputs,
robust triangulation and canonicalization, topology validation, deterministic
point location and compensated QA statistics, XML encoding, durable create-new
publication, and operation-specific limits. No triangulator, point-locator, or
exporter registry is public.

## Persisted directories

Index, Workspace, caller-owned Export storage, and the application Run root are
separate:

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

run-root/
  run.pwf                    # terrain-demo eight-frame Workflow journal
  run.lock                   # exclusive Run lock
  terrain.xml                # caller-owned exactly ensured LandXML Export
  audit.json                 # caller-owned exactly ensured canonical report

evidence/
  round-trip.json            # caller-owned pass/fail evidence outside run-root
~~~

Ownership rules:

- only `point-index` interprets `.pidx` and its sidecars;
- only `point-workspace` interprets the Workspace directory;
- the Source remains outside the Workspace and is never rewritten;
- a Point Set spill is temporary, per-attempt bounded, retained as private
  debris, and ignored by recovery;
- `TerrainSurface`, View, and GPU state are not persisted as authoritative
  document data; LandXML, `audit.json`, and Round-Trip Evidence are
  caller-requested deliverables;
  and
- only `terrain-demo` interprets `run.pwf`; unknown Run-root children are never
  deleted.

## Versioning

Cargo semantic versions, persisted schema versions, deterministic algorithm
versions, and LandXML/journal/report format versions are separate axes. A Cargo
`0.9` version does not imply Workspace disk schema or terrain algorithm version
9.

- Unknown persisted major versions fail explicitly.
- Identity and persisted schema values remain opaque outside their owner.
- A future migration must leave the prior representation recoverable until the
  new representation is complete and durable.
- Algorithm versions change when deterministic artifact meaning changes.
- Golden fixtures are required before a persisted compatibility promise or a
  second persisted version is accepted.

v0.9 keeps every persisted schema at version 1 and does not invent a
migration solely to exercise migration machinery. Source Record version 1,
Spatial Index disk/recipe version 1, Workspace disk/semantic version 1,
Terrain algorithm version 1, Workflow journal version 1, and the supported
LandXML/report subsets evolve independently of Cargo versions. Frozen fixtures
must precede any future second persisted version.

## Implementation order

Completed vertical slices are:

1. render protocol and wgpu engine;
2. adaptive View planning;
3. verified Source contracts with memory/LAS/LAZ adapters;
4. complete persistent Spatial Index and real-cloud View composition;
5. one deep durable classification Workspace;
6. one exact Snapshot Point-row stream plus one deep deterministic Terrain/QA/
   LandXML technical slice; and
7. linked cancellation, exact Revision Audit/LandXML reconciliation, and one
   private durable application Workflow Run with canonical report; and
8. strict read-only Complete-Run qualification, full-ceiling streaming,
   canonical Round-Trip Evidence, frozen version-1 fixtures, and the reviewed
   support/recovery surface.

Any later accepted slice may add only terrain or workflow behavior earned by
its real caller and evidence. COPC, general LandXML, UI, bindings, and other
adapters remain deferred until their own evidence and designs exist.

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
