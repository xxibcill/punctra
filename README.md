# Punctra

Punctra is an embeddable Rust point-cloud foundation with verified Source
access, exact revisioned classification, deterministic Terrain/QA, adaptive
View planning, and wgpu rendering. It is for applications that need
authoritative Point reads, recoverable narrow Edits and deliverables,
progressive display of very large point sets, precise large-world coordinates,
bounded logical residency, generation-safe streaming updates, and Point picking
without adopting a complete editor or product UI.

Version 0.2.0 adds the adaptive planning scope and verification gates recorded
in [the v0.2 design](docs/design/adaptive-view-planning-v0.2.md) on top of
[the v0.1 renderer](docs/design/render-engine-v0.1.md).

Version 0.3.0 completes the accepted
[Real Sources design](docs/design/real-sources-v0.3.md): canonical Source and
Point contracts, runtime-neutral bounded work, one verified Source seam, and
interchangeable in-memory, LAS, and LAZ adapters. File reads preserve exact
position ticks, supported Attributes, ordered VLR/EVLR metadata, and stable
`(SourceId, ordinal)` Point Identity under explicit batch and decoder limits.
`source-las` supports LAS point-data record formats 0–10 and LAZ formats 0–8.
LAZ formats 9 and 10 are rejected explicitly until the layered WavePacket14
codec can preserve waveform values exactly.

Version 0.4.0 completes the accepted
[Out-of-core View design](docs/design/out-of-core-view-v0.4.md): a rebuildable,
resumable persistent Spatial Index, conservative Source-span lookup, bounded
display-only hierarchy samples, efficient fixed-chunk LAZ seeks, and a private
host-owned path from a verified real Source through View planning to atomic
renderer updates. It does not introduce Workspace or exact Query semantics.

Version 0.5.0 completes the accepted
[Durable document core design](docs/design/durable-document-core-v0.5.md). The
narrow slice adds one deep, headless `point-workspace` crate for exact All,
world-box, and explicit-Point-ID classification selections; bounded temporary
Point Sets; immutable classification Revisions; immediate-head Revert Edits;
and durable Operation reconciliation. Source bytes remain immutable.
Screen-through selection, general edits, named Point Sets, terrain, and Source
rewriting remain outside this scope.

Version 0.6.0 completes the implemented technical slice in the accepted
[Terrain and QA benchmark design](docs/design/terrain-qa-benchmark-v0.6.md).
It adds one exact classification-aware `Snapshot::point_rows` stream, one deep
`point-terrain` crate for a deterministic single-worker unconstrained in-memory
TIN, detached Check Point residual QA, a private durable create-new
metric-metre LandXML 1.2 points/faces encoder, and one headless `terrain-demo`
caller. It does not add Breaklines, Profiles, a classifier, terrain
persistence, general LandXML, or coordinate transformation. This repository
completion is not a claim of product, partner, licensed-data, or downstream-
application acceptance.

Version 0.7.0-alpha.1 completes the deliberately technical slice described by
the implemented [Technical partner-alpha readiness
design](docs/design/technical-alpha-readiness-v0.7.md). It adds exact Revision
Audit and Edit Footprint facts, restart-safe LandXML reconciliation, linked
child cancellation, and one durable eight-checkpoint Workflow Run inside
`terrain-demo`. The app now has explicit `start`, `resume`, and journal-only
`inspect` commands, canonical `audit.json` evidence, and structured failures
with one safe recovery action. It does not add Breaklines or establish the
external design-partner, production-data, downstream-application, paid-use,
or human-workflow evidence required by the product milestone.

Version 0.8.0-alpha.1 completes the repository slice in the implemented
[repository interoperability qualification
design](docs/design/design-partner-mvp-v0.8.md). The narrow implementation adds
a private `terrain-demo` semantic LandXML 1.2 comparator, read-only Complete-Run
binding, a streaming verifier covering the v0.7 export ceiling, and separate
canonical pass/fail evidence published without replacement. The v0.7 journal
and `audit.json` remain unchanged. No actual Civil 3D, Bentley, partner,
paid-pilot, conversion, or labor-savings test is claimed, so the product MVP
gates remain outstanding.

