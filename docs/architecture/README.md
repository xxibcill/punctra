# Point-Cloud Foundation Architecture

Status: v0.1 through the narrow v0.6 terrain/QA slice implemented; broader
terrain, export, and product layers remain deferred

The accepted versioned designs are authoritative:

- [v0.1 render engine](../design/render-engine-v0.1.md)
- [v0.2 adaptive View planner](../design/adaptive-view-planning-v0.2.md)
- [v0.3 Real Sources](../design/real-sources-v0.3.md)
- [v0.4 Out-of-core View](../design/out-of-core-view-v0.4.md)
- [v0.5 Durable document core](../design/durable-document-core-v0.5.md)
- [v0.6 Terrain and QA benchmark](../design/terrain-qa-benchmark-v0.6.md)

The current foundation is headless and embeddable. It reads immutable Sources,
prepares a complete rebuildable Spatial Index, resolves progressive display,
renders through a host-owned wgpu lifecycle, stores one narrow class of durable
document Edit, and derives one narrow CPU-authoritative in-memory Terrain
Surface with detached QA and a metric-metre LandXML deliverable. A crate exists
only when its behavior, direct tests, and a caller exist.

## Current module shape

An arrow means “may depend on.” Cycles are forbidden.

~~~mermaid
flowchart TD
    APP["Host applications"] --> WS["point-workspace"]
    APP --> TER["point-terrain"]
    APP --> VIEW["point-view"]
    APP --> RW["render-wgpu"]
    APP --> IDX["point-index"]
    APP --> SRC["point-source"]

    WS --> IDX
    WS --> SRC
    WS --> CT["point-contracts"]
    WS --> RT["foundation-runtime"]

    TER --> WS
    TER --> CT
    TER --> RT

    IDX --> SRC
    IDX --> CT
    IDX --> RT

    VIEW --> RP["render-protocol"]
    RW --> RP
    RW --> CT
    RP --> CT

    SRC --> CT
    SRC --> RT
    LAS["source-las"] --> SRC
    MEM["source-memory"] --> SRC

    TDEMO["terrain-demo"] --> TER
    TDEMO --> WS
    TDEMO --> IDX
    TDEMO --> LAS
~~~

`point-workspace` is intentionally one deep crate. Exact selection and Point-
row streaming, temporary Point Set storage, classification overlays, Revision
persistence, and Operation recovery are private cooperating modules behind its
public `Workspace`, `Snapshot`, `PointSet`, and commit interface. The earlier
four-crate document proposal was not implemented because it would expose
construction seams with only one caller.

## Architecture rules

### One job per crate

Every current crate has one job in [modules.md](modules.md). A new public seam
requires its own behavior, direct interface tests, and at least one real caller.
Private files may remain numerous when that makes a public module deeper.

### Canonical values cross seams

Crates exchange immutable Point and Source values from `point-contracts`,
runtime-neutral work control from `foundation-runtime`, and renderer-neutral
updates from `render-protocol`. They do not share mutable index nodes, decoder
buffers, Workspace journal frames, spill files, or GPU buffers.

### State has one owner

- Source adapters own format decoding.
- `point-index` owns `.pidx` construction, recovery, validation, and lookup.
- `point-workspace` owns its manifest, Point Set spills, effective
  classification overlays, immutable Revisions, and Operation records.
- `point-terrain` owns Ground Input normalization, robust triangulation,
  canonical `SurfaceVertex`/`SurfaceFace` values, detached Check Point QA, and
  the private LandXML encoder.
- `render-protocol` owns generation and replacement semantics.
- `point-view` owns deterministic culling, LOD demand, retention, and safe
  retirement decisions.
- `render-wgpu` owns GPU resources and command recording.

### Exact work and display work stay distinct

Exact Workspace selection and `Snapshot::point_rows` read CPU-authoritative
Source values and apply Revision overlays. Terrain Derivation consumes that
complete Point-row stream; it never consumes display samples. A View may be
partial and may use display samples and origin-relative `f32` coordinates. A
GPU pick is only a Pick Hint; the Workspace can confirm explicit Point
Identities but does not implement complete screen, brush, visible-only, or
occlusion selection.

### Durable and rebuildable state stay distinct

Source bytes and immutable Workspace Revision files are authoritative. The
Spatial Index, in-memory `TerrainSurface`, and all View/GPU state are
rebuildable or disposable. A LandXML file is a caller-requested Export, not
Workspace state. Deleting an index, Surface, or display state never deletes an
Edit.

