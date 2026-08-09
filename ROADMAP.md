# Punctra Roadmap

Status: living guidance
Last reviewed: 2026-08-10

This roadmap communicates direction, not a delivery promise. It has no fixed
dates. Candidate releases may be split, merged, reordered, renamed, or skipped
as technical and customer evidence changes. Milestone outcomes and dependency
order matter more than version numbers.

Only an **Active** release has committed scope. Punctra v0.1 through v0.3 are
complete, and v0.4 is active under its accepted design. Work beyond that design
remains proposed until it receives a separate accepted design or architecture
decision. In particular, Workspace behavior, editing, terrain, export, and
general application UI are not silently made current scope by appearing here.

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

Status: **Active for the narrow v0.4 technical slice; external product evidence remains outstanding**

The [accepted v0.4 design](docs/design/out-of-core-view-v0.4.md) places one
rebuildable Spatial Index in Punctra and keeps View materialization in the host
demo. It does not authorize the broader Workspace, Query, Edit, terrain, or
product-application proposal. Those later boundaries still require their own
evidence and accepted designs.

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

There are six provisional pre-v1 release themes after the completed v0.3. This
is a working count, not a requirement to publish exactly six more releases.

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

Status: **Active**

Accepted outcome: open a real large LAS/LAZ Source progressively and keep
display residency bounded.

Accepted scope:

- persistent, rebuildable, resumable Spatial Index construction;
- deterministic conservative spatial lookup and View hierarchy facts;
- an application-owned bridge that materializes planner requests and applies
  renderer updates without coupling Source, index, planner, or renderer
  internals; and
- real-data time-to-first-visible and steady-state residency measurements.

Evidence of readiness:

- index lookup has no false negatives against a sequential reference oracle;
- interruption and recovery produce the same completed index;
- hierarchy output and View plans remain deterministic;
- large real Sources stay inside point, byte, batch, and host-memory budgets;
  and
- the real-cloud demo reports reproducible latency, throughput, and peak memory.

Exact scope and verification rules are recorded in the
[v0.4 Out-of-core View design](docs/design/out-of-core-view-v0.4.md).

### v0.5 — Durable document core

Status: **Candidate**

Candidate outcome: make exact selections and reversible classification Edits
without changing immutable Source bytes.

Likely scope:

- exact revision-pinned spatial and attribute Queries;
- spillable, immutable Point Sets;
- sparse classification Edits, immutable Revisions, undo, and crash recovery;
- coherent Source, index, Snapshot, and operation lifecycle through a Workspace;
  and
- exact CPU confirmation of provisional display picks.

Evidence of readiness:

- Point Identity survives decode, index, View, pick, Query, Point Set, commit,
  and reopen;
- forced-spill and hard-budget tests keep memory and temporary storage bounded;
- fault injection at persistence boundaries exposes either the complete old or
  complete new state; and
- recovery and retry by Operation Identity never duplicate a commit.

### v0.6 — Terrain and QA benchmark

Status: **Candidate**

Candidate outcome: complete the first end-to-end benchmark/demo milestone on a
narrow, explicitly supported workflow.

Likely scope:

- deterministic CPU-authoritative TIN derivation;
- narrow Breakline, profile, residual, or check-point QA needed by benchmark
  partners;
- reversible ground correction using existing classification, with any
  provisional classifier kept narrow and evidence-led;
- one constrained LandXML export path with independent validation; and
- a minimal host application that exercises LAS/LAZ through terrain delivery.

Evidence of readiness:

- terrain topology is deterministic across runs and supported worker counts;
- degenerate geometry and resource limits have explicit fixture coverage;
- exports independently parse and round-trip through the declared downstream
  application versions; and
- a published comparison measures time to first use, human attention, accuracy,
  and accepted-deliverable time—not only frame rate.

The working product target is five-times faster time to first use and 50% less
human production time on the specific large-project workflows where customer
evidence supports those comparisons. Accuracy cannot be traded for speed.

### v0.7 — Design-partner alpha

Status: **Candidate**

Candidate outcome: turn the benchmark path into something partners can use on
real projects while keeping the supported workflow narrow.

Likely scope:

- robust Breakline and terrain-edit edge cases found in partner data;
- visible QA, changed-region tracking, and classification audit information;
- autosave, cancellation, recovery, and actionable diagnostics;
- workflow UX driven by observed partner use; and
- regression coverage across roughly 5–10 representative production datasets.

Evidence of readiness:

- partner tolerance and deliverable checks pass repeatably;
- interrupted long operations recover without ambiguous visible state;
- failures identify the Source, operation, phase, and safe recovery action; and
- new requests are rejected or deferred when they do not strengthen the chosen
  workflow.

### v0.8 — Design-partner MVP

Status: **Candidate**

Candidate outcome: complete the design-partner MVP milestone and demonstrate
that the workflow has commercial value, not only technical novelty.

Likely scope:

- reliable round trips for the explicitly supported Civil 3D or Bentley
  versions and settings;
- install, update, licensing, and support diagnostics if a distributable product
  is in scope;
- documented limits and recovery procedures; and
- partner-specific polish that generalizes across the supported datasets.

Evidence of readiness:

- the same export path works in at least three firms' actual pipelines without
  bespoke code repair;
- at least three paid pilots provide production evidence;
- at least two partners convert or document enough labor savings to justify
  overlapping incumbent software; and
- supported workflows pass the full local verification and partner regression
  suites.

Commercial signals guide prioritization; they do not replace correctness tests.

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
| Benchmark/demo | v0.3–v0.6 | Measured real-data path from LAS/LAZ to one narrow accepted terrain deliverable. |
| Design-partner MVP | v0.7–v0.8 | Recoverable production workflow validated by partner datasets and paid use. |
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
