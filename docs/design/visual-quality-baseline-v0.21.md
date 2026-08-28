# Visual-Quality Baseline and Regression Corpus Design (v0.21)

Status: **Accepted for bounded repository implementation; cross-browser,
independent-human, independent-adopter, and final visual-quality evidence
remain outstanding**

This design is authoritative for the bounded Punctra v0.21 repository slice.
The maintainer's 2026-08-28 request to continue through v0.21 after v0.20 was
merged activates the repository implementation below.

The roadmap's original activation gate also expected v0.20's fixed scenes to
expose representative sparse, dense, layered, high-dynamic-range,
classification, large-world, and mixed-LOD viewing conditions. They do not.
The v0.20 design explicitly records that its 1,089-Point generated scene and
70,000-Point immutable LAS deployment with one 4,096-Point sampled root are
insufficient for that gate. The maintainer's repository-activation decision
does not retroactively turn those inputs into representative evidence. It
authorizes v0.21 to create, accept, measure, and freeze the missing bounded
corpus without changing Point appearance.

Repository completion may establish the baseline on the one exact attended
browser lane that is actually executed. It cannot manufacture another browser,
adapter, display, independent adopter, independent observer, or permission that
was not exercised and recorded.

## Outcome

Punctra v0.21 establishes reproducible visual evidence before any intentional
point-appearance change.

A maintainer can run one private browser visual-trial module over a closed set
of generated scenes and one derived, licensed Autzen sample. Each trial fixes
its immutable input, camera, projection, display mode, physical viewport,
device-pixel ratio, settling rule, capability state, and resource limits. The
module presents the same frame through the existing browser renderer, reads a
bounded copyable render target back as canonical RGBA8 pixels, publishes
machine-readable observations, and writes only the accepted evidence images.

The resulting baseline records current behavior; it does not approve that
behavior as attractive, optimal, professional, or final. Later visual releases
must cite the exact v0.21 trial and compare against its immutable inputs rather
than selecting a friendlier camera or silently replacing reference bytes.

The deletion test is specific: removing v0.21 would remove the representative
browser visual corpus, capture/readback path, tolerant and temporal comparison,
feature-location checks, interpretation rubric, and reproducible baseline
evidence. It would not change the public viewer interface, rendering policy,
Point geometry, picking, exact Queries, editing, terrain, or export.

## Evidence limits

Repository completion may prove:

- deterministic generated scenes and the derived Autzen fixture regenerate to
  the exact accepted bytes and immutable Source facts;
- the accepted trial matrix covers each required viewing condition and every
  inherited display mode, with both inherited projections represented;
- one exact local Chromium/macOS/Apple-GPU lane can reproduce comparable
  decoded-pixel, temporal, Coverage, feature-location, and logical-resource
  evidence within predeclared tolerances;
- the private capture target uses the same frame, renderer implementation,
  presentation policy, and surface-format class as the attended browser View;
- unstable pixels, any allowed tolerance, capability state, and unavailable
  physical observation remain explicit rather than being hidden by one pass
  flag; and
- a later visual-quality change has one exact baseline against which to report
  image, motion, feature, and resource differences.

It does not prove:

- compatibility or comparable pixels on another browser, browser build,
  operating system, WebGPU backend, adapter, device, display, color-management
  path, or device-pixel ratio that was not executed;
- physical display-panel presentation, GPU completion time, driver/GPU memory,
  process resident memory, energy, thermal behavior, or destructive
  memory-pressure and device-loss behavior;
- that a screenshot is exact geometry, complete Source Coverage, a Query
  result, a classification interpretation, or professional feature
  identification;
- that the derived Autzen fixture represents the complete upstream survey or
  that generated scenes are production data;
- independent human interpretation, independent adoption, registry/CDN
  publication, production hosting, support qualification, or stable pre-v1
  compatibility; or
- improved or final visual quality, beta status, release-candidate status, or a
  v1 promise.

## Terms

### Visual Corpus

The closed, versioned set of immutable generated and licensed derived inputs,
fixed trials, accepted evidence images, and comparison rules used by this
release. Adding, replacing, or deleting an input is a new baseline revision,
not routine evidence regeneration.

### Visual Trial

