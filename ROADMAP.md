# Punctra Roadmap

Status: living guidance
Last reviewed: 2026-08-25

This roadmap communicates direction, not a delivery promise. It has no fixed
dates. Candidate releases may be split, merged, reordered, renamed, or skipped
as technical and customer evidence changes. Milestone outcomes and dependency
order matter more than version numbers.

Among incomplete releases, only an **Active** release has accepted
implementation scope. Punctra v0.1 through v0.17 are Complete repository
technical slices. Their recorded external field, adoption, partner, and
support gates remain historically accurate, but they do not determine the next
product direction. The project now pivots from a desktop terrain-delivery
hypothesis to an embeddable browser point-cloud rendering engine.

v0.15 is the completed repository-verified browser-foundation release, v0.16
is the completed repository-verified HTTP Range streaming release, and v0.17
is the completed repository-verified browser-viewer-API release. Versions
v0.18 through v0.30 remain uncommitted Candidate themes. v0.15–v0.20 establish
browser execution, streaming, the viewer API, embedding, and platform qualification. v0.21–v0.29
then improve and qualify visual quality. v0.30 is the earliest planned browser-
engine release candidate; no earlier future release may be represented as a
product release candidate. The historical v0.9 "Trust and v1 candidate" name
describes its bounded repository compatibility checkpoint, not browser-product
release-candidate status.

## Working direction

The working product hypothesis is an embeddable Rust/WebAssembly point-cloud
rendering engine for browser applications. A host should be able to stream a
very large LAS/LAZ Source, render progressive WebGPU Coverage in a canvas, and
integrate navigation, display modes, picking, highlighting, and exact Queries
without adopting a complete editor, desktop product, or terrain-delivery
workflow.

Punctra owns bounded render state, deterministic View planning, and validated
rendering behavior. The browser host owns application UI, credentials, network
policy, storage policy, WebGPU/canvas lifecycle, and product-specific workflow.
Progressive GPU display remains disposable and non-authoritative. Exact Point
values and Queries remain CPU-authoritative with explicit provenance; the
completed Edit, terrain, QA, and export modules remain available but do not
drive the post-v0.14 roadmap.

The hypothesis must be tested through real browser embeddings. Native examples
remain useful reference and GPU-acceptance hosts, but native success alone is
not browser support evidence.

## How to use this roadmap

- Use the release themes to choose the next coherent vertical slice, not to
  promise dates.
- Accept a short design before starting a candidate release. That design defines
  its exact scope, non-goals, public seams, and verification gates.
- Preserve the proposed browser dependency order: WebAssembly/WebGPU execution,
  remote Source delivery, browser View API, SDK integration, platform
  qualification, measured visual-quality work, then release-candidate soak.
- Add a crate only with its first behavior, direct interface tests, and at least
  one real caller. Do not scaffold the future tree in advance.
- Keep Cargo/API versions, persisted schema versions, and deterministic
  algorithm versions separate.
- Run all applicable verification locally as documented in
  [CONTRIBUTING.md](CONTRIBUTING.md), including GPU acceptance with
  `PUNCTRA_REQUIRE_GPU=1` when a GPU adapter is expected.
- Treat a measured corrective checkpoint as evidence for a short design, not
  as permission to silently broaden a completed release or public seam.
- Let adopter evidence narrow, reorder, pause, or end the browser-engine work.

Roadmap status labels are:

| Status | Meaning |
|---|---|
| **Complete** | Implemented and verified in the repository. |
| **Active** | Accepted scope; this is the current delivery focus. |
| **Exploring** | Evidence or design work is in progress; implementation scope is not committed. |
| **Candidate** | Plausible later direction, subject to evidence and an accepted scope. |
| **Deferred** | Intentionally outside the current path. |

Release status and external evidence maturity are separate. A repository
release can be Complete while every product gate remains outstanding.

| Evidence maturity | Meaning |
|---|---|
| **Repository-verified** | The accepted design, local verification, fixtures, and declared benchmarks pass. |
| **Field-qualified** | Historical field evidence satisfies a completed slice's declared production-data and observed-workflow envelope. |
| **Partner-validated** | Historical partner projects repeatedly satisfy the completed slice's declared tolerance and deliverable checks. |
| **Browser-qualified** | Declared browsers, WebGPU adapters, devices, and representative Sources satisfy the measured functional, resource, and visual envelope. |
| **Adopter-validated** | Independent host applications repeatedly complete the documented embedding path without maintainer-only repairs. |
| **Support-qualified** | The declared browser, device, API, packaging, recovery, and support matrices are maintainable. |

## Current pivot checkpoint

Status: **v0.15 through v0.17 complete and repository-verified for their
bounded local browser-foundation, immutable-LAS streaming, and browser viewer
API slices**

The completed v0.1–v0.14 contracts are inputs to the pivot, not permission to
carry the old product sequence forward. In particular:

- `render-protocol`, `render-wgpu`, and `point-view` provide the renderer,
  generation safety, large-world precision, picking, display, and planning
  baseline;
- `point-source`, `source-las`, and `point-index` provide verified Source data
  and bounded progressive materialization, but no browser networking contract;
- `point-review` provides the separation between provisional GPU identity and
  exact CPU confirmation; and
- Workspace, terrain, QA, LandXML, and downstream-verification work remain
  implemented historical modules, not post-v0.14 product priorities.

The completed [v0.15 browser-foundation
design](docs/design/browser-foundation-v0.15.md) fixes the browser build and
packaging path, supported WebGPU capability floor, example JavaScript boundary,
host ownership model, resource accounting, and local browser acceptance
harness. Its generated in-memory scene intentionally precedes the remote
representative LAS/LAZ Source and measured delivery behavior required to
activate v0.16.

The completed [v0.16 HTTP Range streaming
design](docs/design/http-range-streaming-v0.16.md) fixes one immutable remote
LAS deployment profile, strict bounded HTTP Range behavior, disk-v2 index-root
sample decoding in one worker, identity-versioned browser caching, and one
cold/recreation/warm-cache local acceptance path. It intentionally remains in
the private browser host and does not create the later public viewer or SDK.

The completed [v0.17 Browser Viewer API
design](docs/design/browser-viewer-api-v0.17.md) fixes one framework-neutral
viewer façade inside `browser-demo`, matching TypeScript declarations, five
inherited display modes, host-owned camera/input policy, generation-safe
provisional pick/highlight presentation, and a separate immutable-LAS exact-
Point bridge. It is a checked-in integration boundary, not an installable SDK,
framework adapter, arbitrary-Source loader, or browser-support declaration.
Its exact local environment, command matrix, browser facts, and nonclaims are
recorded in the [v0.17 repository verification
record](docs/releases/v0.17.0.md).

## Pre-v0.13 renderer quality corrective checkpoint

Status: **Complete — repository implementation and generated/local GPU
verification complete; permitted real-cloud field execution remains
outstanding**

The [2026-08-18 local renderer quality
investigation](docs/reviews/render-quality-investigation-2026-08-18.md) found a
strong narrow GPU contract and one major unresolved host/View behavior. On the
default stationary 16.7-million-Point synthetic scene, approximately 120,000
resident Points and 2.7 MiB of resident Point vertices remained effectively
unchanged while cumulative uploads, retirements, and cancellations continued
to grow. One ten-second observation added 44.3 MiB of uploads, 1,890 retired
batches, and 441,315 cancelled requests without reaching `steady`.

The same investigation found visible LOD-tile density transitions, grid/moiré
artifacts, weak depth separation, overloaded truncated title telemetry,
effectively invisible three-Point fixture highlighting, missing spatial and
palette context, and an ambiguous `v0.11` View title in a
`0.12.0-alpha.1` workspace. Focused local GPU acceptance still passed for
depth, circular splats, picking, highlighting, projection, large-world
precision, atomic updates, display mappings, and progressive Coverage. The
completed corrective work preserves those contracts rather than disguising the
host churn with visual effects.

This checkpoint is ordered before any new View-dependent product claim or
approved public screenshot. It does not have to delay unrelated headless
evidence collection, but v0.10 cannot become Field-qualified until the
remaining permitted-source and human-interpretation gates are satisfied.

### Corrective outcome

Make the existing progressive View converge, remain visually stable, and
communicate enough depth, spatial meaning, Coverage, and selection state for a
professional to distinguish real features from display and LOD artifacts. GPU
presentation remains disposable; exact Point, selection, Edit, terrain, QA,
and export authority remains on the CPU paths already defined.

### Implemented activation decision

The
[accepted corrective design](docs/design/renderer-quality-corrective-pre-v0.13.md):

- localizes the stationary churn to the owning planner/host/resource seam;
- defines a deterministic stationary convergence contract and frame ceiling;
- selects one point-density transition policy and one bounded optional depth
  cue;
- identifies the minimal inspection context and selection feedback owned by
  the private host;
- names any required public `point-view`, `render-protocol`, or `render-wgpu`
  changes and rejects seams justified only by hypothetical callers; and