Version 0.9.0 completes the repository trust and version-1 compatibility
candidate described by the implemented [Trust and v1 Candidate
design](docs/design/trust-v1-candidate-v0.9.md). It freezes owner-local
version-1 fixtures for Source Records, Spatial Indexes, Workspaces, Workflow
Runs, reports, LandXML, and Round-Trip Evidence; hardens descriptor-bound
no-replace publication and conservative recovery; and records the reviewed
interface and support boundaries for the existing narrow workflow. The exact
local release results are recorded in the [v0.9 verification
record](docs/releases/v0.9.0.md). This repository candidate is not `1.0.0`, a
product-readiness claim, or evidence of external downstream execution or
customer acceptance.

Version 0.10.0-alpha.1 implements the repository track of the accepted [Field
Qualification and Professional Inspection View
design](docs/design/field-inspection-view-v0.10.md)
from the completed v0.9 repository candidate. It adds deterministic neutral,
RGB, intensity, and classification display; perspective and orthographic
navigation; explicit loading/Coverage state; bounded `PVIEW_*` diagnostics;
the narrow disk-v2 inspection recipe for exact raw display samples; and a
permission-gated local corpus runner with canonical nonclaim-bearing reports.
No production corpus, observed workflow, workstation, usability, partner, or
support qualification is claimed. Spatial Index v1 remains supported and
position-only; v2
inspection samples are bounded rebuildable display data. Sampled colors are
disposable display values, not exact Query, Edit, terrain, QA, or export
results.

Version 0.11.0-alpha.1 completes the repository-verified technical track of the
accepted [Exact Interactive Review and Ground Correction
design](docs/design/exact-interactive-review-v0.11.md). The public
`point-review` crate confirms one provisional renderer Point Identity against
a pinned Workspace Snapshot and materializes one inclusive, screen-through
rectangle as an exact CPU-scanned Point Set, with an optional effective-
classification equality filter. Highlight inputs have an explicit protocol
ceiling and are derived from bounded exact Point Set iteration rather than
resident LOD samples. Durable correction continues to use caller-owned
Operation Identities and the existing Workspace classification commit,
immediate-head Revert, Revision Audit, Edit Footprint, and Operation
reconciliation interfaces. The public `render-wgpu` `third_party_host` example
demonstrates host ownership and provisional picking without depending on
`renderer-demo` private state. This narrow repository slice adds no polygon,
brush, visible-only, or occlusion selection, no arbitrary Attribute or
position Edit, and no general UI or automatic recovery policy.

Repository verification does not satisfy the roadmap's field activation or
adoption evidence gates. No permitted production correction workflow,
independent adopter, professional time saving, reduced rework, partner
acceptance, or product efficacy is claimed.

Version 0.12.0-alpha.1 implements the bounded repository track of the
[Explicit Spatial Reference and Package Publication
design](docs/design/explicit-spatial-reference-v0.12.md). A structured
Coordinate Reference now carries horizontal and vertical EPSG identities,
easting/northing/elevation axes, horizontal and vertical linear units, and
provenance through verified Source metadata, Source Records, Workspace
lineage, Terrain descriptors, detached QA, and LandXML. `source-las` publishes
that profile only from one complete direct GeoTIFF key directory; ambiguity,
indirection, missing facts, unsupported values, and opaque WKT fail closed
without guessing. The Terrain/QA/export path supports metre/metre only and
does no coordinate transformation. Frozen unknown-reference workflows retain
their explicit legacy assertion.

The same release defines the local crates.io/docs.rs path for all twelve public
libraries. They use versioned local/registry dependencies, Rust 1.90, empty
default features, dual-license and repository metadata, and clean package
verification documented in the [library packaging
guide](docs/guides/library-packaging.md). The applications remain private and
no registry publication, external adoption, production corpus, downstream
execution, or partner acceptance is claimed.

To try the implemented View safely, follow the five-minute [first LAS/LAZ
guide](docs/guides/first-las-laz.md). It separates position-only disk-v1 and
attributed disk-v2 caches and explains what progressive Coverage does and does
not mean.

Later direction and the exact external product gates are described in the
[living roadmap](ROADMAP.md). Its candidate themes do not expand accepted
implementation scope by themselves.

## Embedding model

The host owns its wgpu device, queue, command encoder, target texture, data
loading, and View policy. Punctra owns validated resident state and rendering:

