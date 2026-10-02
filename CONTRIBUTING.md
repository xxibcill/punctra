# Contributing

Punctra is intentionally a small set of independently usable modules. Before
adding a new public seam, check it against the accepted
[v0.1 renderer scope](docs/design/render-engine-v0.1.md) and
[v0.2 planning scope](docs/design/adaptive-view-planning-v0.2.md), plus the
completed [v0.3 Real Sources scope](docs/design/real-sources-v0.3.md) and
completed [v0.4 Out-of-core View scope](docs/design/out-of-core-view-v0.4.md).
The completed [v0.5 Durable document core
scope](docs/design/durable-document-core-v0.5.md) permits one deep
`point-workspace` crate for exact classification selection, temporary Point
Sets, sparse classification Revisions, and Operation recovery. The completed
[v0.6 Terrain and QA benchmark
scope](docs/design/terrain-qa-benchmark-v0.6.md) additionally permits the exact
`Snapshot::point_rows` stream, one deep `point-terrain` crate, and the private
headless `terrain-demo` composition. The completed [v0.7 technical-readiness
scope](docs/design/technical-alpha-readiness-v0.7.md) permits linked child
cancellation, exact Revision Audit and Edit Footprint facts, exact LandXML
ensure/reconciliation, and the private durable `terrain-demo` Workflow Run,
canonical report, and structured recovery diagnostics. The completed
[v0.8 repository interoperability qualification
scope](docs/design/design-partner-mvp-v0.8.md) implements the private, bounded
`terrain-demo` LandXML comparison reader, read-only Complete-Run binding,
full-ceiling streaming verification, and separate canonical Round-Trip
Evidence. The completed [v0.9 Trust and v1 Candidate
scope](docs/design/trust-v1-candidate-v0.9.md) permits version-1 compatibility
fixtures, ownership-safe persistence hardening, the frozen support matrix,
interface review, and reproducible local qualification for the same narrow
workflow. Its exact command outcomes and benchmark observations are recorded
in the [repository verification record](docs/releases/v0.9.0.md). The accepted
[v0.10 Field
Qualification and Professional Inspection View
scope](docs/design/field-inspection-view-v0.10.md). v0.10
permits evidence collection and a staged professional display path while
keeping sampled GPU values non-authoritative. Its five display modes,
perspective/orthographic controls, View-state diagnostics, and corpus runner
stay inside the private `renderer-demo` host. The narrow public point-index
additions are an explicit disk-v2 inspection recipe and an ownership-safe
fresh-preparation policy used by corpus and benchmark cold-build measurements;
the disk-v1 path remains byte-compatible. The completed [v0.11 Exact
Interactive Review and Ground Correction
scope](docs/design/exact-interactive-review-v0.11.md) permits one public
`point-review` crate for exact CPU confirmation of a provisional Point
Identity and one inclusive screen-through rectangle at a pinned Snapshot. It
also permits bounded Point Set entry/identity iteration, an explicit renderer
highlight-input ceiling, and the public `render-wgpu` `third_party_host`
example. Durable correction reuses caller-owned Operation Identities and the
existing Workspace commit, immediate-head Revert, Audit/Edit Footprint, and
Operation-resolution seams. The accepted [v0.12 Explicit Spatial Reference and
Package Publication
scope](docs/design/explicit-spatial-reference-v0.12.md) adds one structured
projected-reference profile, strict complete GeoTIFF decoding, reference-bound
Workspace/Terrain behavior, metre-only QA/LandXML propagation, strict
round-trip comparison, and the local path documented in the [library
packaging guide](docs/guides/library-packaging.md). It adds no coordinate
transformation or CRS guessing. The accepted [pre-v0.13 renderer-quality
corrective scope](docs/design/renderer-quality-corrective-pre-v0.13.md) permits
only its enumerated renderer additions: conditional batch presentation, the
display-diameter override, bounded eye-dome configuration and disposition, and
read-only transient-texture and resident-highlight observations required by the
private `renderer-demo` host. It does not authorize a general material, shader,
plugin, or host-UI interface. The accepted [v0.13 Persistent Bounded-AOI
Terrain scope](docs/design/persistent-production-scale-terrain-v0.13.md) permits
one explicit-inclusive-AOI preparation operation in `point-terrain`, complete
verified-input and final-stage checkpoints, checksummed disk-v1 Surface
publication/reopen, stale detection, and bounded file-backed vertex/face
streams. It preserves legacy `derive`, the existing canonical single-worker
full-AOI triangulator, and frozen Workflow Run-v1. It does not authorize true
out-of-core, tiled, parallel, constrained, or production-qualified terrain.
The completed [v0.14 Exact Terrain QA and Correction Loop
scope](docs/design/exact-terrain-qa-correction-v0.14.md) permits one bounded
CPU-authoritative QA operation for an exact Snapshot/Surface pair, one exact
semantic Surface comparison, explicit provenance/freshness/tolerance evidence,
and a public correction/re-derive/compare/Revert example. It also permits the
narrow supporting public `foundation-runtime` scoped cancellation link and the
read-only `PointQuery` bounds/classification inspectors required for linked
pull-stream cancellation, canonical Query hashing, and evidence. Workspace
mutation continues through existing `point-workspace` commits. It does not
authorize terrain constraints, automatic correction, a second edit model,
continuous plane/TIN intersections, general charting/UI, or field claims from
fixtures.
The accepted [v0.15 WebAssembly and WebGPU Browser Foundation
scope](docs/design/browser-foundation-v0.15.md) permits one private
`browser-demo` application that compiles the existing `render-protocol`,
`point-view`, and `render-wgpu` composition to `wasm32-unknown-unknown`. The
JavaScript caller owns its canvas and lifecycle policy; the private Rust host
owns WebGPU resources on that caller's behalf. The generated scene, resource
ceilings, progressive display, provisional pick, and explicit failure states
are local acceptance evidence only. It does not authorize remote LAS/LAZ
delivery, browser decoding, a supported JavaScript SDK, exact browser Queries,
editing, broad browser qualification, or visual-quality claims.
The accepted [v0.16 HTTP Range Streaming, Browser Caching, and Worker Decoding
scope](docs/design/http-range-streaming-v0.16.md) extends only that private host
with one trusted immutable-LAS deployment manifest, strict bounded Range
transport, disk-v2 root-sample decoding in one Worker, explicit cache policy,
identity-versioned keys, deterministic recovery outcomes, and a cold/
recreation/warm-cache local acceptance path. It does not authorize arbitrary
LAS/LAZ URLs, complete browser Source decoding, exact browser Queries, a public
network/viewer seam, credentials policy, service-worker ownership, a supported
SDK, or broad browser qualification.
The completed [v0.17 Browser Viewer API
scope](docs/design/browser-viewer-api-v0.17.md) adds one framework-neutral
viewer façade and matching TypeScript declaration inside `browser-demo`, five
inherited display modes, generation-safe provisional pick/highlight behavior,
an optional policy-free input normalizer, and a separately injected exact-
Point bridge for the immutable LAS fixture. It does not authorize SDK/registry
packaging, a framework adapter, arbitrary Source delivery, general exact
Queries, editing, terrain, export, API stability, or broad browser support.
The completed [v0.18 Embeddable SDK and Framework Integration
scope](docs/design/embeddable-sdk-v0.18.md) packages that same façade as the
`@punctra/viewer` ESM/Wasm tarball, adds explicit bundler/copied-asset
resolution, generated API reference, lifecycle aliases, two clean
TypeScript/Vite embedding trials, and only the thin `@punctra/react` lifecycle
adapter justified by those trials. It does not authorize registry/CDN
publication, another framework adapter, arbitrary Source delivery, API
stability, broad bundler/browser support, independent adoption, support
qualification, or release-candidate claims.
The completed [v0.19 Browser and Device Qualification
scope](docs/design/browser-device-qualification-v0.19.md) adds one exact local
Codex in-app Chromium/macOS/Apple-GPU qualification lane, additive Source-load
timings, fixed functional/latency/resource gates, explicit retry-versus-
recreation recovery evidence, a machine-readable matrix, and a support/issue
playbook. It does not qualify installed Chrome, Safari, another operating
system, adapter, or device; force physical device loss or memory pressure;
establish independent adoption or API stability; expand visual policy; or
claim beta, support-qualified, or release-candidate status.
The completed bounded [v0.20 Stable Browser-Engine Integration Baseline
scope](docs/design/browser-integration-baseline-v0.20.md) consolidates the
supported package exports, clean packed TypeScript quickstart, strict attended
consumer workflow, exact fixture/scene/presentation freeze, recovery boundary,
capability matrix, and known limitations. The repository consumer is not an
independent adopter; registry/CDN publication, other browsers/devices, API
stability, visual-quality completion, support qualification, beta, v1, and
release-candidate status remain outside this scope.
The completed bounded [v0.21 Visual-Quality Baseline and Regression Corpus
scope](docs/design/visual-quality-baseline-v0.21.md) permits a closed private
browser corpus, deterministic generated inputs, one CC BY 4.0 Autzen
derivative, private offscreen GPU capture/readback, lossless canonical-image
encoding, tolerant and temporal comparison, Coverage/feature/authority/resource
reporting, a non-gating interpretation rubric, and machine-readable baseline
and evidence verification. The 2026-08-28 repository activation supplies the
representative corpus that v0.20 did not; it does not pretend the original gate
was already satisfied. It changes no public viewer seam or point-appearance
policy and does not authorize cross-browser/display claims, arbitrary Sources,
independent-human or adopter evidence, improved/final visual quality, support
qualification, API stability, beta, v1, or release-candidate status.
Its exact completed repository observations and pins are recorded in the
[v0.21 verification record](docs/releases/v0.21.0.md).
The active bounded [v0.22 Point Footprint and Edge Quality
scope](docs/design/point-footprint-edge-quality-v0.22.md) adds an explicit
renderer Point-footprint request/status seam, deterministic four-sample circular
color coverage, capability/resource fallback, one private browser projected-
density display diameter, unchanged nominal pick coverage, exact target
accounting, and separate quality/DPR/fallback/pick/cost evidence against the
immutable v0.21 corpus. It does not change geometry, Point identity, Source or
Query authority, View/LOD policy, display mappings, or public browser exports,
and it does not authorize physical-display, cross-browser/device,
independent-human/adopter, support, beta, release-candidate, or v1 claims. Its
final attended evidence remains pending.
Apart from the explicit v0.8 reader exception, the v0.17 browser-demo
exact-query bridge is a narrowly scoped exception for the trusted immutable
LAS fixture described by the accepted design. All other external format
decoding belongs only in accepted Source adapter crates.
Networking, polygon/brush/visible-only/occlusion selection, arbitrary
Attribute or position edits, constrained or true out-of-core terrain, general
export, Source rewriting, automatic recovery, and general host UI remain in
callers or future projects unless the scope is explicitly revised.

