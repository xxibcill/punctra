# Point-Cloud Foundation Architecture

Status: v0.1 through the narrow v0.9 repository trust and version-1
compatibility-candidate slice Complete; broader terrain, export, external
interoperability evidence, and product layers remain deferred

The accepted versioned designs are authoritative:

- [v0.1 render engine](../design/render-engine-v0.1.md)
- [v0.2 adaptive View planner](../design/adaptive-view-planning-v0.2.md)
- [v0.3 Real Sources](../design/real-sources-v0.3.md)
- [v0.4 Out-of-core View](../design/out-of-core-view-v0.4.md)
- [v0.5 Durable document core](../design/durable-document-core-v0.5.md)
- [v0.6 Terrain and QA benchmark](../design/terrain-qa-benchmark-v0.6.md)
- [v0.7 Technical partner-alpha readiness](../design/technical-alpha-readiness-v0.7.md)
- [v0.8 Repository interoperability qualification](../design/design-partner-mvp-v0.8.md)
- [v0.9 Repository Trust and v1 Candidate](../design/trust-v1-candidate-v0.9.md)

The current foundation is headless and embeddable. It reads immutable Sources,
prepares a complete rebuildable Spatial Index, resolves progressive display,
renders through a host-owned wgpu lifecycle, stores one narrow class of durable
document Edit, and derives one narrow CPU-authoritative in-memory Terrain
Surface with detached QA and a metric-metre LandXML deliverable. The headless
`terrain-demo` application can run that path through one durable, resumable,
audited Workflow Run without turning orchestration policy into another
foundation crate. A crate exists only when its behavior, direct tests, and a
caller exist.

The frozen [v0.9 public interface review](v0.9-interface-review.md) classifies
reusable, adapter-author, test-support, and private application surfaces. The
[v0.9 support, upgrade, and recovery matrix](v0.9-support-matrix.md) defines
the exact workflow profile, artifact policies, platform evidence, and operator
actions; the [release record](../releases/v0.9.0.md) owns exact local results.

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
    TDEMO --> SRC
    TDEMO --> CT
    TDEMO --> RT
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
  classification overlays, immutable Revisions, Operation records, and exact
  rebuildable Revision Audits.
- `point-terrain` owns Ground Input normalization, robust triangulation,
  canonical `SurfaceVertex`/`SurfaceFace` values, detached Check Point QA, and
  the private LandXML encoder and exact-target reconciliation.
- `terrain-demo` owns its Run lock, eight-frame journal, cross-module recovery
  policy, Surface Change Envelope, canonical report, read-only Complete-Run
  qualifier, canonical Round-Trip Evidence, and structured actions.
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

The v0.7 application facade fixes the complete caller intent before mutation.
The Run and Workspace Operation identities must be chosen and retained by the
caller before `start_run`. The Workspace is created separately through
`point-workspace`; the baseline is its current
`workspace.head().provenance().revision()` and its selected `U8` Attribute is
Source Attribute 6 (`source-las` classification):

~~~rust,ignore
let paths = WorkflowPaths::new(
    "survey.laz",
    "survey.laz.pidx",
    "survey.pcw",
    "run-root",
);
let intent = WorkflowRunIntent::new(
    caller_run_id,
    caller_operation_id,
    expected_baseline_revision,
    ground_ordinals_to_exclude,
    1,
    TerrainRecipe::new(2),
    detached_check_points,
    landxml_options,
)?;

let receipt = start_run(paths.clone(), intent.clone(), WorkflowLimits::default())
    .blocking_wait()?;

// After interruption, repeat the same paths and complete intent.
let recovered = resume_run(paths, intent, WorkflowLimits::default())
    .blocking_wait()?;
assert_eq!(recovered, receipt);

let status = inspect_and_repair_run("run-root", WorkflowLimits::default())?;
assert!(status.is_complete());
~~~

`start_run` and `resume_run` return `WorkflowJob`, whose active child waits use
linked cancellation. `inspect_and_repair_run` acquires the Run lock, verifies
the journal format, hash chain, and semantic frame links, and explicitly
repairs a torn final suffix without opening or mutating Source, index, or
Workspace state. When repair is needed, it truncates that suffix to the last
verified journal frame, then revalidates Run-root identity; a root replacement
after repair is publication-indeterminate. The private workflow resolves
Committed, Rejected, Retryable, NotRecorded, and Indeterminate Workspace
Operation states with the original identity. It opens but never creates the
Workspace; an absent Workspace is `PWF_INVALID_REQUEST` before Run creation or
Workspace mutation.

## Scope boundary after v0.9

Implemented document and terrain behavior is deliberately narrow:

- one immutable Source and one complete index per Workspace;
- one explicitly selected `U8` classification Attribute;
- exact All, inclusive world-box, and explicit Point-ID selection;
- uniform sparse classification assignment;
- immediate-head Revert only;
- one local exclusive Workspace session;
- one exact, ordered, classification-aware Snapshot Point-row pull stream;
- one-worker unconstrained 2.5D TIN Derivation with immutable in-memory
  `TerrainSurface` output;
- bounded detached Check Point residual QA;
- one private metric-metre LandXML 1.2 points/faces subset with create-new and
  exact-existing reconciliation;
- exact immutable Revision Audits and Edit Footprints; and
- one private eight-frame `terrain-demo` Workflow Run with canonical report,
  exclusive lock, linked cancellation, and structured recovery actions; and
- one private read-only Complete-Run LandXML verifier with bounded streaming
  comparison and separate canonical pass/fail evidence outside the Run root.

General predicate languages, position or other Attribute Edits, named Point
Sets, branches, merge, compaction, multiple Sources, Breaklines, constrained or
persistent terrain, general export/import, networking, autosave policy, and
product UI remain outside v0.9. Licensed-data, partner, named downstream-
application, above-500-million-Point, and human-workflow evidence also remains
outstanding.

The implemented v0.8 design adds private `terrain-demo` semantic LandXML
comparison, read-only Complete-Run binding, full-export-ceiling streaming, and a
separate canonical evidence artifact without changing the public foundation
shape. Every v0.7 journal and report contract remains unchanged.

The completed v0.9 slice freezes the version-1 compatibility fixtures, artifact
support classes, persistence/recovery behavior, platform evidence, and reviewed
interface classifications for that same narrow shape. It adds no new workflow
or product feature family, and a repository v1 candidate is not `1.0.0` or an
external product-readiness claim.

## Document map

- [Canonical domain language](../../CONTEXT.md)
- [Module catalog](modules.md)
- [Cross-module contracts and invariants](contracts.md)
- [Runtime workflows](workflows.md)
- [Repository and dependency layout](repository-layout.md)
- [Verification strategy](testing.md)
- [Architectural decisions](../adr/README.md)