```rust,ignore
let limits = RenderLimits::new(512 * 1024 * 1024, 20_000_000, 4096)
    .with_max_highlight_points(1_000_000);
let config = RendererConfig::new(surface_format, limits);
let mut renderer = WgpuRenderer::new(&device, config)?;

renderer.apply(&RenderUpdate::Reset { view_generation })?;
renderer.apply(&RenderUpdate::Upsert { batch })?;

let viewport = Viewport::new(width, height)?;
let frame = Frame::new(view_generation, camera, viewport)?;
let recorded_frame = renderer.render(&mut encoder, &target, &frame)?;
let report = recorded_frame.report();
queue.submit([encoder.finish()]);
```

Point positions use a finite 64-bit world origin plus finite 32-bit relative
coordinates. Upserts replace complete batches atomically. Stale View
generations and non-increasing batch versions are rejected before publication.
The byte limit covers the fixed 24-byte GPU point vertices; per-batch uniforms,
render targets, allocator padding, and transient command uploads are outside
that logical residency model.

Frame uniform uploads are part of the recorded command stream. A host may
record several Punctra frames before one submission without later camera values
changing earlier frames. Rendering returns a `RecordedFrame`; pass that exact
value back when encoding a `PickRequest` so the pick uses the batches and
identity metadata displayed by that render. Retaining a `RecordedFrame` pins
any replaced GPU resources it references until the value is dropped. Picking
otherwise follows the same host-owned flow: submit the encoder, drive normal
wgpu device polling, and poll the returned `PickTicket` without blocking.

A `PickHit` is only a provisional identity from one recorded display frame.
The host rejects a stale View generation, pins the intended Workspace
Snapshot, and passes only the hinted `PointId` to `point_review::confirm_pick`.
For rectangle review, `point_review::screen_through` scans exact Snapshot rows
with the caller's `Camera`, `Viewport`, inclusive continuous-pixel
`ScreenRect`, and optional effective-classification filter. It ignores GPU
residency and depth occlusion. A host builds `SetHighlights` only after bounded
Point Set identity iteration completes; an oversized highlight input is
rejected before replacing the prior renderer highlight state.

Adaptive View planning is a separate renderer-neutral step. The host reports a
generation-stamped hierarchy snapshot, including which nodes are missing,
requested, or resident. `point-view` frustum-culls it, selects LOD by
screen-space error, reserves point/byte/batch costs, and returns prioritized
requests, required retention, and exact conditional retirements. It performs
no I/O and never mutates renderer state.

## Workspace

- `point-contracts` defines lossless Source, Point, Attribute, coordinate, and
  provenance values.
- `foundation-runtime` provides runtime-neutral Jobs, progress, cancellation,
  and bounded pull-stream control.
- `point-source` verifies immutable Sources and exposes normalized bounded
  reads through one caller-facing interface.
- `source-memory` supplies deterministic in-memory Sources; its opt-in
  `test-support` feature adds conformance faults.
- `source-las` opens local LAS formats 0–10 and LAZ formats 0–8 through the same
  verified, bounded Source interface; LAZ formats 9 and 10 are explicitly
  unsupported pending exact WavePacket14 codec support.
- `point-index` prepares one deterministic checksummed fixed-block BVH, returns
  conservative Source spans, and streams bounded position-only disk-v1 or
  attributed disk-v2 display samples plus complete Source-backed leaves.
- `point-workspace` owns exact revision-pinned classification selection,
  process-scoped spillable Point Sets, immutable sparse classification
  Revisions, immediate-head Revert, Operation-ID recovery, and the narrow exact
  classification-aware `Snapshot::point_rows` pull stream behind one deep
  caller interface. It can also rebuild a bounded exact `RevisionAudit`,
  including transitions and the Edit Footprint, from immutable Revision rows.
- `point-review` composes exact Snapshot rows, renderer-neutral Camera and
  Viewport values, CPU projection, and existing Point Set materialization for
  provisional-pick confirmation and one inclusive screen-through rectangle.
  It owns no GPU, window, gesture, commit, or recovery state.
- `point-terrain` derives one immutable `TerrainSurface` with canonical
  `SurfaceVertex` and `SurfaceFace` values, evaluates detached Check Points,
  and durably creates or exactly reconciles the supported LandXML 1.2 subset.