- fixes local GPU, image, performance, and real-cloud observation gates without
  claiming field suitability from generated fixtures.

### Ordered workstream A — convergence and resource correctness

Priority: **P1; complete before visual polish**

Implementation status: **Complete for the generated repository convergence
gates.** The exact investigated physical viewport settles at frame 780 under
the accepted 1,024-frame ceiling, then remains unchanged for 300 frames. The
focused movement, projection, reset, resize, pause/resume, refine, and coarsen
cases reconverge. Relevant CPU, rustdoc, package, benchmark, and forced-GPU
checks pass locally; this is not field or workstation qualification.

Implemented scope:

- reproduce the stationary churn in a deterministic multi-frame test before
  changing planner or host behavior;
- account for retained, resident, staged, queued, and in-flight work exactly at
  the seam that owns each resource so a new plan cannot reserve the same
  logical budget repeatedly;
- retain demanded in-flight work across an unchanged camera/viewport instead
  of cancelling and reissuing it;
- prevent a one-batch-per-frame materializer from causing perpetual
  parent/child refinement oscillation;
- define truthful `streaming`, `steady`, `loads-paused`, and transition states;
  and
- preserve generation safety, atomic replacement, fallback Coverage,
  deterministic request priority, and exact conditional retirement.

Repository exit gates:

- the default synthetic camera and viewport reach one deterministic
  resident/in-flight cut within the accepted frame ceiling;
- after the first `steady` frame, at least 300 additional stationary presented
  frames produce zero new requests, uploads, cancellations, retirements, and
  resident-set changes;
- a camera move, projection switch, reset, resize, pause/resume, refine, and
  coarsen each converge again without a Coverage hole or stale request;
- queue, staging, and renderer limits hold throughout convergence and cannot be
  bypassed by already requested work; and
- the before/after investigation records the same generated fixture, viewport,
  budgets, adapter/backend facts, convergence frame, cumulative work, and
  settled observation window.

### Ordered workstream B — LOD, point appearance, and depth legibility

Priority: **P1 for false tile/feature impressions; P2 for depth enhancement**

Implementation status: **Complete — repository implementation and local GPU
acceptance are present. Field image qualification remains part of workstream
D and is not implied by these generated regressions.**

Implemented scope:

- the private host derives a projected-spacing Point diameter from the current
  physical viewport and drawn Point count, clamped deterministically to
  1–4 physical pixels;
- parent retirement is held behind an exact eight-presented-frame color-only
  cross-fade to its resident descendants, then remains version-conditional;
- presentation weight affects only color coverage: depth and provisional pick
  identity continue to use the source Point alpha and geometry;
- optional four-neighbour eye-dome lighting uses one sampleable color target
  plus the existing sampleable depth target, for at most eight transient bytes
  per physical pixel; unsupported target formats select the correct unenhanced
  fallback; and
- protocol state-model tests cover conditional presentation changes, while
  forced local GPU tests cover pick independence, bounded EDL allocation, the
  enhanced path, and the unsupported-format fallback.

Likely scope:

- choose a deterministic projected-spacing-aware Point-size policy or another
  bounded treatment that works across perspective, orthographic, sparse, and
  dense display without changing Point position or identity;
- specify a short bounded parent/child visual transition or another treatment
  that removes holes and reduces conspicuous density steps without keeping
  unbounded duplicate Coverage;
- add one optional bounded display-only depth cue, with eye-dome lighting as
  the leading candidate, plus a capability/fallback path that preserves the
  current correct unenhanced render;
- retain raw neutral, elevation, RGB, intensity, and classification modes, and
  keep any later tone/exposure control explicit, reversible, deterministic,
  and presentation-only; and
- select background and contrast defaults against fixed feature-location
  trials rather than aesthetic preference alone.

Repository exit gates:

- tolerant local GPU image regressions cover circular Point shape, depth,
  parent/child replacement, settled Coverage, projection, large-world origin,
  highlight treatment, and the depth-cue fallback;
- fixed generated views contain no missing tile, false platform caused by a
  prolonged mixed-LOD cut, or density discontinuity above the accepted visual
  tolerance after settlement;
- pick coverage and Point Identity remain independent of decorative depth
  enhancement;
- the selected treatment remains within declared transient texture, retained
  GPU byte, encode-time, and frame-time ceilings; and
- reduced-motion and non-color-only interpretation do not depend on a pulsing
  or palette-only cue.

### Ordered workstream C — inspection context, status, and interaction feedback

Priority: **P2**

Implementation status: **Complete — the primary state is rendered on-canvas;
the compact title uses package metadata and the bounded engineering transcript
is printed separately to standard output.**

Implemented scope:

- an application-private bitmap-glyph overlay renders package-derived View
  version, display mode, projection, streaming state, truthful sampled/
  complete display Coverage, Source/drawn/resident Point counts, and explicit
  non-Query-completion wording;
- exact selection state and count, resident locator count, stale/failure
  recovery wording, and the `X` clear action remain visible without relying on
  highlight color;
- north orientation, a 100-physical-pixel target-plane scale, cursor
  target-plane world coordinates, and a mode-specific palette legend are
  included in the primary panel;
- the panel uses an ASCII 5-by-7 glyph atlas, an opaque contrast backing, a
  48-column bound, and a two-level physical scale that fits the 640-by-480
  logical minimum at 200% interface scaling; and
- detailed planner, queue, staging, resource, frame, upload, Coverage, and
  review facts remain in the standard-output transcript.

Likely scope:

- replace the single diagnostic title dump with a compact primary on-canvas
  status layer and a separate expandable or structured diagnostics surface;
- show display mode, projection, loading/steady state, truthful Coverage,
  drawn/resident Point count, and selection state without truncation;
- add the minimal workflow-earned orientation indicator, scale bar, cursor
  world coordinate, and elevation/classification legend;
- distinguish logical Source Points, drawn Points, resident Points, cumulative
  uploads, and authoritative Query completion in wording and layout;
- expose exact selection count, resident-highlight count, stale/nonresident
  state, clear action, and one unmistakable locator treatment that does not
  change exact CPU selection; and
- derive the displayed package/View-feature version from one truthful source
  instead of a stale hard-coded label.

Repository exit gates:

- sampled, complete, authored, streaming, steady, paused, stale, selected,
  nonresident, failed, and recovered states each have a deterministic host
  fixture or state-model test;
- the primary state remains readable at the minimum supported window size and
  200% interface scaling without truncating its required facts;
- controls and state can be recognized from the window without depending on
  console recall or color alone;
- detailed diagnostics retain every current bounded planner/queue/staging/
  resource fact for engineering evidence; and
- the title/package/version convention is tested against the workspace release
  metadata or an explicitly named feature-slice label.

### Ordered workstream D — permitted real-cloud visual qualification

Priority: **P2; field evidence remains separate from repository closure**

Implementation status: **Repository lane complete — permitted field execution
and human interpretation evidence remain outstanding and cannot be created by
repository tests.**

Implemented scope:

- the bounded private manifest can opt into `pre_v0_13_qualification`, which
  requires five projects from three firms, all five display modes in both
  projections, explicit inspect/measure permission, and the complete declared
  known-feature category matrix;
- every initial or navigated pose must converge within a caller-declared ceiling
  no greater than 1,024 rendered frames, then retain identical node/resource
  state with no planner, host, upload, transition, or retirement work for 300
  additional rendered frames;
- the real-cloud lane exercises the adaptive physical-pixel Point policy,
  eight-frame density transition, and optional EDL/fallback path rather than a
  separate legacy presentation path;
- the private no-replace report records first visible Coverage, settlement
  frame/time, quiet-window completion, resident/peak/transient resources,
  cumulative uploads and lifecycle work, adapter/backend/depth-cue state,
  declared known-feature inputs, their explicit unverified nonclaim, and
  bounded failures; and
- a generated LAS GPU process acceptance proves this lane across the ten mode/
  projection combinations without claiming that generated data is a permitted
  field corpus or human interpretation evidence.

Likely scope:

- run the settled View on permitted LAS/LAZ Sources that contain known terrain
  breaks, vegetation, buildings, scan-pattern variation, low/high intensity
  ranges, and representative classifications;
- exercise neutral, elevation, RGB, intensity, and classification modes in
  both projections on declared workstation classes;
- record time to first visible Coverage, time to settled View, resident and
  peak resources, cumulative uploads, feature-location outcomes, and failures;
- compare raw reference mappings with any explicit display-only tone controls;
  and
- collect user interpretation of sampled/complete, selected/stale, scale,
  orientation, and palette meaning without publishing Source material or
  screenshots absent permission.

Field exit gates:

- the existing v0.10 permitted corpus and known-feature requirements are met;
- observed users locate declared features without mistaking LOD, grid, tone,
  or depth-enhancement artifacts for Source geometry;
- the declared workstation envelope reaches and retains settled Views under
  the accepted resource ceilings; and
- reports separate local repository acceptance from field, partner,
  downstream, adoption, and support evidence.

