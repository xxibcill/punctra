# Punctra Roadmap

Status: living guidance
Last reviewed: 2026-08-13

This roadmap communicates direction, not a delivery promise. It has no fixed
dates. Candidate releases may be split, merged, reordered, renamed, or skipped
as technical and customer evidence changes. Milestone outcomes and dependency
order matter more than version numbers.

Among incomplete releases, only an **Active** release has accepted
implementation scope. Punctra v0.1 through v0.9 are Complete repository
technical slices. The v0.8 interoperability qualification and v0.9 trust
candidate are repository-verified, while every external product gate remains
outstanding. The Active
[v0.10 Field Qualification and
Professional Inspection View design](docs/design/field-inspection-view-v0.10.md),
has a complete repository implementation without claiming that its field or
adoption-publication gates are satisfied. v0.11 through v0.20 remain
uncommitted Candidate themes; each needs evidence and an accepted design before
implementation.

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

Release status and external evidence maturity are separate. A repository
release can be Complete while every product gate remains outstanding.

| Evidence maturity | Meaning |
|---|---|
| **Repository-verified** | The accepted design, local verification, fixtures, and declared benchmarks pass. |
| **Field-qualified** | Representative licensed or sanitized production data and observed workflows satisfy the declared scale and usability envelope. |
| **Partner-validated** | Real partner projects repeatedly satisfy their tolerance and accepted-deliverable checks. |
| **Support-qualified** | The declared compatibility, migration, operational, workstation, and support matrices are maintainable. |

## Scope and evidence checkpoint

Status: **v0.8 and v0.9 repository slices Complete; v0.10 professional
inspection View Active with repository implementation present; field and
adoption-publication evidence outstanding**

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

The completed v0.8 slice did not broaden that workflow. Its private verifier
compares the exact v0.7 metric-metre LandXML TIN with a caller-returned LandXML
1.2 file under declared tolerances, with strict read-only Complete-Run binding,
canonical no-replace evidence publication, and full-ceiling exact-byte
streaming outside the Run root. Caller-declared application/version/settings
labels are still not proof that the application ran. The v0.7 journal and
`audit.json` remain unchanged.

The completed v0.9 slice adds no new feature family. It extends the inherited
Spatial Index v1 goldens across the remaining persisted-v1 compatibility
surface, distinguishes authoritative, rebuildable, and temporary artifacts,
hardens recovery and filesystem failure behavior, publishes the exact support
matrix, reviews only exercised public interfaces, and reproduces local
resource/performance gates. Implemented hardening preserves Index filesystem
failures as recoverable I/O diagnostics through the private Workflow seam and
publishes a complete synced initial `.work` header from an owned temporary by
a no-replace link. Unknown, racing, and caller-owned targets remain untouched.
The repository fixture/recovery/support matrix, complete local candidate record,
and independent review are recorded in the v0.9 release evidence.

The accepted v0.10 design adds a separate repository View track. The private
host now owns five deterministic display modes, perspective/orthographic
navigation, truthful loading/Coverage state, structured diagnostics, and a
permission-gated corpus runner. `point-index` retains position-only disk v1 and
adds one bounded attributed disk-v2 recipe with immutable fixtures and explicit
rebuild migration. CPU-authoritative Source, Query, Edit, terrain, QA, and
export contracts are unchanged. This implementation is not evidence of a
permitted production corpus or an observed workflow; those field gates remain
outstanding and are reported separately.

Useful evidence for proceeding includes:

- screen-shared workflows with current users that identify the actual expensive
  step;
- five permitted production LAS/LAZ datasets from at least three unrelated
  firms, including at least two Sources above 500 million Points;
- customer accuracy, coordinate, QA, and downstream export requirements;
- a measured baseline for time to first use, attended editing time, unattended
  processing time, and rework; and
- a clear reason the proposed workflow is meaningfully better than the
  customer's current toolchain.