One immutable tuple of Source or authored scene, camera, projection, display
mode, viewport, device-pixel ratio, settling rule, capability state, selection
state, capture rule, tolerance profile, feature regions, and resource limits.
A trial identifier names that whole tuple; callers cannot override individual
fields at runtime.

### Settled Cut

The declared resident presentation state after every required batch is
published, required replacement or retirement work has ended, scheduled
coalescing is drained, Coverage and generation match the trial, and the private
runner observes its complete quiet-frame precondition. A loading animation or
perpetually changing resident set is not settled.

### Canonical Image

The physical-pixel RGBA8 result decoded from the accepted lossless evidence
artifact after explicit surface-channel normalization. It is renderer evidence
before operating-system composition and display color management. It is not a
screen photograph or authoritative Point geometry.

### Unstable Pixel

A physical pixel whose maximum absolute decoded RGBA channel difference exceeds
the trial's fixed channel threshold. Aggregate image similarity cannot excuse a
failed feature region, excess unstable-pixel count, or excess maximum channel
difference.

### Feature Region

A predeclared image-space region and bounded foreground or centroid expectation
used to detect a lost, displaced, or falsely filled visual feature. It is a
presentation regression check, not a semantic assertion about the Source.

### Interpretation Rubric

The fixed prompts and closed outcomes used to record a person's observations
about depth, shape, density transitions, color meaning, selection, and false
features. A checked-in rubric is repository evidence. A human result exists
only when an identified observation session actually occurred.

## Corpus inputs and permission

The v0.21 corpus has exactly two input families.

### Deterministic generated scenes

Repository code generates a small set of authored scenes with exact Source
identities, Point identities, positions, display attributes, batches, node
roles, world origins, and camera facts. Together they must contain:

- isolated and thin sparse features next to independently bounded dense
  regions;
- at least two depth-separated layers whose projected overlap exposes ordering
  and false-surface behavior;
- fixed low, mid, and high intensity and RGB values, including deliberately
  narrow dark and bright features rather than only a smooth gradient;
- more than one classification value with a fixed expected raw mapping;
- sub-metre relative separation at a large finite world origin; and
- a stable mixed-LOD cut with adjacent regions at different declared sample
  densities, plus a bounded parent/replacement trace that reaches one settled
  state.

The generated facts tool derives the required-condition matrix from those
scene definitions. A handwritten `representative: true` field is insufficient.
The mixed-LOD trial must distinguish adjacent node/sample roles and presentation
versions; splitting one root sample into several transfer batches does not count
as mixed LOD.

Generated scenes are presentation fixtures. Their authored positions and
attributes are exact inputs, but their displayed Coverage remains authored or
sampled as declared and never becomes Source or Query authority.

### Derived licensed Autzen sample

The repository's checked-in `examples/data/autzen-classified.laz` is the sole
upstream real-world input accepted for v0.21. Its upstream revision, byte length,
SHA-256, LAS metadata, and CC BY 4.0 license are already documented. The v0.21
fixture generator may produce one bounded browser-compatible derivative for
visual trials under these rules:

- derivation uses one checked-in, non-configurable recipe with a fixed spatial
  extent or fixed Source-ordinal selection, stable Source order, explicit Point
  ceiling, and deterministic output encoding;
- the accepted display inputs--world position, intensity, classification, and
  RGB--are preserved or converted by one documented exact rule; discarded or
  re-encoded upstream fields are enumerated and cannot be described as
  preserved;
- the output visual-sample payload and manifest have fixed byte lengths and
  SHA-256 identities; any additional deployment, index, or Source-record
  artifact introduced by the implementation is bound the same way;
- the derivation record binds the upstream bytes, recipe/version, selected
  records, output position transform, attribute mapping, and output Source
  identity;
- a license/notice artifact preserves creator, upstream URL and revision,
  CC BY 4.0 attribution, modification notice, and permission to redistribute
  the derivative and its rendered evidence; and
- verification regenerates the derivative in an isolated directory and
  compares exact bytes before any browser trial uses it.

The derivative is accepted because it supplies real spatial structure and raw
display attributes under a documented redistribution and image-publication
license. It is not independent partner data, a permitted proprietary corpus, a
complete Autzen survey, or evidence of professional interpretation. The public
viewer does not gain a general LAZ loader, arbitrary-URL support, derivation
interface, or multiple-Source policy from this private fixture.