### Corrective non-goals

This checkpoint does not authorize photorealism, meshes, texture streaming,
globe/3D-Tiles work, rendering every Point simultaneously, GPU-authoritative
geometry, general CAD/BIM authoring, an arbitrary shader/plugin framework,
silent tone mapping, broad clipping/measurement/annotation tools, automatic
CRS guessing, or a general desktop product UI.

## Release sequence

The completed v0.1–v0.14 sequence remains the historical repository baseline.
The browser-engine pivot begins at v0.15. Six platform Candidate themes,
v0.15–v0.20, establish a browser execution and embedding baseline. Nine
visual-quality Candidate themes, v0.21–v0.29, then improve that baseline under
measured image, motion, resource, and interpretation gates. v0.30 is the first
planned browser-engine release candidate.

This is a planning sequence, not a requirement to publish every number.
Candidates may be narrowed, split, merged, reordered, or stopped before
becoming Active, but visual-quality scope must not be pulled into v0.15–v0.20
without an explicit roadmap revision. No v0.15–v0.29 release is a product
release candidate, and completing v0.30 does not automatically publish v1.

### v0.1 — Renderer foundation

Status: **Complete**

- Generation-safe, bounded renderer-neutral updates.
- wgpu rendering, large-world precision, highlighting, and asynchronous picking.
- Host-owned device, queue, encoder, target, and command submission.

Acceptance is recorded in the
[v0.1 renderer design](docs/design/render-engine-v0.1.md).

### v0.2 — Adaptive View foundation

Status: **Complete**

- Deterministic frustum culling and screen-space-error LOD planning.
- Point, byte, and batch budgets with progressive parent Coverage.
- Exact retention and conditional retirement decisions.

Acceptance is recorded in the
[v0.2 planning design](docs/design/adaptive-view-planning-v0.2.md).

### v0.3 — Real Sources

Status: **Complete**

Implemented outcome: read canonical point data through a bounded, reusable Source
interface without involving a Workspace or GPU.

Delivered scope:

- canonical Point, Point Identity, Attribute, coordinate, and provenance
  contracts;
- runtime-neutral bounded Jobs, streams, progress, cancellation, and budgets;
- an in-memory Source adapter for conformance and fault tests;
- LAS point-data record formats 0–10 and bounded LAZ formats 0–8 with preserved
  metadata and Attributes; and
- an explicit unsupported-format result for LAZ formats 9 and 10 until exact
  layered WavePacket14 codec support is available.

Acceptance evidence:

- adapters pass one shared Source conformance suite;
- repeated and differently partitioned reads preserve Point Identity and values;
- corrupt or changed inputs fail explicitly without panic or unbounded
  allocation;
- source-scale decoding has a benchmark and enforced memory ceiling; and
- each module has a directly usable example and a real caller.

Exact scope and verification rules are recorded in the
[v0.3 Real Sources design](docs/design/real-sources-v0.3.md).

The in-memory adapter is directly exercisable through the
[in-memory Source example](crates/source-memory/examples/memory_source.rs):

```bash
cargo run -p source-memory --example memory_source
```

The LAS/LAZ adapter includes a real file inspector and a source-scale
benchmark:

```bash
cargo run --release -p source-las --example inspect -- survey.laz
cargo bench -p source-las --bench read
```

### v0.4 — Out-of-core View

Status: **Complete**

Implemented outcome: Full-verify a supported LAS/LAZ Source, prepare or open a
complete persistent index, and progressively materialize planner demand while
host staging and renderer residency remain bounded.

Delivered scope:

- deterministic fixed-block BVH construction with append-only resumable work
  frames, checksummed complete artifacts, and no-replace atomic publication;
- conservative inclusive-box lookup returning sorted disjoint Source Spans;
- exact Source-backed leaf reads and checksummed bounded internal display
  samples that preserve Source-aware Point Identity and ticks;
- validated fixed-size LAZ chunk seeking across chunk boundaries, with bounded
  sequential fallback for point-wise and variable-chunk streams;
- an application-owned bridge that materializes planner requests and applies
  renderer updates without coupling Source, index, planner, or renderer
  internals;
- a real LAS/LAZ CLI path plus GPU-free build/open/Upsert smoke coverage; and
- source-scale generated benchmarks and measured memory gates.

Repository acceptance evidence:

- candidate lookup has no false negatives against the sequential oracle;
- interruption, valid-prefix recovery, and resumed completion reproduce the
  same descriptor and artifact bytes;
- corrupt, truncated, incompatible, cancelled, and over-budget cases fail
  explicitly without exposing partial artifacts;
- hierarchy output, display samples, View demand, and renderer update order are
  deterministic;
- the one-million-Point generated benchmark produced a 1,971,528-byte artifact
  and a 3,671,504-byte measured peak for the combined candidate/root/leaf read
  path under its 32 MiB gate; and
- local package, documentation, process-smoke, benchmark, and required GPU
  acceptance commands are documented in [CONTRIBUTING.md](CONTRIBUTING.md).

The one-machine generated benchmark does not establish production-scale or
customer value. Runs on licensed production LAS/LAZ datasets, including the
above-500-million-Point evidence requested by the checkpoint, remain
outstanding and must be reported separately rather than inferred from v0.4.

Exact scope and verification rules are recorded in the
[v0.4 Out-of-core View design](docs/design/out-of-core-view-v0.4.md).

### v0.5 — Durable document core

Status: **Complete**

Implemented outcome: make exact classification selections and reversible
classification Edits durable without changing immutable Source bytes.

Delivered scope:

- one deep headless Workspace over one complete Spatial Index and its verified
  Source;
- exact revision-pinned All, inclusive world-box, and bounded explicit-Point-ID
  selection with an optional effective-classification predicate;
- process-scoped immutable Point Sets with bounded automatic spill;
- sparse uniform classification Edits, immutable linear Revisions,
  immediate-head Revert, and crash recovery; and
- durable caller-owned Operation Identity with committed, rejected, retryable,
  not-recorded, and indeterminate reconciliation.

Repository acceptance evidence:

- Point Identity survives Source decode, index, exact Point-ID confirmation,
  Point Set, classification commit, Revert, and reopen;
- forced-spill and hard-budget tests keep memory and temporary storage bounded;
- fault injection at persistence boundaries exposes either the complete old or
  complete new state; and
- recovery and retry by Operation Identity never duplicate a commit.

The package has 61 tests: 19 integration tests through the public interface and
42 unit, fault-injection, and allocation gates. Generated LAS and LAZ fixtures
exercise selection, commit, Revert, reopen, and unchanged Source bytes.
Persistence fault injection covers staging, hard-link, directory-sync, cleanup,
cancellation, panic, and lost-acknowledgement boundaries. The default
one-million-Point generated benchmark and all declared Criterion cases
completed on the named local reference machine; exact selection's separate
131,073-Point worker-equivalent allocation gate peaked at 6,292,224 bytes under
its 64 MiB ceiling, and the one-million-Point forced-spill payload was
9,009,182 bytes. The one-million-Point benchmark reports sampled process RSS
and does not claim worker heap.

Licensed production-cloud, above-500-million-Point, workflow-observation, and
design-partner evidence remain explicitly outstanding. The generated fixture
results do not satisfy those external gates.

Complete screen-through/brush selection, general Attribute or position edits,
durable named Point Sets, Breaklines, branches, merge, and compaction are not
part of v0.5. Exact scope and verification rules are recorded in the
[v0.5 Durable document core design](docs/design/durable-document-core-v0.5.md).

### v0.6 — Terrain and QA benchmark

Status: **Complete — repository technical slice only**

Implemented outcome: complete the first headless LAS/LAZ-to-terrain technical
benchmark on one narrow, explicitly supported workflow.

Delivered scope:

- one narrow exact `Snapshot::point_rows` stream containing Point Identity,
  exact position ticks, and effective `U8` classification;
- one deep `point-terrain` crate deriving a deterministic, unconstrained,
  in-memory 2.5D TIN from an explicit ground class and optional inclusive world
  bounds;
- strict rejection of insufficient, duplicate-XY, conflicting-elevation,
  collinear, over-budget, and otherwise unsupported degenerate input;
- bounded detached Check Point QA whose signed residual is observed Z minus
  interpolated surface Z and whose outside-surface result is an explicit gap;
- reversible ground correction only through the existing classification
  Revision and immediate-head Revert interfaces;
- one private LandXML 1.2 encoder for an atomic create-new, metric-metre,
  one-TIN-Surface points-and-faces subset, independently parsed by
  `roxmltree`; and
- one headless `terrain-demo` application exercising generated LAS and LAZ
  through Workspace, terrain, QA, and export.

The implementation supports one worker. Terrain Surfaces are immutable in-
memory Artifacts and are not persisted or resumable. Public topology uses
canonical `SurfaceVertex` and `SurfaceFace` values. Breaklines, Profiles,
Source residual Queries, classifiers, boundaries/holes, CRS or unit
transformation, non-metre exports, and general LandXML remain outside v0.6.