- `render-protocol` defines and validates renderer-neutral View updates,
  including a caller-selected complete highlight-input ceiling.
- `point-view` plans deterministic, budgeted hierarchy requests and retirement.
- `render-wgpu` owns renderer GPU resources, pipelines, drawing, and
  provisional picking while the embedding host owns device, queue, encoder,
  target, submission, polling, and exact-confirmation policy.
- `renderer-demo` exercises the engine with either generated point batches or
  one Full-verified indexed LAS/LAZ Source. It privately owns display mapping,
  perspective/orthographic controls, truthful progressive state, structured
  View diagnostics, and the local field-corpus runner.
- `terrain-demo` owns the GPU-free, restartable LAS/LAZ-to-index-to-Workspace-
  to-terrain-to-QA-to-LandXML Workflow Run and its canonical audit report.

Networking, polygon/brush/visible-only/occlusion selection, general editing,
Source rewriting, persistent or constrained terrain, general export, and
general application UI remain outside the accepted scope.

## Examples

Exercise the in-memory Source headlessly with the
[in-memory Source example](crates/source-memory/examples/memory_source.rs):

```bash
cargo run -p source-memory --example memory_source
```

Inspect a real LAS or LAZ Source, including its verified identity, schema,
metadata, bounds, and exact read throughput, with the
[file inspection example](crates/source-las/examples/inspect.rs):

```bash
cargo run --release -p source-las --example inspect -- survey.laz
```

Build, query, and read an index directly over an in-memory Source with:

```bash
cargo run -p point-index --example direct_use
```

Run the deterministic adaptive-LOD demo with:

```bash
cargo run --release -p renderer-demo
```

Run the minimal public offscreen renderer host, including one deliberately
provisional GPU pick, without any `renderer-demo` dependency:

```bash
PUNCTRA_REQUIRE_GPU=1 cargo run -p render-wgpu --example third_party_host
```

The example owns the wgpu lifecycle and reports the provisional Point
Identity. It does not fabricate a Workspace or claim CPU confirmation; an
editing host must validate the View generation and call the public
`point-review` interface against its pinned Snapshot. A missing expected
headless adapter is a failure when `PUNCTRA_REQUIRE_GPU=1` is set.

Create a Workspace over one LAS/LAZ Source, select exact class-2 Points,
classify them, append an immediate-head Revert, and reopen the durable result:

```bash
cargo run --release -p point-workspace --example classify -- \
  survey.laz survey.laz.pidx survey.pcw 6
```

Run the complete generated in-memory Source-to-LandXML terrain composition:

```bash
cargo run -p point-terrain --example derive
```

Start one durable headless LAS/LAZ terrain Workflow Run. The caller must retain
the nonzero Run and Workspace Operation identities and the expected baseline
Revision before invoking the command. Both the Workspace and `RUN_ROOT` must
already exist. `terrain-demo` opens but never creates the Workspace; an absent
Workspace fails with `PWF_INVALID_REQUEST` before Run creation or Workspace
mutation. The Source must carry the supported structured metre/metre profile.
For a legacy Source whose Coordinate Reference is explicitly Unknown, the
optional `--assert-unknown-crs-metric` compatibility flag is an explicit caller
assertion, not CRS inference. Omit that flag for a structured Source:

```bash
cargo run --release -p terrain-demo -- start \
  --run-id "$RUN_ID_HEX" \
  --operation-id "$OPERATION_ID_HEX" \
  --baseline "$BASELINE_REVISION_HEX" \
  --exclude-ground-ordinal 4 \
  --date 2026-08-10 --time 00:00:00Z \
  survey.laz survey.laz.pidx survey.pcw run-root
```

The command records the complete Intent before selection or commit, changes the
listed class-2 Ground ordinals to class 1, audits the resulting Revision,
derives baseline and changed Surfaces, evaluates any repeated
`--check-point ID,X,Y,Z` observations, ensures `terrain.xml`, and ensures
`audit.json`. It returns success only after all eight journal frames are
durable. Resume uses the same paths, options, identities, baseline, ordinals,
and Check Points with `resume` in place of `start`; it never invents a new
Operation Identity. Journal-only status inspection requires only the Run root:

```bash
cargo run --release -p terrain-demo -- inspect run-root
```

The durable v0.7 command replaces the v0.6 one-shot
`--exercise-correction-revert` grammar; it commits the requested correction
rather than automatically reverting it. The v0.6 terrain guarantees remain
covered by regressions: the correction changes the exact Ground Input, an
immediate-head Revert restores geometry, topology, vertices, and faces, a
caller-requested Revert restores the baseline after the correction, and Source
bytes remain unchanged. A v0.7 Workflow does not automatically Revert a
committed classification Revision when a later phase fails.

The first v0.8 slice can compare that exact export with a returned LandXML while
ignoring Point/face order, Point renumbering, and triangle winding. The two
operational paths may resolve to the same regular file or hard-linked content;
semantic identity comes from the captured bytes rather than path identity.
Symbolic links remain invalid, and platforms without stable file identity fail
closed:

```bash
cargo run --release -p terrain-demo -- compare-landxml \
  --application "CALLER-DECLARED APP" \
  --application-version "CALLER-DECLARED VERSION" \
  --settings-profile "CALLER-DECLARED SETTINGS" \
  --horizontal-tolerance-metres 0.001 \
  --vertical-tolerance-metres 0.001 \
  run-root/terrain.xml returned.xml
```

This command rejects unit drift, out-of-tolerance coordinate drift, ambiguous
vertex matches, and topology drift. Its summary is deliberately marked as not
Run-bound and not canonical evidence; the application/version/settings labels
are caller declarations, not proof that the named application ran.

The inherited Run-bound qualification slice adds the strict post-Run command:

```bash
cargo run --release -p terrain-demo -- verify-round-trip \
  --downstream-app "CALLER-DECLARED APP" \
  --downstream-version "CALLER-DECLARED VERSION" \
  --downstream-setting "profile=CALLER-DECLARED SETTINGS" \
  --horizontal-tolerance-metres 0.001 \
  --vertical-tolerance-metres 0.001 \
  run-root returned.xml round-trip-evidence.json
```

It accepts only an existing, exact eight-frame Complete Run, keeps the existing
shared Run lock for the operation, and revalidates `run.pwf`, `terrain.xml`, and
`audit.json` without repair or mutation. The evidence target must be outside
the Run root. Exact existing evidence bytes reconcile; different existing
bytes are never replaced. A fully evaluated semantic mismatch publishes
canonical failed evidence and exits nonzero with its stable `PRT_*` reason.
Malformed input, resource/I/O failure, changed Run artifacts, and publication
uncertainty are operational failures and publish no final result.

The evidence application, version, and settings are caller declarations only.
Even passing evidence explicitly records that Punctra did not observe the
downstream application run and does not establish vendor certification, firm
acceptance, paid use, conversion, or measured labor savings.

Checked-in generated pass and topology-failure evidence fixtures pin the v1
schema's canonical bytes and hashes. The streaming verifier now covers the
exporter's 4-GiB, 10-million-vertex, and 20-million-face ceilings with separate
token, parser-working, retained-working, node, text, and comparison limits.
Those generated fixtures did not by themselves supply independent review; the
separate review is now complete. They still do not supply a complete one-commit
local release record or any external product evidence.

The fixed Run-root children are `run.pwf`, `run.lock`, `terrain.xml`, and
`audit.json`. Existing exact XML/report bytes reconcile; different caller-owned
targets fail without replacement. Inspect may durably truncate a torn final
journal suffix to its last verified frame, but it does not open or mutate the
Source, index, Workspace, LandXML, or report. It then revalidates Run-root
identity; replacement after a durable repair is reported conservatively as
`PWF_PUBLICATION_INDETERMINATE` at the `inspect` stage with publication phase
`journal-checkpoint`.

Create the Workspace separately through the public `point-workspace` API. The
[classification example](crates/point-workspace/examples/classify.rs)
demonstrates setup and prints Revision identities. `terrain-demo` requires that
Workspace's selected `U8` Attribute to be Source Attribute 6, the `source-las`
classification column, and verifies it through the public
`Workspace::schema().classification()` accessor. The workflow baseline is the
current
`workspace.head().provenance().revision()` obtained from a freshly opened
session; retain that identity, then drop every Workspace/Snapshot/PointSet
handle so `terrain-demo` can acquire the exclusive Workspace lock.

