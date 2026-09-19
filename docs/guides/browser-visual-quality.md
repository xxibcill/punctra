# Browser visual-quality baseline

Status: **v0.21 is Complete and repository-verified for the bounded corpus and
one exact attended Chromium/macOS/Apple-GPU lane; broader browser/device,
physical-display, independent-human/adopter, and improved/final-quality claims
remain outstanding**

This guide is the immutable predecessor workflow. The active v0.22
[point-footprint qualification](browser-point-footprint.md) reuses these inputs
and canonical images but publishes separate baseline, evidence, artifacts, and
release observations; it never rewrites the v0.21 records below.

Punctra `0.21.0-alpha.1` establishes a reproducible visual-regression baseline
before any intentional point-appearance change. The accepted
[design](../design/visual-quality-baseline-v0.21.md) authorizes a closed private
Visual Corpus, private capture/readback, tolerant and temporal comparison,
feature and resource reporting, and a non-gating interpretation rubric. It adds
no public screenshot, renderer-target, arbitrary-scene, or comparison interface
to `@punctra/viewer`, `@punctra/react`, or a Rust library.

The 2026-08-28 repository-activation decision is deliberately explicit. v0.20
is complete, but its functional generated scene and sampled LAS root did not
cover the representative sparse, dense, layered, high-dynamic-range,
classification, large-world, and mixed-LOD conditions in the original v0.21
gate. v0.21 creates and freezes the missing bounded corpus; it does not rewrite
v0.20 history or pretend that gate had already passed.

## Closed corpus

The manifest at
`apps/browser-demo/web/fixtures/visual-v1/corpus.json` fixes nine trials:

| Trial | Input | Projection | Mode | Purpose |
|---|---|---|---|---|
| `generated-neutral-mixed-lod-perspective` | Deterministic generated scene | Perspective | Neutral | Sparse/dense regions, large-world position, and a bounded mixed-LOD parent/child transition. |
| `generated-elevation-layered-orthographic` | Deterministic generated scene | Orthographic | Elevation | Overlapping depth layers at a large world origin. |
| `generated-rgb-hdr-perspective` | Deterministic generated scene | Perspective | RGB | Dense dark/bright raw color variation. |
| `generated-intensity-sparse-orthographic` | Deterministic generated scene | Orthographic | Intensity | Thin sparse structure and fixed intensity extremes. |
| `generated-classification-selection-perspective` | Deterministic generated scene | Perspective | Classification | Raw classification mapping plus two presentation-only highlights. |
| `autzen-rgb-perspective` | Derived licensed sample | Perspective | RGB | Sampled real spatial structure with raw display attributes. |
| `autzen-classification-perspective` | Derived licensed sample | Perspective | Classification | Sampled real spatial structure with raw classification values. |
| `autzen-intensity-perspective` | Derived licensed sample | Perspective | Intensity | Sampled real spatial structure with its sampled intensity range. |
| `autzen-elevation-perspective` | Derived licensed sample | Perspective | Elevation | Sampled real spatial structure with layered elevation. |

Together, the generated trials cover every required condition and all five
inherited display modes; the matrix includes both inherited projections. The
generated scene contains 2,103 authored Points in five batches. Its positions,
attributes, Point identities, Source identity, camera facts, presentation
roles, and transfer-v2 payload identity are deterministic. Its Coverage label
is `authored`, not complete Source truth.

The generated mixed-LOD trace uses nine child weights from 0 through 255 and
removes the parent only after the transition. Its settled temporal comparison
must be exact. This is a presentation transition fixture, not proof of complete
hierarchy traversal or production LOD quality.

## Autzen derivation and permission

The sole real-world input is the checked-in
`examples/data/autzen-classified.laz`. The v0.21 generator verifies its exact
74,416,814 bytes, SHA-256, Source identity, 10,653,336 Points, LAS 1.4 point-
format-7 facts, and coordinate bounds before selecting data. The fixed recipe
chooses 64 evenly distributed ordinal blocks of 64 Points each, yielding 4,096
Source-ordered transfer-v2 records with position, intensity, classification,
and RGB values.

The committed derivative is
`apps/browser-demo/web/fixtures/visual-v1/autzen-classified-sample.pvis`; its
manifest is `autzen-classified-sample.json`. The manifest records the exact
upstream and derived identities, recipe, attribute mapping, world origin,
camera, and permission facts. Attribution is “Autzen Stadium point cloud,
PDAL/data contributors,” sourced from the PDAL/data revision named by the
manifest under CC BY 4.0. It explicitly permits publication of the derivative
and its rendered evidence.