Evidence of readiness:

- terrain vertices, faces, descriptor hashes, and export semantics are
  deterministic across repeated single-worker runs and Point-row batchings;
- exact Snapshot overlay input, degenerate geometry, cancellation, and every
  resource family have explicit fixture coverage;
- analytic fixtures prove Check Point interpolation, residual sign, boundary
  inclusion, and gaps;
- an independent `roxmltree` path reconstructs the exported points/faces and
  matches the in-memory semantic digest; and
- generated LAS and LAZ complete the headless caller path while Source bytes
  remain unchanged through classification correction and Revert.

The local 10,000-Point generated benchmark measured Derivation at
11.983–12.049 ms (829.97–834.53 Kpoints/s), detached QA at 94.907–95.164 us
for three Check Points and 19,604 face tests, and durable 1,030,118-byte
LandXML creation at 18.020–18.311 ms (53.650–54.518 MiB/s). The descriptor
reported 135,790,592 accounted peak working bytes, 1,034,176 retained Surface
bytes, and 521,494 topology steps; QA reported 336 accounted peak working
bytes. The named `jjaes-MacBook-Pro.local` evidence record separately reported
one-shot Derivation/QA/LandXML times of 13,371/125/14,656 us. These are
algorithm-accounting and local timing facts. `worker_heap_measurement` is
explicitly `null`, so no observed worker-heap value is claimed.

The working product target is five-times faster time to first use and 50% less
human production time on the specific large-project workflows where customer
evidence supports those comparisons. Accuracy cannot be traded for speed.
Licensed production data, Sources above 500 million Points, design-partner
tolerances, downstream Civil 3D/Bentley round trips, paid use, and published
human-time comparisons remain explicitly outstanding and are not v0.6
repository acceptance claims.

Exact interface, invariants, verification, evidence limits, and exclusions are
recorded in the [implemented v0.6
design](docs/design/terrain-qa-benchmark-v0.6.md).

### v0.7 — Design-partner alpha

Status: **Complete — repository technical-readiness slice only; external
design-partner milestone remains outstanding**

Implemented repository outcome: the exact restart, audit, and reconciliation
guarantees in the [v0.7 design](docs/design/technical-alpha-readiness-v0.7.md).
The slice adds no Breaklines or new public foundation crate. It proves that the
existing narrow LAS/LAZ correction-to-terrain path can:

- durably record caller-owned Run and Operation identities before selection or
  commit;
- resume through an eight-frame checksummed Workflow journal and expose
  journal-only `inspect` status;
- link parent cancellation to synchronously awaited child Jobs;
- derive an exact Revision Audit, classification transitions, and Edit
  Footprint from immutable Workspace state;
- ensure byte-identical LandXML and canonical report targets without overwrite;
  and
- emit bounded structured failures naming stage, certainty, known identities,
  and exactly one safe recovery action.

Repository evidence includes 35 `terrain-demo` tests—18 unit/private, 14
workflow-facade, and three process—every eight-frame resume prefix, 12 public
limit families, generated LAS/LAZ semantic-projection checks, scoped fault and
representative cancellation/corruption coverage, known-identity validation,
dropped-Workflow recovery, and a five-mode generated 10,000-Point benchmark.
The completed Run used a 2,804-byte journal and 11,490-byte report with 115
semantic limit facts.

The product-level design-partner alpha outcome is not complete. Partner
tolerances, production datasets, downstream deliverable checks, paid use, and
measured human workflow results remain external evidence gates. The repository
tests are intentionally not relabeled as those facts.

### v0.8 — Design-partner MVP

Status: **Incomplete product alpha — bounded repository verifier/evidence path
implemented by fold-forward work; external MVP evidence outstanding**

The accepted repository outcome was the exact bounded post-Run verifier and
evidence contract in the [v0.8 design](docs/design/design-partner-mvp-v0.8.md).
The implementation provides the private bounded semantic comparison core,
explicitly non-evidence `compare-landxml` command, and the strict read-only
`verify-round-trip` path. The latter:

- require a Complete, unchanged v0.7 Run and leave its eight-frame journal,
  `terrain.xml`, and `audit.json` untouched;
- accept a caller-returned LandXML 1.2 file plus caller-declared downstream
  application, version, settings, and horizontal/vertical metre tolerances;
- parse the original and returned TINs under cumulative hard limits, rejecting
  malformed, unsupported, partial, ambiguous, or raced input without recovery;
- fail closed on unit drift, unmatched or multiply matched vertices, tolerance
  drift, and any added, removed, duplicated, or changed face topology; and
- create or exactly reconcile a bounded canonical Round-Trip Evidence record
  outside the Run root without overwriting different data.

This inherited scope does not automate or claim a run through Civil 3D, Bentley
software, or another named application. Repository-generated XML variants can
complete technical tests only.

The product-level design-partner MVP requires all of these external gates:

- **three distinct firms** use the same supported export path in their actual
  production pipelines without bespoke code repair and accept the deliverable;
- **three distinct paid pilots** have both payment and production-use evidence;
  and
- **two distinct pilot firms** either convert to continuing paid use or
  document measured labor savings sufficient to justify overlapping incumbent
  software.

Multiple runs at one firm count once per gate. Free evaluations, synthetic
runs, declarations, letters of intent, projected savings, and repository test
fixtures do not count. A passing verifier record is necessary technical
evidence for a qualified round trip but alone satisfies none of the three
external gates. Commercial signals guide prioritization; they do not replace
correctness tests.

### v0.9 — Trust and v1 candidate

Status: **Incomplete alpha — repository compatibility, recovery, support
matrix, and independent review complete; local candidate record carried
forward**

Accepted outcome: qualify the proven scope for a v1 compatibility and support
promise without adding another major feature family, as fixed by the
[v0.9 design](docs/design/trust-v1-candidate-v0.9.md).

Committed scope:

- close the inherited v0.8 Complete-Run binding and canonical-evidence gates
  before making any v0.9 readiness claim;
- publish a tested CRS, vertical-reference, unit, precision, format, platform,
  and device support matrix for the existing workflow;
- retain the inherited Spatial Index v1 goldens and complete owner-local
  persisted-v1 compatibility/recovery fixtures without inventing a second
  schema or migration;
- cover disk exhaustion, corrupt input, cancellation, device loss, and GPU-
  unavailable behavior only where the supported module or host seam owns it;
- reproduce performance and resource ceilings on declared local workstation
  classes; and
- review exercised public interfaces, documentation, examples, upgrade notes,
  and support playbooks.

Implemented trust hardening makes `terrain-demo` report Index filesystem
failures as `PWF_IO` with bounded rendering of the operation, path, and
operating-system error. Because the index error does not expose its publication
boundary, certainty is conservatively `indeterminate(index-target)` and
resuming performs the required reconciliation. The first `.work` header is
written and synced under unique ownership, locked, then no-replace linked into
place; unknown or racing paths are preserved and rejected without check-then-
unlink cleanup.

Evidence of readiness:

- no known release-blocking correctness or data-loss failure remains in the
  supported workflow;
- every supported persisted version has frozen reopen and recovery coverage,
  plus upgrade coverage when a second version actually exists;
- resource ceilings and performance claims are reproducible locally; and
- unsupported formats, transformations, and device capabilities fail clearly.

## Standing boundaries for v0.15–v0.30

- Punctra is an embeddable browser rendering engine, not a complete browser
  application, desktop editor, cloud service, or terrain-delivery product.
- The host owns UI, authentication, authorization, credentials, URL policy,
  caching consent, telemetry consent, and application persistence. Punctra may
  expose narrow mechanisms and facts without silently taking over those
  policies.
- GPU display remains disposable and non-authoritative. Picking is provisional
  until confirmed against exact CPU Source values or another caller-owned
  authority.
- Browser support means execution in the declared browser/device/WebGPU matrix.
  Native wgpu tests are necessary regression evidence but cannot substitute for
  browser acceptance.
- v0.15–v0.20 establish functional browser delivery and embedding. Their image
  gates protect existing correctness and prevent regressions; intentional
  visual-quality expansion begins only at v0.21.
- v0.21–v0.29 visual treatments remain deterministic, reversible, bounded, and
  presentation-only. They never change Point Identity, exact position,
  classification, selection membership, Coverage truth, or Query completion.
- A fixed View must reach a declared settled cut before its frame rate, image,
  feature visibility, or resource use is presented as representative. Smooth
  animation during perpetual request/upload/retirement churn is not success.
- Browser memory, retained GPU bytes, transient textures, network requests,
  decoded staging, worker queues, frame work, and cache use have independent
  limits. One aggregate limit may not hide an unbounded subsystem.
- Unsupported WebGPU capability, browser lifecycle state, Source response, or
  cache condition fails explicitly and leaves the host a safe recovery action.
- v0.30 is the earliest future release that may carry a browser-engine release-
  candidate label. No earlier beta, preview, or package is evidence of that
  qualification.