Pass a real Source to Full-verify it, build or open its index, and render it.
When the optional target is omitted, neutral/elevation default to
`SOURCE.pidx`, while RGB/intensity/classification default to
`SOURCE.inspection-v2.pidx`; the incompatible recipes never choose the same
implicit path. Neutral color remains the default. Elevation is normalized by
complete Source world-Z bounds. RGB and intensity scale exact raw `U16` values
to RGBA8, while classification maps raw `U8` values through a fixed v0.10
palette:

```bash
cargo run --release -p renderer-demo -- survey.laz survey.laz.pidx
cargo run --release -p renderer-demo -- \
  --display elevation survey.laz survey.laz.pidx
cargo run --release -p renderer-demo -- \
  --display rgb survey.laz survey.inspection-v2.pidx
cargo run --release -p renderer-demo -- \
  --display intensity survey.laz survey.inspection-v2.pidx
cargo run --release -p renderer-demo -- \
  --display classification survey.laz survey.inspection-v2.pidx
```

Neutral and elevation use the position-only disk-v1 recipe. RGB, intensity,
and classification share the attributed disk-v2 inspection recipe. The two
recipes cannot share a target: an incompatible complete or work target is
preserved and rejected, so choose separate paths or explicitly move aside the
rebuildable cache family before rebuilding. RGB additionally requires all
three LAS `U16` color Attributes; there is no silent fallback.

The same Source/index/planner/materializer path has a GPU-free process smoke
mode that accepts one atomic CPU-model Upsert:

```bash
cargo run --release -p renderer-demo -- \
  --smoke --display classification survey.laz survey.inspection-v2.pidx
```

Display colors are sampled presentation values. They do not replace exact CPU
inspection or make the GPU authoritative.

- Left-drag orbits, middle-drag pans, and the mouse wheel zooms.
- `P` toggles perspective/orthographic projection without changing the
  target-plane scale.
- `R` resets the camera, `H` toggles stable-ID highlights, and Space pauses or
  resumes new node materialization while planning and safe retirement
  continue.
- Escape exits.

The window title distinguishes LOD demand, load candidates, actually issued
requests, retained/retired nodes, queue/staging facts, and requested/resident
nodes. It names Sampled versus Complete resident Coverage and labels the
projection and paused/streaming/steady state; none is called Query completion.
The synthetic hierarchy represents more than 10 million logical Points; both
paths keep renderer residency at fixed point, byte, and batch limits. The real
path additionally reports Full verification, index disposition and reuse,
first accepted batch latency, queue depth, and staging peaks. Failures use a
stable `PVIEW_*` code, owning phase, bounded detail, and one safe recovery
action.

For reproducible local viewing measurements, copy the checked-in [field-corpus
manifest example](docs/guides/field-corpus.example.json) only after confirming
permission to inspect and measure every Source, then run:

```bash
cargo run --release -p renderer-demo -- corpus \
  --manifest /private/path/to/field-corpus.json \
  --report /private/path/to/viewing-report.json
```

The GPU-backed runner measures Full verification, cold/warm index preparation,
first visible submission, a declared navigation trace, residency, and disk
facts under recorded limits. Reports omit Source/index paths and opaque
project/firm identifiers, publish without replacement, and encode explicit
false nonclaims for production-corpus completion, partner acceptance,
professional preference, terrain capacity, and human-time savings. They may
still contain sensitive Source identity and machine facts and are not approved
for publication by being generated.

Each corpus entry must name a fresh, absent index target so the first timing is
a genuine cold build; an existing or resumable target is rejected without
replacement. The runner then immediately reopens the completed artifact and
records a separate warm-open timing.

## v0.4 benchmark evidence

The checked-in `point-index` Criterion benchmark uses a deterministic
one-million-Point in-memory Source. On the local Apple M5 Pro, 24 GiB,
macOS 26.5.2 reference run with Rust 1.90.0, it produced a 1,971,528-byte
artifact. Median times were 330.515 ms for a cold build, 20.567 ms for a warm
verified open, 606 ns for whole-bounds candidate planning, 1.249 ms for the
4,096-Point internal-root display read, and 122.100 µs for one complete
65,536-Point memory-backed leaf read. The combined candidate/root/leaf
synchronous path peaked at 3,671,504 measured heap bytes under its 32 MiB gate.

