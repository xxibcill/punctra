# Browser integration known limitations

Punctra `0.22.0-alpha.1` implements the bounded Point-footprint continuation,
but its attended qualification is not yet a completed repository claim. The
v0.21 [machine-readable baseline](../releases/v0.21-browser-baseline.json),
[browser matrix](../releases/v0.21-browser-matrix.json), and [verification
record](../releases/v0.21.0.md) remain the latest completed predecessor
authority. The v0.20
[baseline](../releases/v0.20-browser-baseline.json) and
[matrix](../releases/v0.20-browser-matrix.json) remain the immutable completed
historical evidence.

## Unsupported

These conditions cannot create a usable viewer in v0.22:

- insecure context, unavailable WebGPU/adapter, unsupported surface format,
  presentation mode, alpha mode, renderer limit, or invalid physical viewport;
- Source hosting without the required bounded byte ranges, exact lengths,
  identity encoding, strong validators, range digests, and exposed response
  headers;
- arbitrary LAS/LAZ URLs, LAZ decompression in the browser, arbitrary hierarchy
  traversal, multiple active Sources, general exact Queries, WebGL, Canvas,
  software rendering, or a reduced-feature fallback; and
- continuing after partial publication, device loss, surface loss, or another
  fused renderer failure without explicitly disposing and recreating.

Unsupported initialization returns a structured failure. Hosts must explain it;
they must not silently substitute a different renderer, Source, or cache mode.

## Unqualified

Only the exact entry in `v0.21-browser-matrix.json` is currently repository-
qualified. Its completed v0.21 observation does not qualify v0.22 or a
different environment. The planned `v0.22-browser-matrix.json` is not authority
until a fresh attended run and static verification complete.
Installed Chrome, Safari, other Chromium builds, other screens, operating
systems, GPUs, adapters, mobile devices, bundlers, framework versions, CDNs,
authentication stacks, CSP variants, and production networks remain
unqualified until their own attended evidence is recorded.

Initialization success does not promote an unqualified platform to supported.

## Deferred

The following are intentionally outside the accepted v0.22 slice:

- complete Source Coverage, Point-appearance policy beyond the accepted bounded
  footprint, final display policy, cross-browser/compositor/display
  equivalence, and visual-quality sign-off;
- editing, terrain workflows, export, host UI, offline-first behavior, service
  workers, telemetry, application persistence, and automatic recovery;
- registry or CDN publication, independent-adopter completion, setup-time and
  adopter-friction evidence, API stability, support operations, beta,
  release-candidate, v1, and compatibility promises; and
- physical GPU completion, process RSS, physical cache allocation, driver/GPU
  allocation, energy, thermals, or general remote-network performance claims.

The v0.20 fixed generated scene and sampled LAS root still preserve functional
and appearance continuity; they did not satisfy v0.21's representative-corpus
gate. The 2026-08-28 activation decision therefore permits a separate closed
v0.21 corpus rather than relabeling those old scenes.

## Visual-baseline boundary

The repository now contains nine fixed trials: five deterministic generated
trials and four Autzen-derived display-mode trials over one 4,096-Point CC BY
4.0 sample. The derivative binds
upstream/derived identity, deterministic selection, attribution, modification,
and image-publication permission. This is a permitted bounded sample, not the
complete Autzen survey, an independent partner Source, or a general browser
LAS/LAZ loader.

Canonical trials require a 320 by 240 CSS-pixel canvas at requested DPR 2,
exactly 640 by 480 physical pixels, 30 unchanged foreground frames before
capture, and three complete viewer/harness recreations per trial. Capture
is a private offscreen GPU readback normalized to top-left RGBA8; it does not
observe OS composition, display color management, or the physical panel.

Decoded comparisons keep maximum channel delta, unstable-pixel fraction,
Coverage, Feature Region, exact settled-generated temporal, and resource gates
independent. The tolerance caps are channel threshold 2, unstable-pixel
fraction 0.001, maximum channel delta 4, and one physical pixel of feature
displacement. These are regression bounds, not perceptual-quality scores.

The rubric records depth, shape, density-transition, color-meaning, selection,
and false-feature observations but is deliberately non-gating. The completed
repository evidence contains the accepted images, three complete recreations
per trial, machine-readable evidence, implementation/verifier pins, and release
record. All six rubric outcomes are explicitly `not_observed` under a non-human
maintainer label; no favorable or independent-human interpretation is inferred.
See the [visual-quality guide](browser-visual-quality.md).

Record and verify are sequential. The attended record stage supplies only the
baseline PNGs and commit-free baseline-input manifest that cross the
implementation pin. After the pinned build repeats inherited qualification, a
separate attended verify stage supplies final evidence. Rubric review follows
capture, and one uncompressed TAR transports repository-relative artifacts
without becoming evidence itself. Standard Blob download remains primary. The
opt-in `transport=server` path is only a same-origin, no-overwrite fallback for
an in-app browser that does not materialize the Blob; its local-server receipt
and TAR are still not evidence.

## Point-footprint boundary

The preferred four-sample path is presentation-only and requires the declared
color/depth format capabilities and a physical viewport no larger than
1,310,720 pixels. An unsupported capability selects `unsupported_fallback`; a
larger valid viewport selects `resource_fallback`. Both preserve the inherited
single-sample hard circle. They are usable fallbacks, not evidence that another
adapter or display is qualified.

The browser display diameter varies with non-retired resident density between
2.0 and 6.0 physical pixels. Nominal provisional-pick coverage remains a
separate 7.0-physical-pixel hard circle. v0.22 does not change View planning,
LOD transitions, Point geometry, Source/Query authority, display mappings,
tone, camera controls, exact selection, or host-owned DPR/resize policy. See the
[point-footprint guide](browser-point-footprint.md).

## Host-owned

Applications remain responsible for canvas/layout, DPR and resize decisions,
visibility, camera/navigation policy, credentials, Source allowlists, CSP,
authentication/authorization, cache and telemetry consent, retry UI, issue-data
redaction, and teardown. Presentation picks and highlights remain provisional;
only a configured exact authority may confirm a Source record.

See the [recovery playbook](browser-qualification.md#host-recovery-playbook) for
the retry-in-place and recreation-required boundary.