This derivative is a modified bounded sample. It is not the complete Autzen
survey, independent partner data, a professional interpretation, or permission
for unrelated Sources. Regeneration verifies exact committed bytes by default:

```bash
cargo run -p browser-demo --bin generate_visual_source_fixture
```

Do not use `--write` during qualification. Writing replacement bytes is a
reviewed baseline change, not evidence regeneration.

## Canonical capture contract

Every canonical trial fixes:

- a 320 by 240 CSS-pixel canvas;
- requested device-pixel ratio 2;
- an exact 640 by 480 physical capture, or 307,200 pixels;
- its camera, projection, display mode, background, selection/highlight, Source
  or scene identity, Coverage label, batches, and View generation;
- 30 unchanged foreground frames after all required publication, replacement,
  retirement, highlight, and scheduled rendering work has ended; and
- three complete viewer/harness recreations.

The runner rejects a different observed DPR, bitmap size, zoom/visual-viewport
scale where exposed, moving generation, incomplete batch facts, pending work,
or failure to settle within 240 capture-poll frames. A noncanonical diagnostic
capture cannot replace an accepted trial.

The private capture module renders the same accepted frame to a copyable GPU
texture, uses the submitted-work-done and readback-map callbacks as bounded
synchronization preconditions, copies through a row-aligned staging buffer,
removes padding, and explicitly normalizes BGRA or RGBA input to top-left
RGBA8. The current corpus labels the canonical byte encoding
`linear`, while recording the configured surface color space (`srgb`) as a
separate capability fact; it applies no unrecorded ICC or display transform.
Accepted artifacts use lossless PNG with a fixed RGBA8/filter-0 encoding. The
record binds both encoded bytes and decoded canonical pixels.

Callback intervals start at the begin-capture monotonic origin and end when the
submitted-work-done or readback-map callback runs. They include callback and
browser-scheduling delay, do not establish callback ordering, and are not
physical GPU-completion time. Their independent ceilings bound the private
capture workflow only.

This is `offscreen_not_presented` renderer evidence. The attended canvas is
visible, but the readback does not observe the operating-system compositor,
ICC/display color management, panel presentation, or physical GPU memory.

## Comparison and tolerances

The canonical lane compares decoded physical pixels. Its fixed profile is:

| Gate | Inclusive limit |
|---|---:|
| Per-channel unstable-pixel threshold | 2 |
| Maximum channel delta | 4 |
| Mean channel delta | 0.25 |
| RMS channel delta | 0.75 |
| p95 channel delta | 2 |
| Unstable-pixel fraction | 0.001 |
| Coverage-fraction delta | 0.001 |
| Feature-occupancy-fraction delta | 0.005 |
| Feature-centroid distance | 1 physical pixel |

The settled generated temporal profile requires zero for every image,
Coverage, occupancy, and centroid difference. A trial reports exact-equal and
unstable pixels, maximum/mean/RMS/p95 channel deltas, the worst temporal pair,
and every predeclared Feature Region independently. No aggregate score can
override a maximum-delta, temporal, Coverage, feature, authority, or resource
failure. Tolerances may be tightened from three raw repetitions but cannot be
widened after a failure without a reviewed baseline revision.

These limits detect regression; they do not rate beauty, readability,
professional fitness, or final visual quality.

## Authority, Coverage, and resources

Each result records the input identity, generation, authored or sampled
Coverage, displayed/drawn/resident Point and batch facts, display mode,
projection, selection/highlight state, and the exact frame used for readback.
Canonical images and Feature Regions have `presentation_only` authority.
Picking remains `provisional_gpu_hint`; only the immutable-record bridge returns
`exact_source_record`. Images cannot change Point Identity, exact position, raw
classification, selection membership, Coverage truth, or Query completion.

Capture resources are independently bounded:

- canonical pixels: 1,228,800 bytes;
- capture texture: 1,228,800 bytes;
- staging buffer and row-aligned readback accounting: 1,228,800 bytes each;
- PNG scanlines: 1,229,280 bytes;
- encoder working memory: 2,524,096 bytes;
- encoded PNG: at most 1,310,720 bytes;
- comparison workspace: 65,536 bytes;
- retained canonical images: at most two at once.

The complete evidence transport is bounded separately:

- encoded PNG artifacts across the run: at most 1,207,959,552 bytes;
- evidence JSON: at most 33,554,432 bytes;
- baseline-input manifest: at most 1,048,576 bytes;
- uncompressed USTAR entries: at most 896;
- archive structure: at most 1,048,576 bytes;
- evidence/manifest/structure overhead: at most 35,651,584 bytes; and
- complete private transport archive: at most 1,243,611,136 bytes.