- Every Candidate needs an accepted design with one coherent outcome, explicit
  non-goals, local verification, and a repository-activation decision before
  implementation.

## Browser-engine adoption track

The post-v0.14 adoption path is browser-first while retaining directly usable
Rust libraries. Public attention and repository completeness do not establish
independent adoption. These exit requirements do not expand technical scope.

| Release range | Adoption exit requirement |
|---|---|
| **v0.15–v0.17** | Publish one minimal browser host that renders a representative Source and documents WebGPU capability checks, host ownership, memory limits, progressive Coverage, picking authority, and unsupported states. |
| **v0.18** | Publish the supported SDK packages, generated API reference, one plain TypeScript integration, and only the framework adapters justified by real callers. |
| **v0.19–v0.20** | At least one independent adopter completes the documented embedding path. Record setup time, browser/device facts, failures, unclear APIs, and resulting fixes. v0.20 publishes an accurate browser support and limitation matrix without claiming release-candidate status. |
| **v0.21–v0.25** | Publish reproducible visual fixtures and before/after evidence for each quality release, including settings, camera, viewport, device-pixel ratio, browser, adapter, and fallback state. |
| **v0.26–v0.29** | Independent hosts exercise interaction and visual-quality behavior on the declared browser/device matrix. Approved examples may be showcased only with Source and image permission. |
| **v0.30** | Complete the browser-engine release-candidate review: packages, quickstart, API docs, examples, changelog, security and support policies, compatibility matrix, known limitations, visual evidence, and locally reproducible verification. |

Adoption evidence is counted conservatively. Stars, downloads, praise,
generated examples, and maintainer-run integrations are useful signals but do
not equal independent production use. No benchmark, screenshot, or customer
dataset is published without permission.

### v0.10 — Field qualification and professional inspection View

Status: **Complete — repository implementation and pre-v0.13 renderer-quality
remediation complete; field qualification and adoption publication
outstanding**

Accepted outcome: qualify Source opening and viewing on representative field
data while making known survey features clear enough for professional
inspection, as fixed by the [v0.10
design](docs/design/field-inspection-view-v0.10.md).

Field-qualification gate:

- obtain permission to inspect at least one licensed or sanitized production
  dataset and observe the workflow, workstation, failure mode, and current time
  baseline it represents.

Repository implementation was explicitly activated on 2026-08-12 and now
contains the accepted code, fixtures, local runner, tests, and documentation
path. That does not satisfy the field-qualification gate above; field
qualification and its exit evidence remain outstanding.

Accepted scope:

- RGB, intensity, classification, and elevation display modes;
- bounded versioned display samples carrying only the Attributes needed by the
  selected modes;
- fixed initial point appearance, perspective/orthographic navigation, and
  explicit loading/LOD/Coverage status whose professional suitability still
  requires field observation;
- actionable Source, index, GPU, and resource-limit diagnostics; and
- a reproducible corpus runner for open, index, first-use, navigation,
  residency, memory, and disk measurements.

The completed accepted scope above remains historically accurate. The later
[renderer quality
investigation](docs/reviews/render-quality-investigation-2026-08-18.md) found
that the default stationary synthetic View did not settle and that its LOD
density, depth, status, spatial context, and selection feedback were not yet
strong enough for a professional-inspection claim. Those findings were
remediated by the completed pre-v0.13 checkpoint rather than retroactively
relabeled as v0.10 acceptance. The missing permitted-source and human-
observation evidence remains a v0.10 field gate.

Before field qualification or an approved representative screenshot:

- close the P1 stationary convergence and visually conspicuous LOD-transition
  gates;
- record a settled generated before/after result under identical declared
  inputs;
- expose truthful settled/streaming/Coverage state without relying on a
  truncated title; and
- exercise the refined View on permitted real Sources before claiming feature
  legibility or professional suitability.

Field exit evidence:

- a permitted corpus contains five projects from at least three unrelated
  firms, including at least two Sources above 500 million Points, without
  implying permission to redistribute them;
- every display mode has exact CPU-to-GPU mapping tests and tolerant local GPU
  image regressions;
- declared workstation resource ceilings hold for the measured viewing path;
  and
- observed users can locate known features without mistaking sampled display
  values for exact results.

Open-source adoption state is recorded separately. The repository now has an
accurate public capability description, a reproducible local corpus runner,
and a five-minute first-LAS/LAZ guide. Repository topics/homepage publication,
an approved screenshot or demonstration, and a permitted published benchmark
remain outstanding. A local generated report is not a published production
benchmark.

### v0.11 — Exact interactive review and ground correction

Status: **Complete — repository-verified technical slice only; external
activation/adoption evidence outstanding**

Accepted outcome: connect the progressive View to exact CPU inspection and the
existing reversible ground-classification correction seams without treating
GPU samples as Query authority.

Activation gate:

- v0.10 evidence identifies interactive inspection or classification
  correction as a material source of attended time or rework.

The user explicitly activated the bounded repository implementation on
2026-08-13. That decision does not satisfy the field activation gate above;
the gate remains outstanding and no product-efficacy conclusion is inferred.

Accepted repository scope:

- one public `point-review` crate that confirms a provisional renderer Point
  Identity against a pinned Workspace Snapshot;
- one exact full-CPU-scan, inclusive screen-through rectangle using public
  `Camera` and `Viewport` values, with optional effective-classification
  equality at that pinned Revision;
- renderer highlights derived only by complete bounded exact Point Set
  identity iteration, plus a caller-selected complete highlight-input limit;
- caller-owned Operation Identities and existing durable classification
  commit, immediate-head Revert, Revision Audit/Edit Footprint, and Operation
  reconciliation interfaces;
- one public `render-wgpu` `third_party_host` example and focused rustdoc that
  explain host ownership and provisional-pick confirmation; and
- version, documentation, public-interface tests, a generated exact-review
  benchmark, and required local GPU qualification.

Explicit exclusions:

- polygon, lasso, brush, visible-only, front-most, splat-coverage, or
  occlusion-aware selection;
- arbitrary Attribute or position Edit, Source rewriting, named persistent
  Point Sets, selection algebra, or general undo;
- a general desktop UI, automatic retry/Revert/recovery, or a new persisted
  workflow or format; and
- production, professional-preference, independent-adoption, partner, paid-use,
  reduced-rework, or support claims from repository fixtures and examples.

Repository exit gates:

- every accepted rectangle matches a sequential full-Source CPU oracle,
  including projection boundaries, classification filters, and stale-Revision
  cases;
- Point Identity survives display hint, exact Query, spill, Edit, Revert, and
  reopen;
- highlight vectors are exactly traceable to bounded Point Set iteration rather
  than resident LOD samples or Pick tokens;
- commit, Revert, Audit/Edit Footprint, and every Operation-resolution state
  retain the existing durable semantics; and
- the complete local format, lint, test, rustdoc, fuzz, benchmark, example, and
  required-GPU sequence passes from one exact commit.

External exits remain separate: observe a permitted field workflow that
establishes material correction cost, and record independent adoption of the
public integration path. Neither is supplied by repository completion.

### v0.12 — Explicit spatial-reference contract

Status: **Complete for the bounded repository slice; external activation and
acceptance evidence outstanding**

Repository outcome: one explicit projected survey-coordinate profile now
retains horizontal and vertical EPSG identities,
easting/northing/elevation axes, separate units, precision through exact Source
scale/offset, and provenance without silent assumptions. Complete direct
GeoTIFF metadata can supply it; Workspace reopen, Terrain identity, QA,
LandXML, and round-trip comparison preserve or reject it deterministically.
The supported Terrain/QA/export profile is metre/metre and no transformation is
performed. All public libraries also have the locally exercised package/docs.rs
path required by the v0.12 adoption track.

Activation gate:

- the production corpus identifies one recurring profile that the existing
  metric-metre path cannot represent correctly.

This external activation gate remains unsatisfied. The maintainer activated a
bounded fail-closed repository contract and generated-fixture path; no
untracked example was assigned a missing reference and no field-qualified
profile or transformation is claimed.

Implemented repository scope:

- explicit horizontal reference, vertical reference, axis, unit, precision,
  and provenance metadata at every authoritative boundary;
- deterministic metre-only tolerance and export rules shared by terrain, QA,
  and export, without conversion; and
- clear rejection of missing, ambiguous, unsupported, or contradictory
  metadata.

Repository exit evidence:

- generated support fixtures exercise complete and malformed GeoTIFF metadata,
  frozen reopen compatibility, structured reference identity, and axis/unit
  rejection;
- reopen, Revision, Surface, QA, and export retain the same explicit reference
  identity and declaration provenance;
- unit and axis drift fail closed; and
- no fixture or repository path depends on automatic CRS or datum guessing.

Independent reference-coordinate/control-point comparison, a permitted
production corpus, downstream application observation, publication, adoption,
partner validation, and support qualification remain external exits rather
than repository claims.

