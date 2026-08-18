# Renderer quality investigation — 2026-08-18

Status: local investigation complete; bounded corrective design accepted and
convergence remediation implemented in the working tree

Repository state: `main` at `87b8476`, workspace version `0.12.0-alpha.1`

## Purpose

This document records a local inspection of Punctra's current rendered output,
the evidence collected during that inspection, and the quality gaps that must
be closed before the View can be described as a professionally legible and
settled inspection experience.

The investigation distinguishes three different questions:

1. whether the GPU renderer preserves the accepted geometry, depth, color,
   identity, and lifecycle contracts;
2. whether the private `renderer-demo` host converges to a stable progressive
   View; and
3. whether the displayed result communicates depth, spatial context, loading
   state, and interaction state clearly enough for professional inspection.

A correct renderer is necessary but does not by itself answer the second or
third question.

## Scope and limitations

The visual inspection used the built-in deterministic 16,777,216-Point
synthetic scene at its default camera, plus its perspective/orthographic and
highlight controls. No private or production Source was accessed. RGB,
intensity, elevation, and classification behavior for a real LAS/LAZ Source
was assessed only from the implemented mapping contracts and GPU acceptance
tests, not from a visual field-data trial.

The run used:

- macOS on an Apple M5 Pro;
- the Metal backend and integrated Apple M5 Pro adapter;
- the optimized `renderer-demo` binary;
- a 1280 by 832-point window on a Retina display; and
- the default 600,000-Point, 14,400,000-byte, 640-batch resident ceilings.

This is a local generated-fixture observation. It is not field qualification,
professional preference, production-corpus evidence, a workstation support
promise, or proof of downstream acceptance.

## Commands exercised

The interactive result was opened with:

```bash
cargo run --release -p renderer-demo
```

The following focused verification passed locally:

```bash
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test display_gpu
cargo test -p renderer-demo --test headless_smoke
```

Observed results:

| Verification | Result | Relevant guarantee |
|---|---:|---|
| `render-wgpu` offscreen acceptance | 10 passed | Circular splats, depth, highlights, picking, projection, large-world precision, atomic lifecycle, and recorded-frame behavior |
| planner-to-renderer GPU acceptance | 1 passed | Progressive Coverage retention and exact conditional retirement |
| display mapping GPU acceptance | 1 passed | Accepted CPU display bytes survive GPU upload and drawing |
| renderer headless smoke | 12 passed | Synthetic and LAS/LAZ bridge behavior, diagnostics, recipes, and exact-review smoke |

The focused checks provide strong evidence for the narrow contracts they
exercise. They do not currently prove that a stationary multi-frame View
converges or that its point-density transitions are visually inconspicuous.

## Executive assessment

| Dimension | Assessment | Summary |
|---|---:|---|
| GPU/render correctness | 9/10 | Strong focused acceptance coverage; no observed geometry, identity, depth, or color-contract failure |
| Performance on the observed adapter | 8/10 | High reported frame rate and very low encode/upload intervals, but continuous churn invalidates a simple efficiency conclusion |
| Point-cloud legibility | 6/10 | Dense and immediately recognizable, with visible grid, moiré, and LOD-tile discontinuities |
| Professional inspection readiness | 5/10 | Missing depth enhancement, legend, scale, orientation, coordinate readout, and clear interaction feedback |
| Overall current render quality | 6.5/10 | Technically credible alpha inspection host; not yet a settled professional View |

The current result is best described as a strong technical renderer presented
through an alpha-grade inspection host. The core GPU path is substantially
more mature than the visual communication and host convergence behavior.

## What the current result looks like

The default frame uses a nearly black, subtly blue background. A dense point
surface fills approximately the lower two-thirds of the window and leaves a
large unoccupied area above the terrain. The surface reads as one central
elevated landform with ridges and low channels. Perspective gives the model a
clear overall silhouette; orthographic projection switches successfully and
preserves the target-plane scale.

At the default zoom, the 2.4-physical-pixel circular splats are large enough to
make individual samples recognizable while dense regions approach a continuous
surface. The dark background gives cyan, green, tan, and near-white points
strong separation. The appearance is custom and technical rather than a
generic glowing or glass-like interface.

