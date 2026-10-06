# LOD Density and Transition Continuity Design (v0.23)

Status: **Accepted; bounded repository implementation active**

The maintainer's 2026-10-06 instruction to finish each version authorizes this
bounded continuation after the completed v0.22 PR. The
[local investigation](../reviews/lod-transition-investigation-v0.23.md)
measures midpoint background leakage, equal-center batch-order color changes,
and immediate coarsening. Those observations select the treatment below.

## Outcome and authority

Refinement and coarsening use complementary physical-pixel visibility over
exactly eight presented frames. They preserve opaque Source colors and normal
visible-depth ordering instead of multiplying a pair of alpha weights.
The host computes display density from each side's declared coverage fraction,
so publishing a hidden replacement or retiring a fully hidden outgoing batch
does not pulse diameter. Stop, reset, generation change, and interrupted cuts
settle without stale geometry or unlimited duplicate residency.

The treatment changes disposable rendering only. Source payloads, positions,
Point identities, exact Query completion, display mappings, nominal pick masks,
and the inherited color-only `PresentationWeight` contract remain unchanged.
The public browser SDK exports stay unchanged. Its strict root-range loader
does not acquire a general browser child-LOD streaming contract.

## Minimal renderer seam

`render-protocol` adds one validated `RasterTransition` value with full and
hidden constants, an incoming/outgoing side, a caller-selected 32-bit group
seed, and an integer progress step from zero through eight. Invalid steps fail
before mutation. `RenderUpdate::SetBatchRasterTransition` conditions the update
on exact View generation, batch key, and resident version. The corresponding
effect/report authorizes only that resident batch's presentation change.

`render-wgpu` owns the pixel selector. It folds integer physical-pixel x/y and
the seed through a fixed unsigned hash to an eight-bit rank. Incoming coverage
accepts ranks below `step * 32`; outgoing accepts the complement. Full bypasses
the selector, and hidden accepts no color or visibility-depth fragments.
Every sample of one pixel uses the same side; ordinary circular MSAA edge
coverage remains independent. Color and EDL visibility depth use the same
selector. The nominal pick fragment never uses it.

This uses the existing 32-byte batch uniform's padding for three integer facts;
it adds no attachment, readback, Point vertex field, or caller-managed texture.
Reset and upsert restore full raster coverage. Recorded frames snapshot the
transition facts, so later updates cannot alter an earlier deferred frame.
Read-only frame diagnostics expose the count of controlled batches. The caller
does not receive shader, sample-mask, material, or pipeline hooks.

