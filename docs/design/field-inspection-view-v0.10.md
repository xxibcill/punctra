# Field Qualification and Professional Inspection View Design (v0.10)

Status: **Accepted and Active — repository implementation complete; field and
adoption-publication evidence outstanding**

This design is authoritative for the Punctra v0.10 repository track. Its
development base was commit `3dc4cb1`, the `0.9.0-alpha.1` state. The branch is
now reconciled with the completed v0.9 repository candidate, including the six
post-base v0.8 comparator correctness and regression commits and the final v0.9
qualification fixes.

The instruction to start v0.10 accepted and activated repository
implementation. The resulting code, fixtures, local measurement path, and
documentation do not manufacture external evidence. No permitted production
corpus, observed professional workflow, workstation baseline, failure
baseline, or current-time baseline has been recorded in this repository. Those
field gates remain prerequisites to any field-qualified or support-qualified
claim.

## Outcome

Qualify Source opening and progressive viewing on representative field data and
make known survey features clear enough for professional inspection without
letting sampled GPU presentation masquerade as an exact result.

The deletion test for v0.10 is specific: removing this work would remove the
field-corpus measurement path and the explicit professional display choices,
but would not change authoritative Source, Query, Edit, terrain, QA, or export
semantics.

## Evidence state

Repository delivery status and external evidence maturity are independent.

| Evidence | Current state |
|---|---|
| Accepted design and repository implementation | Complete |
| Later v0.8 comparator correctness fixes reconciled | Present |
| Position-only disk-v1 and attributed disk-v2 fixtures | Present |
| Five CPU mappings and local GPU mapping regression | Present |
| Perspective/orthographic View and planner regression | Present |
| Permission-gated local corpus runner and report contract | Present |
| Licensed or sanitized production Source with permission to inspect | Outstanding |
| Observed workflow, workstation, failure mode, and current-time baseline | Outstanding |
| Five-project, three-firm corpus with two Sources above 500 million Points | Outstanding |
| Observed professional feature-location trials | Outstanding |
| v0.9 independent Standards/Spec review | Complete — 2026-08-13, zero P0–P3 findings on both axes |
| v0.9 complete one-commit local candidate record | Complete — recorded with the v0.9 release evidence |

Generated fixtures, public example data, maintainer-operated runs, declared
application labels, and repository tests may prove code behavior. They do not
count as production, partner, usability, paid-use, or accepted-deliverable
evidence.

## Terms

### Display Mode

A host-selected rule that chooses which non-authoritative value is encoded as
the color of each displayed Point. The implemented modes are neutral,
elevation, RGB, intensity, and classification. Unavailable required input
fails explicitly; it is never guessed or silently replaced with another mode.

### Display Mapping

A deterministic CPU conversion from explicit sampled inputs to linear RGBA8
bytes accepted by `render-protocol::RenderPoint`. It is presentation policy,
not a Query or an Attribute conversion. GPU upload must preserve the emitted
bytes exactly.

### Display Sample

A versioned, bounded, identity-preserving subset used only for progressive
display. Sampled Coverage is never complete Query Coverage, even when every
sample value originated from the authoritative Source.

### Field Corpus

A local manifest of permitted Sources and non-sensitive observation metadata
used to reproduce opening, indexing, first-use, navigation, residency, memory,
and disk measurements. Source redistribution permission is separate from
permission to inspect or measure it.

### Viewing Report

A bounded record of one corpus run containing declared machine and operation
inputs plus measured viewing facts. It makes no terrain, partner, support, or
human-time claim beyond the observations actually attached to it.

## Architecture boundary

The existing module split remains authoritative:

- `point-contracts` and `point-source` retain exact Source values and typed
  Attribute projection;
- `point-index` retains the complete rebuildable hierarchy, version-1
  position-only display samples, and the narrow version-2 inspection sample
  recipe defined below;
- `point-view` plans deterministic demand without I/O or display policy;
- `render-protocol` accepts caller-supplied RGBA8 and does not learn semantic
  display modes;
- `render-wgpu` uploads and draws those bytes without becoming authoritative;
  and
- private `renderer-demo` code owns mode selection, display mapping, and the
  first corpus runner because it is the only current real-cloud View host.

No new public crate or foundation seam is justified by one caller. A reusable
display-policy interface requires a second real caller and its own accepted
contract. The two point-index preparation operations added below are index
ownership contracts rather than display policy: attributed recipes bind
persisted bytes, while fresh preparation atomically proves an absent target
family for both corpus and benchmark cold-build measurements.

## Implemented display mappings and CLI

The CLI grammar is:

```text
renderer-demo [--smoke]
  [--display neutral|elevation|rgb|intensity|classification]
  [--projection perspective|orthographic]
  [SOURCE [INDEX_TARGET]]
```

