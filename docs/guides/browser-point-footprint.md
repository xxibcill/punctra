# Browser Point-footprint qualification

Status: **v0.22 implementation active; final attended evidence pending**

Punctra `0.22.0-alpha.1` implements the bounded [Point Footprint and Edge
Quality design](../design/point-footprint-edge-quality-v0.22.md). This is a
private repository qualification lane, not a supported screenshot API or a
physical-display, cross-browser, independent-human, or adopter claim.

The lane reuses the immutable v0.21 Visual Corpus, cameras, display mappings,
and canonical PNGs as predecessor evidence. It writes only v0.22 paths. Never
replace or relabel a file under `fixtures/visual-v1` or a `v0.21-*` release
record while reproducing this workflow.

## Renderer and host contract

The browser and native renderer demo request `PointFootprint::Antialiased`.
`render-wgpu` selects one status for a validated viewport:

- `multisample4x` means the requested deterministic four-sample circular color
  coverage is active;
- `unsupported_fallback` means a required color/depth multisample capability is
  unavailable;
- `resource_fallback` means the physical viewport exceeds 1,310,720 pixels; and
- `single_sample` means single-sample was explicitly requested, not that a
  fallback occurred.

Both fallback statuses use the complete inherited single-sample path. They do
not move Point centers, change geometry or colors, reorder depth, or change
Point identity.

The browser computes one display diameter per frame:

```text
clamp(
  sqrt(physical viewport pixels / max(non-retired resident Points, 1)) * 0.55,
  2.0,
  6.0
) physical pixels
```

Replacement Points count even while their presentation weight is changing.
The nominal pick diameter remains exactly `7.0` physical pixels and uses its
own hard circular mask. Display sizing is decorative and cannot enlarge,
shrink, or authorize a pick.

Diagnostics and capture facts expose exactly this nested object:

```json
{
  "point_footprint": {
    "requested": "antialiased",
    "selected": "multisample4x",
    "nominal_pick_size_physical_pixels": 7.0,
    "display_size_physical_pixels": 4.25
  }
}
```

`selected` may instead be `unsupported_fallback` or `resource_fallback` for
the browser request. Evidence must report the actual renderer selection; a
test-forced condition is separate test provenance, never renderer truth.

## Resource and measurement boundary

The preferred non-EDL color/depth attachments use 32 transient bytes per
physical pixel. Retained single-sample pick color and depth raise that to 40
bytes per pixel. EDL may add resolved color plus visibility depth, producing a
48-byte-per-pixel high-water mark. At 640 by 480, the complete preferred EDL
plus pick set is therefore at most 14,745,600 bytes. The resource fallback uses
the unenhanced inherited hard-circle path even when EDL was enabled at renderer
construction, and uses only the single-sample depth and pick targets, at most 8
bytes per pixel. `FrameReport::eye_dome_lighting_applied()` reports that
per-frame suppression explicitly;
the renderer-wide transient ceiling remains 67,108,864 bytes.

These are exact texture-accounting facts. Canvas bytes, resident Point buffers,
capture texture, readback staging, decoded image, PNG, Worker staging, cache,
JavaScript heap, driver allocation, and physical GPU memory remain separate.
Frame intervals and synchronous submission durations are not physical GPU
completion time.

## Static and GPU checks

Run the repository-wide sequence in [CONTRIBUTING.md](../../CONTRIBUTING.md).
The focused v0.22 checks are:

```bash
cargo test -p render-wgpu --test contracts
cargo test -p browser-demo
cargo test -p renderer-demo --bin renderer-demo appearance::tests
node --test \
  apps/browser-demo/web/footprint-corpus.test.mjs \
  apps/browser-demo/web/footprint-evidence.test.mjs \
  apps/browser-demo/web/footprint-export.test.mjs \
  apps/browser-demo/web/footprint-runner-core.test.mjs \
  apps/browser-demo/web/visual-footprint-metrics.test.mjs \
  apps/browser-demo/web/range-server.test.mjs
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --bin renderer-demo \
  appearance::gpu_tests
```

A missing adapter must fail the required GPU commands; do not omit
`PUNCTRA_REQUIRE_GPU=1` when a local adapter is expected.

Commit the record-stage implementation and start from a clean working tree.
The browser runner requires a local artifact bound to that exact `HEAD`:

```bash
PUNCTRA_REQUIRE_GPU=1 \
PUNCTRA_POINT_FOOTPRINT_EVIDENCE_PATH=apps/browser-demo/web/fixtures/footprint-v1/local-test-evidence.json \
cargo test -p render-wgpu --test offscreen \
  write_point_footprint_test_evidence -- --ignored --exact
```

The ignored producer fails on a dirty tree and records its own command, commit,
local operating system, adapter, backend, fallback masks and picks, quality
matrix, and transient-resource measurements. It is not browser evidence.

## 1. Record candidate baselines

Build the current package and start the strict local server:

```bash
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
python3 scripts/serve-browser-demo.py --port 8000
```