## Required trial matrix

The machine-readable baseline contains exactly nine trials--five deterministic
generated trials and four Autzen-derived display-mode trials over the one
licensed sample--plus a derived condition-coverage table. Within that closed
matrix:

- every sparse, dense, layered, high-dynamic-range, classification,
  large-world, and mixed-LOD condition is exercised by a generated trial;
- the Autzen derivative exercises dense real-world structure, classification,
  intensity, RGB, elevation, and large-world coordinates only where its exact
  facts justify those labels;
- neutral, elevation, RGB, intensity, and classification display modes each
  appear in at least one accepted trial;
- perspective and orthographic projection each appear in at least one accepted
  trial, without requiring an unbounded Cartesian product;
- at least one trial has no selection and one has a fixed presentation-only
  highlight whose Point Identity and nominal pick coverage are checked
  independently from its decorative pixels;
- at least one fixed camera exposes the stable mixed-LOD cut, and the temporal
  trace records the bounded path to settlement; and
- every trial declares its Coverage as authored, sampled, or complete without
  inferring Query completion.

The condition matrix records which exact input fact justifies each condition.
One Autzen label or attractive image cannot satisfy several conditions without
the corresponding measurable Source, scene, or trial facts.

## Camera, viewport, and settling contract

Each camera is recorded as the exact public perspective or orthographic camera
inputs, including finite world-space eye or scale, target, up vector, near/far
planes, and projection parameters. A prose pose name is not sufficient. The
trial additionally binds logical viewport, physical viewport, device-pixel
ratio, display mode, background, point-style values, and highlight state.

The accepted canonical lane uses a 320 by 240 CSS-pixel canvas at requested DPR
2, producing exactly 640 by 480 physical pixels. The runner records requested
and observed DPR, CSS size, canvas bitmap size, browser zoom/visual-viewport
scale when exposed, and rejects the trial if those facts do not produce the
accepted physical viewport. It also retains the v0.20 physical-dimension and
area ceilings: no axis exceeds 4,096 physical pixels and no capture exceeds
8,388,608 physical pixels. An explicitly noncanonical diagnostic capture may
use another bounded size, but it cannot replace or relax a canonical trial. One
DPR observation does not establish DPR portability.

Before a canonical capture, the private runner requires:

1. the exact expected Source or authored-scene identity and View generation;
2. the trial's complete expected batch, Point, Coverage, and presentation facts;
3. no pending load, requested batch, publication, replacement, retirement,
   recolor, highlight, or scheduled render work;
4. the declared presentation-latency drain; and
5. a 30-foreground-frame quiet window with unchanged generation, camera,
   viewport, Coverage, drawn/resident counts, and logical resources.

The runner records the first settled frame, quiet-window completion, and any
failure. A trial that reaches its frame ceiling without satisfying every
condition fails; it cannot capture the last moving frame as its baseline.

## Private browser trial and capture module

The visual runner remains private to `browser-demo` and the repository
verification scripts. Its interface accepts one checked-in trial identifier and
returns one bounded structured result. Scene construction, licensed-fixture
loading, settling, frame capture, channel normalization, comparison, artifact
publication, and cleanup remain inside the module. Tests cross the same
interface. There is no shallow collection of caller-configurable capture
methods.

The runner uses the existing viewer/renderer ownership and frame description.
After a trial is settled, it renders the accepted frame to a private copyable
texture whose format matches the configured surface-format class. It copies the
texture into a row-aligned bounded readback buffer, maps it only after queue
completion, removes row padding, normalizes BGRA or RGBA ordering explicitly,
and emits canonical top-left-origin RGBA8 pixels. Surface format, color space,
alpha mode, present mode, normalization rule, and readback layout are evidence
facts.

The normal caller-owned canvas is still presented during the attended run. The
copyable target does not claim to observe operating-system composition, display
ICC transforms, panel presentation, or physical GPU allocation. Capture encode,
copy, map, canonical-pixel, and artifact bytes are accounted separately from
representative viewer frame submission. The runner retains at most one
reference and one comparison frame at a time and releases every capture target
and mapped buffer before the next trial.