`neutral` and `perspective` remain the defaults. Every non-neutral mode
requires a real LAS/LAZ Source so the no-argument synthetic demonstration
retains its authored colors. Duplicate mode/projection options, missing values,
unknown options, unavailable Attributes, and incompatible index recipes fail
explicitly. Repeating the idempotent `--smoke` switch has the same effect as
specifying it once. With no explicit index target, neutral/elevation append
`.pidx` to the complete Source path, while attributed modes append
`.inspection-v2.pidx`; incompatible recipe families never select one implicit
path.

Neutral emits `[190, 205, 220, 255]` for every real-cloud Point. For each exact
indexed sample position, elevation mapping:

1. decodes exact world Z with the Source `PositionTransform`;
2. normalizes it against the complete Source world-bounds Z minimum and
   maximum;
3. clamps the result to the inclusive unit interval;
4. uses `0.5` when the Source has zero Z extent; and
5. linearly interpolates adjacent RGBA8 stops in this fixed viridis-style
   palette, with alpha `255`:

| Normalized elevation | RGB |
|---:|---:|
| `0.00` | `[68, 1, 84]` |
| `0.25` | `[59, 82, 139]` |
| `0.50` | `[33, 145, 140]` |
| `0.75` | `[94, 201, 98]` |
| `1.00` | `[253, 231, 37]` |

Adjacent elevation stops are interpolated per channel and rounded to the
nearest byte. For RGB, each raw `U16` channel `v` becomes
`(v * 255 + 32767) / 65535` by integer division. Intensity applies the same
rule once and repeats the byte across red, green, and blue. Alpha is always
`255`.

Classification values 0–18 use this fixed table:

| Class | RGBA8 | Class | RGBA8 |
|---:|---:|---:|---:|
| 0 | `[120,120,120,255]` | 10 | `[60,100,210,255]` |
| 1 | `[155,155,155,255]` | 11 | `[40,180,210,255]` |
| 2 | `[139,95,57,255]` | 12 | `[230,170,60,255]` |
| 3 | `[80,180,80,255]` | 13 | `[220,120,40,255]` |
| 4 | `[45,150,45,255]` | 14 | `[235,80,150,255]` |
| 5 | `[20,110,20,255]` | 15 | `[170,70,170,255]` |
| 6 | `[220,70,70,255]` | 16 | `[255,220,80,255]` |
| 7 | `[200,200,200,255]` | 17 | `[100,220,190,255]` |
| 8 | `[170,120,220,255]` | 18 | `[245,245,245,255]` |
| 9 | `[80,150,230,255]` | | |

For any value `c` from 19 through 255, the RGB bytes are wrapping `u8`
arithmetic `(73c + 41, 151c + 97, 199c + 17)`. The mapping is defined for all
256 values and never assigns semantic meaning beyond the raw class byte.

These palettes are initial inspectable defaults, not evidence that
professionals prefer them. Every mode preserves Point Identity, batch origin,
position, Coverage, generation, version, planning, residency, and staging
limits. A mode changes only the four display color bytes constructed by the
host.

## Attributed display samples

RGB, intensity, and classification modes need Source Attributes. Internal
Spatial Index v1 samples persist only ordinal and position ticks, deliberately
avoiding expensive sparse LAZ replay before the first visible frame. v0.10
does not disguise that limitation with unbounded or latency-unstated reads.

Attributed modes select `IndexRecipe::InspectionV1` and disk version 2. The
existing `prepare` interface remains the exact disk-v1/recipe-v1 position-only
path. `prepare_with_recipe` adds explicit persisted-recipe selection.
`prepare_fresh_with_recipe` adds the orthogonal no-resume policy required for
an honest cold-build measurement: it preserves and rejects an existing or
concurrently appearing complete/work path before consuming it. Both operations
remain generic point-index ownership contracts, have direct interface tests,
and are used independently by the point-index/viewing benchmarks and the
private corpus host. A successful build retains its recognized rebuildable
`.work` cache. This avoids a check-then-unlink race against caller bytes because
portable filesystems do not provide identity-conditional unlink for the
predictable work pathname; callers may remove the cache family explicitly when
no preparation is active.

The inspection recipe binds exactly five caller-supplied Attribute identities:
intensity is required `U16`, classification is required `U8`, and red, green,
and blue are either all absent or all present as `U16`. Partial or mistyped RGB
and missing required Attributes fail before filesystem mutation. RGB absence
is a recorded unavailable capability with encoded RGB values fixed at zero;
the host rejects RGB mode instead of silently substituting another field.