Open `http://127.0.0.1:8000/footprint.html`, select **Record v0.22 baseline**,
keep the page visible, and click **Run bounded qualification**. The click must
have both a trusted event and active browser user activation. Do not invoke the
runner from the console or synthesize the event.

The record run exercises all nine inherited trials through three fresh viewers
and 30 quiet foreground frames, plus the focused DPR and fallback cases. A
passing record-stage TAR is `v0.22-browser-point-footprint-evidence.tar`.
Standard Blob download is primary. If the in-app browser reports success but
does not materialize the TAR, create a fresh export directory, restart the
server with the existing bounded export opt-in, and repeat only that attended
stage:

```bash
mkdir -p target/v0.22-footprint-record-export
python3 scripts/serve-browser-demo.py --port 8000 \
  --footprint-export-dir target/v0.22-footprint-record-export
```

Open `http://127.0.0.1:8000/footprint.html?transport=server`, select record
mode, and use the same visible trusted Run button. The page POSTs the identical
bounded `application/x-tar` archive to same-origin
`/qualification-footprint-export`. Publication is no-replace. The
`punctra-browser-point-footprint-export-receipt-v1` response and TAR remain
transport, not release evidence. Use another fresh directory if final verify
transport later needs the same fallback.

Inspect and extract it into a fresh directory:

```bash
tar -tf /path/to/v0.22-browser-point-footprint-evidence.tar
mkdir -p target/v0.22-footprint-record
tar -xf /path/to/v0.22-browser-point-footprint-evidence.tar \
  -C target/v0.22-footprint-record
```

The first record pass is calibration. Retain only its canonical and focused
baseline PNGs under
`apps/browser-demo/web/fixtures/footprint-v1/baselines/`; discard its manifest,
record-mode evidence, and diagnostic/candidate artifacts. Check in those PNGs
without changing any qualified implementation path. That new clean commit is
the accepted implementation pin.

## 2. Pin, rebuild, and repeat functional qualification

The accepted baseline pin must bind the implementation commit, every declared
implementation-path digest, verifier path/length/SHA-256, package version and
runtime artifact digests, Point-footprint corpus, and immutable v0.21
predecessor identities. Record the verifier identity locally:

```bash
git rev-parse HEAD
wc -c < scripts/verify-browser-point-footprint.mjs
shasum -a 256 scripts/verify-browser-point-footprint.mjs
```

The calibration artifact is bound to the earlier commit and is now ineligible.
Regenerate the local GPU JSON from the clean accepted pin with the ignored
producer command above, rebuild, and run **Record v0.22 baseline** a second
time. Every returned PNG must be byte-identical to the checked-in image at the
accepted pin. Retain only this second pass's uncommitted
`docs/releases/v0.22-browser-point-footprint-baseline.json`; its pin names the
accepted commit without attempting to include the self-referential manifest in
that commit.

Rebuild the exact pinned implementation, then regenerate the API reference and
repeat package plus functional browser qualification:

```bash
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
node scripts/generate-browser-sdk-reference.mjs --check
node scripts/verify-browser-integration-baseline.mjs
node scripts/verify-browser-qualification.mjs
```

This repetition must produce fresh v0.22 functional records at:

- `docs/releases/v0.22-browser-quickstart.json`;
- `docs/releases/v0.22-browser-matrix.json`; and
- `docs/releases/v0.22-browser-baseline.json`.

Their package/artifact digests, implementation commit, verifier identity,
environment, timings, and resource observations must come from this run. Do not
copy v0.21 values. Until these files exist and their verifiers point to them,
the v0.22 functional continuation is pending.

## 3. Verify the pinned baseline

Restart the strict server from the pinned rebuilt tree, open
`http://127.0.0.1:8000/footprint.html`, select **Verify pinned v0.22
baseline**, and use the visible trusted Run button. The server-provided running
implementation, verifier, runtime, corpus, and predecessor tuple must equal the
accepted baseline tuple before verify mode may run.

The final TAR must contain verify-mode evidence at
`docs/releases/v0.22-browser-point-footprint-evidence.json` and its recorded
PNGs under `docs/releases/v0.22-browser-point-footprint-artifacts/`. Extract it
into a fresh directory and copy only the repository-relative paths recorded by
the archive. Do not substitute screenshots, record-mode evidence, or console
reconstruction.

Run the final static verifier with both explicit inputs:

```bash
node scripts/verify-browser-point-footprint.mjs \
  --baseline docs/releases/v0.22-browser-point-footprint-baseline.json \
  --evidence docs/releases/v0.22-browser-point-footprint-evidence.json
```

The verifier binds pins, environment, corpus/predecessor/runtime identities,
canonical and focused images, edge/density metrics, feature preservation,
fallback and nominal-pick behavior, frame costs, independent resource bounds,
artifact digests, and closed nonclaims.

## Acceptance status

The record, pin, rebuilt functional records, verify evidence, verifier result,
and [v0.22 release record](../releases/v0.22.0.md) are one sequential gate. At
the time of this documentation update, the final attended observations are
pending. Do not mark the roadmap item Complete or replace a pending release-
record field until every stage above has actually passed.