No capture function is exported from `@punctra/viewer`, `@punctra/react`, or a
public Rust crate. The raw Wasm module, renderer target, readback buffer, PNG
encoder, artifact writer, and trial manifest remain implementation details at
the private visual-trial seam. A future public screenshot or visual-testing
interface requires a separate accepted design and a second real caller.

## Image and temporal comparison

Evidence artifacts use one lossless encoding with fixed dimensions and channel
semantics. The baseline records both encoded byte length/SHA-256 and decoded
canonical RGBA8 SHA-256 so encoder drift cannot masquerade as pixel drift and a
byte-identical compressed file cannot bypass decoded-fact checks.

The private comparison module reports, per trial:

- compared dimensions and total physical pixels;
- exact-equal pixels and unstable pixels;
- maximum, mean, root-mean-square, and p95 absolute channel difference;
- unstable-pixel count and fraction under the named tolerance profile;
- per-feature foreground count, bounded centroid or occupancy result, and any
  missing or falsely filled feature region;
- background-only and foreground-only difference summaries where the scene
  facts supply an exact mask; and
- the worst temporal pair and its difference artifact during the settled quiet
  window.

The v0.21 tolerance profile is calibrated from three complete viewer/harness
recreations on the declared lane and frozen before final attended evidence is
recorded. Calibration records every raw comparison and may select a tighter
profile, but it may not permit a per-channel threshold above 2, an unstable-
pixel fraction above 0.001, a maximum channel difference above 4, or a feature
displacement above one physical pixel. The settled generated temporal trials
additionally require zero unstable pixels because their camera, inputs,
resident state, and presentation state are unchanged. A failed run is not
repaired by widening tolerance after observation; changing these caps or a
trial-specific profile requires a reviewed design and baseline revision.

These are regression tolerances, not perceptual quality scores. No SSIM-like or
whole-image aggregate can override the independent maximum, unstable-pixel,
feature, temporal, Coverage, and resource gates. Adapter-specific tolerances
remain absent until another exact adapter is executed and recorded as a
separate profile.

## Coverage, feature, and authority reporting

Every captured result records Source or scene identity, View generation,
Coverage label, covered Source Points when known, displayed Points, drawn
Points, resident Points, batch identities and versions, presentation weights,
display mode, projection, selection/highlight counts, and the exact frame facts
used for readback.

Feature regions are declared before canonical image observation. Generated
regions are derived from authored Point identities and exact projection inputs.
Autzen regions bind fixed image rectangles and measurable foreground/attribute
expectations without assigning unverified semantic meaning. A maintainer may
name a region `stadium-bowl` for navigation, but the machine gate checks only
the declared projected evidence and does not claim that a user identified a
professional feature correctly.

Canonical images and feature reports have `presentation_only` authority.
Provisional pick remains `provisional_gpu_hint`; exact immutable-record
confirmation remains `exact_source_record`. A visual comparison cannot change
Point Identity, exact position, raw classification, selection membership,
Coverage truth, or Query completion.

## Independent resource and frame reporting

Each trial preserves the inherited independent v0.20 resource facts and adds
capture-specific accounting. The record separates:

- renderer resident Points and logical vertex bytes;
- canvas surface bytes and renderer transient-texture bytes;
- retained decoded records, Worker staging, concurrent response, and verified
  cache bytes for streamed trials;
- copyable capture texture bytes, row-aligned readback bytes, retained canonical
  pixel bytes, encoder working bytes, and encoded artifact bytes;
- main-thread frame submission, capture encoding, elapsed-to-submitted-work-done
  callback, elapsed-to-readback-map callback, comparison, and artifact-encoding
  intervals; and
- a nullable JavaScript heap observation, separately from process RSS, physical
  cache allocation, and physical driver/GPU allocation, which remain
  unavailable.

Both callback intervals start at the begin-capture monotonic origin. They
include callback and browser-scheduling delay, do not establish callback
ordering, and are not physical GPU-completion time.

The baseline declares an independent ceiling for every owned subsystem. Capture
retains no more than two canonical frames at once, temporal comparison streams
through the 30-frame window, and evidence encoding cannot silently lower image
dimensions or omit a trial to stay within memory. One total-memory or total-time
number may not hide an unbounded subsystem.