## Local verification

Install the pinned Rust toolchain and run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
scripts/verify-packages.rb
cargo fmt --manifest-path fuzz/Cargo.toml --all --check
cargo clippy --manifest-path fuzz/Cargo.toml --all-targets -- -D warnings
cargo check --manifest-path fuzz/Cargo.toml --bin index_persistence
cargo check --manifest-path fuzz/Cargo.toml --bin terrain_persistence
cargo test --manifest-path fuzz/Cargo.toml --lib
cargo check -p browser-demo --target wasm32-unknown-unknown
cargo clippy -p browser-demo --all-targets --all-features -- -D warnings
cargo clippy -p browser-demo --target wasm32-unknown-unknown --all-targets \
  --all-features -- -D warnings
cargo run -p browser-demo --bin generate_stream_fixture
cargo run -p browser-demo --bin generate_visual_source_fixture
node --test apps/browser-demo/web/*.test.mjs packages/react/*.test.mjs scripts/*.test.mjs
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
node scripts/verify-browser-qualification.mjs
node scripts/verify-browser-integration-baseline.mjs
node scripts/verify-browser-visual-baseline.mjs
node scripts/generate-browser-sdk-reference.mjs --check
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
cargo run -p source-memory --example memory_source
cargo run -p point-index --example direct_use
cargo test -p point-workspace --all-features
cargo test -p point-review --test interface
cargo test -p render-protocol --test state_model
cargo run -p point-terrain --example derive
cargo run -p point-terrain --example persistent_surface
cargo run -p point-terrain --example exact_terrain_qa
cargo test -p point-terrain --all-features
cargo test -p point-terrain --test persistence
cargo test -p terrain-demo --lib --all-features
cargo test -p terrain-demo --test workflow
cargo test -p terrain-demo --test process
cargo test -p renderer-demo --test headless_smoke
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test headless_smoke \
  corpus_success_binds_trace_inputs_and_separate_resource_measurements -- --exact
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --bin renderer-demo \
  appearance::gpu_tests
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test headless_smoke \
  corpus_pre_v0_13_repository_lane_records_settlement_and_the_declared_matrix \
  -- --exact
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test planner
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --test display_gpu
PUNCTRA_REQUIRE_GPU=1 cargo run -p render-wgpu --example third_party_host
test -f docs/guides/first-las-laz.md
test -f docs/guides/library-packaging.md
test -f docs/guides/persistent-terrain.md
test -f docs/guides/exact-terrain-qa.md
test -f docs/guides/browser-foundation.md
test -f docs/guides/browser-streaming.md
test -f docs/guides/browser-viewer.md
test -f docs/guides/browser-sdk.md
test -f docs/guides/browser-qualification.md
test -f docs/guides/browser-quickstart.md
test -f docs/guides/browser-known-limitations.md
test -f docs/guides/browser-visual-quality.md
test -f docs/guides/browser-point-footprint.md
test -f docs/api/browser-sdk.md
ruby -rjson -e 'ARGV.each { |path| JSON.parse(File.read(path)) }' \
  docs/guides/field-corpus.example.json \
  docs/releases/v0.20-browser-baseline.json \
  docs/releases/v0.20-browser-quickstart.json \
  docs/releases/v0.20-browser-matrix.json \
  docs/releases/v0.21-browser-baseline.json \
  docs/releases/v0.21-browser-quickstart.json \
  docs/releases/v0.21-browser-matrix.json \
  docs/releases/v0.21-browser-visual-baseline.json \
  docs/releases/v0.21-browser-visual-rubric-template.json \
  apps/browser-demo/web/fixtures/visual-v1/corpus.json \
  apps/browser-demo/web/fixtures/visual-v1/autzen-classified-sample.json \
  apps/browser-demo/web/fixtures/footprint-v1/corpus.json
git diff --check
```

After `scripts/build-browser-sdk.sh`, run
`scripts/serve-browser-demo.py --port 8000` and open
`http://127.0.0.1:8000/` in a secure-context WebGPU browser. A generic static
server is insufficient for the v0.16–v0.20 acceptance fixture because exact Range,
strong-validator, identity-encoding, exposed CORS-header, and bounded fault
behavior is part
of the contract. The document must publish `PASS` after the inherited v0.15
lifecycle checks, cold bounded Source/index requests, worker decode and
transfer, progressive render before complete Source transfer, explicit viewer
and Worker recreation, and an identity-matched warm-cache run with zero binary
network requests. The v0.17 continuation must additionally exercise the typed
public viewer only, all five inherited display modes, both projections,
normalized-input wiring, provisional pick/highlight/clear, exact confirmation
of the same immutable Source record, cancelled exact confirmation, and stale-
generation rejection. The v0.18 continuation must import the packaged SDK
entry, exercise pause/resume/dispose lifecycle spelling, and report the current
package version. The v0.19 continuation must additionally reject an
over-limit resize without changing the prior viewport, change and restore DPR,
skip one hidden frame, resume, classify pre-publication Worker and disconnected-
network failures as recoverable without generation change, sample 30 settled
foreground frames, capture nullable heap facts, and pass every checked-in
qualification ceiling. The harness must also acknowledge its deliberately
delayed Fetch cancellation within 1,000 milliseconds. Record the exact browser,
operating system, adapter, surface
format, viewport, and reported transport/cache/worker/main-thread plus
logical/surface/transient resource facts. The step qualifies only that exact
local browser environment. The v0.20 continuation must additionally build the
clean packed quickstart, import only supported package entries, complete the
deterministic cancellation/load/display/projection/navigation/pick/highlight/
exact/pause/resume/dispose workflow, and pass the machine-readable integration
baseline verifier. Run:

```bash
node scripts/verify-browser-integration-baseline.mjs
scripts/serve-browser-demo.py --root target/browser-quickstart --port 8000
```

The v0.21 continuation first regenerates the generated and licensed-derived
visual inputs and verifies the static visual policy:

```bash
cargo run -p browser-demo --bin generate_visual_source_fixture
node --test apps/browser-demo/web/visual-*.test.mjs \
  apps/browser-demo/web/range-server.test.mjs \
  scripts/verify-browser-visual-baseline.test.mjs
node scripts/verify-browser-visual-baseline.mjs
```

The attended visual lane is a mandatory sequential record-then-verify workflow,
not a choice between two equivalent modes. Both stages use the private runner
through the strict local server, keep the canvas at exactly 320 by 240 CSS
pixels and requested DPR 2, reach 640 by 480 physical pixels, and complete 30
unchanged foreground frames before capture. Every one of the nine fixed trials
runs through three complete viewer/harness recreations. Build and serve the
working implementation for the record stage with:

```bash
scripts/build-browser-sdk.sh
python3 scripts/serve-browser-demo.py --port 8000
```

Keep the page visible at browser DPR 2 and 100% zoom. First open
`http://127.0.0.1:8000/visual.html?mode=record` and click **Run
three-recreation corpus**. Captures must finish before the rubric is available.
Wait for the post-capture exact images to load in the visible document, confirm
the bounded session label, record all six non-gating rubric outcomes, and click
**Submit post-capture review**. `not_observed` is valid when the maintainer did
not evaluate a prompt; do not invent a favorable observation. Only after
`document.body.dataset.visualBaseline === "passed"` may the record-stage
repository bundle be downloaded.

The standard transport is one browser Blob download. If the in-app browser
reports success but no TAR materializes, do not fall back to per-artifact
downloads or console reconstruction. Create a fresh empty export directory,
restart the strict local server with its explicit visual-export opt-in, and
repeat only the attended stage whose archive was not transported. For example,
the record fallback is:

```bash
mkdir -p target/v0.21-visual-record-export
python3 scripts/serve-browser-demo.py --port 8000 \
  --visual-export-dir target/v0.21-visual-record-export
```

Open
`http://127.0.0.1:8000/visual.html?mode=record&transport=server`. For the later
verify stage, use a different fresh empty export directory and append
`transport=server` to the fully pinned verify URL documented below. The page
POSTs the same bounded `application/x-tar` body to the same-origin
`/qualification-visual-export` endpoint. It publishes only
`v0.21-browser-visual-evidence.tar` and rejects cross-origin requests or an
existing target rather than overwriting it. The exported TAR and the
`punctra-browser-visual-export-receipt-v1` response remain private transport,
not release evidence.

The record-stage `v0.21-browser-visual-evidence.tar` is private transport, not
release evidence. Inspect and extract it into a fresh directory rather than
overwriting repository files directly:

```bash
tar -tf /path/to/v0.21-browser-visual-evidence.tar
mkdir -p target/v0.21-visual-record
tar -xf /path/to/v0.21-browser-visual-evidence.tar \
  -C target/v0.21-visual-record
```

Retain from that bundle only the nine canonical baseline PNGs and the
commit-free `apps/browser-demo/web/fixtures/visual-v1/baseline-inputs.json`.
The record-mode evidence, rubric, recreation images, transition images, and
difference images are calibration output and must not be published as final
evidence. Check in the retained baseline inputs, freeze every qualified
implementation path, create the implementation pin, and refresh the static
baseline digests.

Rebuild that exact pinned implementation, then repeat the inherited packed
quickstart and browser qualification before final visual evidence is accepted.
Record the pinned facts before opening verify mode:

```bash
git rev-parse HEAD
wc -c < scripts/verify-browser-visual-baseline.mjs
shasum -a 256 scripts/verify-browser-visual-baseline.mjs
```

Substitute those exact values into this single-line URL:

```text
http://127.0.0.1:8000/visual.html?mode=verify&implementation_commit=<40hex>&verifier_byte_length=<decimal>&verifier_sha256=<64hex>
```

The runner fixes the attended lane to
`codex-iab-chromium-151-macos-26-apple-m5-pro`, `browser_trusted_activation`, and
`exact_observed_lane_only`; the URL cannot substitute a different lane. The
visible **Run three-recreation corpus** button remains disabled until all three
pin values match both the checked-in visual baseline and the commit plus
verifier bytes reported by the strict local server. Click it to run the same
nine-trial, three-recreation
corpus against the checked-in baselines, wait for the exact post-capture images,
record and submit the final maintainer-labelled rubric, and wait for
`document.body.dataset.visualBaseline === "passed"`.
The Run click, every rubric selection, and rubric submission each require active
browser transient user activation; `event.isTrusted` alone is not sufficient.
Download the single
repository TAR bundle; separate evidence-JSON and per-artifact links are
diagnostic conveniences, not the documented transport workflow. For the
server fallback, append `&transport=server` to that same pinned URL. Inspect
and extract the verify bundle into a fresh directory, then place its evidence
JSON and PNG artifacts at their recorded repository-relative paths. Do not
substitute screenshots or a development-console reconstruction.

Only verify-mode evidence is eligible. The extracted evidence is accepted only
after this command passes:

```bash
node scripts/verify-browser-visual-baseline.mjs \
  --evidence docs/releases/v0.21-browser-visual-evidence.json
```

The completed v0.21 repository run followed that sequence: all nine trials
passed through three complete recreations, 873 PNG artifacts were retained,
and all six rubric outcomes were explicitly `not_observed` under
`codex-local-maintainer-not-human`. The [v0.21 verification
record](docs/releases/v0.21.0.md) pins the exact implementation, verifier,
environment, evidence, and remaining nonclaims. Future reproductions must not
replace those observations with placeholders. See the [browser visual-quality
guide](docs/guides/browser-visual-quality.md).

The v0.22 continuation is another mandatory sequential workflow. It never
rewrites the v0.21 corpus, baselines, evidence, artifacts, or release record.
First run the focused static and GPU checks:

```bash
cargo test -p render-wgpu --test contracts
cargo test -p browser-demo
cargo test -p renderer-demo --bin renderer-demo appearance::tests
scripts/build-browser-sdk.sh
node --test \
  apps/browser-demo/web/footprint-artifacts.test.mjs \
  apps/browser-demo/web/footprint-corpus.test.mjs \
  apps/browser-demo/web/footprint-evidence.test.mjs \
  apps/browser-demo/web/footprint-export.test.mjs \
  apps/browser-demo/web/footprint-records.test.mjs \
  apps/browser-demo/web/footprint-runner-core.test.mjs \
  apps/browser-demo/web/visual-footprint-metrics.test.mjs \
  apps/browser-demo/web/range-server.test.mjs
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --bin renderer-demo \
  appearance::gpu_tests
```

Commit the complete record-stage implementation and make sure the working tree
is clean. Generate the local GPU artifact from that exact `HEAD` before opening
the browser runner; the ignored producer fails closed for a dirty tree and
records its implementation commit, adapter, backend, operating system, exact
invocation, hard-circle fallback masks, nominal picks, quality matrix, and
resource measurements:

```bash
PUNCTRA_REQUIRE_GPU=1 \
PUNCTRA_POINT_FOOTPRINT_EVIDENCE_PATH=apps/browser-demo/web/fixtures/footprint-v1/local-test-evidence.json \
cargo test -p render-wgpu --test offscreen \
  write_point_footprint_test_evidence -- --ignored --exact
```

Then build and serve that record-stage implementation:

```bash
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
python3 scripts/serve-browser-demo.py --port 8000
```

Open `http://127.0.0.1:8000/footprint.html`, select **Record v0.22
baseline**, keep the page visible, and click **Run bounded qualification**.
The click must be both trusted and covered by active browser user activation;
console or synthetic invocation is ineligible. The canonical canvas is 320 by
240 CSS pixels at requested DPR 2 and 640 by 480 physical pixels. All nine
inherited trials run through three fresh viewers and 30 quiet foreground frames;
focused DPR and fallback trials run separately.

Download the single `v0.22-browser-point-footprint-evidence.tar`, inspect it,
and extract it into a fresh directory:

```bash
tar -tf /path/to/v0.22-browser-point-footprint-evidence.tar
mkdir -p target/v0.22-footprint-record
tar -xf /path/to/v0.22-browser-point-footprint-evidence.tar \
  -C target/v0.22-footprint-record
```

This first pass is calibration. Retain only the canonical/focused PNGs under
`apps/browser-demo/web/fixtures/footprint-v1/baselines/`. Discard its baseline
manifest, record-mode evidence, and other candidate/diagnostic artifacts; they
are bound to the pre-baseline commit and are not final evidence. The standard
browser Blob TAR is the primary
transport. If the in-app browser reports success but no TAR materializes, use a
fresh no-overwrite directory and repeat only that attended stage through the
verified same-origin fallback:

```bash
mkdir -p target/v0.22-footprint-record-export
python3 scripts/serve-browser-demo.py --port 8000 \
  --footprint-export-dir target/v0.22-footprint-record-export
```

Open `http://127.0.0.1:8000/footprint.html?transport=server`, select the same
mode, and use the visible trusted Run button. The fixed
`/qualification-footprint-export` endpoint accepts only the identical bounded
`application/x-tar` body and publishes
`v0.22-browser-point-footprint-evidence.tar` without replacement. Its
`punctra-browser-point-footprint-export-receipt-v1` response and the TAR remain
transport rather than evidence. Use a different fresh directory for a verify-
stage fallback. Do not reconstruct evidence from the console or screenshots.

Check in the retained candidate PNGs without changing any qualified
implementation path. That clean commit is the accepted implementation pin.
Record its exact inputs:

```bash
git rev-parse HEAD
wc -c < scripts/verify-browser-point-footprint.mjs
shasum -a 256 scripts/verify-browser-point-footprint.mjs
```

The baseline-image commit changes `HEAD`, so discard the calibration-stage
local GPU JSON and run the exact ignored producer command above again from the
clean accepted pin. Rebuild and run **Record v0.22 baseline** a second time.
Require every returned PNG to be byte-identical to its checked-in image and
retain only this second pass's uncommitted
`docs/releases/v0.22-browser-point-footprint-baseline.json`. That manifest pins
the accepted commit without trying to include itself in its own Git object. A
local test artifact or manifest bound to the pre-baseline commit is ineligible.

The accepted baseline pin binds the implementation commit and path digests,
verifier path/length/SHA-256, package version and runtime artifact digests,
Point-footprint corpus, and immutable v0.21 predecessor identities. Rebuild
that exact pin and repeat package/reference verification:

```bash
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
node scripts/generate-browser-sdk-reference.mjs --check
```

Repeat the attended packed quickstart and exact browser qualification using the
strict server. Create fresh observations at
`docs/releases/v0.22-browser-quickstart.json`,
`docs/releases/v0.22-browser-matrix.json`, and
`docs/releases/v0.22-browser-baseline.json`. The integration and qualification
verifier/test live pointers must already target those files before commit I.
Before running the qualification verifier, put P into the v0.22 release record
and CHANGELOG entry and put the exact qualification-verifier SHA-256 into the
release record. Then run:

```bash
node scripts/verify-browser-integration-baseline.mjs
node scripts/verify-browser-qualification.mjs
```

Do not copy v0.21 observation dates, environment facts, timings, resources,
artifact digests, verifier digests, or implementation commit. Until these
fresh files and live pointers exist and pass, the functional continuation is
pending.

Finally restart the strict server from the pinned rebuilt tree, open
`http://127.0.0.1:8000/footprint.html`, select **Verify pinned v0.22
baseline**, and click the visible trusted Run button. The server-provided
running implementation, verifier, runtime, corpus, and predecessor tuple must
match the accepted baseline tuple. Extract the final TAR into another fresh
directory and place only its recorded repository-relative verify evidence and
artifacts at:

```text
docs/releases/v0.22-browser-point-footprint-evidence.json
docs/releases/v0.22-browser-point-footprint-artifacts/
```

Only verify-mode evidence is eligible. Accept it only after:

```bash
node scripts/verify-browser-point-footprint.mjs \
  --baseline docs/releases/v0.22-browser-point-footprint-baseline.json \
  --evidence docs/releases/v0.22-browser-point-footprint-evidence.json
```

Fill the pending fields in `docs/releases/v0.22.0.md` only from that completed
lane, rerun the complete local command matrix above, and leave the v0.22 roadmap
item Active until every required result and artifact actually exists. See the
[browser Point-footprint guide](docs/guides/browser-point-footprint.md).

See the [browser streaming
guide](docs/guides/browser-streaming.md) and [browser viewer API
guide](docs/guides/browser-viewer.md), [browser SDK
guide](docs/guides/browser-sdk.md), [browser quickstart
guide](docs/guides/browser-quickstart.md), [known limitations](docs/guides/browser-known-limitations.md),
[browser qualification guide](docs/guides/browser-qualification.md), and
[browser visual-quality guide](docs/guides/browser-visual-quality.md), and
[browser Point-footprint guide](docs/guides/browser-point-footprint.md).

The default `point-index` benchmark generates one million Points. Use only the
documented scale values when a larger local run is intended, for example:

```bash
PUNCTRA_POINT_INDEX_BENCH_POINTS=10000000 cargo bench -p point-index --bench index
PUNCTRA_POINT_WORKSPACE_BENCH_POINTS=10000000 \
  cargo bench -p point-workspace --bench document
PUNCTRA_TERRAIN_BENCH_POINTS=100000 \
  cargo bench -p point-terrain --bench terrain
PUNCTRA_PERSISTENT_TERRAIN_EXAMPLE_POINTS=100000 \
  cargo run -p point-terrain --example persistent_surface
PUNCTRA_TERRAIN_WORKFLOW_BENCH_POINTS=100000 \
  cargo bench -p terrain-demo --bench journal -- \
  --save-baseline "qualification-$$-$(date +%s)"
PUNCTRA_RENDERER_VIEW_BENCH_POINTS=1000000 \
  cargo bench -p renderer-demo --bench viewing
```

The terrain benchmark accepts positive generated sizes through one million
Points; its intended scales are 10,000, 100,000, and 1,000,000. The default is
10,000. Its descriptor and QA byte facts are algorithm accounting. A null
`worker_heap_measurement` means that no observed worker-heap measurement is
claimed.

The persistent-Surface example accepts generated sizes from 3 through
1,000,000 Points and defaults to 10,000. It reports cold, verified-input resume,
warm-open, and bounded-stream facts. Direct stage bytes, worker heap, process
resident memory, allocated filesystem blocks, QA, LandXML, View, and field
accuracy remain explicit null observations rather than inferred values.

The exact-QA example uses a generated seeded defect and exercises the public
correct, re-derive, compare, recheck, and Revert composition. Its default
temporary artifacts are removed on exit. Set
`PUNCTRA_QA_EXAMPLE_OUTPUT_DIR` to an absent path to retain the generated
Workspace, JSON evidence, and SVG profile. Every SVG station carries a pointer
to exact JSON evidence; this is repository traceability, not field activation,
observed workflow timing, or independent adoption. `TerrainQaLimits` bound
Point rows, Source residuals, Check Points, profile stations, their combined
observation count, results, prepared-Surface materialization, face tests, and
combined work. Surface comparison has independent face, retained-record-byte,
working-byte, and work-unit ceilings.

The Workflow benchmark accepts exactly 10,000, 100,000, or 1,000,000 generated
Points. It measures cold start, committed-Edit resume, Retryable-intent resume,
LandXML/report reconciliation, and Complete revalidation. Its journal/report
bytes and semantic limit facts are deterministic generated evidence. It does
not measure worker peak heap or establish production, partner, downstream, or
human-workflow acceptance.

The qualification command saves each run under a unique baseline name because
these wall-time intervals intentionally include durable filesystem syncs and
an unknown prior workstation state is not a valid comparator. A fresh name
preserves current intervals and resource facts for inspection without loading
historical results or emitting unattributable `Performance has regressed`
labels. A cross-Revision performance claim instead requires a deliberate named
same-machine, same-target A/B/A run: save the base, compare the head to that
name, then rerun the unchanged base against the same name. If the base
self-check moves materially, do not attribute the head/base difference to
code.

The renderer viewing benchmark defaults to 100,000 generated Points and
measures a warm verified position-only index open plus the first bounded root
display batch. `PUNCTRA_RENDERER_VIEW_BENCH_POINTS` accepts a positive size
through ten million. It prints generated Point/node counts plus artifact and
observed index-temporary bytes. This is a local generated-fixture
microbenchmark, not a GPU frame benchmark, production-corpus result, first-use
promise, or professional workflow observation.

The exact review benchmark uses 20,000 generated Snapshot Points to measure the
public CPU screen-through path with both resident and forced-spill Point Set
construction. It prints the generated Source/match counts, declared composite,
row, Point Set, and retained-match working ceilings, plus the resident and
temporary Point Set ceilings for each disposition. It also records the
completed review's conservative algorithm-accounted working high-water and the
stable owned-fixture file-count/logical-file-length delta while a verified
resident or forced-spill Point Set remains alive. These are not measured heap,
allocated filesystem blocks, or process-wide disk observations. The benchmark
is not a GPU frame benchmark, production-corpus observation,
interaction-latency promise, or evidence that correction reduces attended time
or rework.

The v0.11 exact-review seam is intentionally narrower than general screen
selection. `point_review::screen_through` evaluates the center of every exact
Snapshot Point against one perspective or orthographic Camera, one physical
Viewport, and one inclusive top-left-origin continuous-pixel rectangle. Its
optional classification predicate uses the effective value at the pinned
Revision. GPU residency, splat coverage, transparency, and occlusion never
exclude an otherwise matching Point. `point_review::confirm_pick` accepts only
a provisional Point Identity after the host has validated its View generation;
it confirms exact ticks, world position, effective classification, and a
one-Point Point Set from the Snapshot.

Renderer highlights must come from a complete bounded Point Set identity read,
not from resident batches or Pick tokens. `RenderLimits` independently bounds
complete highlight-update input. Classification changes continue through
`CommitRequest::set_classification`; immediate undo is
`CommitRequest::revert_head`; Audit/Edit Footprint and uncertain-publication
reconciliation remain the existing `point-workspace` operations. Callers
record a nonzero Operation Identity before each commit or Revert and never
invent a replacement identity after an indeterminate result.

The public `third_party_host` example owns its wgpu instance, device, queue,
encoder, target, submission, and polling and imports no `renderer-demo` state.
It demonstrates a provisional Pick only; repository example execution is not
independent adoption or field evidence. Set `PUNCTRA_REQUIRE_GPU=1` for the
required local run so absence of the expected headless adapter fails.

Exercise the complete real-cloud process path without requiring a GPU:

```bash
cargo run -p source-las --example inspect -- path/to/source.laz
cargo run --release -p renderer-demo -- --smoke path/to/source.laz path/to/source.pidx
cargo run --release -p renderer-demo -- \
  --smoke --display classification path/to/source.laz \
  path/to/source.inspection-v2.pidx
cargo run --release -p point-workspace --example classify -- \
  path/to/source.laz path/to/source.pidx path/to/workspace.pcw \
  6
cargo run --release -p terrain-demo -- start \
  --run-id "$RUN_ID_HEX" --operation-id "$OPERATION_ID_HEX" \
  --baseline "$BASELINE_REVISION_HEX" --exclude-ground-ordinal 4 \
  --date 2026-08-10 --time 00:00:00Z \
  path/to/source.laz path/to/source.pidx path/to/workspace.pcw path/to/run-root
cargo run --release -p terrain-demo -- inspect path/to/run-root
cargo run --release -p terrain-demo -- verify-round-trip \
  --downstream-app "$DECLARED_APP" --downstream-version "$DECLARED_VERSION" \
  --downstream-setting "profile=$DECLARED_SETTINGS" \
  --horizontal-tolerance-metres 0.001 --vertical-tolerance-metres 0.001 \
  path/to/run-root path/to/returned.xml path/to/round-trip-evidence.json
```

Neutral and elevation use the position-only disk-v1 index recipe. RGB,
intensity, and classification use the disk-v2 inspection recipe and can share
one v2 target. Do not point both recipe families at one path: incompatible
complete/work targets are preserved and rejected. RGB requires all three LAS
`U16` channels. The [five-minute first LAS/LAZ
guide](docs/guides/first-las-laz.md) documents display mappings, projection
controls, loading/Coverage truth, and recovery behavior.

The exact private mapping contract is: neutral `[190,205,220,255]`; elevation
normalizes complete Source world Z, clamps to `[0,1]`, uses `0.5` for zero
extent, and interpolates the five stops recorded in the [v0.10
design](docs/design/field-inspection-view-v0.10.md#implemented-display-mappings-and-cli).
Each RGB/intensity `U16` value `v` becomes `(v * 255 + 32767) / 65535` by
integer division. Classification 0–18 uses the checked-in fixed table; 19–255
uses wrapping `u8` `(73c+41, 151c+97, 199c+17)`. Alpha is always 255. These
bytes are presentation only.

The local GPU corpus runner accepts only bounded manifests whose entries state
both inspection and measurement permission:

```bash
cargo run --release -p renderer-demo -- corpus \
  --manifest path/to/private-field-corpus.json \
  --report path/to/new-private-viewing-report.json
```

Start from the [example manifest](docs/guides/field-corpus.example.json), use
opaque identifiers, replace every placeholder, and give each entry a fresh
absent index target. The runner requires a genuine cold build and then records
a separate immediate warm open; existing or resumable indexes are preserved
and rejected. A report target is
published without replacement; use a new target for a new timed run. Reports
are viewing measurements with explicit nonclaims, not field qualification,
terrain capacity, partner acceptance, downstream support, or human-time
evidence. Do not publish Sources, manifests, reports, screenshots, or benchmark
facts without the required permission.

`RUN_ID_HEX` and `OPERATION_ID_HEX` are caller-owned nonzero 32-character hex
identities; `BASELINE_REVISION_HEX` is the exact expected 64-character
Workspace head identity. Create the Workspace separately through
`point-workspace`; its classification example demonstrates setup and prints
Revision identities. Its selected `U8` Attribute must be Source Attribute 6,
the `source-las` classification column. `terrain-demo` opens but never creates
it, and an absent Workspace is `PWF_INVALID_REQUEST` before Run creation or
Workspace mutation. Retain the current head identity, then drop all
Workspace/Snapshot/PointSet handles before starting so the app can acquire the
exclusive Workspace lock.

`verify-round-trip` is a strictly read-only consumer of a Complete Run and
requires the existing `run.lock`; it never creates or repairs Run files. Its
evidence target must have an existing parent outside the Run root. Caller
labels do not prove application execution, vendor support, partner acceptance,
paid use, conversion, or labor savings.

The verifier streams the exact captured bytes through a bounded local XML
scanner/parser and uses borrowed `quick-xml` token views; it accepts
at most 4 GiB per input, 10,000,000 Points, and 20,000,000 faces. The additional
defaults are 70,000,128 XML nodes, 4 GiB of XML text/attribute bytes, a 4-KiB
lexical token, 8 MiB of accounted parser working bytes per input, 32,000,000
candidate comparisons, and 4 GiB of accounted retained working bytes across
both inputs and comparison. Evidence reports deterministic algorithm charges,
not allocator metadata/slack, process RSS, or measured heap. Oversized tokens,
changed/extended captured files, namespace-stack growth, and every retained
collection projection fail before the corresponding algorithm-accounted
reserve is requested.
`path/to/run-root` must already be a directory. Resume repeats the identical
command and request with `resume` in place of `start`. At least one repeated
`--exclude-ground-ordinal ORDINAL` is required, and every listed ordinal must
be in the baseline class-2 Ground Input. Optional detached observations use
repeated `--check-point ID,X,Y,Z` arguments.

`terrain-demo` requires the complete supported structured metre/metre profile
and performs no transformation. Unknown, opaque WKT, and unsupported profiles
fail closed. The frozen legacy assertion field remains readable only by the
private legacy reconciliation verifier; no current CLI or public writer can
set it.

The durable v0.7 command replaces the v0.6 one-shot
`--exercise-correction-revert` grammar. It still validates the correction
ordinals against the baseline class-2 Ground Input, commits the requested
non-Ground classification, and derives both baseline and changed Surfaces.
The retained v0.6 regressions prove exact immediate-head Revert restoration of
geometry, topology, vertices, and faces and keep the Source bytes unchanged.
The v0.7 Workflow does not automatically Revert a committed classification
Revision when a later Derivation, QA, export, or report phase fails.

GPU acceptance tests and the public renderer example use any available
headless wgpu adapter. Without `PUNCTRA_REQUIRE_GPU=1`, GPU tests may skip and
the example may exit without exercising rendering when no adapter is present.
Required local qualification sets `PUNCTRA_REQUIRE_GPU=1`, making a missing
adapter fail. Run all verification locally; the repository does not use hosted
CI.

The stable fuzz-crate tests run the checked-in short index corpus and generated
valid terrain artifact/work seeds through the same bounded harnesses as
libFuzzer. Longer local campaigns may use `cargo-fuzz` and a nightly toolchain:

```bash
cargo +nightly fuzz run index_persistence fuzz/corpus/index_persistence -- \
    -max_len=262144 -timeout=5
cargo +nightly fuzz run terrain_persistence -- \
    -max_len=262144 -timeout=5
```

Keep public behavior documented, add interface-level tests for changes, avoid
unsafe code, and preserve caller-owned wgpu submission.
