# Browser qualification and recovery

Punctra `0.22.0-alpha.1` carries forward the contract for one exact local
browser/device lane over the fixed repository workload and packed quickstart.
The latest completed machine-readable [browser
matrix](../releases/v0.21-browser-matrix.json) is immutable v0.21 predecessor
evidence: a platform absent from its `qualified_entries` is unqualified even
when the SDK initializes successfully. A v0.22 matrix becomes authoritative
only after the exact pinned rebuild is run and its fresh observations pass.

The immutable [v0.20 browser
matrix](../releases/v0.20-browser-matrix.json) remains the immutable historical
authority for the earlier predecessor package. v0.21 repeated that bounded
functional qualification without converting the earlier evidence to a moving
target; v0.22 must do the same rather than copying either predecessor record.

This is deliberately narrower than a browser-support promise. The most recent
completed lane is the Codex in-app browser reporting Chromium 151 on macOS
26.6.2 build 25G83,
arm64, and the local Apple M5 Pro machine. The browser exposed a generic WebGPU
adapter name,
so the physical-GPU mapping is recorded as a local-system inference rather than
a browser-reported fact. Installed Google Chrome and Safari were not controlled
by the available browser surface and remain unqualified.

## Reproduce the lane

Run the repository checks and build the packed SDK:

```bash
node --test apps/browser-demo/web/*.test.mjs packages/react/*.test.mjs scripts/*.test.mjs
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
node scripts/verify-browser-integration-baseline.mjs
node scripts/verify-browser-qualification.mjs
```

Then start the strict server:

```bash
scripts/serve-browser-demo.py --port 8000
```

Open `http://127.0.0.1:8000/` in the browser/device entry being qualified. A
generic static server is insufficient: real `206` Range responses, strong
validators, identity content encoding, CORS-exposed headers, bounded disconnect
and delay faults, and immutable cache behavior are part of the contract.

The page passes only after it exercises:

- package creation, bounded invalid-resize rejection, DPR change/restore,
  hidden-frame skip, resume, disposal, and recreation;
- a deliberate Worker crash and a disconnected manifest request before Source
  publication, both retaining the existing viewer and generation;
- explicit cancellation within 1,000 milliseconds;
- cold verified Range delivery, viewer/Worker recreation, and an identity-
  matched persistent-cache run with zero binary requests;
- five display modes, perspective and orthographic cameras, normalized pointer,
  touch, wheel, and keyboard input, provisional pick/highlight, exact immutable-
  LAS confirmation, cancelled confirmation, and stale-generation rejection;
- 30 settled foreground frames plus first-Coverage, settled-View, main-thread,
  network, worker, cache, logical renderer, canvas, and optional heap evidence;
  and
- every fixed ceiling inherited from the accepted v0.20 design and current
  fresh v0.22 functional baseline.

Record the raw `punctra-browser-qualification-v1` artifact before changing the
browser, viewport, display, adapter, operating system, package, fixture, server,
or cache. A rerun after any such change is a different observation.

## Interpret measurements truthfully

`firstCoverageMilliseconds` ends after the first validated remote batch is
published and a sampled frame is submitted. `settledViewMilliseconds` ends
after the final bounded root-sample batch and settled frame submission. Neither
is GPU-completion time.

Foreground callback intervals measure browser scheduling. Submission samples
measure synchronous viewer work on the main thread. Canvas, decoded records,
transferred records, renderer vertices, transient textures, and verified cache
bodies are logical/accounting facts. `performance.memory.usedJSHeapSize` is
recorded only where the browser exposes that non-standard API. Process RSS,
physical cache allocation, and physical GPU/driver allocation remain `null`.

The generous latency ceilings protect this fixed loopback workload from gross
regression. They do not promise remote-network or production performance.

## Host recovery playbook

| Failure | Host action |
|---|---|
| `resize_viewport` | Keep the prior surface, correct CSS size/DPR, resize again, then request a frame. |
| `offline`, `retry_exhausted` before publication | Keep the viewer and last frame; retry only after connectivity or server recovery. |
| `worker_failed` before publication | Keep the viewer and last frame; start a new load, which creates a new Worker. |
| `cache_quota`, `cache_unavailable` | Explicitly retry with `memory` or `none`, or clear only the caller-owned cache namespace. |
| `cancelled` before publication | Keep the viewer; begin a new operation only when the host still wants the Source. |
| Failure after partial publication | Dispose the fused viewer and create a new one before any Source load. |
| `device_lost`, `surface_lost`, or another fused renderer failure | Dispose idempotently and recreate the viewer/device explicitly. |
| `stale_generation` | Drop the late pick/query/presentation result; never rewrite it to the current generation. |
| Hidden/background document | Pause, retain bounded state, and resume plus request a frame when visible. |