The same density also exposes the regular synthetic grid. Long aligned rows
form moiré-like bands, and changes in sampled density appear as rectangular
tiles. Some foreground regions look like discrete platforms or blocks even
though the synthetic height function is continuous. The illusion is created
by point alignment, fixed screen-space splat size, and visibly different LOD
density rather than authored vertical faces.

## Point appearance

The renderer expands every Point to one screen-facing quad and discards
fragments outside a unit circle. The demo selects:

- a fixed 2.4-physical-pixel Point diameter;
- an orange-red highlight color `[1.0, 0.24, 0.06]`; and
- a dark clear color `[0.008, 0.012, 0.02, 1.0]`.

What works:

- circular splats avoid the harsh square-pixel appearance of raw point sprites;
- the outline remains clean against the background;
- constant physical-pixel size keeps distant samples visible;
- source alpha is preserved when highlighting; and
- GPU picking uses the same circular coverage rule as drawing.

What remains weak:

- one fixed diameter cannot match both sparse and dense projected spacing;
- overlapping dense samples brighten regions and emphasize tile boundaries;
- sparse regions expose holes and regular rows;
- no transition treatment conceals parent/child density replacement; and
- there is no user or host policy for point-size adjustment by scene,
  projection, density, or workstation.

A later design should compare projected-spacing-aware point size, bounded
parent/child transition treatment, and their interaction with deterministic
picking. It must not blur or invent authoritative geometry.

## Depth and surface readability

The narrow depth contract is strong. Drawing uses a `Depth32Float` attachment,
depth writes, and `LessEqual` comparison. Occlusion and pick identity passed the
focused offscreen suite, including orthographic projection and billion-unit
world origins.

Perceptual depth is weaker. The View currently has no:

- eye-dome lighting or comparable screen-space depth cue;
- normal-based shading;
- ambient occlusion;
- depth fog;
- contours;
- ground or horizon reference;
- bounding box; or
- adjustable contrast/background treatment.

The user must infer shape primarily from perspective, occlusion, point density,
and color. Broad landforms remain understandable, but shallow slopes, small
breaks, overlapping layers, and density transitions are harder to separate.
Regular point alignment can create false wall or terrace impressions.

One bounded, deterministic, display-only depth enhancement should be selected
by an accepted design. Eye-dome lighting is the leading candidate because it
does not require authoritative surface normals, but the design must specify
its GPU resource ceiling, edge behavior, pick independence, disabled/fallback
path, and tolerant image acceptance.

## Color presentation

The authored synthetic fixture assigns five discrete ranges:

| Height | Approximate appearance |
|---:|---|
| below `-14` | blue |
| `-14` to below `1` | green |
| `1` to below `17` | olive green |
| `17` to below `29` | tan |
| `29` and above | near white |

The palette gives the scene an immediate elevation-like reading, but the hard
thresholds can suggest false contour discontinuities. Near-white peaks also
receive much more visual emphasis than darker low regions.

Real-cloud mappings are deterministic and presentation-only:

- neutral uses one fixed blue-grey;
- elevation uses the accepted continuous viridis-style ramp;
- RGB maps each exact `U16` channel directly to `U8`;
- intensity maps exact `U16` intensity directly to grayscale; and
- classification maps every raw class byte to one fixed opaque color.

The deterministic raw mappings are appropriate reference modes. They are not a
complete visualization policy. A narrow inspection host may additionally need
explicit, reversible display-only tone controls for RGB and intensity. Any such
control must expose its parameters, preserve a raw reference mode, never alter
Source values, and never silently substitute one display mode for another.

The canvas currently has no palette legend, class/value explanation, or visible
display-mode label. Meaning therefore depends on documentation and memory.

## Perspective and orthographic behavior

`P` successfully changed between perspective and orthographic projection while
preserving the apparent scale at the target plane. No large jump, inversion,
or obvious loss of depth ordering was observed.

Orthographic projection makes tile-shaped density changes easier to see because
perspective no longer masks them with distance scaling. It also demonstrates
why scale, orientation, and projection state should be present on the canvas
rather than only near the end of a truncated title.

## Performance observation

Typical title telemetry on the observed adapter reported:

- 97 to 120 frames per second;
- approximately 8.3 to 10.3 milliseconds for the reported frame interval;
- approximately 0.01 to 0.08 milliseconds of command encoding;
- approximately 0.04 to 0.06 milliseconds for one displayed upload;
- 117 to 118 draw calls;
- approximately 119,800 to 120,800 drawn Points; and
- approximately 2.7 MiB of resident Point vertices.