Disk v2 preserves the v1 magic and fixed hierarchy/node encoding. Its artifact
header is 240 bytes: the v1 208-byte header followed by display schema version
`1`, capability bits, five little-endian `u32` Attribute identities, and sample
record width `42`. Its work header is the v1 168-byte body plus the same
32-byte extension and a 32-byte checksum, totaling 232 bytes. A v2 sample is:

| Offset | Field |
|---:|---|
| 0 | Source ordinal `u64 LE` |
| 8, 16, 24 | x, y, z ticks `i64 LE` |
| 32 | intensity `u16 LE` |
| 34 | classification `u8` |
| 35 | reserved zero `u8` |
| 36, 38, 40 | red, green, blue `u16 LE` |

Bottom-k ordinal priority and sorted-unique output are unchanged. Raw
Attributes travel with each selected ordinal through the block accumulator and
bottom-up merges. Internal nodes read persisted samples; leaves issue one
contiguous Source read projecting the bound Attributes. The v2 sample hash
domain is `punctra-index-samples-v2`; v1 remains unchanged.

Byte ceilings use 32 bytes per v1 persisted/display sample or 42 per v2 sample.
Attributed Source projection charges 27 bytes per Point without RGB or 33 with
RGB, including position. Work frames, sample spool, final Artifact, read
buffers, emitted batches, and build memory remain explicitly bounded.
`PrepareReport::peak_temporary_disk_bytes` is the exact observed combined peak
of the retained rebuildable work cache, sample spool, and unpublished
complete-artifact temporary for build/resume, counting each owned file's
logical length once. It is zero for an existing complete open.

Disk v1 complete and work fixtures remain supported byte-identically. A v1
target requested through v2, or vice versa, fails without replacement. Unknown
versions fail explicitly. Migration is a caller-owned move/delete of the
rebuildable target, `.work`, and construction sidecar family followed by
rebuild; point-index never silently deletes or overwrites an incompatible
cache.

Only Attributes required by accepted modes may be retained. Runtime point
schemas, arbitrary shader inputs, GPU-authoritative Attributes, and copying all
Source Attributes for convenience remain excluded.

## Navigation, appearance, and state

The implemented initial policy supports left-drag orbit, middle-drag pan,
wheel zoom, `P` projection toggle, `R` camera reset that preserves projection,
`H` stable-identity highlight toggle, Space materialization pause/resume, and
Escape. Perspective is the default. Orthographic preserves the perspective
target-plane scale when toggled and is represented explicitly across camera
protocol, View frustum/SSE planning, renderer depth/picking, and the host
controller. Professional preference, contrast, and future depth enhancement
still require observed workflow evidence.

The host distinguishes:

- Source verification;
- index build, resume, or open;
- demanded nodes, load candidates, actually issued requests, retention,
  retirement, queue/staging, and requested/resident display nodes;
- Sampled versus Complete Coverage;
- paused, failed, cancelled, and resource-limited work; and
- disposable display values versus exact CPU results.

Pausing issues zero new requests but does not stop planning or safe retirement.
The title always labels resident display Coverage `Sampled` and `Complete` and
states that it is not Query completion.

Errors use one stable code from `PVIEW_INVALID_REQUEST`, `PVIEW_SOURCE`,
`PVIEW_INDEX`, `PVIEW_RESOURCE_LIMIT`, `PVIEW_CANCELLED`, `PVIEW_GPU`,
`PVIEW_IO`, or `PVIEW_INTERNAL`; one owning phase; at most 1,024 bytes of
detail; and exactly one safe action. A loading indicator or color never claims
exact completion.

## Field corpus and measurements

The reproducible local corpus path measures these operations separately:

- Full Source verification;
- cold index build and warm index open;
- time to first accepted visible batch;
- a declared navigation trace;
- peak queued, staged, and resident display resources;
- retained index and temporary disk bytes; and
- failure code, phase, and recovery action when the run does not complete.

Each entry requires a fresh absent index target. The first prepare must report
`Built`; an existing or resumable target is preserved and rejected. The runner
then performs and records a distinct immediate prepare that must report
`Opened`, so “cold build” and “warm open” are never inferred from one timing.

The JSON manifest is capped at 1 MiB, 64 entries, 128 finite navigation steps
per entry, and 256 submitted frames per initial/trace pose. It rejects unknown fields and requires both
`inspect_permission` and `measure_permission` to be true before opening a
Source. Every entry names opaque local corpus/project/firm identities, local
Source and index paths, display/projection choice, and its trace.

Every report binds Source identity, point count, format, index recipe and disk
versions, display mode, projection, limits, executable version, declared and
observed machine facts, measurement disposition, Coverage, and any structured
failure. The canonical report is capped at 4 MiB and published from a synced
stage without replacement; exact existing bytes reconcile and different bytes
conflict. It omits Source/index paths and project/firm identifiers, but Source
identity and machine facts may remain sensitive.

