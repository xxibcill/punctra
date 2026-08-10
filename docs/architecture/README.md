# Point-Cloud Foundation Architecture

Status: broader platform proposal deferred; the v0.1 renderer and v0.2 adaptive
View planner are implemented

> Punctra's accepted current contracts are the reusable
> [v0.1 render engine](../design/render-engine-v0.1.md) and renderer-neutral
> [v0.2 adaptive View planner](../design/adaptive-view-planning-v0.2.md). The
> remaining broader document is research for possible host projects, not the
> current implementation plan.

This package defines a reusable, headless foundation for very large point-cloud documents. It is aimed at learning, experimentation, and reuse by desktop applications, command-line tools, language bindings, and future research code.

The architecture optimizes for one property: every module has one job and can be exercised without starting the whole product.

## What “works individually” means

An independently usable module:

1. has one sentence that completely states its job;
2. exposes an interface expressed in canonical contracts rather than another module's private representation;
3. can be constructed with an in-memory or fixture adapter;
4. can be built, tested, fuzzed, and benchmarked from its own crate;
5. does not require a window, GPU, network connection, or Workspace unless its one job inherently requires that capability; and
6. may depend only on modules below it in the dependency graph.

Independence does not mean a network process. These are in-process Rust libraries. Keeping them in one address space preserves performance, deterministic ordering, and simple debugging while still allowing direct reuse.

## Goals

- Open immutable LAS, LAZ, and COPC Sources without loading the whole Source.
- Preserve stable Point Identity and exact Attributes.
- Build or consume a resumable Spatial Index.
- Stream bounded, Revision-pinned Queries.
- Materialize exact, spillable Point Sets without requiring all identities in memory.
- Store sparse Edits as crash-safe immutable Revisions.
- Prepare renderer-neutral, progressive View Batches.
- Derive deterministic Terrain Surfaces on the CPU.
- Export explicit Terrain Surface topology to LandXML.
- Keep the engine useful without a GUI or GPU.
- Let additional adapters reuse the same behavior without copying it.

## Non-goals for the first foundation

- photogrammetry, scan registration, or sensor calibration;
- editing Source bytes in place;
- automatic CRS or vertical-datum guessing;
- reprojection or vertical-datum transformation;
- general CAD or BIM authoring;
- collaboration, cloud storage, or distributed execution;
- more than one Source per Workspace;
- View output before Source registration and a complete Spatial Index;
- exact visible-only or occlusion-aware screen selection;
- Profiles, residual analysis, and classification algorithms;
- E57, raster terrain, or arbitrary geometry formats;
- rewritten or classified LAS/LAZ output;
- native hierarchy import and remote Source range reads;
- a public plugin registry for hypothetical extensions; or
- authoritative geometry computed from GPU display values.

## Dependency graph

An arrow means “may depend on.” Cycles are forbidden.

~~~mermaid
flowchart TD
    APP["Application adapters"] --> WS["point-workspace"]
    APP --> RW["render-wgpu"]
    APP --> SET["point-set"]
    APP --> VIEW["point-view"]
    APP --> TER["terrain-model"]
    APP --> XML["landxml"]
    APP --> RT["foundation-runtime"]

    WS --> SRC["point-source"]
    WS --> IDX["point-index"]
    WS --> REV["point-revisions"]
    WS --> QRY["point-query"]

    REV --> SET
    IDX --> SRC
    QRY --> SRC
    QRY --> IDX
    QRY --> REV
    VIEW --> RP["render-protocol"]
    TER --> CT["point-contracts"]
    XML --> TER
    XML --> CT
    RW --> RP
    RW --> CT
    RP --> CT

    SRC --> CT
    IDX --> CT
    SET --> CT
    REV --> CT
    QRY --> CT
    WS --> CT

    SRC --> RT
    IDX --> RT
    SET --> RT
    REV --> RT
    QRY --> RT
    TER --> RT
    XML --> RT
    WS --> RT

    LAS["source-las adapter"] --> SRC
    COPC["source-copc adapter"] --> SRC
    MEM["source-memory adapter"] --> SRC
~~~

The graph deliberately has two levels of use:

- The individual modules are for researchers, specialized tools, and tests that need one capability.
- **point-workspace** is the deep module for coherent Source, Spatial Index, Revision, and Query lifecycle. Application adapters compose optional viewing, terrain, export, and rendering modules without moving their behavior into the Workspace.