These values are transport allocation ceilings, not expected artifact sizes.
The TAR is only a way to move repository-relative files from the attended
browser; it is not itself an evidence artifact and is not checked in.

The standard path creates one browser Blob download. Some attended in-app
browsers report a download without materializing it. For that case only, the
strict local server has an explicit opt-in same-origin export endpoint for the
same bounded TAR. The endpoint is disabled in the normal server configuration,
accepts no cross-origin POST, chooses no caller-supplied filename or repository
path, and uses no-replace publication under the operator-selected local export
directory. An existing target is a conflict rather than an overwrite. The
endpoint receipt and exported TAR are transport diagnostics, not evidence.

Renderer resident vertices, canvas/surface bytes, renderer transient textures,
streamed records, Worker staging, concurrent responses, verified cache,
capture/readback, canonical pixels, encoding, comparison, and timing facts stay
separate. Capture and comparison intervals are not presented as representative
viewer frame cost. Process RSS, physical cache allocation, physical GPU/driver
allocation, energy, and thermal measurements remain unavailable unless an
actual later observation records them.

## Interpretation rubric

The template at
`docs/releases/v0.21-browser-visual-rubric-template.json` binds six prompts:
depth, shape, density transition, color meaning, selection, and false feature.
Each answer is `clear`, `ambiguous`, `false_feature`, `not_visible`, or
`not_observed`, with an optional note of at most 280 characters.

One attended maintainer-labelled verify session is required for repository
evidence, but the rubric is not a pass gate. Rubric controls remain disabled
until all captures finish and the exact prompt-bound images load in the visible
document. The maintainer then confirms the session label, records every outcome,
and submits the post-capture review. Unfavorable answers are preserved rather
than edited away. A maintainer observation is not independent-human,
professional-usability, or adopter evidence; a prompt not evaluated may remain
`not_observed` even though its bound image was shown. The required record-stage
review is calibration-only and is discarded except for the canonical baseline
PNGs and baseline-input manifest.

## Local verification

Run the static fixture, codec, policy, and tamper checks locally:

```bash
cargo run -p browser-demo --bin generate_visual_source_fixture
node --test apps/browser-demo/web/visual-*.test.mjs \
  apps/browser-demo/web/range-server.test.mjs \
  scripts/verify-browser-visual-baseline.test.mjs
node scripts/verify-browser-visual-baseline.mjs
```

The static verifier binds
`docs/releases/v0.21-browser-visual-baseline.json`, the Visual Corpus,
derivative manifest/payload, runtime implementation, tolerance/resource/rubric
policy, and inherited v0.20 appearance boundary. A recorded `passed` field
cannot override a derived failure.

Final acceptance uses a mandatory sequential record-then-verify workflow on the
strict local server and exact Codex in-app Chromium/macOS/Apple-integrated-GPU
lane inherited from v0.20. Record and verify are not interchangeable choices.
Each stage runs all nine trials through the private identifier-only runner with
three complete recreations and explicit cleanup. Build and serve the working
implementation for the record stage with:

```bash
scripts/build-browser-sdk.sh
python3 scripts/serve-browser-demo.py --port 8000
```

Keep the browser page visible at DPR 2 and 100% zoom. First open
`http://127.0.0.1:8000/visual.html?mode=record` and click **Run
three-recreation corpus**. Wait until capture finishes and the exact bound
images load under **Post-capture interpretation review**. Confirm the bounded
session label, record all six outcomes, click **Submit post-capture review**,
and wait for `document.body.dataset.visualBaseline === "passed"`. Download the
single `v0.21-browser-visual-evidence.tar` repository bundle.

Use the standard Blob download first. If that download does not materialize,
create a fresh empty directory and restart the strict server with the local-
export opt-in:

```bash
mkdir -p target/v0.21-visual-record-export
python3 scripts/serve-browser-demo.py --port 8000 \
  --visual-export-dir target/v0.21-visual-record-export
```

Repeat only the affected stage at
`http://127.0.0.1:8000/visual.html?mode=record&transport=server`. Use
the fully pinned verify URL with `&transport=server` and a different fresh
directory for the later verify fallback.
The page POSTs the same already-bounded `application/x-tar` body to
`/qualification-visual-export`; it does not upload individual PNGs or write
repository files. The single `Host` must be a loopback authority using the
server's bound port, the request `Origin` must exactly equal `http://` plus that
`Host`, and its positive decimal `Content-Length` may not exceed 1,243,611,136
bytes. The server grants no cross-origin POST. It streams 64 KiB chunks to an
exclusive temporary `.part`, computes SHA-256, verifies the exact written
length, fsyncs, and publishes only `v0.21-browser-visual-evidence.tar` without
replacement.