The report explicitly records false nonclaims for production-corpus
completion, partner acceptance, professional preference, terrain resource
envelope, and human-time savings. Running the local corpus command does not
approve the manifest or report for publication.

Opening and viewing a Source does not establish that deriving complete terrain
fits the same resource envelope. Viewing reports never extrapolate terrain,
partner, downstream, or human-time performance.

The checked-in generated microbenchmark is reproducible with
`cargo bench -p renderer-demo --bench viewing`; the optional
`PUNCTRA_RENDERER_VIEW_BENCH_POINTS` accepts a positive size through ten
million and defaults to 100,000. It measures warm verified position-only index
open and first bounded root display batch without a GPU. It is not a substitute
for a permitted Field Corpus run, and no reference number is claimed here
without retained output from a named machine.

## Verification

Every implementation slice runs the complete local sequence in
`CONTRIBUTING.md`. In addition, v0.10 requires:

- exact unit tests for normalization, clamping, degenerate bounds, palette
  stops, interpolation, `U16` scaling, all 256 classification inputs, and byte
  rounding;
- disk-v1/v2 golden reopen/resume, cold/warm read, exact Attribute alignment,
  incompatible-target preservation, accounting, corruption, and limit tests;
- process tests for default neutral behavior, all four explicit Source modes,
  perspective/orthographic selection, rejected CLI grammar, and generated LAS
  and LAZ build/open paths;
- identity and geometry regressions proving a mode switch changes only color;
- tolerant offscreen GPU tests for each accepted mapping, orthographic depth,
  large-world picking, and planner-to-renderer retirement;
- deterministic corpus-report fixtures and hard resource-limit failures; and
- `PUNCTRA_REQUIRE_GPU=1` for the documented GPU acceptance tests when the
  local adapter is expected.

The v0.10 GPU commands are exactly:

```bash
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test display_gpu
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test headless_smoke \
  corpus_success_binds_trace_inputs_and_separate_resource_measurements -- --exact
```

All verification runs locally. This design does not add or authorize hosted
CI.

## Non-goals

v0.10 does not add:

- exact screen selection, Attribute inspection, classification correction, or
  general editing;
- arbitrary persisted Attribute schemas, shader-defined display recipes, or a
  migration beyond caller-owned rebuild between the accepted disk v1 and v2;
- Source rewriting or GPU-authoritative position, identity, or Attribute data;
- automatic coordinate-reference, unit, datum, or display-mode guessing;
- persistent or constrained terrain, profiles, Breaklines, or new export;
- a general desktop, CAD, BIM, point-cloud, globe, or geospatial editor;
- Cesium parity, 3D Tiles, imagery, textures, photorealism, or every-Point
  rendering;
- networking, cloud storage, telemetry upload, or redistribution of private
  corpus data; or
- field-qualified, partner-validated, support-qualified, or v1 claims based on
  repository work alone.

## Exit gates

Repository delivery may be called complete only when the accepted display,
state, diagnostic, corpus, documentation, and local verification slices pass.
Field qualification additionally requires:

- five permitted projects from at least three unrelated firms, including at
  least two Sources above 500 million Points;
- declared workstation viewing ceilings holding across the measured corpus;
- exact CPU-to-GPU mapping and tolerant local GPU regressions for every
  supported mode; and
- observed users locating known features without mistaking sampled display
  values for exact results.

The open-source adoption exit separately requires one accurate public
description, repository topics and homepage, an approved screenshot or short
demonstration, a permitted published reproducible viewing benchmark, and a
five-minute first-LAS/LAZ guide. The repository now contains the description,
local runner, and guide; topics/homepage publication, approved visual material,
and a permitted published benchmark remain outstanding. None may overstate the
evidence table above.

## Repository implementation record

1. Reconciled inherited comparator fixes, accepted this design, bumped release
   metadata, and implemented neutral/elevation mapping without changing v1.
2. Added the bounded v2 inspection contract and RGB/intensity/classification
   mapping while retaining frozen v1 behavior and explicit rebuild migration.
3. Added perspective/orthographic navigation, exact matching View planning,
   truthful loading/Coverage state, and structured View diagnostics.
4. Added the permission-gated corpus manifest, reproducible local runner,
   canonical report/nonclaims, local GPU regressions, and first-file guide.
5. Kept field qualification, approved public visual material, topics/homepage,
   a permitted published benchmark, partner evidence, and support
   qualification explicitly outside repository-generated proof.

No slice may call v0.10 field-qualified or claim professional preference,
production scale, partner value, downstream acceptance, support readiness, or
v1 status without the corresponding evidence.