## Architecture rules

### One job per module

Every module has a job statement in [modules.md](modules.md). Adding behavior that does not fit that sentence requires placing it in another existing module or proposing a new one. A module name is not permission to become a grab bag.

### The interface is the test surface

Tests exercise each module through its public interface. Private data structures may change freely. Tests may inspect private state only when fault injection cannot be expressed through the interface.

### Canonical contracts cross seams

Modules exchange immutable Point values from **point-contracts**, execution control from **foundation-runtime**, and renderer-neutral deltas from **render-protocol**. They do not share mutable index nodes, memory maps, GPU buffers, journal pages, or triangulator internals.

### State has one owner

- Source adapters own decoding.
- **point-index** owns index persistence and lookup.
- **point-set** owns compressed and spilled Point Set materialization.
- **point-revisions** owns logical history and crash recovery.
- **render-protocol** owns frame-generation and replacement semantics.
- **terrain-model** owns terrain topology rules.
- **render-wgpu** owns GPU resources.

No other module writes those representations.

### Exact work and display work stay distinct

Queries, Edits, Terrain Surfaces, and Exports use authoritative CPU values. Views may be partial and use origin-relative 32-bit display positions. A View can suggest Point Identities for picking, but an exact Query confirms the Point Set.

### Derived state is disposable

A Spatial Index, View cache, or derived cache may be deleted and rebuilt. The Source and Revision journal are durable Workspace state. Terrain Surfaces and their Recipes are immutable host-owned results in v0.1. Recovery never treats a cache as the source of truth.

### Seams must be earned

The Source interface is a real seam because LAS/LAZ, COPC, and in-memory adapters exist. Filesystem and fault-injection storage adapters form private test seams inside their owning modules; they are not public extension interfaces. LandXML remains a concrete module until a second export format proves a useful export seam. There is no public analysis-plugin seam in v0.1.

## Typical headless composition

The Workspace is the coherent document-access module, while the host composes independent selection, terrain, and export modules:

~~~rust
let source = source_las::open_candidate("survey.laz")?;
let Opened {
    workspace,
    index_status,
    ..
} = Engine::open(
    "survey.pcw",
    source,
    VerificationPolicy::Fast,
    OpenOptions::default(),
).await?;

if !index_status.is_ready() {
    workspace.prepare_index(IndexOptions::default()).await?;
}

let snapshot = workspace.head()?;
let point_set = point_set::materialize(
    snapshot.query(select_query),
    PointSetBudget::default(),
).await?;

let edits = EditBatch::reclassify(point_set, GROUND);
let operation = OperationId::generate()?;
// The host records only the identity needed to ask for the outcome after restart.
host_recovery.reserve(workspace.identity(), operation)?;
let outcome = workspace
    .commit(
        operation,
        snapshot.revision(),
        edits,
    )
    .await?;
let revision = match outcome {
    CommitOutcome::Committed { revision } => revision,
    CommitOutcome::Rejected { reason } => return Err(reason.into()),
    CommitOutcome::Indeterminate { operation } => {
        return Err(NeedsReconciliation(operation).into());
    }
};

let terrain_snapshot = workspace.snapshot(revision)?;
let terrain_input = TerrainInput::snapshot(
    terrain_snapshot.provenance().clone(),
    terrain_snapshot.query(terrain_query),
    terrain_snapshot.breaklines(terrain_region),
);
let surface = terrain_model::derive(
    terrain_input,
    TerrainRecipe::default(),
    TerrainLimits::default(),
).await?;

let xml = LandXml::encode(&surface, LandXmlOptions::default())?;
atomic_file::write_stream(output, xml)?;
~~~

The composition is intentionally explicit. Format decoding, index construction, Point Set spill, sparse overlay resolution, deterministic terrain rules, encoding, and recovery retain locality in the modules that own them.

## Document map

- [Canonical domain language](../../CONTEXT.md)
- [Module catalog](modules.md)
- [Cross-module contracts and invariants](contracts.md)
- [Runtime workflows](workflows.md)
- [Repository and dependency layout](repository-layout.md)
- [Verification strategy](testing.md)
- [Architectural decisions](../adr/README.md)
