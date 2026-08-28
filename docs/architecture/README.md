# Point-Cloud Foundation Architecture

Status: v0.1 through the narrow v0.9 repository trust and version-1
compatibility-candidate slice Complete; the v0.10 professional inspection View
repository implementation is complete with field/adoption publication
outstanding; the v0.11 exact-review technical slice is repository-verified
with field-activation and independent-adoption evidence outstanding; the v0.12
explicit spatial-reference and packaging repository slice is complete with
production-corpus, downstream, adoption, and support evidence outstanding;
v0.13: Complete and repository-verified for the bounded persistent-
terrain slice; field activation, production-scale accuracy, true out-of-core
adoption, independent adoption, partner validation, and support qualification
outstanding; v0.14 exact Terrain QA and correction-loop bounded repository
slice Complete and repository-verified with field activation, observed workflow
timing, independent adoption, partner validation, and support qualification
outstanding; v0.15 local WebAssembly/WebGPU browser-foundation slice Complete
and repository-verified; v0.16 bounded immutable-LAS HTTP Range, browser-cache,
and worker-decoding slice Complete and repository-verified; v0.17 bounded
framework-neutral browser viewer API and immutable-LAS exact-Point bridge plus
v0.18 packed viewer SDK and thin React lifecycle adapter Complete and
repository-verified; v0.19 exact local browser/device qualification and v0.20
clean packed-consumer integration baseline Complete and repository-verified;
v0.21 private visual-corpus, capture, comparison, rubric, and evidence workflow
Accepted and in progress while attended record/verify evidence and release
verification remain outstanding; arbitrary Source delivery, broad bundler/
framework/browser qualification, independent adoption, API stability, and
support qualification outstanding; broader terrain, export, external
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
- [v0.10 Field Qualification and Professional Inspection View](../design/field-inspection-view-v0.10.md)
- [v0.11 Exact Interactive Review and Ground Correction](../design/exact-interactive-review-v0.11.md)
- [v0.12 Explicit Spatial Reference and Package Publication](../design/explicit-spatial-reference-v0.12.md)
- [v0.13 Persistent Bounded-AOI Terrain](../design/persistent-production-scale-terrain-v0.13.md)
- [v0.14 Exact Terrain QA and Correction Loop](../design/exact-terrain-qa-correction-v0.14.md)
- [v0.15 WebAssembly and WebGPU Browser Foundation](../design/browser-foundation-v0.15.md)
- [v0.16 HTTP Range Streaming, Browser Caching, and Worker Decoding](../design/http-range-streaming-v0.16.md)
- [v0.17 Browser Viewer API](../design/browser-viewer-api-v0.17.md)
- [v0.18 Embeddable SDK and Framework Integration](../design/embeddable-sdk-v0.18.md)
- [v0.19 Browser and Device Qualification](../design/browser-device-qualification-v0.19.md)
- [v0.20 Stable Browser-Engine Integration Baseline](../design/browser-integration-baseline-v0.20.md)
- [v0.21 Visual-Quality Baseline and Regression Corpus](../design/visual-quality-baseline-v0.21.md)

The current foundation is headless and embeddable. It reads immutable Sources,
prepares a complete rebuildable Spatial Index, resolves progressive display,
renders through a host-owned wgpu lifecycle, stores one narrow class of durable
document Edit, and derives one narrow CPU-authoritative in-memory Terrain
Surface with detached QA and a metric-metre LandXML deliverable. The headless
`terrain-demo` application can run that path through one durable, resumable,
audited Workflow Run without turning orchestration policy into another
foundation crate. The completed v0.13 scope adds a separate rebuildable disk-v1
Surface preparation/reopen path for one explicit AOI while preserving legacy
in-memory Derivation and Run-v1. Its topology phase still retains the complete
AOI in memory. A crate exists only when its behavior, direct tests, and a caller
exist.

The completed v0.14 scope remains inside `point-terrain`. It binds exact Source
residuals, detached Check Points, and station profiles to one frozen
Snapshot/Surface pair, exposes explicit freshness, and compares semantic faces
by authoritative Point Identity. Correction and Revert remain existing
`point-workspace` operations.

