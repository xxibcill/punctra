# Browser LOD continuity qualification

The private v0.23 runner measures authored parent/child cut fixtures through
the actual Wasm/WebGPU renderer. It does not add a child-LOD loader to the
public SDK. The separate native `ViewLifecycle` GPU test exercises the real
planner and materializer.

The [accepted design](../design/lod-density-transition-continuity-v0.23.md)
fixes image, timing, identity and resource gates before execution. Transitions
select complementary whole physical pixels for eight presented steps. Visible
Source colors remain opaque; hidden Points retain their GPU byte charges and
nominal pick identities until retirement. The inherited color-only
presentation weight remains a separate contract.

## Record at a clean implementation

Run locally with an available GPU adapter:

```bash
PUNCTRA_REQUIRE_GPU=1 cargo test -p render-wgpu --test offscreen
PUNCTRA_REQUIRE_GPU=1 cargo test -p renderer-demo --bin renderer-demo appearance::gpu_tests
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
node scripts/stage-browser-lod-predecessor.mjs
```

Commit the qualified implementation, including the runner, corpus and verifier,
before collecting observations. The server binds every tracked qualified file,
the packed viewer, loaded JS/Wasm, corpus and immutable v0.22 predecessors.
The runner accepts two explicitly declared observed screen profiles for this
local browser/OS/adapter, and rejects all other or mixed profiles. Each capture
qualifies only its actual profile. Start and end screen/host facts must match;
record, verify, main functional evidence and packed quickstart must bind the
same profile and implementation. A historical screen observation does not
describe a later session.
Stage the exact frozen v0.22 packed runtime as described in CONTRIBUTING before
starting the server. Every canonical recreation runs that frozen control and
the current renderer in the same session, checks both against the immutable
canonical pixels, and compares raw capture-free timing samples. Historical
submission costs remain a separate two-times gate; historical callback cadence
is reported as a different-session observation.

```bash
mkdir -p target/lod-record-export
scripts/serve-browser-demo.py --port 8000 --lod-export-dir target/lod-record-export
```

Open `http://127.0.0.1:8000/lod.html?transport=server`, select **Record**, and
click **Run bounded qualification**. Keep the page visible. An agent-operated
trusted click is recorded as agent-operated; it is not an independent human
interpretation trial.

Also run the packed quickstart and main functional acceptance page at that same
implementation. After functional acceptance passes, open **Raw bounded
diagnostics** and choose **Export acceptance JSON**. The configured local server
persists one bounded `v0.23-browser-functional-observation.json` without
replacement. Preserve that raw observation alongside its derived matrix and
integration baseline. A receipt or a prior release's pass is not a current run.

The closed run recreates six transitions three times at DPR 1, 2 and 4. Each
candidate has outgoing/incoming endpoint captures at its exact current camera
and display diameter. It also recreates all nine canonical images, resource
fallback and invalid raw controls, including old callbacks after Source change.

Only a completed passing run publishes a TAR. Extract it into a fresh staging
directory. Copy its baseline JSON and **record** artifacts into their recorded
repository-relative paths. Audit the record before accepting it:

```bash
node scripts/verify-browser-lod-continuity.mjs --record-only
```

## Verify independently

Restart the server with a different empty export directory and unchanged
qualified implementation. Select **Verify** and run the whole corpus again.
Stage its verify JSON and **verify** artifacts at their recorded paths, then:

```bash
node scripts/verify-browser-lod-continuity.mjs
```

The offline verifier reads pinned Git objects, exact local runtime bytes and
lossless PNGs. It reconstructs authored Sources and camera/pick inputs,
recomputes metrics and timing summaries, checks every artifact role, hidden
residency and cleanup, and compares all verify captures with the baseline.
Missing, duplicated, substituted or changed inputs fail.

Capture/readback time is separate from the capture-free frame window. Declared
canonical byte bounds and renderer texture charges are algorithm accounting;
null heap and driver-memory fields do not imply observed memory consumption.

Run the complete applicable local matrix in [CONTRIBUTING](../../CONTRIBUTING.md)
before closure. Validate frozen predecessors in checkouts at their recorded
implementation/closure commits with their corresponding packed runtime.
Historical evidence cannot qualify a changed v0.23 checkout.

Physical-display presentation, additional browsers/devices, independent
interpretation, adoption and support qualification remain separate evidence.