These are one-machine generated-fixture baselines, not universal latency claims
and not licensed production-data evidence. Licensed real-cloud and
design-partner runs remain explicitly outstanding.

## v0.5 benchmark evidence

At v0.5, the `point-workspace` acceptance suite recorded 61 package tests: 19
integration tests through the public interface and 42 unit, fault-injection,
and allocation gates. The merged v0.7 suite now has 83 package tests—33
integration and 50 unit/private—after adding exact row-stream and Revision
Audit coverage. The retained tests include generated LAS and LAZ selection,
commit, Revert, reopen, Source immutability, forced spill, hard limits,
corruption, retry, and injected persistence-boundary cases.

On the local Apple M5 Pro, 24 GiB, arm64, macOS 26.5.2 reference machine with
Rust 1.90.0, the default generated one-million-Point benchmark completed its
evidence pass and all declared Criterion cases completed locally. A separate
131,073-Point synchronous test
of the same selection worker path measured 6,292,224 bytes of
worker-equivalent peak heap under its 64 MiB gate. The one-million-Point
benchmark does not claim worker heap: its public Point-ID iteration peaked at
2,621,440 measured caller-thread bytes and retained zero, while selection
memory is reported as sampled process RSS. Resident-selection RSS was
62,668,800 bytes. Forced-spill RSS started at 62,685,184 bytes and sampled at
62,832,640 bytes, a 147,456-byte delta; its sealed temporary file was 9,009,182
bytes and was removed with the final Point Set handle.

A sparse 10,000-Point classification/Revert pair took approximately
16.442/15.818 ms and added 20.100 logical bytes per changed Point. A dense
500,000-Point pair took approximately 34.973/35.778 ms and added 20.004 logical
bytes per changed Point. Reopen at Revision depths 2, 4, and 8 took
approximately 1.231, 37.753, and 74.968 ms. The final Workspace contained
40,812,316 logical directory-entry bytes; shared hard links occupied
20,418,560 physical bytes according to `du`.

These values are one-machine generated-fixture evidence, not universal
performance claims and not licensed production-data or design-partner
evidence. Those external evidence gates remain outstanding.

## v0.6 benchmark evidence

The checked-in `point-terrain` benchmark composes a generated in-memory Source,
complete Spatial Index, Workspace Snapshot, Terrain Derivation, detached QA,
and durable LandXML export through public interfaces. On the local Apple M5 Pro
(`Mac17,9`), 24 GiB, arm64, macOS 26.5.2 reference machine with Rust 1.90.0,
the 10,000-Point run measured Derivation at 11.983–12.049 ms
(829.97–834.53 Kpoints/s). Detached QA took 94.907–95.164 us for three Check
Points and 19,604 face tests. Durable LandXML creation took 18.020–18.311 ms
(53.650–54.518 MiB/s) for 1,030,118 bytes.

The emitted evidence record names `jjaes-MacBook-Pro.local` (`macos`,
`aarch64`) and records one-shot Derivation, QA, and LandXML times of 13,371 us,
125 us, and 14,656 us respectively. Those one-shot values are retained
separately from the Criterion intervals. It records 10,000 Ground Input Points,
10,000 vertices, 19,602 faces, and 396 hull vertices.

The descriptor reported 135,790,592 accounted peak working bytes, 1,034,176
retained Surface bytes, and 521,494 topology steps. QA reported 336 accounted
peak working bytes. These are explicit algorithm ledgers. The benchmark's
`worker_heap_measurement` is `null`: no observed worker-heap value is claimed.
The benchmark supports generated 10,000, 100,000, and 1,000,000-Point scales
through `PUNCTRA_TERRAIN_BENCH_POINTS`; only the completed 10,000-Point run is
reported here.

These results are one-machine generated-fixture technical evidence, not
universal performance, licensed-production, above-500-million-Point, partner,
downstream Civil 3D/Bentley, paid-use, or human-time evidence. Every such
external gate remains outstanding.

## v0.7 benchmark evidence