Memory pressure has no portable accepted signal. Enforce the independent
ceilings and let the Browser Host choose cache eviction, viewer disposal, or a
smaller application workload. Do not infer headroom from one heap observation.

## Unsupported and unqualified states

An insecure context, missing WebGPU, missing adapter, unsupported surface
format/presentation/alpha mode, or renderer capability failure is
**unsupported**. Initialization returns no viewer and the host should explain
the missing requirement.

A browser that initializes but is absent from the matrix is **unqualified**.
Hosts may run an attended qualification trial, but must not label the platform
supported from initialization alone. There is no WebGL, Canvas, software, or
reduced-feature fallback.

## Issue evidence template

Include only:

```text
Punctra package:
Qualification matrix entry (or "unqualified"):
Browser and exact version:
Operating system and exact version:
WebGPU adapter/backend/surface facts:
Viewport and DPR:
Deployment profile and Source identity (no signed URL):
Expected result:
Actual result and structured error code:
Safe action attempted:
Minimal reproduction steps:
Permission to share screenshot or Source facts: yes/no
```

Attach the bounded qualification artifact when permitted. Remove access tokens,
cookies, authorization headers, signed URL query strings, private Source bytes,
precise customer locations, proprietary filenames, and unrelated browser or
system logs. The harness itself uploads nothing and reads no browser profile,
cookies, credentials, or unrelated storage.

## v0.22 point-footprint evidence is a separate lane

The active v0.22 lane requests anti-aliased Point footprints, records the
selected `multisample4x`, `unsupported_fallback`, or `resource_fallback` path,
and keeps nominal pick diameter at 7.0 physical pixels while display diameter
is clamped independently to 2.0 through 6.0 physical pixels. Its canonical
trials reuse the immutable v0.21 corpus and predecessor images; focused trials
exercise requested DPR 1, 2, and 4, capability/resource fallbacks, pick
identity, exact transient accounting, and frame-cost ceilings.

Passing the functional matrix does not manufacture those observations. Follow
the sequential record, pin, rebuild, and verify workflow in the
[point-footprint guide](browser-point-footprint.md). The v0.22 footprint
baseline, evidence, functional records, and release acceptance are pending
until that workflow and the static verifier pass.

## v0.21 visual evidence is a separate predecessor lane

The accepted v0.21 visual baseline adds nine fixed private trials, exact 640 by
480 physical capture at requested DPR 2, 30 quiet frames, three complete
viewer/harness recreations per trial, tolerant/temporal image
comparison, and separate Coverage, feature, authority, resource, and rubric
facts. Passing the functional matrix does not manufacture those observations.
Final visual acceptance first records and checks in canonical baseline inputs,
then pins and rebuilds the implementation, repeats this functional lane, and
only afterward runs the attended verify stage. Rubric review follows capture;
one private TAR transports repository-relative artifacts. Standard Blob
download is primary; the explicitly enabled same-origin local-server export is
only a no-overwrite fallback for a download that does not materialize. Follow
the [visual-quality guide](browser-visual-quality.md). Its attended verify-mode
evidence and release pins are complete for this exact lane; they do not qualify
another lane or establish a physical-display claim.

## Remaining evidence

v0.22 does not qualify installed Chrome, Safari, another operating system/GPU,
mobile, registry/CDN deployment, authentication, offline-first behavior,
independent adoption, API stability, compositor/display presentation,
cross-browser visual equivalence, independent human interpretation, improved
or final visual quality, or support operations. Those remain external evidence
or later accepted work; this release is not a beta, v1, or release candidate.

The [v0.20 repository verification record](../releases/v0.20.0.md) pins the
exact predecessor implementation commit and historical observations. The
[v0.21 repository verification record](../releases/v0.21.0.md) pins the
immutable predecessor implementation, environment, functional and visual
observations, verifier identities, artifacts, and outstanding exits. The
[v0.22 release record](../releases/v0.22.0.md) deliberately remains pending
until its own complete evidence exists.