The current transport additionally caps encoded run artifacts at 1,207,959,552
bytes, evidence JSON at 33,554,432 bytes, the baseline-input manifest at
1,048,576 bytes, USTAR entry count at 896, archive structure at 1,048,576 bytes,
archive overhead at 35,651,584 bytes, and the complete uncompressed archive at
1,243,611,136 bytes. These are private transport-allocation ceilings rather
than expected output sizes. The archive is not an evidence artifact and is not
checked in.

The primary transport is one standard browser Blob download of that archive.
For an attended in-app browser that does not materialize the Blob, the strict
local server may expose a separately enabled, same-origin-only archive export.
The fallback accepts only the already bounded private TAR, rejects cross-origin
POST and caller-selected filenames or repository paths, and publishes beneath
one operator-selected local export directory without replacing an existing
target. The fixed endpoint is `/qualification-visual-export`; the exact media
type is `application/x-tar`, and the positive decimal request length cannot be
zero or exceed the 1,243,611,136-byte archive ceiling. The single `Host` must
name a loopback authority at the server's actual bound port, and the request
`Origin` must exactly match `http://` plus that `Host`; no cross-origin POST
grant exists. The server streams 64 KiB chunks to an exclusive `.part`,
verifies the exact written length,
computes SHA-256, fsyncs, and no-replace publishes the fixed
`v0.21-browser-visual-evidence.tar` name. Its HTTP 201
`punctra-browser-visual-export-receipt-v1` response and output archive remain
transport artifacts rather than evidence.

Representative frame-cost observations exclude the explicitly labelled capture
and comparison work. Capture overhead is still measured and bounded; it is not
presented as normal viewer frame cost.

## Interpretation rubric

The machine-readable baseline freezes one small rubric with these prompts:

- **depth**: can the observer distinguish the declared near and far layers;
- **shape**: can the observer trace the declared thin or bounded shape without
  inventing a connection;
- **density transition**: does a declared sparse/dense or mixed-LOD seam appear
  to be a hole, wall, platform, or other false Source feature;
- **color meaning**: can the observer distinguish raw display mapping from
  exact semantic or measurement authority;
- **selection**: can the observer identify selected, unselected, stale, and
  nonresident presentation without relying on color alone; and
- **false feature**: did any grid, moiré, point footprint, depth cue, palette,
  or LOD artifact look like Source geometry.

Each answer is one of `clear`, `ambiguous`, `false_feature`, `not_visible`, or
`not_observed`, with one bounded optional note and an anonymous observer/session
label. The rubric stores no name, contact information, employer, credentials,
private Source path, or unrelated browser data.

Repository tests verify rubric schema, prompt/image binding, bounded text,
post-capture presentation/selection ordering, and explicit `not_observed`
handling. They do not fabricate an observer. Rubric controls remain disabled
until capture finishes and every exact prompt-bound image loads in a visible
document. The maintainer then confirms the bounded session label, records every
outcome, and submits the review. Repository completion requires one such
attended maintainer-labelled verify session, but no favorable answer is
required: `ambiguous`, `false_feature`, `not_visible`, and an honestly selected
`not_observed` are preserved as baseline findings rather than release failures.
That session is not independent-human or professional-usability evidence. The
record-stage rubric is calibration-only and is not the final interpretation
record.

## Machine-readable baseline and evidence

The policy record uses schema `punctra-browser-visual-baseline-v1`. It binds:

- release and exact predecessor v0.20 integration-baseline path, byte length,
  and SHA-256;
- package/runtime identity and the unchanged v0.20 point-appearance policy,
  including relevant shader, renderer, display-mapping, point-style,
  background, depth, blend, and presentation-version facts;
- every generated scene and derived Autzen payload/manifest/derivation/license
  artifact, plus any deployment/index artifact actually introduced, by path,
  byte length, SHA-256, and semantic fact generator;
- the commit-free baseline-input manifest and nine accepted baseline PNGs by
  encoded and decoded identities;
- the closed trials, exact cameras, viewport/DPR, modes, projections, settling
  rules, capability states, feature regions, condition-coverage derivation,
  tolerance profiles, and independent resource ceilings;
- capture format, channel normalization, row layout, lossless encoder, decoded
  pixel semantics, private USTAR transport policy, and interpretation-rubric
  version; and
- the closed external-evidence and nonclaim fields that remain false.

The observation record uses schema `punctra-browser-visual-evidence-v1`. It
binds:

- exact implementation commit, visual-baseline verifier SHA-256, observation
  date, package artifact, and attended lane;
- browser, operating system, device, display, WebGPU adapter/backend, surface,
  color-space, alpha, present-mode, viewport, DPR, and capability/fallback facts;
- three complete viewer/harness recreations for every required trial;
- settlement, quiet-window, image, temporal, Coverage, feature, resource,
  timing, cleanup, and interpretation outcomes;
- each lossless image/difference artifact by encoded and decoded identities;
  and
- explicit unqualified platforms, unavailable physical measurements, and
  outstanding external evidence.

The baseline verifier derives facts from checked-in scene definitions,
generated and licensed fixture bytes, derivation records, package artifacts,
runtime sources, images, and observed evidence. It decodes evidence images and
recomputes image, feature, temporal, resource, and tolerance outcomes. A
recorded `passed` flag cannot override a derived failure. Tests must prove that
tampering with an input, image, camera, condition mapping, tolerance, feature
region, environment fact, predecessor digest, authority label, external-
evidence field, implementation pin, or verifier hash fails verification.

The release record, observation record, and verifier pin the same full
implementation commit. Qualified implementation paths may not change after
that pin. Documentation-only attestation may follow, but any code, fixture,
capture, comparison, or verifier correction requires a new implementation pin
and complete rerun. The verifier source has its own SHA-256 because the final
observation record is added after executable implementation is pinned.

## Exact attended browser lane

The accepted repository lane is the exact locally available Codex in-app
Chromium/macOS/Apple-integrated-GPU/WebGPU path inherited from v0.20, rebuilt
from the `0.21.0-alpha.1` packed artifacts. The final record uses only facts
reported or independently identified for the session; it does not infer browser
support from a user-agent family or adapter support from one GPU label.

The strict local Range server remains required for streamed trials. Final
acceptance is a mandatory sequential record-then-verify workflow, not a choice
between equivalent modes:

1. In attended `record` mode, verify every generated and licensed derivative
   artifact, execute all nine identifiers, and capture three complete
   viewer/harness recreations after each 30-frame quiet window.
2. After all record captures finish, visibly load every exact prompt-bound
   rubric image, submit the calibration-only rubric, and download one bounded
   USTAR transport bundle.
3. Extract that bundle into a fresh directory. Retain only the nine canonical
   baseline PNGs and commit-free baseline-input manifest; record-mode evidence,
   rubric, recreation, transition, and difference artifacts are not final
   evidence.
4. Check in the retained baseline inputs, freeze every qualified implementation
   path, create the implementation pin, and refresh every dependent static
   digest.
5. Rebuild that exact pinned implementation and pass the inherited packed
   quickstart and browser qualification before visual evidence is accepted.
6. Open attended `verify` mode with the exact 40-hex implementation commit,
   verifier byte length, and 64-hex verifier SHA-256 in the page URL. The runner
   fixes the accepted attended-lane identity and keeps its visible Run control
   disabled until all three pins are valid. Use that visible control to execute
   all nine identifiers and three complete recreations against the checked-in
   baselines, deriving every image, temporal, Coverage, feature, timing,
   cleanup, and independent resource gate.
7. After verify captures finish, visibly load the bound images and submit the
   final maintainer-labelled interpretation rubric without treating favorable
   answers as a pass gate.
8. Download the single bounded USTAR bundle, extract its evidence JSON and PNGs
   to their recorded repository paths, and require the static verifier to derive
   a pass. Only `verify`-mode evidence is eligible for final acceptance.
9. Dispose every viewer, Worker, mapped buffer, texture, and capture module in
   both stages.

The attended operator uses the standard Blob download first. If that download
does not materialize, the operator may restart the strict server with its
explicit local-export opt-in and repeat the affected stage so the same-origin
page opened with `transport=server` POSTs the identical bounded archive. The
server opt-in is `--visual-export-dir <existing-fresh-dir>`. This is a transport
fallback, not a second evidence path; a cross-origin request or existing export
target fails closed.

Installed Chrome, Safari, another browser, another display, another adapter,
software rendering, and every other platform remain unqualified until the same
exact sequence is actually executed there. There is no WebGL, Canvas, software,
silent, or reduced-feature fallback. Any unavailable optional renderer path is
named in the trial's capability/fallback facts and compared only against the
corresponding accepted profile.