### v0.13 — Persistent production-scale terrain

Status: **Complete and repository-verified for the bounded persistent-terrain
slice; field activation, production-scale accuracy, true out-of-core adoption,
independent adoption, partner validation, and support qualification
outstanding**

Accepted repository outcome: add one durable, resumable Surface preparation
path for an explicit inclusive AOI without changing v0.6's exact Ground Input
or canonical full-AOI topology meaning. The prepared handle reopens a
checksummed disk-v1 Artifact and streams bounded canonical vertices/faces;
legacy in-memory `derive` remains available.

Activation gate:

- field measurements establish the required AOI size, ground-Point count,
  latency, memory, temporary storage, and supported workstation classes.

The maintainer activated the bounded repository implementation on 2026-08-19.
That decision does not satisfy the external gate above. The accepted design is
the [v0.13 Persistent Bounded-AOI Terrain
design](docs/design/persistent-production-scale-terrain-v0.13.md).

Accepted repository scope:

- one explicit inclusive `WorldBounds` AOI and the existing single-worker,
  deterministic full-AOI triangulator under hard memory limits;
- complete verified Ground-Input and final-stage checkpoints, resume, safe
  no-replace publication, warm reopen, stale binding detection, and explicit
  rebuild decisions; after publication the verified stage and any work sibling
  remain because identity-conditioned unlink is not portable;
- one immutable checksummed Surface disk-v1 Artifact bound to Snapshot,
  Recipe/AOI, transform, spatial reference, algorithm, and canonical hashes;
- a file-backed prepared handle with separately bounded ordered vertex and face
  streams and attempt/resource facts; and
- a reproducible generated example and report that separate Source
  verification, indexing, cold/resumed/warm Terrain work, and unmeasured phases.

Repository exit gates, completed at implementation commit
`d99ed34324e8938fd0211344fbf65d539bb37178` and recorded in the
[v0.13 verification record](docs/releases/v0.13.0.md):

- the persistent and legacy paths reproduce the same canonical small-fixture
  topology, descriptors, and hashes;
- uninterrupted, resumed, differently batched, and warm-opened runs preserve
  canonical Artifact meaning and complete bytes;
- fault fixtures cover truncation, corruption, cancellation, disk exhaustion,
  checkpoint boundaries, and publication certainty;
- retained memory, staged/work bytes, final Artifact bytes, stream buffers, and
  temporary storage remain within independent hard limits; and
- the complete local verification sequence passes from one exact commit.

External exits remain separate:

- true external-memory/out-of-core triangulation and the corresponding
  open-source adoption exit remain outstanding; and
- at least two above-500-million-Point Source projects from unrelated firms
  complete their declared terrain AOIs and pass their accepted accuracy
  baselines; benchmarks publish that measured envelope without extrapolating
  from Source-viewing or small generated runs.

### v0.14 — Exact terrain QA and correction loop

Status: **Complete and repository-verified for the bounded exact Terrain QA and
correction-loop slice; field activation, observed workflow timing, independent
adoption, partner validation, and support qualification outstanding**

The completed bounded repository scope is defined by the
[v0.14 Exact Terrain QA and Correction Loop design](docs/design/exact-terrain-qa-correction-v0.14.md).
Its exact implementation and local qualification are recorded in the
[v0.14 repository verification record](docs/releases/v0.14.0.md). Repository
completion does not satisfy the external evidence gates below.

Bounded outcome: let a caller locate, explain, correct, and recheck terrain
defects without treating display colors as measurements.

Activation gate:

- observed acceptance work identifies the exact QA views, tolerances, and
  reports that change a deliverable decision or reduce repeated inspection.

Completed repository scope:

- exact profiles or cross-sections for the accepted workflow;
- Source-Point residual Queries, detached Check Point results, and bounded
  visualizations of those authoritative values;
- stale-Surface and changed-region tracking after classification correction;
- repeatable correct, re-derive, compare, and Revert flow; and
- QA evidence with explicit units, gaps, tolerances, Snapshot, Surface, and
  operation provenance.

Repository exit evidence:

- numeric results match analytic and independent reference fixtures within
  declared tolerances;
- every displayed profile or residual resolves to an authoritative frozen
  Snapshot/Surface pair;
- stale results cannot be presented as current after an Edit; and
- observed trials measure time to find, explain, and correct seeded or known
  defects.

### v0.15 — WebAssembly and WebGPU browser foundation

Status: **Complete — repository implementation and exact local browser/GPU
verification complete; remote delivery, broad browser qualification,
independent adoption, SDK stability, and support qualification outstanding**

Completed outcome: render one deterministic Punctra scene inside a browser
canvas through a bounded WebAssembly/WebGPU path.

Activation gate:

- satisfied by the accepted [WebAssembly and WebGPU Browser Foundation
  design](docs/design/browser-foundation-v0.15.md), which selects the Rust-to-
  WebAssembly toolchain, example JavaScript boundary, WebGPU capability floor,
  browser test harness, and canvas/device ownership model.

Implemented scope:

- `wasm32`-compatible renderer, protocol, math, and View-planning paths;
- explicit asynchronous initialization and capability diagnostics;
- browser-safe frame recording, resize, device-pixel-ratio, visibility, and
  shutdown behavior; and
- one local static browser host using generated in-memory data, without remote
  Source delivery or framework integration.

Repository exit evidence:

- the declared browser opens the example and renders the fixed scene through
  WebGPU without native-only shims;
- protocol generation, batch-version, resource-limit, and picking invariants
  match the native reference path;
- unsupported capabilities fail before partial publication with one safe host
  action; and
- package and browser tests run locally without claiming broad browser support.

### v0.16 — HTTP range streaming, browser caching, and worker decoding

Status: **Complete and repository-verified for one bounded immutable-LAS HTTP
Range, browser-cache, and worker-decoding slice; arbitrary Source delivery,
exact browser Queries, broad browser qualification, independent adoption, SDK
stability, and support qualification outstanding**

Outcome: progressively view a remote LAS/LAZ Source without loading
the complete file or blocking the browser main thread.

Activation:

- satisfied by the accepted [HTTP Range Streaming, Browser Caching, and Worker
  Decoding design](docs/design/http-range-streaming-v0.16.md), which records one
  representative immutable LAS fixture, exact hosting/CORS/Range behavior, a
  compatible disk-v2 Spatial Index, explicit cache policy, fixed per-task work
  ceilings, and the required local browser observations.

Implemented scope:

- bounded Fetch/HTTP Range reads with explicit status, length, validator, CORS,
  content-encoding, cancellation, retry, and changed-Source handling;
- one deployment profile binding an immutable remote Source to a compatible
  Punctra Spatial Index and display-sample recipe;
- bounded Web Worker decoding and transfer with request backpressure;
- host-selected memory and persistent-cache policy with versioned cache keys and
  explicit invalidation; and
- progressive index/sample delivery that preserves Coverage and Point Identity
  contracts.

Repository exit evidence:

- request count, concurrent bytes, decoded staging, worker queues, cache bytes,
  cancellation latency, and main-thread work stay under independent limits;
- missing Range support, validator drift, truncation, corruption, offline state,
  quota failure, and worker failure produce deterministic recoverable outcomes;
- reload and warm-cache behavior never combine bytes from different Source
  identities; and
- progressive rendering starts before full Source transfer on the declared
  fixture. An arbitrary raw LAS/LAZ URL without a compatible index is rejected
  rather than silently downloaded or scanned in full.

### v0.17 — Browser viewer API

Status: **Complete and repository-verified for one bounded framework-neutral
browser viewer API and immutable-LAS exact-Point bridge; SDK packaging,
arbitrary Sources and Queries, broad browser qualification, independent
adoption, API stability, and support qualification outstanding**

Outcome: expose one coherent browser API for camera control, display
modes, picking, highlighting, and exact Query handoff.

Activation:

- satisfied by the accepted [Browser Viewer API
  design](docs/design/browser-viewer-api-v0.17.md), derived from the v0.16
  example's smallest real-host needs without exposing renderer, planner,
  worker, or Source-publication internals separately.

Implemented scope:

- typed lifecycle, camera, perspective/orthographic projection, viewport,
  display-mode, render scheduling, and bounded state-report interfaces;
- provisional GPU picking and highlighting with explicit Point Identity and
  generation semantics;
- a separate asynchronous exact-Query bridge that cannot promote display
  samples to authority; and
- normalized pointer, wheel, keyboard, and touch inputs as optional mechanisms,
  while host applications retain interaction policy.

Repository exit evidence:

- a plain browser host implements navigation, five inherited display modes,
  pick, highlight, clear, and exact confirmation using only the public API;
- stale generations, destroyed viewers, device loss, cancelled Queries, and
  nonresident highlights fail without stale presentation;
- TypeScript declarations and runtime errors agree; and
- no terrain, export, editor, framework, or application-UI policy enters the
  viewer interface.

### v0.18 — Embeddable SDK and framework integration

Status: **Candidate**

