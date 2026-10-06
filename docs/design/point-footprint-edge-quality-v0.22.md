# Point Footprint and Edge Quality Design (v0.22)

Status: **Accepted; bounded repository implementation and verification Complete**

This design is authoritative for the narrow v0.22 repository slice. The
maintainer's 2026-08-29 request to continue after the v0.21 merge activates the
bounded work below. It does not turn the roadmap's broader browser, device,
physical-display, independent-adoption, support, beta, release-candidate, or v1
gates into repository facts.

## Outcome

Punctra's browser View renders circular Points with deterministic four-sample
edge coverage when the required target capabilities are present. The browser
host chooses one bounded projected-density display diameter. An explicit
single-sample fallback preserves the same Point centers, geometry, colors,
depth ordering, presentation weights, and Point identities.

Both paths remain disposable presentation. The existing nominal pick diameter
and hard circular pick test remain independent of the decorative color edge.
Exact Source records, Point Sets, Queries, edits, terrain, and export remain the
only authorities for their existing meanings.

## Inherited evidence and release boundary

The v0.21 Visual Corpus, cameras, display mappings, canonical PNGs, and final
evidence remain immutable predecessor evidence. v0.22 reuses every one of the
nine accepted inputs and canonical cameras. It publishes a separate v0.22
quality baseline and evidence record; it does not overwrite or relabel v0.21
images.

The canonical before/after comparison uses the v0.21 640 by 480 physical-pixel
images at requested DPR 2. Focused scale trials additionally exercise requested
DPR 1, 2, and 4 and the corpus's perspective and orthographic camera families.
These are offscreen GPU readbacks, not operating-system composition or
physical-display observations. General browser zoom, responsive composition,
fullscreen, accessibility zoom, and cross-device equivalence remain v0.28 or
later work.

## Renderer module and interface

`render-wgpu` owns one deep Point-footprint module behind the existing
`RendererConfig` construction seam:

- `PointFootprint::SingleSample` requests the inherited one-sample hard circle;
- `PointFootprint::Antialiased` requests four-sample circular coverage;
- `RendererConfig::with_point_footprint` carries that request; and
- `WgpuRenderer::point_footprint_status` reports `SingleSample`,
  `Multisample4x`, `UnsupportedFallback`, or `ResourceFallback` for a validated
  viewport.

No caller selects a sample count, creates a resolve texture, supplies a shader,
or computes transient bytes. Those details stay inside `render-wgpu`.

The anti-aliased path is active only when the configured color format is a
blendable render attachment with guaranteed four-sample and multisample-resolve
support and `Depth32Float` has guaranteed four-sample render-attachment support.
The preferred path is additionally limited to 1,310,720 physical pixels so its
complete EDL-plus-pick target set cannot exceed the inherited 64 MiB renderer
transient ceiling. A missing format capability becomes `UnsupportedFallback`;
a larger viewport becomes `ResourceFallback`. Both use the complete inherited
single-sample path. Construction still rejects a color format that cannot
satisfy the renderer's existing blendable render-attachment contract.

The color pipeline expands the same six-vertex quad and evaluates the same unit
circle at four physical samples. The interpolated corner coordinate uses WGSL
per-sample interpolation, so the circle edge is coverage-resolved rather than
softened with a second alpha policy. Source alpha and batch presentation weight
retain their existing meanings. Picking and the eye-dome visibility-depth pass
remain single-sample and use the nominal hard circle.

The renderer owns four-sample color and depth attachments and resolves color
into the caller's single-sample target. When eye-dome lighting is active, the
resolved color and separate single-sample visibility depth retain their existing
sampling interface. Pick color and pick depth remain separate single-sample
targets. The renderer retains compatible single- and four-sample pipelines but
creates multisample textures only for an active preferred frame. Resizing
replaces the complete target set atomically.

## Browser display-size policy

