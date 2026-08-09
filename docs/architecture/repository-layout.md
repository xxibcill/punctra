# Repository and Dependency Layout

Status: proposed v0.1

The repository is one Cargo workspace containing independently buildable crates. A crate is created only when its implementation and at least one caller exist; the tree below is the intended destination, not a requirement to scaffold empty directories.

## Target layout

~~~text
Cargo.toml
CONTEXT.md
LICENSE-APACHE
LICENSE-MIT

crates/
  point-contracts/
    src/
      lib.rs

  foundation-runtime/
    src/
      lib.rs
      job.rs
      stream.rs
      budget.rs

  point-source/
    src/
      lib.rs
      verification.rs
      validation.rs

  source-las/
    src/
      lib.rs
      header.rs
      records.rs
      attributes.rs

  source-copc/
    src/
      lib.rs
      hierarchy.rs
      ranges.rs

  source-memory/
    src/
      lib.rs

  point-index/
    src/
      lib.rs
      build.rs
      disk_format.rs
      recover.rs
      select.rs

  point-set/
    src/
      lib.rs
      compress.rs
      spill.rs
      iterate.rs

  point-revisions/
    src/
      lib.rs
      journal.rs
      commit.rs
      recover.rs
      overlay.rs

  point-query/
    src/
      lib.rs
      plan.rs
      read.rs
      overlay_join.rs
      predicate.rs

  render-protocol/
    src/
      lib.rs
      frame.rs
      delta.rs
      reference_state.rs

  point-view/
    src/
      lib.rs
      priority.rs
      floating_origin.rs
      pack.rs

  terrain-model/
    src/
      lib.rs
      thin.rs
      constraints.rs
      triangulate.rs
      validate.rs

  landxml/
    src/
      lib.rs
      encode.rs
      validate.rs

  point-workspace/
    src/
      lib.rs
      manifest.rs
      open.rs
      recovery.rs
      jobs.rs

  render-wgpu/
    src/
      lib.rs
      resident.rs
      pipelines.rs
      frame.rs
      recover.rs

apps/
  point-cli/
    src/
      main.rs

  viewer-desktop/
    src/
      main.rs
      app_state.rs
      tools.rs

bindings/
  point-python/          # add only with its first real caller

test-support/
  point-fixtures/
    src/
      lib.rs

fixtures/
  synthetic/
  public-domain/
  corrupt/
  landxml/

fuzz/
  fuzz_targets/

benches/
  scenarios/

docs/
  architecture/
  adr/
  formats/
~~~

Files inside one crate are private implementation structure, not extra public modules. For example, **point-index/build.rs** and **point-index/select.rs** support the single point-index job. Promoting every algorithm stage into its own crate would create shallow interfaces and reduce locality.

## Cargo dependency direction

The root manifest uses explicit workspace dependencies and denies wildcard versions. Each crate's manifest lists only the dependencies permitted by [modules.md](modules.md).

The intended graph is:

~~~text
point-contracts
foundation-runtime

point-source -> point-contracts + foundation-runtime
source adapters -> point-source
point-index -> point-source + point-contracts + foundation-runtime
point-set -> point-contracts + foundation-runtime
point-revisions -> point-set + point-contracts + foundation-runtime
point-query -> point-source + point-index + point-revisions
render-protocol -> point-contracts
point-view -> point-query + point-index + render-protocol
terrain-model -> point-contracts + foundation-runtime
landxml -> terrain-model + foundation-runtime
point-workspace -> point-source + point-index + point-revisions + point-query
render-wgpu -> render-protocol + point-contracts

Application adapters compose the modules they directly need.
~~~

The textual tree is a readability aid; the allowlist in [modules.md](modules.md) is normative.

## Dependency enforcement

CI should inspect Cargo metadata and reject any edge not present in the allowlist. This catches architectural drift that normal compilation accepts.

Additional rules:

- **point-contracts** cannot depend on I/O, async runtimes, wgpu, windowing, XML, or a point format.
- **foundation-runtime** cannot depend on a point format, domain algorithm, async runtime, wgpu, or windowing.
- A seam crate cannot depend on any of its adapters.
- A lower module cannot depend on **point-workspace** or an application adapter.
- Only **render-wgpu** and the desktop adapter may depend on wgpu.
- Only application adapters choose concrete Source adapters.
- No crate named common, utils, helpers, plugin, storage, or manager is allowed without an accepted architecture change explaining its one job.
- Feature flags cannot create a reverse dependency or change correctness semantics.

A small repository tool may enforce the graph:

~~~text
cargo metadata --format-version 1
        |
        v
compare workspace package edges with docs/architecture/dependencies.toml
        |
        +-- exact match: continue
        +-- unknown edge: fail CI
~~~

If this automation is implemented, the machine-readable allowlist becomes generated from, or checked against, [modules.md](modules.md) so the two cannot silently diverge.

## Independent build and use

Every crate provides:

- crate-level documentation containing its one-job sentence;
- one minimal direct-use example;
- interface-level unit and integration tests;
- a package-specific check command;
- package-specific benchmark or complexity evidence when it handles Source-scale data; and
- no requirement to initialize unrelated modules.

Expected commands:

~~~bash
cargo check -p point-index
cargo test -p point-revisions
cargo test -p terrain-model
cargo bench -p point-query
cargo run -p point-cli -- inspect fixture.laz
~~~

Examples of direct use:

~~~rust
// Decode without a Workspace.
let source = source_las::open_candidate(path)?;
let verified = source
    .verify(SourceExpectation::New, VerificationPolicy::Full)
    .await?;
let event = verified.source.read(spans, fields, budget).next(&cancel)?;