Candidate outcome: make the browser viewer repeatable to install, bundle,
instantiate, and dispose inside real web applications.

Activation gate:

- at least two embedding trials identify the actual package, bundler, worker,
  asset-URL, and framework-lifecycle requirements.

Likely scope:

- versioned ES-module/WebAssembly packages with generated TypeScript types;
- one framework-neutral integration and only the framework adapters justified
  by observed adopters;
- explicit worker and WebAssembly asset resolution for supported bundlers;
- lifecycle-safe mount, resize, pause, resume, and dispose behavior; and
- Content Security Policy, cross-origin isolation, and deployment guidance for
  the features that actually require them.

Candidate exit evidence:

- clean example applications install from packed artifacts rather than
  repository-relative paths;
- development, production, code-split, worker, and cache-busted builds reproduce
  the same public behavior;
- repeated mount/unmount and hot-reload trials leak no owned worker, listener,
  canvas resource, or GPU allocation; and
- framework adapters remain thin translations over the same viewer API.

### v0.19 — Browser and device qualification

Status: **Candidate**

Candidate outcome: define and reproduce the functional, performance, recovery,
and support envelope of the browser engine before visual-quality expansion.

Activation gate:

- v0.18 adopters provide a bounded set of browsers, operating systems, GPU
  classes, integrated/discrete devices, and mobile expectations worth
  supporting.

Likely scope:

- an explicit browser/OS/adapter/capability matrix with tested fallback and
  unsupported states;
- repeatable first-Coverage, settled-View, frame-time, network, worker, CPU,
  JavaScript heap, retained GPU, and cache measurements where the platform can
  report them truthfully;
- device-loss, tab-backgrounding, resize, zoom, DPR change, network loss,
  offline/cache, memory-pressure, and worker-crash recovery; and
- browser-facing diagnostics, issue evidence, support playbooks, and security
  review for remote Source handling.

Candidate exit evidence:

- the declared functional suite passes on every supported matrix entry and
  unsupported entries fail before use;
- fixed workloads remain within declared resource and latency ceilings;
- recovery never displays stale generations or combines changed Source data;
  and
- v0.19 preserves the inherited visual baseline rather than introducing new
  appearance policies.

### v0.20 — Stable browser-engine integration baseline

Status: **Candidate — not a release candidate**

Candidate outcome: consolidate v0.15–v0.19 into one documented and independently
embeddable functional baseline before visual-quality work begins.

Activation gate:

- v0.19 is Browser-qualified for its declared matrix and at least one
  independent adopter has completed the v0.18 embedding path.

Likely scope:

- close release-blocking correctness, lifecycle, recovery, packaging, and
  documentation gaps in the accepted browser surface;
- review public API depth and remove accidental seams without promising final
  pre-v1 compatibility;
- publish a browser quickstart, embedding guide, capability matrix, performance
  report, recovery guide, and exact known limitations; and
- freeze the input scenes, browser matrix, and existing visual behavior that
  v0.21 will use as its measured starting point.

Candidate exit evidence:

- an adopter can install, stream, render, navigate, pick, highlight, query,
  recover, and dispose without maintainer-only patches;
- the complete functional and resource suite reproduces from packed artifacts;
- no known release-blocking correctness, security, data-mixing, lifecycle, or
  recovery defect remains in the declared baseline; and
- v0.20 is described as an integration baseline, not a beta, release candidate,
  v1 promise, or claim of completed visual quality.

### v0.21 — Visual-quality baseline and regression corpus

Status: **Candidate**

Candidate outcome: establish reproducible evidence for visual changes before
changing point appearance.

Activation gate:

- v0.20 is complete and its fixed scenes expose representative sparse, dense,
  layered, high-dynamic-range, classification, large-world, and mixed-LOD
  viewing conditions.

Likely scope:

- fixed generated and permitted real-Source camera trials with immutable input
  facts;
- browser image capture, tolerant comparison, temporal-difference, Coverage,
  feature-location, and resource reporting;
- declared viewport, DPR, browser, adapter, color-space, projection, mode,
  settling, and capability/fallback facts; and
- a small human interpretation rubric for depth, shape, density transitions,
  color meaning, selection, and false-feature impressions.

Candidate exit evidence:

- the same accepted inputs reproduce comparable evidence without hiding
  adapter-specific variation;
- unstable pixels and allowed tolerances are bounded and explained;
- every later visual-quality claim can cite a v0.21 baseline; and
- screenshots remain evidence, never authoritative geometry.

### v0.22 — Point footprint and edge quality

Status: **Candidate**

Candidate outcome: make individual Points and dense Point coverage read cleanly
across DPR and zoom without changing geometry or picking authority.

Likely scope:

- deterministic anti-aliased Point footprints and projected-size behavior;
- bounded sparse/dense sizing that avoids avoidable holes, blobs, square edges,
  and moiré;
- explicit capability fallback with the same geometry and identity; and
- image and frame-cost comparison against the v0.21 baseline.

Candidate exit evidence:

- fixed trials improve accepted edge and density metrics without hiding thin
  features or creating false surfaces;
- pick coverage remains defined independently of decorative edge treatment;
- physical-pixel behavior is stable across the declared DPR range; and
- shader, transient-memory, and frame-time costs remain under declared limits.

### v0.23 — LOD density and transition continuity

Status: **Candidate**

Candidate outcome: reduce visible tile boundaries, density popping, holes, and
mixed-LOD false features during refinement, coarsening, and motion.

Likely scope:

- evidence-selected refinement presentation beyond the inherited bounded
  cross-fade;
- transition-aware point density and parent/child coverage treatment;
- deterministic motion, stop, settle, refine, coarsen, and cancellation rules;
  and
- exact duplicate-Coverage, transition-byte, and transition-frame ceilings.

Candidate exit evidence:

- fixed moving and stationary trials remain hole-free and settle without churn;
- tile edges and density steps stay below accepted image and interpretation
  thresholds;
- transitions never change Query completion or Point Identity; and
- rapid camera changes cannot retain unbounded duplicate Coverage.

### v0.24 — Depth and shape legibility

Status: **Candidate**

Candidate outcome: improve separation of terrain, structures, vegetation, and
overlapping layers without implying geometry that is absent from the Source.

Likely scope:

- evidence-selected depth enhancement building on the inherited bounded EDL
  path;
- projection-aware depth parameters and large-world depth precision;
- clear fallback when required texture, format, or sampling capabilities are
  unavailable; and
- reversible controls whose defaults are justified by feature-location trials.

Candidate exit evidence:

- users locate declared shapes more reliably without mistaking enhancement
  halos or occlusion artifacts for Source geometry;
- pick, depth test, Point Identity, and exact position remain unchanged;
- sparse, dense, perspective, and orthographic cases have explicit regressions;
  and
- transient texture and frame-time ceilings hold on the supported matrix.

### v0.25 — Color, tone, and attribute legibility

Status: **Candidate**

Candidate outcome: make RGB, intensity, elevation, classification, and neutral
display predictable and readable across supported browsers and displays.

Likely scope:

- explicit linear/sRGB handling and browser canvas color assumptions;
- deterministic reversible exposure, contrast, and intensity/elevation range
  controls selected from measured cases;
- accessible palettes and non-color-only legends for categorical and continuous
  modes; and
- an always-available raw/reference mapping for comparison.

Candidate exit evidence:

- numeric CPU-to-GPU mappings and captured colors match declared transfer rules
  within tolerance;
- clipping, banding, washed-out RGB, and indistinguishable classification cases
  improve against v0.21 fixtures;
- no silent normalization changes the meaning of repeated views; and
- fallback and raw modes remain deterministic across the supported matrix.

### v0.26 — Camera and temporal visual stability

Status: **Candidate**

Candidate outcome: keep the image spatially and temporally stable during
navigation, projection changes, large-world origin changes, and settling.

Likely scope:

- quantified jitter, shimmer, popping, and origin-rebase behavior;
- deterministic camera interpolation and optional host-selected motion damping;
- projection-switch and reset treatments with reduced-motion behavior; and
- cross-frame diagnostics that distinguish camera, LOD, streaming, and shader
  causes.

Candidate exit evidence:

- fixed camera paths remain below accepted temporal-difference and position-
  stability thresholds;
- stop and reset converge to the same canonical View cut;
- reduced-motion mode does not rely on animation to communicate state; and
- no temporal treatment retains stale geometry or weakens generation safety.

### v0.27 — Selection, highlighting, and locator clarity

Status: **Candidate**

Candidate outcome: keep selected, provisional, exact, stale, and nonresident
states visually distinguishable in sparse and dense scenes.

Likely scope:

- bounded highlight, outline, locator, and selection-state treatments that do
  not rely on color alone;
- clear provisional-pick versus exact-confirmation presentation facts;
- dense-selection and nonresident-selection degradation rules; and
- host-composable state needed for accessible DOM labels or legends.

Candidate exit evidence:

- interpretation fixtures distinguish every supported state at minimum size,
  high DPR, and supported contrast modes;