The private browser host requests `PointFootprint::Antialiased`. The native
renderer demo requests the same mode through its existing appearance
configuration. Third-party callers retain an explicit choice; the renderer does
not silently infer host policy.

For one frame, the browser computes one display diameter from the physical
viewport and the complete non-retired resident Point count:

```text
projected spacing = sqrt(physical viewport pixels / max(resident Points, 1))
display diameter  = clamp(projected spacing * 0.55, 2.0, 6.0) physical pixels
```

The count includes resident replacement Points regardless of presentation
weight so a color cross-fade cannot pulse Point size. A retirement, publication,
resize, or new generation may change the next frame's size. Color mode,
classification, highlight state, and source alpha cannot. Zero resident Points
is treated as one Point before the formula is clamped; it does not create a
draw and reaches the 6.0-pixel upper bound only when the viewport area makes
the computed diameter at least that large.

The inherited browser nominal pick diameter remains exactly 7.0 physical
pixels. Decorative display sizing therefore cannot enlarge or shrink the
provisional pick footprint. This intentional difference is reported in frame
diagnostics and the v0.22 evidence.

## Exact resource bounds

The preferred non-EDL color path owns at most 32 transient bytes per physical
pixel: four RGBA8 color samples and four `Depth32Float` samples. A retained
single-sample pick target and pick depth add at most 8 bytes per physical pixel,
for a 40-byte-per-pixel high-water mark.

The EDL path may additionally retain one four-byte resolved color texel and one
four-byte single-sample visibility-depth texel per pixel. With a pick target its
high-water mark is therefore 48 bytes per physical pixel. The 1,310,720-pixel
preferred-path ceiling caps that complete set at 62,914,560 bytes. Above that
area the resource fallback uses the unenhanced inherited hard-circle path,
even when eye-dome lighting was enabled at construction, and owns only the
inherited single-sample depth and pick
targets, at most 8 bytes per pixel and 67,108,864 bytes at the inherited maximum
canvas. Requested single-sample and capability-fallback frames likewise suppress
eye-dome lighting whenever their complete 12-byte-per-pixel EDL-plus-pick set
would exceed the same ceiling; their footprint status remains unchanged and the
frame report exposes the suppression. Fixed pipeline,
binding, and texture objects remain separately described but are not presented
as observed driver allocation bytes.

`WgpuRenderer::depth_cue_status` remains the construction-time capability
disposition. `FrameReport::eye_dome_lighting_applied` is the per-frame truth and
is false for `ResourceFallback` frames so the bounded resource disposition is
observable rather than silent.

The exact renderer transient ceiling therefore remains 67,108,864 bytes. The
v0.22 canonical 640 by 480 capture may retain at most 14,745,600 renderer
transient bytes. Resident Point buffers,
canvas surface bytes, capture texture, readback staging, canonical image,
encoded PNG, worker staging, queues, cache, and evidence JSON retain independent
limits; one aggregate total cannot hide an overrun.

The shader adds no storage buffer, sampled texture, sampler, workgroup memory,
or per-Point vertex field. The preferred path adds per-sample fragment work and
two multisample attachments only.

## Measured quality gates

### Corrective measurement inputs (2026-10-06)

The maintainer's instruction to complete each roadmap version activates the
corrective work identified by the October 2 closeout investigation. The three
focused DPR trials now use `isolated_authored_subset_v1`: a separate settled
render subset containing only their bound authored ordinals. The full unchanged
Source is streamed in at most eight ordered batches of at most 1,024 Points;
neighbor batches retire after stream completion and before measurement. Exact Point bytes,
Source identity, world origin, camera, and display mappings remain unchanged.
The inherited nine mixed scenes remain the canonical density, feature, and
topology inputs. Focused captures, including DPR 2, have their own image pins.

Before any focused GPU execution, CPU projection must prove that every complete
measurement rectangle excludes all neighboring display disks with the accepted
0.75-pixel radius margin, fits inside the capture, and has an unobstructed
seven-pixel nominal-pick target. The offline verifier independently regenerates
the sampled payload, projections, rectangles, and separation facts. It rejects
unbound image centers or fabricated isolation observations.

