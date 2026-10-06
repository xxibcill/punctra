# LOD transition investigation (v0.23)

Status: **Generated local investigation complete; candidate treatment unverified**

The 2026-10-06 request to continue through v0.30 supplies the work direction.
The completed v0.22 branch is proposed in
[PR #25](https://github.com/xxibcill/punctra/pull/25). This investigation runs on
`codex/v0.23-lod-continuity` against its unchanged inherited renderer behavior.
The [machine-readable observations](lod-transition-investigation-v0.23.json)
bind the inherited commit, instrumented source digests, commands, and results.

## Measured behavior

Two opaque red Points occupy the same projected center at different depths,
over an opaque blue background. The world origin exceeds one billion units;
the physical viewport is 64 by 64 and the nominal diameter is 18 pixels.
The inherited eight-step complementary color weights produce this center:

| Step | Red | Green | Blue |
|---|---|---|---|
| 0 | 255 | 0 | 0 |
| 1 | 227 | 0 | 28 |
| 2 | 207 | 0 | 48 |
| 3 | 195 | 0 | 60 |
| 4 | 191 | 0 | 64 |
| 5 | 195 | 0 | 60 |
| 6 | 207 | 0 | 48 |
| 7 | 227 | 0 | 28 |
| 8 | 255 | 0 | 0 |

The result is identical in perspective and orthographic projection, with
selected `SingleSample` and `Multisample4x` paths. Midpoint background leakage
is 64/255, despite both endpoint images covering the center with opaque red.
The nominal pick remains Point ordinal 2301 at every step. This experiment
keeps both batches resident, so the zero-weight endpoint still deliberately
preserves that pick; actual retirement changes residency separately.

A second fixture holds geometry and colors fixed while swapping batch keys.
Each batch has one central Point and an offscreen balancing Point, giving the
batches equal bounds centers. The renderer's depth sort therefore reaches its
key tie-breaker. The midpoint changes from `[64, 128, 63]` to `[128, 64, 63]`;
the nearest nominal pick remains ordinal 2401. This is a bounded demonstration
of batch-level alpha ordering, not a claim that every batch order is wrong.

The actual planner's coarse orthographic cut retains parent batch 1 and
retires resident child batches 2 and 3. The inherited `DensityTransitions`
returns immediate retirements and no active transition. Its replacement
search handles descendants of a retiring parent, not the reverse relationship.

Both forced-GPU characterization tests and the planner/host CPU test pass.
Formatting and warning-denying clippy for both affected applications pass.
The initial color-order probe used distinct batch depths and correctly exposed
the existing back-to-front sort; the final equal-center fixture isolates the
tie case. It does not alter production rendering to force the observation.

## Owning seams and treatment decision

The color pipelines render opaque batches first. Translucent batches preserve
depth testing but disable depth writes and blend back to front by batch bounds
center, then key. The problem is alpha overlap and approximate batch ordering;
a claim that every zero-weight color fragment writes visual depth would be
incorrect. Picking has its own nominal-depth pass. The existing
`PresentationWeight` contract must remain intact.

An explicit complementary raster-coverage treatment can retain Source color
and ordinary depth writes while choosing one LOD side at each physical pixel.
Its masks must be deterministic, complementary, bounded, and shared by color
and EDL visibility depth. Picking must ignore the decorative mask. The native
host also needs symmetric coarsening groups and a presentation-aware density
count whose endpoint is unchanged by retirement.

This decision is accepted for implementation in the
[v0.23 design](../design/lod-density-transition-continuity-v0.23.md). Its
quality, temporal, resource, actual-host, and browser gates remain unverified.
No browser execution, physical-display observation, human interpretation,
independent adoption, or broader qualification is supplied by these fixtures.