- visual selection never changes exact membership or promotes a sampled Point;
- large selections remain within highlight and frame-time limits; and
- clearing, staleness, device loss, and recovery leave no ghost highlight.

### v0.28 — Canvas composition and adaptive presentation

Status: **Candidate**

Candidate outcome: preserve visual intent when Punctra is embedded in varied
browser layouts, canvas sizes, backgrounds, and device conditions.

Likely scope:

- explicit opaque and transparent composition behavior where WebGPU permits it;
- deterministic resize, DPR, browser zoom, fullscreen, and responsive-layout
  presentation;
- bounded host-selected quality tiers that degrade declared visual treatments
  without changing authoritative behavior; and
- high-contrast, reduced-motion, and non-color-only presentation hooks.

Candidate exit evidence:

- composition fixtures cover supported alpha/background, size, DPR, and quality
  combinations without stale or stretched frames;
- adaptive quality respects resource ceilings and cannot silently alter Source
  or Query meaning;
- minimum supported canvases retain required visual state; and
- unsupported composition paths fail explicitly rather than rendering a
  misleading image.

### v0.29 — Cross-browser visual qualification and hardening

Status: **Candidate — not a release candidate**

Candidate outcome: freeze the accepted visual feature set and prove it is
maintainable across the supported browser/device matrix.

Activation gate:

- v0.21–v0.28 evidence identifies the exact visual defaults, optional controls,
  fallbacks, and quality tiers worth supporting.

Likely scope:

- full visual, temporal, interaction, fallback, resource, and recovery
  reproduction across the declared matrix;
- closure or explicit deferral of open release-blocking visual defects;
- reviewed shader/material interfaces, defaults, diagnostics, examples, and
  visual migration notes; and
- independent interpretation trials on permitted Sources.

Candidate exit evidence:

- supported matrix entries reproduce accepted images and temporal behavior
  within declared tolerances;
- no known release-blocking false-feature, Coverage, depth, color, selection,
  motion, or composition defect remains;
- every fallback is visible in diagnostics and covered by acceptance; and
- v0.29 is a qualification checkpoint, not a browser-engine release candidate.

### v0.30 — Browser render-engine release candidate

Status: **Candidate — earliest planned release candidate**

Candidate outcome: hold the qualified functional and visual scope stable long
enough to decide whether it deserves a maintained compatibility and support
promise.

Activation gate:

- v0.20's browser integration baseline and v0.29's visual-quality baseline are
  Support-qualified, and independent adopters have scheduled representative
  production embeddings inside the declared matrix.

Likely scope:

- functional and visual feature freeze, release-candidate packaging, API and
  asset upgrade rehearsal, rollback guidance, and extended browser soak;
- full local WebAssembly, browser, native GPU, image, fault, resource,
  performance, recovery, packaging, and compatibility reproduction;
- user, integrator, security, recovery, support, and upgrade documentation; and
- final review of support capacity, known limitations, compatibility promises,
  and deferred scope.

Candidate exit evidence:

- independent hosts repeatedly install and run representative Sources without
  maintainer-only code repair inside the supported matrix;
- package upgrade, cache invalidation, rollback, device loss, Source change,
  and diagnostics pass their declared rehearsals;
- no known release-blocking correctness, security, recovery, resource, visual,
  packaging, or support defect remains; and
- the evidence record supports an explicit ship, extend-soak, narrow, or stop
  decision. Completing v0.30 does not automatically publish v1.

### v1.0 — Trustworthy supported browser engine

Status: **Candidate**

Release v1 only after the v0.30 candidate survives its declared production soak,
independent browser hosts repeatedly reproduce the supported path, resource use
remains bounded, visual evidence remains stable, and the public compatibility
promise can be maintained. Neither the historical v0.9 repository checkpoint
nor completion of v0.20 is a reason to publish v1.

## Product milestone map

| Product milestone | Release range | Delivery state | Outcome |
|---|---|---|---|
| Renderer and planning foundations | v0.1–v0.2 | Complete; repository-verified | Reusable bounded display engine and adaptive View planner. |
| Source, document, terrain, and workflow baseline | v0.3–v0.7 | Complete; repository-verified only | Headless technical path from verified LAS/LAZ to one narrow resumable terrain deliverable; field and product evidence remains separate. |
| Qualifier and trust baseline | v0.8–v0.9 | Complete; repository-verified only | Close inherited qualification gates and harden only the existing narrow repository compatibility surface. |
| Field inspection and exact correction | v0.10–v0.11 | Repository implementation complete; renderer quality corrected; field and adoption exits outstanding | Qualify representative Sources and connect a professional View to CPU-authoritative review and reversible correction. |
| Renderer quality corrective checkpoint | pre-v0.13 | Complete; repository-verified only, with permitted field execution outstanding | Stationary LOD converges, bounded density/depth treatments and truthful inspection context are implemented, and the private permitted-source lane records settled evidence without manufacturing field claims. |
| Spatial contract and production terrain | v0.12–v0.13 | v0.12 bounded repository contract complete; v0.13 bounded persistent-terrain slice complete and repository-verified; external spatial, production-scale, out-of-core, adoption, partner, and support exits outstanding | Make reference semantics explicit, then persist one bounded-AOI Surface without confusing repository durability with field-scale qualification. |
| Terrain acceptance tooling | v0.14 | Complete and repository-verified for the bounded slice; external historical exits outstanding | Preserve exact Terrain QA and correction as an available module without extending it in the current browser-engine path. |
| Browser execution and streaming | v0.15–v0.16 | Complete and repository-verified for the bounded private slices; arbitrary delivery and external qualification outstanding | Establish WebAssembly/WebGPU execution, then bounded remote Source delivery, browser caching, and worker decoding. |
| Browser viewer and embedding | v0.17–v0.20 | v0.17 bounded viewer API complete and repository-verified; v0.18–v0.20 Candidate | Expose the viewer API, package the SDK, qualify the browser/device envelope, and consolidate a stable integration baseline without release-candidate status. |
| Measured visual quality | v0.21–v0.25 | Candidate | Establish visual evidence, then improve point footprints, LOD continuity, depth, and color. |
| Visual interaction and qualification | v0.26–v0.29 | Candidate | Improve temporal, selection, and composition clarity, then freeze and qualify the visual surface without release-candidate status. |
| Browser-engine release candidate | v0.30 | Candidate; earliest planned release candidate | Freeze, soak, and explicitly decide whether the supported browser engine should ship or narrow. |
| Trustworthy supported browser engine | v1.0 | Candidate after v0.30 soak | Publish v1 only when independent use and maintainable functional, visual, compatibility, resource, and support evidence justify the promise. |

## Deferred until evidence changes

The current path does not include:

- E57, scan registration, sensor calibration, or photogrammetry;
- general CAD or BIM authoring;
- AI feature extraction or a broad classification suite;
- automatic CRS or vertical-datum guessing;
- broad coordinate-transformation coverage added without a selected workflow;
- multi-Source Workspaces;
- a hosted point-cloud service, authentication system, collaboration backend,
  or distributed processing platform;
- Punctra-owned credential, authorization, telemetry-consent, or application
  persistence policy;
- new terrain constraints, broader terrain authoring, or automatic correction;
- new downstream export work, named Civil 3D/Bentley qualification, a generic
  export framework, or a simultaneous downstream compatibility promise;
- a complete browser application, editor UI, design system, or framework-owned
  product shell;
- a public plugin registry or arbitrary shader execution;
- Cesium visual/platform parity, globe-scale 3D Tiles, global imagery or
  terrain, texture streaming, photorealistic meshes, or rendering every Point
  simultaneously;
- runtime point schemas or GPU-authoritative geometry; or
- broad format support added only for completeness.

COPC, additional Source formats, collaboration, terrain expansion, and export
features may move forward only when a real browser adopter earns their seams
and the roadmap is explicitly revised.

## Maintenance

Review this file when a release starts, finishes, or materially changes
direction. A roadmap update should:

1. record repository delivery status and external evidence maturity separately;
2. identify the single Active release, if one exists;
3. move unsupported ideas to Deferred instead of leaving ambiguous promises;
4. link the accepted design or ADR for newly Active scope; and
5. record why a release was split, merged, reordered, or stopped;
6. update browser and open-source adoption evidence without treating popularity
   metrics as product, correctness, or release acceptance;
7. link measured corrective investigations and carry their unresolved P1/P2
   gates into the owning release or pre-release checkpoint without converting
   local generated observations into browser, adopter, or support evidence; and
8. preserve the v0.15–v0.20 functional / v0.21–v0.29 visual-quality boundary and
   the rule that v0.30 is the earliest browser-engine release candidate unless
   an explicit roadmap decision changes them.

The completed foundation architecture is described in
[docs/architecture](docs/architecture/README.md). It constrains ownership and
module ordering, but it does not yet describe or authorize the Candidate
browser runtime, networking adapter, viewer API, or SDK.