A private raw-Wasm control constructor provides two diagnostic single-sample
captures: the inherited seven-pixel display diameter and the candidate's exact
projected-density diameter. Both reuse the unchanged Source, camera, display,
settled batch state, and highlights. This constructor is outside the public
viewer SDK exports. Their PNGs and exact projection inputs are bound into the
final evidence and independently checked offline.

### Corrective center contract (2026-10-06)

The local in-app browser has upgraded from Chromium 151 to 154. v0.22's
functional continuation explicitly qualifies only the observed Chromium
154/macOS 26.6.2/Apple M5 Pro lane. Its release-specific lane module leaves the
v0.21 Chromium 151 lane and evidence unchanged. Both the packed functional
suite and Point-footprint records must be reproduced at the final pin; the
new browser version is not inferred to pass from the predecessor.

The original mixed-scene binary occupancy-centroid gate is superseded, not
reported as passing. A feature's occupancy centroid is not a Point position:
changing a decorative diameter reweights differently colored, overlapping,
and clipped disks. The attended controls reproduced all nine immutable
seven-pixel predecessor images exactly. In the five failing scenes, changing
only the single-sample diameter moved occupancy centroids by 2.25 to 4.57
pixels; the final candidate differed from the predecessor by 1.21 to 2.55
pixels. These observations invalidate that statistic as a geometry-displacement
test. They do not excuse missing named features or new topology bridges.

Canonical center acceptance now requires identical Source identity and payload,
world origin, authored camera, display mapping, highlight ordinals, viewport,
generation, settled batch identities, versions, counts, and presentation weights
for candidate and both controls. Each inherited-diameter control must match
the immutable predecessor's decoded PNG bytes exactly. The offline verifier
regenerates these inputs from the pinned corpus and Source bytes, compares the
decoded control pixels, and checks that both WGSL vertex entry points forward
the same `point_vertex_values` projection helper's clip position unchanged.

A paired native GPU fixture independently renders the same Frame and PointBatch
through the legacy seven-pixel path and preferred diameters two through six.
It covers both camera families, all eight subpixel phases, eight different
depths, and a billion-unit world origin. Coverage-weighted centroid error from
the declared projected center must be at most one physical pixel for each
path, and paired centroid distance must also be at most one pixel. Those maxima
are bound to the clean implementation pin in the local GPU result artifact.
The isolated browser DPR trials retain their existing one-pixel centroid bound.
Mixed-scene occupancy centroids and component registration remain diagnostics;
they make no claim about movement of individual Points or tiny raster islands.

The v0.22 verifier records both predecessor and candidate metrics rather than
requiring an intentionally changed image to match v0.21.

An isolated-footprint fixture covers diameters 2 through 6 and at least eight
subpixel center phases. A deterministic 16 by 16 CPU area sampler supplies the
presentation-only reference mask. Across the preferred-path samples:

- coverage root-mean-square error must be at least 20% lower than the inherited
  single-sample result and no greater than 0.18;
- no foreground sample may occur beyond 0.75 physical pixel outside the ideal
  radius;
- every ideal center remains foreground; and
- all four quad-corner regions remain clear, rejecting square footprints.

For every inherited canonical trial, the verifier records foreground fraction,
partial-edge pixels, connected foreground components, clear-background
components, two-by-two solid blocks, and the existing feature occupancy and
centroid facts. Acceptance requires:

- every inherited named feature remains present, and canonical projected
  centers satisfy the corrective center contract above;
- foreground fraction remains between 50% and 105% of the predecessor value,
  preventing both hidden sparse structure and new blob coverage;
- the generated dense regions reduce two-by-two solid-block excess when the
  predecessor exceeds 0.80 of possible two-by-two cells, or retain it within 2%
  when the predecessor is already at or below that accepted bound;