If the evidence points elsewhere, revise this roadmap before building the next
module. The detailed discovery signals and pivot criteria live in the
[market-validation research](docs/research/saas-point-cloud-market-validation.md#customer-tests-and-kill-criteria).
Production-data access, downstream observations, and paid-pilot evidence are
long-lead work. Collection may proceed during v0.10 without silently expanding
any later repository release.

## Release sequence

There is one Active repository track after the completed v0.9. Ten provisional
Candidate themes, v0.11 through v0.20,
extend the path after v0.10. This is a planning sequence, not a requirement to
publish every number or to publish v1 after v0.20. Candidates may be narrowed,
split, merged, reordered, or stopped before becoming Active.

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

## Standing boundaries for v0.10–v0.20

- GPU display remains disposable and non-authoritative. Exact inspection,
  selection, Edit, terrain, QA, and export use revision-pinned CPU values.
- Opening and viewing a large Source is not evidence that deriving its complete
  terrain fits the same resource envelope. Measure and report those paths
  separately.
- The proposed desktop host supports one narrow survey-to-terrain workflow. It
  is not a general CAD, BIM, point-cloud, or geospatial editor.
- Visual quality means a clear professional inspection View, not Cesium parity,
  photorealism, a globe, 3D Tiles, texture streaming, or rendering every Point
  simultaneously.
- Coordinate systems, units, vertical references, transformations, downstream
  products, and settings are explicit supported profiles. Nothing is guessed.
- Repository status and external evidence maturity are recorded separately.
  Tests, generated data, free evaluations, and declarations do not become
  partner, paid-use, or accepted-deliverable evidence.
- Every Candidate needs an accepted design with one coherent outcome, explicit
  non-goals, local verification, and a repository-activation decision before
  implementation. External evidence gates may remain outstanding only when
  they are named separately and no field, partner, or support maturity is
  claimed from repository work.

## Open-source library adoption track

Punctra is intended to remain useful as an embeddable Rust library, not only as
the implementation behind its own desktop host. Public attention, independent
adoption, and funding are not inferred from repository completeness. The
following are explicit exit requirements for the corresponding Candidate
releases; they do not expand a release's technical feature scope.

| Release | Open-source adoption exit requirement |
|---|---|
| **v0.10** | Publish one accurate public description, repository topics and homepage, an approved screenshot or short demonstration, a reproducible viewing benchmark, and a five-minute “view your first LAS/LAZ Source” guide. Clearly separate current capabilities from roadmap claims. |
| **v0.11** | Provide a minimal third-party renderer integration example that does not depend on demo-private state, plus focused rustdoc explaining host ownership, provisional GPU picks, exact CPU confirmation, and resource limits. |
| **v0.12** | Define and exercise the crates.io/docs.rs packaging path for the supported library crates. Package metadata, licenses, minimum supported Rust version, feature flags, dependency roles, and pre-v1 compatibility expectations must be explicit. |
| **v0.13** | Publish a reproducible out-of-core terrain example and resource report that distinguishes Source viewing, indexing, and full Terrain Derivation and does not extrapolate beyond measured data. |
| **v0.14** | Publish one end-to-end QA example whose documentation lets an adopter trace every displayed profile, residual, tolerance, and gap to authoritative inputs. |
| **v0.15** | Publish a constrained-terrain example and non-goals guide covering the exact supported constraint grammar, failure cases, and boundary between Punctra and general CAD authoring. |
| **v0.16** | Publish the exact named downstream integration guide and only customer-approved, anonymized round-trip evidence. Do not present declared application labels or repository fixtures as observed interoperability. |
| **v0.17** | At least one independent adopter, outside the maintainer and repository demos, completes the documented library quickstart or embedding path; record setup time, failures, unclear APIs, and resulting documentation fixes. |
| **v0.18** | Publish contributor onboarding, issue and pull-request templates, a code of conduct, security-reporting policy, support channels, and a small set of genuinely bounded contributor issues. Document which help is community support and which is paid product support. |
| **v0.19** | Publish the reviewed public API surface, semantic-versioning and deprecation policy, compatibility matrix, migration or rebuild guidance, release notes, and locally reproducible verification commands for downstream maintainers. |
| **v0.20** | Complete an open-source release-readiness review covering the landing page, quickstart, API docs, examples, changelog, known limitations, support expectations, and approved showcase material. Add a funding or GitHub Sponsors path only with transparent goals and only after independent use exists; funding is not a v1 correctness gate. |

Adoption evidence is counted conservatively. Stars, downloads, praise, generated
examples, and maintainer-run integrations are useful signals but do not equal
independent production use. No release is delayed merely to reach a vanity
metric, and no benchmark or customer dataset is published without permission.

### v0.10 — Field qualification and professional inspection View

Status: **Active — repository implementation complete; field qualification and
adoption publication outstanding**

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

Status: **Candidate**

Candidate outcome: connect the progressive View to exact inspection and
reversible ground-classification correction in one narrow desktop workflow.

Activation gate:

- v0.10 evidence identifies interactive inspection or classification
  correction as a material source of attended time or rework.

Likely scope:

- CPU-confirmed inspection after provisional GPU picking;
- an evidence-selected subset of screen-through, polygon, and brush selection;
- exact effective-classification filtering at a pinned Revision;
- visible Point Set, Revision, changed-region, Revert, and recovery state; and
- ground-classification painting without general Attribute or position edits.

Candidate exit evidence:

- every accepted selection matches a sequential CPU oracle, including boundary
  and stale-Revision cases;
- Point Identity survives display hint, exact Query, spill, Edit, Revert, and
  reopen;
- overlays are traceable to exact Point Sets rather than resident LOD samples;
  and
- interruption and recovery reopen into one documented, unambiguous state.

### v0.12 — Explicit spatial-reference contract

Status: **Candidate**

Candidate outcome: support the coordinate, linear-unit, and vertical-reference
profile selected by field evidence without silent assumptions.

Activation gate:

- the production corpus identifies one recurring profile that the existing
  metric-metre path cannot represent correctly.

Likely scope:

- explicit horizontal reference, vertical reference, axis, unit, precision,
  and provenance metadata at every authoritative boundary;
- one evidence-selected transformation path only if required by the workflow;
- deterministic conversion and tolerance rules shared by Query, terrain, QA,
  and export; and
- clear rejection of missing, ambiguous, unsupported, or contradictory
  metadata.

Candidate exit evidence:

- published support fixtures match independent reference coordinates and
  control-point calculations within declared tolerances;
- reopen, Revision, Surface, QA, and export retain the same explicit reference
  identity and conversion provenance;
- unit and axis drift fail closed; and
- no fixture or product path depends on automatic CRS or datum guessing.

### v0.13 — Persistent production-scale terrain

Status: **Candidate**

Candidate outcome: replace v0.6's single-worker in-memory Surface with a
bounded, durable terrain path for the field-qualified scale envelope.

Activation gate:

- field measurements establish the required AOI size, ground-Point count,
  latency, memory, temporary storage, and supported workstation classes.

Likely scope:

- explicit AOIs and deterministic bounded-memory or out-of-core Derivation;
- checksummed persistent Surface Artifacts with Snapshot, spatial-reference,
  algorithm-version, and provenance bindings;
- resumable work, safe publication, reopen, stale-artifact detection, and
  explicit rebuild decisions; and
- bounded parallel execution only where supported worker counts reproduce a
  canonical result.

Candidate exit evidence:

- uninterrupted, resumed, and supported-worker runs reproduce canonical
  topology and descriptors;
- fault fixtures cover truncation, corruption, cancellation, disk exhaustion,
  checkpoint boundaries, and publication certainty;
- retained and temporary storage remain within declared hard limits; and
- at least two above-500-million-Point Source projects from unrelated firms
  complete their declared terrain AOIs and pass their accepted accuracy
  baselines; benchmarks publish that measured envelope without extrapolating
  from Source-viewing or small generated runs.

### v0.14 — Exact terrain QA and correction loop

Status: **Candidate**

Candidate outcome: let a surveyor locate, explain, correct, and recheck terrain
defects without treating display colors as measurements.

Activation gate:

- observed acceptance work identifies the exact QA views, tolerances, and
  reports that change a deliverable decision or reduce repeated inspection.

Likely scope:

- exact profiles or cross-sections for the accepted workflow;
- Source-Point residual Queries, detached Check Point results, and bounded
  visualizations of those authoritative values;
- stale-Surface and changed-region tracking after classification correction;
- repeatable correct, re-derive, compare, and Revert flow; and
- QA evidence with explicit units, gaps, tolerances, Snapshot, Surface, and
  operation provenance.

Candidate exit evidence:

- numeric results match analytic and independent reference fixtures within
  declared tolerances;
- every displayed profile or residual resolves to an authoritative frozen
  Snapshot/Surface pair;
- stale results cannot be presented as current after an Edit; and
- observed trials measure time to find, explain, and correct seeded or known
  defects.

### v0.15 — Evidence-selected terrain constraints

Status: **Candidate**

Candidate outcome: add only the constraint behavior proven necessary for the
selected terrain and downstream-delivery workflow.

Activation gate:

- field evidence shows that missing Breaklines, boundaries, or holes—not
  classification, QA, scale, or coordinate handling—is the next recurring
  source of unacceptable terrain or rework.

Likely scope:

- one narrow revisioned constraint grammar containing only the selected kinds;
- exact snapping, noding, validation, audit, Revert, and recovery rules;
- deterministic constrained terrain integrated with persistent Surface
  provenance; and
- explicit rejection of self-intersection, ambiguity, unsupported topology,
  and tolerance-dependent degeneracy.

Candidate exit evidence:

- independent fixtures prove every supported constraint changes topology as
  specified and survives reopen and Revert;
- input order, interruption, resume, and supported worker count do not change
  canonical output;
- adversarial near-degenerate cases fail deterministically; and
- the scope does not grow into general linework or CAD authoring.

### v0.16 — Named downstream interoperability

Status: **Candidate**

Candidate outcome: deliver one exact terrain profile to one named downstream
application version and settings matrix chosen from partner evidence.

The working hypothesis is Autodesk Civil 3D. The activation gate must still
name the exact supported version and settings from observed evidence.

Activation gate:

- multiple observed firms use the same named downstream profile and agree on
  units, references, precision, topology, constraints, and acceptance checks.

Likely scope:

- the exact LandXML subset needed by the selected product/version/settings;
- deterministic points, faces, units, references, precision, and only the
  v0.15 constraints required by that profile;
- the already-closed generic Complete-Run binding and no-replace canonical
  round-trip evidence specialized to the named product/version/settings; and
- bounded semantic comparison of the exported and caller-returned deliverable.

Candidate exit evidence:

- fixtures cover accepted, malformed, ambiguous, partial, raced, unit-drift,
  reference-drift, tolerance-drift, and topology-drift cases;
- the same unmodified export is accepted in three firms' observed production
  round trips without bespoke code repair;
- application, version, and settings declarations are not represented as
  proof unless matching external execution evidence exists; and
- no claim is made for both Autodesk and Bentley ecosystems or for a generic
  exporter framework.

### v0.17 — Narrow design-partner alpha

Status: **Candidate**

Candidate outcome: package the field-qualified path as one usable end-to-end
desktop workflow for selected design partners.

Activation gate:

- v0.10 through v0.16 identify a stable repeated workflow and at least one
  partner willing to use the declared supported profile on production work.

Likely scope:

- open, inspect, correct, derive, QA, and export through one focused host;
- installable packaging for a declared platform and workstation envelope;
- autosave policy, progress, cancellation, recovery, safe retry, and actionable
  diagnostics; and
- consent-aware local evidence capture without uploading Source data.

Candidate exit evidence:

- partners complete the supported workflow on representative projects and can
  distinguish exact, stale, sampled, running, failed, and recovered states;
- every failure names the Source or Artifact, operation, phase, certainty, and
  one safe recovery action;
- packaging, reopen, cancellation, and recovery pass local acceptance on each
  supported machine class; and
- requests outside the narrow workflow are recorded rather than silently added.

### v0.18 — Design-partner MVP and operations

Status: **Candidate**

Candidate outcome: make the narrow alpha repeatable to install, operate,
support, and evaluate across several firms.

Activation gate:

- alpha projects repeatedly produce acceptable deliverables and the delivery,
  update, licensing, privacy, and support model has been explicitly selected.

Likely scope:

- reproducible installation, update, rollback, configuration, and diagnostic
  collection for the declared support matrix;
- licensing only if the selected delivery model requires it;
- corpus regression, support playbooks, issue evidence, and recovery drills;
  and
- workflow measurement that separates time to first use, attended work,
  unattended work, rework, and downstream acceptance.

Candidate exit evidence:

- repository acceptance proves the declared packaging and operational paths;
- external evidence separately carries forward the v0.8 product gates: three
  firms use the same supported path, three paid pilots reach production use,
  and two firms convert or document sufficient measured labor savings;
- multiple runs at one firm, free trials, fixtures, projected savings, and
  declarations do not multiply or substitute for those external gates; and
- accuracy and accepted-deliverable checks remain non-negotiable when measuring
  speed or labor reduction.

### v0.19 — Expanded-scope compatibility and support qualification

Status: **Candidate**

Candidate outcome: freeze new feature families and qualify the expanded
workflow for a maintainable compatibility and support promise.

Activation gate:

- the v0.18 workflow is Partner-validated and its exact supported surface is
  narrow enough to maintain.

Likely scope:

- final format, coordinate, precision, platform, device, workstation, and
  downstream-product support matrices;
- persisted-schema compatibility, migration only where a second version
  exists, upgrade/rollback fixtures, and artifact ownership policy;
- fault injection, resource/performance reproduction, security review, public
  interface review, upgrade notes, and support playbooks; and
- removal or explicit deferral of experimental paths outside the promise.

Candidate exit evidence:

- no known open release-blocking correctness or data-loss defect remains in the
  declared support matrix;
- the enumerated recovery contract passes its fault-injection fixtures;
- old supported Artifacts reopen or follow a tested explicit migration/rebuild
  path; and
- declared workstation ceilings and downstream round trips reproduce locally.

### v0.20 — Product v1 release candidate and production soak

Status: **Candidate**

Candidate outcome: hold the qualified scope stable long enough to decide
whether it deserves the v1 compatibility and support promise.

Activation gate:

- v0.19 is Support-qualified and design partners have scheduled representative
  production work within the declared matrix.

Likely scope:

- feature freeze, release-candidate packaging, upgrade/rollback rehearsal, and
  extended production soak;
- full local regression, GPU acceptance, fault, resource, performance, and
  downstream-compatibility reproduction;
- user, administrator, recovery, support, and upgrade documentation; and
- final review of support capacity, known limitations, and compatibility
  commitments.

Candidate exit evidence:

- repeated partner projects produce accepted deliverables without bespoke code
  repair inside the supported profile;
- release-candidate installation, update, rollback, recovery, and diagnostics
  pass across the declared machine matrix;
- no known release-blocking correctness, recovery, security, or support defect
  remains; and
- the evidence record supports an explicit ship, extend-soak, narrow, or stop
  decision. Completing v0.20 does not automatically publish v1.

### v1.0 — Trustworthy supported scope

Status: **Candidate**

Release v1 when the narrow supported workflow repeatedly produces accepted
deliverables, the enumerated recovery contract passes its fixtures, resource
use is bounded, and the public compatibility promise can be maintained.
Neither the v0.9 repository trust baseline nor completion of the provisional
v0.20 theme is by itself a reason to publish v1.

## Product milestone map

| Product milestone | Release range | Delivery state | Outcome |
|---|---|---|---|
| Renderer and planning foundations | v0.1–v0.2 | Complete; repository-verified | Reusable bounded display engine and adaptive View planner. |
| Source, document, terrain, and workflow baseline | v0.3–v0.7 | Complete; repository-verified only | Headless technical path from verified LAS/LAZ to one narrow resumable terrain deliverable; field and product evidence remains separate. |
| Qualifier and trust baseline | v0.8–v0.9 | Complete; repository-verified only | Close inherited qualification gates and harden only the existing narrow repository compatibility surface. |
| Field inspection and exact correction | v0.10–v0.11 | v0.10 repository implementation complete but field/adoption exits outstanding; v0.11 Candidate | Qualify representative Sources and connect a professional View to CPU-authoritative review and reversible correction. |
| Spatial contract and production terrain | v0.12–v0.13 | Candidate | Make reference semantics explicit before persisting a bounded production-scale Surface. |
| Terrain acceptance tooling | v0.14–v0.15 | Candidate | Add exact QA and only the constraints earned by field evidence. |
| Downstream and partner product | v0.16–v0.18 | Candidate | Qualify one named downstream profile, then package and validate one narrow partner workflow. |
| Product v1 qualification | v0.19–v1.0 | Candidate | Freeze, support-qualify, soak, and explicitly decide whether the maintained scope deserves v1. |
| Open-source library adoption | v0.10–v0.20 | Active with v0.10, cross-cutting | Progress from an accurate public story and first-file quickstart to independent adoption, contributor readiness, stable integration guidance, and an evidence-backed funding path. |

## Deferred until evidence changes

The current path does not include:

- E57, scan registration, sensor calibration, or photogrammetry;
- general CAD or BIM authoring;
- AI feature extraction or a broad classification suite;
- automatic CRS or vertical-datum guessing;
- broad coordinate-transformation coverage added without a selected workflow;
- multi-Source Workspaces;
- raw-cloud hosting, cloud collaboration, or distributed execution;
- remote Source reads or networking policy;
- a public plugin registry, generic export framework, or simultaneous Autodesk
  and Bentley compatibility promise;
- Cesium visual/platform parity, globe-scale 3D Tiles, global imagery or
  terrain, texture streaming, photorealistic meshes, or rendering every Point
  simultaneously;
- runtime point schemas, arbitrary shaders, or GPU-authoritative geometry; or
- broad format support added only for completeness.

COPC, bindings, additional export formats, and team-review features may move
forward only when a real caller or partner workflow earns their seams.

## Maintenance

Review this file when a release starts, finishes, or materially changes
direction. A roadmap update should:

1. record repository delivery status and external evidence maturity separately;
2. identify the single Active release, if one exists;
3. move unsupported ideas to Deferred instead of leaving ambiguous promises;
4. link the accepted design or ADR for newly Active scope; and
5. record why a release was split, merged, reordered, or stopped; and
6. update open-source adoption evidence without treating popularity metrics as
   product, correctness, or release acceptance.

The broader candidate architecture is described in
[docs/architecture](docs/architecture/README.md). It is a source of design
constraints and module ordering, not an implementation commitment.