The completed v0.15 scope adds one private `browser-demo` host over the
existing `point-view`, `render-protocol`, and `render-wgpu` seams. Those core
paths compile to `wasm32-unknown-unknown`; the browser adapter owns a WebGPU
canvas lifecycle and runs one deterministic generated scene under independent
logical, surface, and transient-texture ceilings. It does not add browser
networking, LAS/LAZ decoding, a supported SDK, or a public browser crate.

The v0.16 implementation remains inside that private host. A trusted deployment
manifest binds one immutable HTTP LAS representation to its fully verified
Source identity and compatible disk-v2 index. JavaScript owns Fetch, Worker,
cache, retry, and recovery policy; the worker validates bounded ranges and
decodes the index root's Sampled Coverage; the Rust Wasm host validates and
publishes bounded renderer batches. Native fixture generation depends on
`source-las` and `point-index`, but the browser runtime adds no foundation-crate
dependency and exposes no public networking or viewer seam.

The v0.17 implementation added its first
coherent framework-neutral host seam: `viewer-api.js` plus matching TypeScript
declarations own lifecycle, camera, rendering, streaming, state, pick,
highlight, and exact-handoff composition. Raw worker and Wasm publication
methods remain private. `viewer-input.js` normalizes bounded input facts without
owning navigation policy, and `exact-query.js` supplies only the fixture's
separate one-record LAS authority.

The v0.18 implementation gives `apps/browser-demo/web` the closed
`@punctra/viewer` package entry, package-relative or explicit Wasm/Worker asset
resolution, packed declarations, generated API documentation, and lifecycle
aliases over the same viewer. `packages/react` may depend only on that package
and React; its hook translates asynchronous mount, resize, active state,
unmount, and replay cleanup without adding UI or another viewer model. The two
checked-in applications under `examples/` are clean packed-artifact trials,
not application modules or broad compatibility claims.

The v0.19 private qualifier records one exact local browser/device lane without
adding a package export or broad support promise. The v0.20 baseline freezes the
packed package, quickstart, fixture, generated-scene, presentation, recovery,
and matrix facts as repository evidence rather than another runtime module.

The accepted v0.21 implementation adds one closed private Visual Trial seam.
Its nine-trial corpus, Autzen derivative, offscreen readback, PNG/USTAR codecs,
comparison, post-capture rubric, baseline-input manifest, and verifier remain
inside `browser-demo` and repository scripts. Record mode creates the baseline
inputs before the implementation pin; only a later attended verify-mode run of
that pinned build can become final evidence. None of these parts is a public
viewer, React, or Rust screenshot/testing interface. Standard Blob download is
the primary TAR transport; an explicitly enabled same-origin local-server
export is only a no-overwrite fallback when an attended in-app browser does not
materialize that download. Final verify provenance comes from the exact pinned
page URL; the runner fixes the accepted attended lane and disables its visible
Run control until the complete pin tuple is valid.

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
    APP --> REV["point-review"]
    APP --> TER["point-terrain"]
    APP --> VIEW["point-view"]
    APP --> RW["render-wgpu"]
    APP --> IDX["point-index"]
    APP --> SRC["point-source"]

    RDEMO["renderer-demo"] --> VIEW
    RDEMO --> RW
    RDEMO --> REV
    RDEMO --> IDX
    RDEMO --> LAS
    RDEMO --> SRC

    BDEMO["browser-demo"] --> VIEW
    BDEMO --> RW
    BDEMO --> RP

    WS --> IDX
    WS --> SRC
    WS --> CT["point-contracts"]
    WS --> RT["foundation-runtime"]

    REV --> WS
    REV --> RP["render-protocol"]

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
- `point-review` owns CPU-authoritative projection of exact Snapshot rows for
  inclusive screen-through rectangles and exact one-Point confirmation. It
  owns no GPU, window, Workspace persistence, or mutation policy.
- `point-terrain` owns Ground Input normalization, robust triangulation,
  canonical `SurfaceVertex`/`SurfaceFace` values, exact Snapshot-bound QA,
  semantic Surface comparison, and the private LandXML encoder and exact-target
  reconciliation. It also owns the
  Surface disk-v1 target/work/stage family, including validation, resume,
  no-replace publication, and conservative offline-cleanup rules.
- `terrain-demo` owns its Run lock, eight-frame journal, cross-module recovery
  policy, Surface Change Envelope, canonical report, read-only Complete-Run
  qualifier, canonical Round-Trip Evidence, and structured actions.