### Limits are part of correctness

Source reads, index operations, selection, Point-row iteration, Point-ID
iteration, Workspace open/commit, Terrain Derivation, QA, and LandXML export
each have explicit hard ceilings. A limit failure cannot downgrade an exact
result to partial Coverage or publish a partial Surface, report, or durable
value.

### Seams must be earned

The Source seam is proven by memory, LAS, and LAZ implementations. The
`point-workspace` seam is proven by its direct example, generated LAS/LAZ
integration, and public interface tests. The `point-terrain` seam is proven by
its public example, package benchmark, interface/resource/topology/QA/LandXML
tests, and `terrain-demo` process caller. COPC, constrained or persisted
terrain, general LandXML, remote reads, screen selection, general Edits, and
application UI remain deferred until an accepted design and caller earn them.

## Typical headless composition

This uses the implemented v0.6 signatures. The full recovery branch is shown
in the [classification example](../../crates/point-workspace/examples/classify.rs).

~~~rust,ignore
let source = source_las::open("survey.laz").blocking_wait()?;
let index = point_index::prepare(
    source,
    "survey.laz.pidx",
    PrepareLimits::default(),
).blocking_wait()?;

let workspace = point_workspace::create(
    "survey.pcw",
    index,
    WorkspaceSchema::new(classification_attribute),
    OpenLimits::default(),
).blocking_wait()?;

let root = workspace.head();
let selected = root.select(
    PointQuery::within(bounds).classification_is(2),
    PointSetLimits::default(),
).blocking_wait()?;

let operation = OperationId::generate()?;
host_recovery.save(workspace.identity(), operation)?;
let outcome = workspace.commit(
    CommitRequest::set_classification(operation, selected, 1),
    CommitLimits::default(),
).blocking_wait()?;

let revision = match outcome {
    CommitOutcome::Committed(receipt) => receipt.revision(),
    CommitOutcome::Rejected(reason) => return Err(reason.into()),
    CommitOutcome::Indeterminate(uncertainty) => {
        host_recovery.mark_indeterminate(
            uncertainty.operation(),
            uncertainty.phase(),
        )?;
        return Err("drop the session, reopen, and resolve this Operation".into());
    }
};

let snapshot = workspace.snapshot(revision)?;

let surface = point_terrain::derive(
    snapshot,
    TerrainRecipe::new(2),
    TerrainLimits::default(),
).blocking_wait()?;

let qa = surface
    .check_points(check_points, CheckPointLimits::default())
    .blocking_wait()?;
let export = surface
    .export_landxml(target, options, LandXmlLimits::default())
    .blocking_wait()?;
~~~

After an indeterminate acknowledgement, the host drops every session handle,
reopens with the same complete index and verified Source, and calls
`resolve_operation` with the retained `OperationId`. A `Retryable` result is
resumed with `retry_operation`; the host does not reconstruct the expired Point
Set or invent a replacement identity.

## Scope boundary after v0.6

Implemented document and terrain behavior is deliberately narrow:

- one immutable Source and one complete index per Workspace;
- one explicitly selected `U8` classification Attribute;
- exact All, inclusive world-box, and explicit Point-ID selection;
- uniform sparse classification assignment;
- immediate-head Revert only; and
- one local exclusive Workspace session;
- one exact, ordered, classification-aware Snapshot Point-row pull stream;
- one-worker unconstrained 2.5D TIN Derivation with immutable in-memory
  `TerrainSurface` output;
- bounded detached Check Point residual QA; and
- one private durable create-new metric-metre LandXML 1.2 points/faces subset.

General predicate languages, position or other Attribute Edits, named Point
Sets, branches, merge, compaction, multiple Sources, Breaklines, constrained or
persistent terrain, general export/import, networking, autosave policy, and
product UI remain outside v0.6. Licensed-data, partner, named downstream-
application, above-500-million-Point, and human-workflow evidence also remains
outstanding.

## Document map

- [Canonical domain language](../../CONTEXT.md)
- [Module catalog](modules.md)
- [Cross-module contracts and invariants](contracts.md)
- [Runtime workflows](workflows.md)
- [Repository and dependency layout](repository-layout.md)
- [Verification strategy](testing.md)
- [Architectural decisions](../adr/README.md)
