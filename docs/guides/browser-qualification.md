# Browser qualification and recovery

Punctra `0.19.0-alpha.1` qualifies one exact local browser/device lane for the
fixed repository workload. The machine-readable [browser matrix](../releases/v0.19-browser-matrix.json)
is the support authority: a platform absent from its `qualified_entries` is
unqualified even when the SDK initializes successfully.

This is deliberately narrower than a browser-support promise. The v0.19 lane is
the Codex in-app browser reporting Chromium 151 on macOS 26.5.2/arm64 and the
local Apple M5 Pro machine. The browser exposed a generic WebGPU adapter name,
so the physical-GPU mapping is recorded as a local-system inference rather than
a browser-reported fact. Installed Google Chrome and Safari were not controlled
by the available browser surface and remain unqualified.

## Reproduce the lane

Run the repository checks and build the packed SDK:

```bash
node --test apps/browser-demo/web/*.test.mjs packages/react/*.test.mjs scripts/*.test.mjs
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
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
- every fixed ceiling in the accepted v0.19 design.

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

## Remaining evidence

v0.19 does not qualify installed Chrome, Safari, another operating system/GPU,
mobile, registry/CDN deployment, authentication, offline-first behavior,
independent adoption, API stability, visual quality, or support operations.
Those remain external evidence or later accepted work; this release is not a
beta or release candidate.
