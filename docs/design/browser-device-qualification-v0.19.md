# Browser and Device Qualification Design (v0.19)

Status: **Complete and repository-verified for the bounded local qualification
slice**

This design is authoritative for the bounded Punctra v0.19 repository slice.
The maintainer's request to continue through v0.19 activates the scope below.
The selected lane is intentionally limited to the browser, operating system,
adapter class, packed SDK, immutable deployment, and local Range server that
can be executed on the qualification machine. Repository completion may qualify
that exact lane; it cannot manufacture independent adoption or support for an
untested platform.

## Outcome

Punctra v0.19 defines and reproduces the functional, latency, resource, and
recovery envelope of the v0.18 browser SDK on one declared local browser/device
lane before visual-quality work begins.

The packed `@punctra/viewer` host records first sampled Coverage, settled View,
foreground frame cadence and main-thread submission time, HTTP Range traffic,
worker staging, decoded retention, logical cache bytes, canvas bytes, renderer
bytes, optional JavaScript heap observations, and every capability fact the
browser and WebGPU adapter expose truthfully. The same acceptance path exercises
bounded resize/DPR change, hidden/resumed rendering, pre-publication failure and
retry, worker failure, explicit cancellation, warm-cache recreation, Source
generation replacement, and recreation-required failure behavior.

## Evidence boundary

Repository completion may prove:

- one exact Chromium/macOS/arm64/Apple-integrated-GPU/Metal lane passes the
  packed SDK functional suite through the strict local Range server;
- the checked-in 70,000-Point immutable LAS deployment reaches 4,096-Point
  Sampled Coverage inside explicit latency and resource ceilings;
- the host can distinguish retry-in-place from recreation-required failures;
- resize, DPR, visibility, worker, network, cancellation, cache, device-loss,
  and stale-generation paths preserve the last safe state or destroy it before
  unsafe continuation; and
- unsupported initialization fails before a usable viewer is returned.

It does not prove:

- another Chrome build, Safari, Firefox, Edge, Windows, Linux, Android, iOS,
  another WebGPU backend, discrete GPUs, software adapters, or external displays;
- registry/CDN publication, authentication, service workers, offline-first
  operation, hostile-server integrity, physical Cache allocation, driver/GPU
  allocation, process RSS, energy use, thermal stability, or memory-pressure
  notification behavior;
- arbitrary LAS/LAZ, COPC, EPT, 3D Tiles, hierarchy traversal, multiple Sources,
  complete Source Coverage, general exact Queries, editing, or terrain; or
- independent adoption, a stable API, support qualification, visual-quality
  expansion, beta status, or a browser-engine release-candidate promise.

## Declared qualification matrix

The release record pins the exact observed build values. The accepted lane is:

| Dimension | Declared entry |
|---|---|
| Browser | Codex in-app browser with Chromium 151.0.0.0 user agent |
| Operating system | macOS 26.5.2, arm64 |
| Device class | Apple silicon laptop, integrated Apple GPU |
| Adapter/backend | Browser WebGPU over Metal; exact exposed facts recorded |
| Display | caller-owned canvas on the built-in Retina display |
| Delivery | strict loopback HTTP server with real `206` Range responses |
| Package | locally packed `@punctra/viewer` and `@punctra/react` `0.19.0-alpha.1` |
| Workload | checked-in immutable 70,000-Point LAS deployment and disk-v2 root sample |

Every other matrix entry is **unqualified**, not necessarily blocked. The SDK
still performs the existing secure-context, WebGPU, adapter, surface, and
renderer capability checks. A missing required capability is **unsupported**
and returns no viewer; a platform that passes initialization but is absent from
the matrix remains unqualified and carries no v0.19 support claim.

There is no Canvas, WebGL, software, reduced-feature, or visual fallback.

## Measurement contract

The public Source-load result adds one immutable `timings` record:

- `firstCoverageMilliseconds`: monotonic time from the load request until the
  first validated remote batch is published and a Sampled-Coverage frame is
  submitted;
- `settledViewMilliseconds`: monotonic time from the load request until the
  final bounded root-sample batch is published, stream completion is accepted,
  and the settled frame is submitted; and
- `mainThreadBatchMillisecondsHighWater`: the largest observed main-thread
  decode-transfer publication and frame-submission task.

The existing `mainThreadMillisecondsHighWater` spelling remains as a deprecated
compatibility alias during this pre-1 release. These are browser main-thread
observations, not GPU completion time.

After the View settles, the private qualification runner samples 30 foreground
animation frames and records p50/p95/max callback interval plus p50/p95/max
synchronous viewer-submission time. The interval is scheduling evidence, not a
promise that the display panel presented every frame or that the GPU completed
within that interval.

Where `performance.memory.usedJSHeapSize` is available, the runner records
before/after/high-water observations with the API name and its non-standard
status. Otherwise each JavaScript-heap value is explicit `null`. Process RSS,
physical cache allocation, and physical GPU/driver allocation are always
`null`; logical retained bytes remain separately named.

## Fixed workload ceilings

The exact local lane must satisfy all of these gates:

| Measurement | Ceiling |
|---|---:|
| First sampled Coverage | 10,000 ms |
| Settled bounded View | 15,000 ms |
| Foreground animation-frame interval p95 | 50 ms |
| Main-thread frame submission p95 | 16.7 ms |
| Explicit cancellation acknowledgement | 1,000 ms |
| Physical canvas dimension | 4,096 px per axis |
| Physical canvas area | 8,388,608 px |
| Resident display Points | 8,192 |
| Renderer resident logical bytes | 192 KiB |
| Canvas surface accounting | 32 MiB |
| Renderer transient textures | 64 MiB |
| Retained decoded records | 256 KiB |
| Worker decoded staging | 320 KiB |
| Concurrent response bytes | 256 KiB |
| Persistent verified cache bodies | 4 MiB |

