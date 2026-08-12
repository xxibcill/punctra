# Punctra Roadmap

Status: living guidance
Last reviewed: 2026-08-13

This roadmap communicates direction, not a delivery promise. It has no fixed
dates. Candidate releases may be split, merged, reordered, renamed, or skipped
as technical and customer evidence changes. Milestone outcomes and dependency
order matter more than version numbers.

Only an **Active** release has committed scope. Punctra v0.1 through v0.8 are
complete repository technical slices. v0.8 is complete only for the narrow
[repository interoperability qualification
design](docs/design/design-partner-mvp-v0.8.md): a private post-Run semantic
LandXML 1.2 comparison and separate evidence record. Run binding, bounded
streaming comparison, canonical pass/fail evidence publication, and local
repository verification are implemented. All external product evidence remains
outstanding. Broader terrain, export, general editing, downstream automation,
and application UI remain uncommitted.

## Working direction

The working product hypothesis is a local-first sidecar that helps survey and
civil teams move from very large LAS/LAZ Sources to an accepted terrain
deliverable without manual tiling or decimation, while materially reducing
attended production time.

Punctra should contribute reusable, bounded modules to that workflow while
preserving two important boundaries:

- progressive GPU display is disposable and never authoritative geometry; and
- exact Queries, Edits, terrain, and exports operate on CPU-authoritative values
  with explicit provenance.

The hypothesis must be tested against real workflows. A fast viewer alone is
not sufficient product evidence.

## How to use this roadmap

- Use the release themes to choose the next coherent vertical slice, not to
  promise dates.
- Accept a short design before starting a candidate release. That design defines
  its exact scope, non-goals, public seams, and verification gates.
- Preserve the proposed dependency order: contracts and runtime, Source access,
  index, durable document state, terrain, then export.
- Add a crate only with its first behavior, direct interface tests, and at least
  one real caller. Do not scaffold the future tree in advance.
- Keep Cargo/API versions, persisted schema versions, and deterministic
  algorithm versions separate.
- Run all applicable verification locally as documented in
  [CONTRIBUTING.md](CONTRIBUTING.md), including GPU acceptance with
  `PUNCTRA_REQUIRE_GPU=1` when a GPU adapter is expected.
- Let customer evidence narrow, reorder, pause, or end the product-facing work.

Roadmap status labels are:

| Status | Meaning |
|---|---|
| **Complete** | Implemented and verified in the repository. |
| **Active** | Accepted scope; this is the current delivery focus. |
| **Exploring** | Evidence or design work is in progress; implementation scope is not committed. |
| **Candidate** | Plausible later direction, subject to evidence and an accepted scope. |
| **Deferred** | Intentionally outside the current path. |

## Scope and evidence checkpoint

Status: **v0.8 repository interoperability-qualification slice Complete;
product evidence outstanding**

The [implemented v0.5 design](docs/design/durable-document-core-v0.5.md) places
exact classification selection, temporary Point Sets, sparse Revisions, and
Operation recovery behind one deep `point-workspace` interface. Repository
tests and generated-source benchmarks close that technical slice. The
implemented [v0.6 design](docs/design/terrain-qa-benchmark-v0.6.md) adds only
its exact `Snapshot::point_rows` input, one-worker in-memory unconstrained TIN,
detached Check Point residual, and metric-metre LandXML points/faces path. It
does not authorize broader terrain, screen selection, general Edit, or product-
application proposals, and neither v0.5 nor v0.6 repository evidence
substitutes for licensed field data or partner validation.

The implemented v0.7 slice closes the repository restart/audit gap for that
narrow path: caller-owned intent precedes selection/commit, one eight-frame Run
can be resumed or inspected, the changed Revision has an exact Audit/Edit
Footprint, and exact LandXML/report targets reconcile without overwrite. Its
generated tests and benchmark establish only those technical guarantees; they
do not satisfy any external evidence item below.

The implemented v0.8 slice does not broaden that workflow. It adds one private,
post-Run verifier that compares the exact v0.7 metric-metre LandXML TIN with
a caller-returned LandXML 1.2 file under declared tolerances and publish a
separate canonical evidence record. Caller-declared application/version/
settings labels are not proof that the application ran. The v0.7 journal and
`audit.json` remain unchanged, and milestone-start documentation is not
implementation acceptance.

