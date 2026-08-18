# Renderer Quality Corrective Design (pre-v0.13)

Status: repository implementation complete; permitted field execution remains
outstanding

This design is authoritative for the bounded pre-v0.13 renderer-quality
checkpoint. It responds to the
[2026-08-18 renderer quality investigation](../reviews/render-quality-investigation-2026-08-18.md)
without reopening the completed v0.10 through v0.12 feature scopes or
activating the v0.13 persistent-terrain candidate.

## Outcome

The existing progressive View must converge to a deterministic resident cut,
remain visually stable after settlement, and communicate enough depth,
Coverage, spatial context, and selection state for a professional to
distinguish Source features from display artifacts.

GPU presentation remains disposable. Point Identity, exact position,
selection, Edit, terrain, QA, and export authority remain on the existing CPU
paths.

## Ownership and interfaces

`point-view` remains the deep module that owns frustum culling, screen-error
policy, hysteresis, target-cut selection, transition budgeting, Coverage
retention, request priority, and conditional retirement. Its existing public
interface remains `ViewPlanner::plan`; convergence does not add a scheduler,
loader, renderer, or host callback to that interface.

The private `renderer-demo` Scene bridge owns Missing, Requested, and Resident
status, request admission, materialization, renderer acknowledgement, and
cumulative lifecycle facts. Its synchronous implementation materializes at
most one batch per pump and therefore admits at most one new planner request
per pump. Retained demanded work remains queued; camera-stale work is
cancelled before replacement work is admitted.

`render-protocol` and `render-wgpu` retain their existing atomic batch,
generation, version, resource-limit, depth, pick, and highlight contracts for
the convergence slice. No public renderer change is justified by stationary
convergence.

## A — convergence and resource correctness

Planner resource accounting describes the post-reconciliation plan: retained
resident batches, still-demanded Requested batches, and newly emitted
requests. Requested work absent from `ViewPlan::demanded_nodes()` is owned by
the host until reconciliation, but it cannot consume the selected cut's
planning budget because the same plan directs the host to cancel it.

For one unchanged generation, camera, viewport, hierarchy, and budget, the
planner replays still-valid hysteresis history before considering new
refinements. Replayed refinements do not treat already-retired intermediate
ancestors as new transition targets. New refinements still pass the complete
transition-footprint check, including fallback Coverage, before they are
accepted.

The deterministic generated acceptance case is:

- the existing 16,777,216-Point synthetic hierarchy and default camera;
- a 2560 by 1664 physical-pixel viewport, matching the investigated Retina
  window;
- the existing 600,000-Point, 14,400,000-byte, and 640-batch limits;
- one admitted and materialized batch per presented-frame pump; and
- settlement within 1,024 pumps.

The first quiet frame has no demand, new request, issued request, upload, or
retirement and has an empty host queue. The following 300 stationary pumps
must produce zero requests, uploads, cancellations, retirements, or node-state
changes. Camera movement, projection switching, reset, resize, pause/resume,
refinement, and coarsening must each converge again under the same invariants.

The generated acceptance proves repository behavior only. It is not a
workstation promise or field qualification.

## B — LOD and depth legibility

After convergence is closed, the selected density-transition policy is one
bounded eight-presented-frame parent/child cross-fade. It begins only after all
replacement Coverage is resident, retains at most the transition footprint
already admitted by the planner, and ends in the same deterministic child-only
cut. Opacity affects color presentation only: depth ordering, pick coverage,
Point Identity, Coverage truth, and exact selection do not depend on it.

If implementation requires per-batch presentation weight, the only permitted
public addition is one generation- and expected-version-conditional batch
presentation update in `render-protocol`, implemented by `render-wgpu`. It
must have a real `renderer-demo` caller and direct state-model/GPU tests before
acceptance; no general material, shader, or plugin interface is authorized.