- no new foreground connected component bridges two predecessor components
  separated by at least two clear physical pixels; and
- the focused sparse trial retains every bound thin-feature center.

The DPR 1, 2, and 4 scale trials use the same Source positions and camera facts.
Each must report the selected policy diameter inside 2.0 through 6.0 physical
pixels, the same preferred/fallback status for one adapter, no square-corner
leakage, all bound feature centers present, and normalized footprint coverage
error no more than 0.18. Exact decoded images are compared only within the same
declared DPR, adapter, backend, and implementation pin.

Capability and oversized-viewport fixtures must respectively report
`UnsupportedFallback` and `ResourceFallback`, allocate no multisample target,
preserve the v0.21 hard-circle mask, and return the same Point identity for every
nominal pick probe as the preferred renderer. A requested single-sample path
reports `SingleSample`, not fallback.

## Cost and timing gates

The v0.22 attended browser record retains v0.21's 30 settled foreground frames
and three complete viewer recreations for the nine canonical trials. It records
CPU frame-encoding/submission intervals, not physical GPU completion time.

- representative frame-interval p95 remains at most 50 ms;
- representative frame-submission p95 remains at most 16.7 ms;
- candidate p95 for each metric is at most 2.0 times the same-trial v0.21 value
  when that predecessor value is nonzero;
- first Coverage remains at most 10,000 ms and settled View at most 15,000 ms;
  and
- every exact transient subsystem remains within the bounds above.

The two-times comparison is a regression ceiling, not a performance promise.
Callback elapsed time, browser scheduling delay, driver work, compositor work,
power, temperature, and physical GPU completion remain explicitly distinct or
unobserved.

## Verification and evidence order

Repository acceptance proceeds in this order:

1. unit tests fail for capability selection, size policy, target accounting,
   quality metrics, and evidence validation;
2. renderer GPU tests compare preferred and single-sample footprints, nominal
   pick identity, resize, EDL composition, and fallback selection;
3. the full local formatting, linting, test, rustdoc, package, fuzz-build,
   native example, benchmark, and forced-GPU matrix passes;
4. the packed browser SDK and inherited functional qualification are rebuilt;
5. an attended record run creates the v0.22 canonical images and predecessor
   comparison inputs without claiming final verification;
6. the implementation and verifier are pinned, rebuilt, and checked clean; and
7. a separate attended verify run supplies the final eligible evidence and the
   human-readable release record.

When a later qualification-only correction changes the implementation pin,
already committed canonical and focused PNGs may remain at that new clean pin
only if fresh attended record runs reproduce every image byte exactly. The
new baseline must use that pin's complete implementation, verifier, rebuilt
runtime, local GPU observations, and actual browser environment. Fresh timings
and functional observations are collected again; prior timings do not become
evidence for the new pin. A separate attended verify run still supplies final
acceptance. Any image change requires the ordinary image-commit and clean-pin
sequence above.

The v0.22 baseline, evidence, and release record bind the exact package,
implementation commit, verifier bytes, browser, operating system, adapter,
backend, viewport, DPR, fallback status, display-size policy, images, metrics,
timings, resources, and unavailable observations.

All commands run locally. Hosted CI is not authorized.

## Non-goals

v0.22 does not change View planning, LOD selection, parent/child transition
policy, Point positions, source colors, display mappings, tone/exposure,
eye-dome parameters, depth precision, camera controls, pick authority, exact
Query behavior, highlights, annotations, measurements, responsive layout,
browser-zoom policy, cache behavior, streaming protocol, package stability, or
registry delivery.

LOD continuity belongs to v0.23; depth and shape treatment to v0.24; color and
tone to v0.25; motion to v0.26; selection treatment to v0.27; composition and
general DPR/browser-zoom behavior to v0.28; broader qualification to v0.29 and
later. No v0.22 result implies final visual quality, independent human or
adopter approval, cross-browser/device support, support qualification, beta,
release-candidate status, or v1.