Useful evidence for proceeding includes:

- screen-shared workflows with current users that identify the actual expensive
  step;
- several sanitized production LAS/LAZ datasets from unrelated firms, including
  datasets above 500 million Points;
- customer accuracy, coordinate, QA, and downstream export requirements;
- a measured baseline for time to first use, attended editing time, unattended
  processing time, and rework; and
- a clear reason the proposed workflow is meaningfully better than the
  customer's current toolchain.

If the evidence points elsewhere, revise this roadmap before building the next
module. The detailed discovery signals and pivot criteria live in the
[market-validation research](docs/research/saas-point-cloud-market-validation.md#customer-tests-and-kill-criteria).

## Release sequence

There is no Active repository slice after the completed v0.8 and one
provisional theme follows it. This is a working count, not a requirement to
publish exactly one more release.

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

Status: **Complete — repository interoperability-qualification slice only;
product MVP remains outstanding**

Implemented repository outcome: the exact bounded post-Run verifier and
evidence contract in the [v0.8 design](docs/design/design-partner-mvp-v0.8.md).
The private `terrain-demo` path:

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

This implemented scope does not automate or claim a run through Civil 3D, Bentley
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

Status: **Candidate**

Candidate outcome: qualify the proven scope for a v1 compatibility and support
promise without adding another major feature family.

Likely scope:

- a tested, published CRS, vertical-reference, unit, and precision support
  matrix;
- robust terrain and export edge cases from the production regression corpus;
- persisted-schema migration and recovery fixtures;
- disk exhaustion, corrupt input, cancellation, device loss, and GPU-unavailable
  behavior;
- performance across declared commodity workstation classes;
- public API review, documentation, examples, upgrade notes, and support
  playbooks; and
- local review packages or audit metadata where partners require them.

Evidence of readiness:

- no unresolved correctness or data-loss failure exists in the supported
  workflow;
- every supported persisted version has upgrade and recovery coverage;
- resource ceilings and performance claims are reproducible locally; and
- unsupported formats, transformations, and device capabilities fail clearly.

### v1.0 — Trustworthy supported scope

Status: **Candidate**

Release v1 when the narrow supported workflow repeatedly produces accepted
deliverables, recovery guarantees are proven, resource use is bounded, and the
public compatibility promise can be maintained. Reaching v0.9 is not by itself
a reason to publish v1.

## Product milestone map

| Product milestone | Candidate releases | Outcome |
|---|---|---|
| Renderer and planning foundations | v0.1–v0.2 | Reusable bounded display engine and adaptive View planner. |
| Benchmark/demo | v0.3–v0.6 | Headless technical path from verified LAS/LAZ to one narrow terrain deliverable; external workflow evidence remains separate. |
| Design-partner MVP | v0.7–v0.8 | Complete repository interoperability qualification; product completion still requires three firms, three paid pilots, and two conversion-or-measured-savings firms. |
| Trustworthy v1 | v0.9–v1.0 | Explicitly supported, regression-tested, maintainable compatibility surface. |

## Deferred until evidence changes

The current path does not include:

- E57, scan registration, sensor calibration, or photogrammetry;
- general CAD or BIM authoring;
- AI feature extraction or a broad classification suite;
- automatic CRS or vertical-datum guessing;
- multi-Source Workspaces;
- raw-cloud hosting, cloud collaboration, or distributed execution;
- remote Source reads or networking policy;
- a public plugin registry or generic export framework;
- runtime point schemas, arbitrary shaders, or GPU-authoritative geometry; or
- broad format support added only for completeness.

COPC, bindings, additional export formats, and team-review features may move
forward only when a real caller or partner workflow earns their seams.

## Maintenance

Review this file when a release starts, finishes, or materially changes
direction. A roadmap update should:

1. mark completed evidence rather than only completed code;
2. identify the single Active release, if one exists;
3. move unsupported ideas to Deferred instead of leaving ambiguous promises;
4. link the accepted design or ADR for newly Active scope; and
5. record why a release was split, merged, reordered, or stopped.

The broader candidate architecture is described in
[docs/architecture](docs/architecture/README.md). It is a source of design
constraints and module ordering, not an implementation commitment.