These are promising local observations. The title's `16.7M logical` value is
the authored scene size, not the simultaneous draw count. The rendered frame
contained about 120,000 Points under a 600,000-Point ceiling.

Frame-rate and frame-interval facts in the title are sampled differently and
should not be combined into a precise benchmark claim. A reproducible frame
benchmark must define warm-up, settlement, presentation mode, refresh/VSync,
capture interval, adapter, viewport, and whether time includes acquire/present.

## Major finding: stationary View churn

The default camera was left completely stationary. The View did not converge
to `steady`; cumulative work continued while resident content stayed almost
unchanged.

Over one observed ten-second interval:

| Fact | First observation | Ten seconds later | Change |
|---|---:|---:|---:|
| resident Points | about 120,800 | about 120,800 | effectively unchanged |
| resident Point bytes | 2.7 MiB | 2.7 MiB | unchanged |
| cumulative uploaded bytes | 55.5 MiB | 99.8 MiB | +44.3 MiB |
| retired batches | 2,254 | 4,144 | +1,890 |
| cancelled requests | 517,307 | 958,622 | +441,315 |

The title continued reporting approximately 462 demanded/candidate/issued
nodes, 117 retained nodes, one immediate retirement, and `streaming`. A later
snapshot before pausing showed:

- 176.8 MiB cumulatively uploaded;
- 7,429 retired batches;
- 1,725,897 cancelled requests; and
- approximately 119,800 resident Points.

After Space paused loads, the queue and issued-request count fell to zero and
the displayed resident set stabilized.

This is the highest-priority finding. A stationary deterministic scene should
reach a stable cut within a declared bounded number of frames. Continuous
request, upload, cancellation, and retirement without a material improvement
in resident display indicates host/planner convergence or in-flight budgeting
behavior that the current focused acceptance does not cover.

Consequences include:

- visible tile-density changes or flicker;
- unnecessary CPU/GPU work and memory bandwidth;
- unnecessary energy use;
- misleading `streaming` state;
- harder reproduction of image-quality observations; and
- higher risk on less capable adapters or I/O-backed real clouds.

The evidence does not show a core depth, identity, or atomic-update failure.
The dedicated regression below localizes the narrower cause within the
multi-frame planner/host integration.

## Convergence remediation observation

The accepted
[pre-v0.13 corrective design](../design/renderer-quality-corrective-pre-v0.13.md)
localizes the cycle to planner-history replay and private host request
admission. The original behavior was reproduced through the existing public
`ViewPlanner::plan` and private Scene interfaces with the same generated
hierarchy, default camera, 2560-by-1664 physical-pixel viewport, residency
limits, and one-batch-per-frame materializer.

Before the correction, frame 2,048 still alternated between a seven-node
coarse demand and a 462-node fine demand. The fine frame issued 462 requests,
uploaded one batch, retired one batch, retained 118 batches and 120,832 Points,
left 461 requests queued, and had accumulated 441,653 cancellations and 1,930
retirements.

After the correction, the same deterministic host-state acceptance reached its
first quiet frame at frame 780 with 583 resident batches, 596,992 resident
Points, no queued request, no cancellation, and 195 cumulative transition
retirements. The next 300 stationary pumps produced no demand, request, issue,
upload, cancellation, retirement, or node-state change. Camera movement,
projection switching, resizing, refinement, coarsening, pause/resume, and reset
also reconverged in the focused acceptance.

This is a generated CPU host/planner lifecycle result using the renderer state
model. After the correction, forced local GPU acceptance re-passed all ten
`render-wgpu` offscreen cases, the planner-to-renderer Coverage transition, the
display-mapping handoff, the GPU corpus process case, and the third-party host
example on the expected local adapter. These results do not supersede the
remaining image-quality work, real-cloud observation, field qualification, or
workstation support evidence.

## LOD transition quality

The acceptance suite proves that retained ancestors prevent a missing target
from destroying progressive Coverage and that conditional retirements are
generation safe. The still frame nevertheless has conspicuous tile-shaped
density and brightness boundaries.

Correct logical Coverage and visually uniform Coverage are different
requirements. Remediation needs both:

1. deterministic multi-frame tests showing that resident and in-flight work
   converge without oscillation; and