The runner additionally requires exactly 4,096 displayed Points, four transfer
batches, 131,072 retained transfer-v2 bytes, 98,304 renderer vertex bytes, and
zero binary network requests for the identity-matched warm-cache recreation.
These exact accounting facts detect behavior drift; they are not physical
memory measurements.

The latency ceilings are a generous regression envelope for the fixed loopback
workload, not a general performance promise. A result outside a ceiling fails
the lane instead of being averaged away. The release record reports observed
values without tightening the contract around one fast sample.

## Recovery contract

Recovery remains an explicit Browser Host decision. The SDK never recreates a
device, viewer, worker, Source, or cache operation implicitly.

| Condition | Required safe outcome |
|---|---|
| Invalid resize or DPR | Reject atomically; retain the prior surface and accept a later valid resize. |
| Hidden/background document | Cancel scheduled presentation, skip hidden frames, retain bounded state, and render only after explicit resume. |
| Network/offline failure before publication | Retain the current viewer/frame; start a new operation only after the host decides connectivity is restored. |
| Worker crash before publication | Terminate the worker, retain the current viewer/frame, and permit a new load. |
| Cancellation before publication | Acknowledge within 1,000 ms and retain the viewer. |
| Failure after partial Source publication | Destroy the viewer before returning; recreate before any new Source load. |
| Device loss or fused renderer/surface failure | Destroy the viewer and reject all later work with `viewer_destroyed`; recreate explicitly. |
| Source identity or generation change | Cancel stale scheduled work, clear pick/highlight state, and reject late exact results. |
| Persistent-cache quota/unavailability | Fail explicitly; the host may retry with `memory` or `none`. |
| Memory pressure | Rely on the fixed independent ceilings; no portable pressure signal or automatic cache eviction is claimed. |

Unsafe destructive probes such as physical device loss and browser-process
memory pressure are not forced in the attended browser lane. Deterministic
facade/raw-viewer tests exercise their exact state transitions, while the real
browser run proves the same public recreation path after explicit disposal.

## Diagnostics and qualification artifact

The private browser acceptance record advances to
`punctra-browser-qualification-v1`. It contains:

- package and harness schema versions;
- browser user agent, language, logical processors, viewport, DPR, screen facts,
  visibility state, secure-context state, and reported WebGPU facts;
- exact workload identity and Coverage;
- cold, warm, timing, frame, transport, worker, cache, JavaScript-heap, canvas,
  logical renderer, and explicit unavailable measurements;
- recovery outcomes and the safe action for recreation-required failures; and
- a closed list of nonclaims.

The record is data, not telemetry. The harness performs no upload and reads no
cookies, credentials, browser profile, local files, or unrelated storage.

## Security review

v0.19 retains the v0.16-v0.18 trust boundary:

- only an explicitly supplied deployment manifest may identify remote bytes;
- every binary request remains bounded, `206`-only, identity-encoded,
  validator-bound, length-checked, and digest-verified before use;
- cross-Source cache keys include immutable identity and representation facts;
- worker messages and external errors remain schema-checked and bounded;
- partial publication never permits retry in the same viewer; and
- credentials, authorization, URL allowlists, CSP, hosting, and user-facing
  recovery policy remain caller responsibilities.

The qualification guide includes an issue-evidence template containing only
the bounded diagnostic artifact, reproduction steps, expected/actual outcome,
and opt-in screenshot or Source permission. It forbids access tokens, cookies,
signed URLs, private Source bytes, and proprietary filenames.

## Public interface changes

The only public SDK change is the additive `SourceLoadResult.timings` record.
No new viewer method, framework adapter, Rust crate, renderer policy, Source
format, capability fallback, telemetry seam, or automatic recovery controller
is accepted. The thin React adapter continues to translate only lifecycle and
viewport ownership.

All Rust libraries and both JavaScript packages advance together to
`0.19.0-alpha.1`. Persisted format, transfer, cache, diagnostics, and renderer
algorithm versions do not change merely because the package version changes.

## Verification and completion

Repository completion requires:

- deterministic JavaScript tests for timing capture, frame summaries, nullable
  heap observations, qualification ceiling evaluation, retry-in-place,
  recreation-required failures, worker crashes before/after publication,
  resize/DPR recovery, hidden/resumed state, and declaration/runtime agreement;
- the inherited Rust, JavaScript, packed-artifact, TypeScript, Vite, Range
  server, Wasm, example, benchmark, documentation, and forced-GPU suites from
  `CONTRIBUTING.md`;
- the real packed-SDK browser path passing through the strict local server on
  every declared supported matrix entry;
- an exact machine-readable matrix/observation artifact and human-readable
  release record pinned to the implementation commit and local environment;
- updated API reference, architecture, guides, changelog, roadmap, context,
  README, contribution commands, and package metadata; and
- explicit separation between the qualified local lane, unqualified platforms,
  and the still-missing independent-adopter gate.

No hosted CI is added. Completion wording will be: **Complete and repository-
verified for one exact local Chromium/macOS/Apple-GPU browser qualification lane;
other browsers, operating systems, adapters, devices, independent adoption,
support qualification, visual-quality expansion, and release-candidate status
remain outstanding.**