Success is HTTP 201 with a bounded JSON receipt whose schema is
`punctra-browser-visual-export-receipt-v1` and whose fields are `filename`,
absolute local `path`, `byte_length`, and `sha256`. The receipt and archive are
private transport, not evidence. Preserve the conflict if the export directory
already contains the fixed archive name.

Inspect and extract the bundle into a fresh directory:

```bash
tar -tf /path/to/v0.21-browser-visual-evidence.tar
mkdir -p target/v0.21-visual-record
tar -xf /path/to/v0.21-browser-visual-evidence.tar \
  -C target/v0.21-visual-record
```

Retain only the nine canonical files under
`apps/browser-demo/web/fixtures/visual-v1/baselines/` and the commit-free
`apps/browser-demo/web/fixtures/visual-v1/baseline-inputs.json`. Discard the
record-mode evidence, rubric, recreation, transition, and difference artifacts
as final evidence. Check in the retained inputs, freeze every qualified path,
create the implementation pin, and refresh the dependent static digests.

Rebuild the exact pinned `0.21.0-alpha.1` implementation. Before accepting
visual evidence, repeat the inherited packed quickstart and browser
qualification. Record `git rev-parse HEAD`, the byte length of
`scripts/verify-browser-visual-baseline.mjs`, and that file's SHA-256. Substitute
them into this one-line URL:

```text
http://127.0.0.1:8000/visual.html?mode=verify&implementation_commit=<40hex>&verifier_byte_length=<decimal>&verifier_sha256=<64hex>
```

The page fixes the attended lane to
`codex-iab-chromium-151-macos-26-apple-m5-pro`, `browser_trusted_activation`, and
`exact_observed_lane_only`. It disables the visible Run button until every pin
is present and matches both the checked-in visual baseline and the running
checkout/verifier identity exposed by the strict local server. Click that
button, run the same corpus against the
checked-in baselines, wait for the post-capture images, record and submit the
final maintainer-labelled rubric, and wait for the page to report `passed`.
The Run click, every rubric selection, and rubric submission each require active
browser transient user activation; a trusted event without active transient
activation is rejected.
Download the one repository TAR bundle, inspect and extract it into a fresh
directory, and place its evidence JSON and PNG artifacts at their recorded
repository-relative paths. Only this verify-mode evidence is eligible for
final static acceptance. The separate JSON and per-artifact download links are
diagnostic conveniences, not the documented transport workflow.

The automation seam is private: `window.__PUNCTRA_BROWSER_VISUAL__` exposes
`run({ mode, provenance })`, `state()`, `draft()`, `report()`, `artifacts()`,
`baselineInputs()`, `submitReview(answers?)`, `downloadEvidence()`, and
`downloadBundle()` for the repository harness. `submitReview` cannot bypass
post-capture image loading or visible presentation. This seam is not a package
export. `transport=server` changes only where `downloadBundle()` moves the one
archive; it does not supply or relax verify provenance, and standard Blob
transport remains the default. Final attended verification uses the pinned URL
and visible Run button above. The private API inventory is descriptive and is
not a console workaround or an alternative final attended path. After placing
the extracted verify artifacts at their recorded paths,
verify them with:

```bash
node scripts/verify-browser-visual-baseline.mjs \
  --evidence docs/releases/v0.21-browser-visual-evidence.json
```

The completed record stage produced the checked-in baseline inputs. The later
verify stage at implementation commit
`f5d04d2c6091deda1136c2304cf8f97b9b40a755` passed all nine trials through
three complete recreations and retained 873 PNG artifacts. All six rubric
outcomes are explicitly `not_observed` under
`codex-local-maintainer-not-human`; they are not favorable or independent-human
observations. The [v0.21 repository verification
record](../releases/v0.21.0.md) pins the exact lane, verifier identities,
evidence record, functional measurements, artifacts, and remaining nonclaims.

Follow [CONTRIBUTING.md](../../CONTRIBUTING.md) for the complete local Rust,
Wasm, JavaScript, package, GPU, browser, benchmark, documentation, and JSON
sequence. No hosted CI is added.

## Claims this baseline does not make

Even after the exact local evidence passes, v0.21 does not establish another
browser, operating system, adapter, backend, device, display, DPR, compositor,
or color-management path; independent human interpretation or adoption;
registry/CDN publication; stable pre-v1 interfaces; improved or final visual
quality; professional fitness; support qualification; beta; v1; or release-
candidate status. It does not authorize intentional footprint, anti-aliasing,
LOD, depth, palette, tone, background, highlight, or other appearance changes.
Those require later accepted releases and comparison against this exact
baseline.