2. tolerant local GPU image checks at fixed cameras showing that parent/child
   replacement does not create holes, conspicuous density steps, or prolonged
   mixed-LOD artifacts beyond an accepted transition window.

The acceptance must cover perspective, orthographic, reset, resize, pause,
resume, and a camera movement that crosses a refinement threshold.

## Selection and highlight feedback

Toggling `H` changed the title state but produced no obvious canvas change in
the default frame. The synthetic fixture names only three highlight Point
identities. A selected identity may not be resident, and a resident
2.4-pixel Point can be difficult to locate in a dense scene.

The technical highlight contract remains useful, but the host needs explicit
feedback:

- selected exact Point count;
- resident-highlight count;
- a visible nonresident/stale state;
- an unmistakable marker, halo, pulse, or temporary locator that does not
  change exact selection semantics;
- clear/escape affordance; and
- optional zoom-to-selection owned by the host.

Color alone is not sufficient selection feedback, especially for a tiny or
nonresident result.

## Spatial and inspection context

The current canvas omits:

- axis or north/orientation indicator;
- scale bar;
- cursor world-coordinate readout;
- elevation/classification legend;
- visible display-mode and projection labels;
- bounding box or ground reference;
- settled/sampled/complete Coverage indicator;
- point-size or contrast controls; and
- measurement or clipping context.

Without those cues, the cloud is visually attractive but spatially anonymous.
A professional cannot determine orientation, distance, palette meaning, or
whether a subtle feature is authoritative, sampled, unsettled, or purely a
display artifact from the canvas alone.

The first corrective slice should add only context required to interpret the
existing narrow View. General CAD authoring, broad measurement tooling, and a
complete desktop UI remain separate later decisions.

## Status presentation and cognitive load

The window title carries logical/resident Points, bytes, draw calls, frame and
upload timing, LOD demand, candidates, issue/retention/retirement, queue and
staging facts, node states, Coverage, projection, review, streaming, and
highlight state. The operating system truncates the title well before the
facts most users need.

This creates several problems:

- advanced diagnostics compete with the primary View;
- important state is present but not visible;
- related facts are not grouped;
- the user must remember console-only controls; and
- sampled, complete, authored, streaming, and settled distinctions are too
  difficult to parse quickly.

A compact on-canvas status layer should show only:

- display mode and projection;
- loading state and truthful Coverage;
- drawn/resident Point count;
- selection/highlight state; and
- one actionable failure or warning.

Detailed planner, queue, staging, cancellation, retirement, memory, and timing
facts should move to an expandable diagnostics view or structured report. The
title should retain only a stable product/view name and one short high-level
state if required by platform conventions.

The hard-coded title identifies the View as `v0.11` while the workspace is
`0.12.0-alpha.1`. If `v0.11` refers to the exact-review feature slice, the
wording must say so. Otherwise the displayed version should follow the package
version from one source of truth.

## Usability health assessment

This score applies only to the private demo host.

| Heuristic | Score | Main gap |
|---|---:|---|
| Visibility of system status | 3/4 | Extensive facts exist but are truncated and poorly prioritized |
| Match to professional language | 2/4 | Planner/residency terminology dominates the presentation |
| User control and freedom | 3/4 | Orbit, pan, zoom, reset, projection, and pause are available |
| Consistency and standards | 3/4 | Core controls and visual behavior are consistent |
| Error prevention | 3/4 | Strong input validation and immutable-Source boundary |
| Recognition rather than recall | 1/4 | Canvas does not expose controls, legends, or meanings |
| Flexibility and efficiency | 2/4 | Useful shortcuts, but no visual controls and continuous background churn |
| Aesthetic and minimalist design | 2/4 | Clean canvas; overloaded title and missing hierarchy |
| Error recovery | 3/4 | Structured diagnostics and one safe action are implemented |
| Help and documentation | 2/4 | Good guide, little contextual help in the View |
| **Total** | **24/40** | **Acceptable technical host; significant inspection UX work remains** |

The cognitive-load checklist has five failures: chunking, grouping, minimal
visible choices, recognition/working-memory support, and progressive
disclosure. The cloud itself has a clear focus; the status and interaction
model do not.

## Prioritized findings

### P1 — Stationary demand does not converge

Why it matters: persistent churn undermines visual stability, wastes resources,
and makes `streaming` untrustworthy.

Required outcome: a fixed camera, viewport, scene state, budgets, and
projection reach one deterministic resident/in-flight cut within an accepted
frame ceiling and remain there until an input changes.