Point diameter remains in physical pixels. Fixed views will compare the
existing 2.4-pixel reference with a bounded projected-spacing-aware policy
clamped to 1.0 through 4.0 physical pixels. The adaptive policy is accepted
only if it reduces settled density discontinuities without hiding holes,
changing pick coverage, or exceeding the recorded frame/resource ceilings.

The selected optional depth cue is eye-dome lighting. It is a deterministic
display-only post-process with an explicit disabled path and automatic fallback
to the current unenhanced render when required texture or pipeline capabilities
are unavailable. Owned transient color/depth resources are capped at eight
bytes per physical pixel plus fixed pipeline and binding objects. Strength and
sample radius are bounded configuration, never inferred Source values.

## C — inspection context and state

The private host will replace the diagnostic title dump with a small primary
on-canvas status model and a separate detailed diagnostic transcript. The
primary model contains only:

- display mode and projection;
- loading, settling, steady, or loads-paused state;
- Sampled, Complete, or authored Coverage;
- logical, drawn, and resident Point counts with unambiguous labels;
- exact selection count and resident-highlight status;
- orientation, scale, cursor world coordinate, and the active palette legend;
  and
- one clear recovery or clear-selection action when applicable.

The status model is a private pure module tested independently from glyph
rendering. Any glyph atlas remains host-owned and bounded. Required state must
remain readable at the minimum supported window and 200% interface scaling
without using color alone. Detailed planner, queue, staging, resource, and
timing facts remain available outside the compact primary layer.

The displayed version comes from workspace package metadata through
`CARGO_PKG_VERSION`; hard-coded historical feature-version titles are removed.

## D — permitted real-cloud qualification

After A and the P1 portion of B pass, permitted LAS/LAZ Sources exercise all
five display modes and both projections. Records include time to first visible
Coverage, settlement frame and time, the 300-frame quiet window, resident and
peak resources, cumulative lifecycle work, known-feature outcomes, and
failures. Source material, paths, reports, screenshots, identities, and
benchmark claims remain private unless their owner grants the required
permission.

The implemented opt-in manifest validates the complete five-display by
two-projection matrix, five projects from three firms, explicit inspect/measure
permission, a bounded settlement ceiling, and outcomes for the fixed known-
feature categories. The local runner exercises the same adaptive Point size,
eight-frame density transition, and EDL/fallback path as the interactive View.
A generated LAS process acceptance proves this repository lane and records
`not_observed` for field-only feature outcomes; it is not field evidence.

Repository acceptance, field qualification, partner validation, downstream
acceptance, adoption, and support qualification remain distinct claims.

## Verification gates

- Public `ViewPlanner::plan` tests cover stale-request exclusion, replay past
  retired ancestors, Coverage retention, budget limits, movement, projection,
  generation reset, refinement, and coarsening.
- The private Scene acceptance runs the exact default synthetic frame loop to
  settlement and through the 300-frame quiet window.
- Existing local GPU tests continue to cover circular splats, depth, picking,
  highlights, projection, large-world precision, display mappings, and atomic
  updates with `PUNCTRA_REQUIRE_GPU=1`.
- Later cross-fade and depth-cue slices add tolerant fixed-view image tests,
  capability fallback tests, and explicit transient resource/frame ceilings.
- Before/after evidence records the same generated fixture, physical viewport,
  camera, budgets, adapter/backend facts, settlement frame, lifecycle totals,
  and observation window.

All formatting, linting, tests, rustdoc, package checks, benchmarks, and GPU
acceptance run locally as documented in `CONTRIBUTING.md`. This design does not
authorize hosted CI.

## Non-goals

This checkpoint does not add photorealism, meshes, texture streaming, globe or
3D-Tiles support, rendering every Point simultaneously, GPU-authoritative
geometry, Source rewriting, general CAD/BIM authoring, a shader/plugin system,
silent tone mapping, arbitrary clipping/measurement/annotation tools,
automatic coordinate-reference guessing, or a general desktop product UI.