- `render-protocol` owns generation and replacement semantics.
- `point-view` owns deterministic culling, LOD demand, retention, and safe
  retirement decisions.
- `render-wgpu` owns GPU resources and command recording.
- `renderer-demo` owns private display mapping, camera controls, real-cloud
  scheduling/state presentation, `PVIEW_*` diagnostics, and local corpus
  measurement/report policy.

### Exact work and display work stay distinct

Exact Workspace selection and `Snapshot::point_rows` read CPU-authoritative
Source values and apply Revision overlays. Terrain Derivation and
`point-review` consume that complete Point-row stream; neither consumes display
samples. A View may be partial and may use display samples and origin-relative
`f32` coordinates. A GPU pick is only a Pick Hint. `point-review` confirms one
identity or evaluates one complete inclusive screen-through rectangle against
a pinned Snapshot; brush, polygon, visible-only, and occlusion selection remain
unimplemented.

### Durable and rebuildable state stay distinct

Source bytes and immutable Workspace Revision files are authoritative. The
Spatial Index, in-memory `TerrainSurface`, prepared Surface disk-v1 family, and
all View/GPU state are rebuildable or disposable. A LandXML file is a caller-
requested Export, not Workspace state. Deleting an index, Surface, or display
state never deletes an Edit. After Surface publication, the verified stage and
any work sibling remain because identity-conditioned unlink is not portable;
an uninspected work sibling is not trusted, and removal is optional owner-
controlled offline maintenance.

### Limits are part of correctness

Source reads, index operations, selection, Point-row iteration, Point-ID
iteration, Workspace open/commit, Terrain Derivation and preparation, file-
backed Surface streams, exact QA, Surface comparison, and LandXML export each
have explicit hard ceilings.
A limit failure cannot downgrade an exact result to partial Coverage or publish
a partial Surface, report, or durable value.

### Seams must be earned

The Source seam is proven by memory, LAS, and LAZ implementations. The
`point-workspace` seam is proven by its direct example, generated LAS/LAZ
integration, and public interface tests. The `point-review` seam is proven by
public interface tests and the private renderer host that composes it with an
existing Workspace. The `point-terrain` seam is proven by
its public example, package benchmark, interface/resource/topology/QA/LandXML
tests, `exact_terrain_qa` correction-loop example, and `terrain-demo` process
caller. v0.14 adds only the bounded exact-QA and comparison path described
above. COPC, constrained or true out-of-core
terrain, general LandXML, remote reads, polygon/brush/visible-only selection,
general Edits, and application UI remain deferred until an accepted design and
caller earn them.

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

## Current bounded scope

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
- one accepted explicit-AOI preparation path for a rebuildable checksummed
  disk-v1 Surface with input/final checkpoints, warm reopen, and bounded
  file-backed vertex/face streams;
- bounded detached Check Point residual QA;
- one private metric-metre LandXML 1.2 points/faces subset with create-new and
  exact-existing reconciliation;
- exact immutable Revision Audits and Edit Footprints;
- one exact CPU screen-through rectangle and one provisional-pick confirmation
  path, each pinned to an immutable Snapshot and returning a spillable Point
  Set; and
- one private eight-frame `terrain-demo` Workflow Run with canonical report,
  exclusive lock, linked cancellation, and structured recovery actions; and
- one private read-only Complete-Run LandXML verifier with bounded streaming
  comparison and separate canonical pass/fail evidence outside the Run root.

General predicate languages, position or other Attribute Edits, named Point
Sets, branches, merge, compaction, multiple Sources, Breaklines, constrained or
true out-of-core terrain, general export/import, networking, autosave policy,
and product UI remain outside accepted scope. Licensed-data, partner, named
downstream-application, above-500-million-Point, and human-workflow evidence
also remains outstanding.

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
- [First LAS/LAZ guide](../guides/first-las-laz.md)
- [Library packaging and compatibility](../guides/library-packaging.md)
- [Persistent bounded-AOI terrain guide](../guides/persistent-terrain.md)
- [Browser SDK and deployment guide](../guides/browser-sdk.md)
- [Browser qualification and recovery guide](../guides/browser-qualification.md)
- [Packed browser quickstart](../guides/browser-quickstart.md)
- [Browser integration known limitations](../guides/browser-known-limitations.md)
- [Browser visual-quality baseline guide](../guides/browser-visual-quality.md)