// Index generated Points through the memory adapter, without a Workspace.
let source = source_memory::from_batches(generated_batches)?;
let verified = source
    .verify(SourceExpectation::New, VerificationPolicy::Full)
    .await?;
let index = point_index::IndexBuilder::build_or_resume(
    verified.source,
    target,
    options,
).await?;

// Derive terrain without a Source or Spatial Index.
let surface = terrain_model::derive(
    TerrainInput::detached(generated_batches, breaklines, content_hash),
    recipe,
    limits,
).await?;

// Encode a stored Terrain Surface fixture without the engine.
let xml = LandXml::encode(&surface, options)?;

// Draw synthetic render-protocol deltas without opening point data.
renderer.apply(RenderDelta::Points(synthetic_view_delta))?;
renderer.render(&target, &frame)?;
~~~

## Private depth inside crates

The external interface of a module should be much smaller than its implementation. For example:

~~~text
point-query public interface
  Snapshot.query(PointQuery) -> BatchStream<PointBatch, ExactPointSummary>

private behavior hidden behind it
  candidate planning
  span coalescing
  bounded reads
  sparse overlay join
  exact spatial predicates
  Attribute predicates
  stable merge order
  exact count and provenance summary
  cancellation and resource limits
~~~

That ratio is module depth. Callers receive leverage from one operation, and query knowledge retains locality inside one crate.

Do not expose:

- index pages or node structs;
- memory maps or borrowed decoder buffers;
- journal frames or overlay tables;
- triangulator half-edges;
- GPU buffers or shader bindings;
- cache directories or eviction internals; or
- scheduler task handles.

Expose stable domain values, operations, progress, and errors instead.

## Features and platform isolation

Keep optional heavy dependencies at adapter edges:

- **source-las** owns LAS/LAZ codec dependencies.
- **source-copc** initially owns local COPC hierarchy and byte-range decoding only.
- **landxml** owns XML encoding and validation dependencies.
- **render-wgpu** owns wgpu and shader dependencies.
- **viewer-desktop** owns windowing and UI dependencies.
- **point-python** owns the Python binding dependency.

Headless users that select **point-workspace**, **point-query**, or **terrain-model** must not compile wgpu or a windowing stack.

Feature flags may select an adapter capability such as LAZ compression. Remote readers wait for a real caller and a separately reviewed trust and retry contract. Feature flags may not switch between two subtly different identity, Revision, Query, or topology semantics.

## Persisted directories

A Workspace directory should make ownership visible:

~~~text
example.pcw/
  manifest.json             # owned by point-workspace
  revisions.pcrev           # committed Revisions and staged operations; owned by point-revisions
  indexes/
    source-id.pcidx         # owned by point-index
  cache/
    ...                     # disposable; safe to remove
~~~

Ownership rules:

- only **point-workspace** reads or writes the manifest, which records Workspace Identity and the one versioned SourceRecord returned by verification;
- only **point-revisions** reads or writes the revision journal;
- only **point-index** reads or writes pcidx files;
- any process may request cache deletion through its owning module, but no module interprets another module's private cache; and
- the one immutable Source remains outside the Workspace and is referenced by the manifest.

Terrain Surfaces and LandXML outputs are host-owned values or files in v0.1. The Workspace does not persist an Artifact catalog.

## Versioning

Use two separate version axes:

1. Cargo semantic versions describe Rust interface compatibility.
2. On-disk schema versions describe persisted representation compatibility.

During 0.x development, crate releases may be synchronized for convenience, but callers must not assume that Cargo version 0.4 implies disk format 4.

Rules:

- identifiers and persisted schema versions are opaque outside their owner;
- unknown persisted major versions fail explicitly;
- migrations create a new representation and leave the old one recoverable until success;
- golden fixtures from every supported persisted version stay in the repository; and
- algorithm versions change whenever deterministic Artifact meaning changes.

## Implementation order

Build vertical evidence in this order:

1. **point-contracts**, **foundation-runtime**, **point-source**, and **source-memory** with conformance tests.
2. **source-las** for small LAS, then bounded LAZ decoding.
3. **point-index** with a synthetic oracle, persistence, interruption, and resume.
4. **point-set** with forced spill, bounded iteration, and content hashing.
5. **point-revisions** with sparse classification Edits and fault-injected recovery.
6. **point-query** and **point-workspace** with exact Snapshot Queries.
7. **render-protocol**, **point-view**, **render-wgpu**, and a minimal desktop adapter.
8. **source-copc** using the same PointSource and foundation-index path.
9. **terrain-model** with complete TerrainLimits and deterministic fixtures.
10. **landxml** with independent parsing and semantic fixtures.
11. Bindings and additional adapters only after the Rust interfaces settle.

Each step must produce a directly usable library and executable example. Avoid scaffolding later crates before their first behavior exists.

## Definition of ready for a module

A module is ready for other software to build on when:

- its one-job sentence still describes every public operation;
- public contracts document invariants, ordering, resource limits, errors, and effects;
- the module passes its direct interface suite;
- at least one real caller uses it without private access;
- persisted formats have versioning and recovery tests where applicable;
- Source-scale paths have a benchmark and enforced memory ceiling;
- determinism is tested across repeat runs and worker counts where promised;
- no disallowed dependency edge exists; and
- deleting the module would force meaningful knowledge into more than one caller.

The last test protects depth. A thin pass-through crate should be folded into its caller; a module that concentrates hard knowledge should stay independent.

## Licensing

For an open-source foundation, dual MIT and Apache-2.0 licensing provides a familiar permissive choice and an explicit patent grant. Third-party fixture and dependency licenses must be recorded separately; public point-cloud fixtures must include provenance and redistribution terms.