`terrain-demo` now has 133 package tests: 109 unit/private fault and contract
tests, 15 public workflow-facade tests, eight process tests, and one checked-Run
v1 golden-corpus test. The retained
v0.7 suites cover every eight-frame resume prefix, 12 limit families,
known-identity validation, and dropped-Workflow recovery. The fold-forward v0.8
coverage adds strict Complete-Run qualification, full-ceiling exact-byte
streaming, canonical pass/fail evidence, checked-in v1 evidence fixtures,
exact reconciliation, boundary/over-boundary allocation gates, and
non-mutation cases. The private fault and algorithm-accounting scope is
documented precisely in the [verification
strategy](docs/architecture/testing.md).

The checked-in `terrain-demo` Criterion benchmark exercises five restart modes
through the public workflow facade with generated local LAS data. The local
10,000-Point smoke used ten samples and reported these confidence intervals:

| Mode | Lower | Estimate | Upper |
|---|---:|---:|---:|
| Cold start | 153.38 ms | 157.84 ms | 161.25 ms |
| Resume after committed Edit | 113.23 ms | 114.88 ms | 117.08 ms |
| Resume from retryable Workspace intent | 123.76 ms | 126.67 ms | 129.66 ms |
| LandXML and report reconciliation | 96.871 ms | 97.629 ms | 98.365 ms |
| Complete revalidation | 87.233 ms | 88.181 ms | 89.112 ms |

The completed Run had an eight-frame 2,804-byte journal and an 11,490-byte
canonical report containing 115 semantic limit facts. The benchmark accepts
only the documented generated 10,000, 100,000, and 1,000,000-Point modes through
`PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS`; only the 10,000-Point smoke is recorded
here. These are generated local technical observations. Worker peak heap was
not measured, and partner, production, downstream round-trip, and human-time
acceptance remain unmeasured.

## v0.10 reproducible viewing measurement

The checked-in renderer viewing microbenchmark defaults to 100,000 generated
Points and accepts a positive size through ten million:

```bash
PUNCTRA_RENDERER_VIEW_BENCH_POINTS=1000000 \
  cargo bench -p renderer-demo --bench viewing
```

It measures a warm verified position-only index open and first bounded root
display batch, and prints generated Point/node counts plus artifact and
observed index-temporary bytes. No reference number is published here until a
named local run is retained. This generated CPU/index benchmark is distinct
from the permission-gated GPU corpus runner and proves neither production
scale nor professional workflow performance.

## Development

Install the pinned Rust toolchain, then run the authoritative local verification
sequence from [CONTRIBUTING.md](CONTRIBUTING.md):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo bench -p point-view --bench planner
cargo bench -p source-memory --bench read
cargo bench -p source-las --bench read
cargo bench -p point-index --bench index
cargo bench -p point-workspace --bench document
cargo bench -p point-review --bench review
cargo bench -p point-terrain --bench terrain
cargo bench -p terrain-demo --bench journal -- \
  --save-baseline "qualification-$$-$(date +%s)"
cargo bench -p renderer-demo --bench viewing
cargo run -p point-index --example direct_use
cargo test -p point-review --test interface
cargo test -p render-protocol --test state_model
cargo run --release -p point-workspace --example classify -- \
  survey.laz survey.laz.pidx survey.pcw 6
cargo run -p point-terrain --example derive
PUNCTRA_REQUIRE_GPU=1 cargo run -p render-wgpu --example third_party_host
cargo test -p terrain-demo --lib --all-features
cargo test -p terrain-demo --test workflow
cargo test -p terrain-demo --test process
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test headless_smoke \
  corpus_success_binds_trace_inputs_and_separate_resource_measurements -- --exact
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test display_gpu
test -f docs/guides/first-las-laz.md
ruby -rjson -e 'JSON.parse(File.read(ARGV.fetch(0)))' \
  docs/guides/field-corpus.example.json
git diff --check
```

Punctra currently targets Rust 1.90 and wgpu 30. The renderer demo requires a graphics
adapter supported by wgpu; renderer-neutral protocol tests do not.

GPU-backed tests are separated from renderer-neutral contract tests so the
protocol remains testable on machines without a graphics adapter. The required
commands above set `PUNCTRA_REQUIRE_GPU=1` so a missing headless adapter is a
local test failure. All repository verification is run locally; no hosted CI
workflow is configured.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE)); or
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