The implementation uses fragment discard and concrete unsigned arithmetic.
WGSL specifies discarded output and modulo integer overflow; it also identifies
input sample masks as a portability hazard. The selector therefore does not
depend on an input sample mask. See the [WGSL specification](https://www.w3.org/TR/WGSL/).

## Actual native host and private browser caller

`DensityTransitions` groups exact outgoing and incoming resident cuts, including
many child batches coarsening to one ancestor. A batch belongs to at most one
active group. Hidden replacements wait for their complete selected cut; the
host hides them before their first presentation. Eight successful presentations
advance the group, then retire the outgoing cut and restore the incoming cut
to full. Pausing does not advance progress. An invalidated group cancels to
the planner's current retained cut rather than restoring both sides blindly.
New residency stays blocked during an active group, preserving the existing
bounded host admission rule. Generation reset clears all group state.

Native and private browser display density use integer eighth fractions for
raster-controlled batches, with one final rounded sum. Fully hidden batches
contribute zero. Uncontrolled batches retain the complete non-retired resident
count policy, including the legacy color-weight harness. This count is a
diameter policy input, not observed Point visibility or authoritative Coverage.

The private raw Wasm fixture host applies the same conditional transition value
and captures its actual facts. Its ABI validates finite integral numbers before
narrowing, and requires canonical decimal u64 strings for generation/version
identities so JavaScript coercion cannot alias stale or invalid controls.
Its browser traces are explicitly authored
parent/child fixture transitions. Separate native `ViewLifecycle` traces
exercise the actual planner, materializer, and host. Neither kind substitutes
for the other or for independent embeddings.

Private endpoint captures temporarily hold display diameter at the current
candidate's measured value within the inherited 2–6-pixel range, then restore
the ordinary density policy. This lets coverage comparisons bind both the exact
current camera and diameter while recording diameter changes separately.

## Accepted gates

Before closure, all of the following must pass:

- Legacy `PresentationWeight` GPU outputs and nominal picks remain unchanged.
  Missing/stale generations, keys, versions, and invalid progress fail without
  changing state, resources, or a recorded frame.
- Both camera families, both footprint paths, large world origin, depth layers,
  highlights, EDL, and capability/resource fallbacks have explicit GPU cases.
- The opaque coincident-endpoint fixture has no midpoint background leakage
  beyond one RGBA8 quantization byte, at every progress step. Equal-center key
  swaps with a fixed group seed do not change its candidate opaque result.
- For fixed stationary fixture views, no new clear pixel appears where both
  endpoint images have at least 25% foreground coverage. The final canonical
  nine-trial images reproduce the v0.22 decoded pixels; named feature, Source,
  camera, viewport, and nominal pick inputs remain bound.
- Eight-step stationary traces change at most 25% of common endpoint foreground
  pixels per step, rounding the allowed pixel count upward and allowing at most
  one byte of unchanged-pixel quantization.
  Whole-image normalized temporal RMSE is at most 0.40. Moving traces compare
  coverage against endpoint controls at each exact current camera rather than
  attributing deliberate camera motion to a shader defect.
- Native refinement, many-to-one coarsening, motion, stop, rapid interruption,
  pause/resume, projection, resize, reset, and generation replacement settle.
  The inherited stationary ceiling remains 1,024 frames followed by 300 quiet
  frames with no upload, retirement, cancellation, or presentation churn.
- Active groups last at most eight presented frames. Duplicate Points and
  their exact 24-byte vertex costs remain charged to the existing hard Point,
  byte, and batch limits. Cancelled groups leave no controlled or ghost batch.
  No extra renderer target is allocated; the existing 64-MiB transient ceiling
  and separate capture/host ceilings remain enforced.
- Attended browser DPR 1/2/4 refinement/coarsening/interruption trials and three
  fresh recreations bind lossless images, exact camera/Source/batch/transition
  facts, authority boundaries, fallback disposition, and recomputed metrics.
  Source-to-first Coverage stays within 10 seconds, settlement within 15 seconds,
  quiet-frame callback p95 within 50 ms, and submission p95 within 16.7 ms.
  Matching canonical p95 costs remain within twice their nonzero v0.22 values.
  Capture/readback time is reported separately from frame encoding/submission.
- A fresh packed SDK/React build, quickstart, functional matrix, and complete
  applicable local CONTRIBUTING verification pass at a clean implementation
  pin. A separate browser verify run reproduces the recorded transition corpus.

Historical verifiers must continue to validate frozen predecessor evidence as
historical records. Current qualification must bind the v0.23 runtime and
implementation; a historical pass cannot qualify the changed checkout.
All checks run locally with `PUNCTRA_REQUIRE_GPU=1` for expected GPU cases.

## Non-goals and external evidence

This slice adds no general material interface, OIT targets, interpolated Point
positions, synthetic geometry, planner selection policy, network contract,
arbitrary browser Source loader, public SDK LOD controls, camera damping, tone
mapping, new selection authority, or hosted CI. Temporary raster grain is an
explicit tradeoff to measure, not a claim of universally smoother motion.

Physical-display, broader browser/device, independent-human interpretation,
independent adoption, final visual quality, support qualification, beta,
release-candidate, and v1 evidence remain separate. v0.30 activation still
requires real support and independent production evidence.