## Interface and versioning constraints

The existing `@punctra/viewer`, `@punctra/viewer/input`,
`@punctra/viewer/exact-query`, and `@punctra/react` interfaces remain the public
browser seams. The visual-trial module, licensed derivation, capture target,
readback, comparison, artifact encoding, and verifier are private repository
modules. Their depth comes from one small trial interface over the complete
evidence lifecycle; callers do not assemble renderer internals themselves.

All public Rust libraries and both JavaScript packages advance together to
`0.21.0-alpha.1`. Persisted Source, index, transfer, cache, diagnostics,
renderer, integration-baseline, visual-baseline, and evidence schemas remain
independently versioned and do not advance solely because the package version
changes.

The v0.20 machine-readable baseline remains immutable historical evidence.
v0.21 references its exact bytes and publishes a separate visual baseline; it
does not rewrite v0.20 package, quickstart, matrix, or release records.

## Verification and completion

Repository completion requires:

- deterministic generated-scene and Autzen-derivative regeneration with exact
  byte, Source, license, attribution, condition, and feature facts;
- unit and fault tests for the private trial interface, settling state,
  row-aligned readback, format/channel normalization, lossless image encoding,
  tolerant comparison, temporal worst-case selection, feature checks, resource
  ceilings, cleanup, rubric validation, and verifier tamper rejection;
- the inherited Rust, JavaScript, Wasm, fixture, packed-artifact, TypeScript,
  React, Vite, API-reference, documentation, fuzz, example, benchmark, and
  `PUNCTRA_REQUIRE_GPU=1` lanes in `CONTRIBUTING.md`;
- one attended record stage, an exact implementation pin containing its
  accepted baseline inputs, the inherited packed quickstart and qualification,
  and one attended verify stage through the strict local server;
- a machine-readable visual baseline and evidence record, checked-in lossless
  artifacts, human-readable release verification record, exact implementation
  pin, and visual-verifier SHA-256;
- one attended, maintainer-labelled interpretation record that preserves
  ambiguous, false-feature, not-visible, and not-observed outcomes without
  promoting it to independent-human evidence;
- explicit audit confirmation that the v0.20 point-appearance policy did not
  change and that no known release-blocking correctness, data-mixing, capture,
  evidence, permission, packaging, or documentation defect remains inside the
  accepted repository slice; and
- explicit preservation of every unqualified platform, unavailable physical
  observation, independent-adopter gap, independent-human gap, and later visual
  nonclaim.

No hosted CI is added. Completion wording will be: **Complete and repository-
verified for the bounded v0.21 visual-quality baseline and regression corpus,
including deterministic generated scenes, one derived licensed Autzen sample,
and one exact local Chromium/macOS/Apple-GPU attended lane; other browsers,
operating systems, adapters, devices, independent human interpretation,
independent adoption, final visual-quality completion, support qualification,
beta, v1, and release-candidate status remain outstanding.**

## Explicit non-goals

v0.21 does not authorize:

- intentional changes to Point footprint, anti-aliasing, point-size policy,
  density blending, LOD transition treatment, depth cue, shader color,
  background, contrast, exposure, tone mapping, palette, highlight appearance,
  or any other visual default; those begin only in later accepted releases;
- a public screenshot, readback, golden-image, test-runner, renderer-target,
  arbitrary-scene, or visual-comparison interface;
- arbitrary LAS/LAZ URLs, general browser LAZ decompression, COPC, EPT, 3D
  Tiles, hierarchy traversal, complete Source Coverage, multiple Sources, or a
  Source conversion product feature;
- changing Point Identity, exact position, raw Attributes, classification,
  pick authority, selection membership, Coverage truth, Query completion,
  editing, terrain, QA, or export through visual evidence;
- hiding adapter variation with a broad global tolerance, resizing or cropping
  evidence after capture, changing a camera after observing its image, or
  accepting an aggregate score while a feature/resource gate fails;
- publishing a private Source, screenshot, or human observation without the
  corresponding permission and attribution;
- independent adoption, professional usability, field qualification, broad
  browser/device support, API stability, production support, registry/CDN
  publication, beta, release-candidate, or v1 claims; or
- GitHub Actions or another hosted CI service.