### P1 — LOD and point-density transitions are visually conspicuous

Why it matters: tile boundaries and grid artifacts can be mistaken for real
terrain features.

Required outcome: point appearance and parent/child transition policy make
Coverage changes legible without visible holes, false platforms, or prolonged
density discontinuities.

### P2 — Depth perception is too dependent on color and perspective

Why it matters: shallow breaks and overlapping structures are difficult to
inspect reliably.

Required outcome: one bounded optional depth cue, with a safe fallback and
tolerant GPU acceptance, materially improves fixed professional feature trials.

### P2 — Telemetry is overloaded and truncated

Why it matters: the system has status facts but users cannot interpret the
important ones quickly.

Required outcome: a compact primary status overlay and separate expandable or
recorded diagnostics.

### P2 — Spatial meaning is missing from the canvas

Why it matters: orientation, scale, coordinates, projection, palette, and
Coverage cannot be determined without external documentation.

Required outcome: the minimal orientation, scale, coordinate, legend, mode,
projection, and Coverage cues required by the selected workflow.

### P2 — Selection/highlighting can be effectively invisible

Why it matters: users cannot tell whether an action succeeded, is stale, is
nonresident, or simply selected a point too small to see.

Required outcome: explicit selection counts/states and a locator treatment
that does not alter exact CPU selection authority.

### P2 — Real-cloud visual suitability is not yet observed

Why it matters: a synthetic terrain cannot validate vegetation, building
edges, noise, scan patterns, RGB exposure, intensity range, or professional
feature location.

Required outcome: permitted field trials across the five display modes with
known features and declared workstation/source facts.

### P3 — Displayed version identity is ambiguous

Why it matters: a `v0.11` title in a `0.12.0-alpha.1` workspace looks stale and
weakens diagnostic provenance.

Required outcome: one truthful package/view-feature version convention.

## Recommended remediation order

1. Diagnose and stop stationary planner/host churn before assessing final
   image quality.
2. Add deterministic convergence and transition tests that fail on the
   observed behavior.
3. Refine point-size/LOD transition presentation and select one bounded depth
   cue.
4. Add compact status, spatial context, legends, and unambiguous selection
   feedback.
5. Align the displayed version and structured diagnostics.
6. Re-run generated local acceptance, then conduct permitted real-cloud trials
   without converting those observations into unsupported field claims.

## Required evidence before closing the investigation

Repository closure requires:

- a checked-in deterministic stationary-view regression that reaches `steady`
  within the design's declared frame ceiling;
- no request, upload, cancellation, retirement, or resident-set change for a
  declared stationary observation window after settlement;
- budget accounting that includes retained, resident, staged, and in-flight
  work at their owning seams;
- perspective and orthographic transition acceptance for reset, resize,
  pause/resume, refinement, and coarsening;
- tolerant local GPU image regressions for point shape, depth cue, Coverage,
  and highlight visibility;
- exact CPU-to-GPU mapping tests retained for every raw display mode;
- a compact status/context presentation with tests for sampled, complete,
  authored, streaming, steady, paused, stale, selected, and failed states;
- the complete relevant local verification sequence from `CONTRIBUTING.md`;
  and
- an updated investigation record containing before/after facts from the same
  generated fixture and declared adapter.

Field closure remains separate and requires permitted real Sources, known
feature-location trials, workstation and Source facts, and explicit user
observation. A passing generated regression or attractive screenshot does not
satisfy that field gate.

## Non-goals of the corrective work

The findings do not justify:

- making sampled GPU display authoritative;
- rendering every Source Point simultaneously;
- photorealism, meshes, texture streaming, a globe, or Cesium parity;
- general CAD/BIM authoring;
- arbitrary shaders or an unbounded post-processing framework;
- silent tone mapping or guessed classification/coordinate semantics;
- broad measurement, clipping, or annotation features without workflow
  evidence; or
- a field-qualified, partner-validated, or support-qualified claim from this
  local investigation.

## Roadmap disposition

The corresponding work is tracked in `ROADMAP.md` as the **pre-v0.13 renderer
quality corrective checkpoint** with status **Exploring**. It is not an Active
release and does not authorize implementation until a short accepted design
selects the exact convergence contract, visual treatment, host context, public
seams, non-goals, and local verification gates.
