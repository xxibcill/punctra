# Stable Browser-Engine Integration Baseline Design (v0.20)

Status: **Accepted for the bounded repository implementation; independent-
adopter evidence remains outstanding**

This design is authoritative for the bounded Punctra v0.20 repository slice.
The maintainer's 2026-08-27 request to continue through v0.20 activates the
repository implementation below after v0.19 was merged. The roadmap's original
independent-adopter activation evidence has not been supplied. Repository work
may therefore establish an independently installable technical baseline and
one exact locally qualified consumer path; it cannot describe a maintainer-run
trial as independent adoption.

## Outcome

Punctra v0.20 consolidates the v0.15-v0.19 browser work into one packed,
documented, consumer-shaped integration baseline before visual-quality work
begins.

A web developer can install the local `@punctra/viewer` tarball, create a viewer
on a caller-owned canvas, stream the declared immutable LAS deployment, apply
host-owned navigation, switch the five inherited display mappings and both
projections, obtain a provisional pick, highlight it, confirm the exact Source
record, recover according to the structured failure, and dispose every owned
resource. The clean consumer does this through supported package entry points;
it does not import repository-private renderer, Worker, cache, protocol, or raw
Wasm seams.

The exact package graph, fixture identities, generated scene, display modes,
projection modes, browser lane, resource ceilings, recovery dispositions, and
existing presentation policy are frozen in a machine-readable baseline. This
freeze detects later drift; it is not a claim that the current image quality is
complete.

## Evidence boundary

Repository completion may prove:

- the framework-neutral package and thin React adapter install from exact local
  tarballs into clean applications with no repository-relative dependency;
- the plain TypeScript quickstart builds and executes the complete accepted
  browser workflow through the strict Range server and supported exports only;
- the supported entry-point declarations match their runtime exports and no
  decoded-record, Worker, cache, renderer, or generated-Wasm implementation
  helper is an accidental package export;
- one exact local Chromium/macOS/Apple-GPU lane reproduces the inherited
  functional, latency, resource, and recovery gates from the packed artifact;
- the checked-in baseline binds every accepted input and presentation-policy
  fact that v0.21 may measure; and
- the documentation tells an integrator what is supported, host-owned,
  provisional, exact, recoverable, recreation-required, and unqualified.

It does not prove:

- that an independent adopter completed the path, that the packages were
  published to a registry or CDN, or that an external hosting/authentication/
  CSP deployment was qualified;
- compatibility outside the exact recorded browser, operating system, device,
  adapter, display, bundler, TypeScript, and React versions;
- arbitrary LAS/LAZ URLs, LAZ decompression, hierarchy traversal, complete
  Source Coverage, multiple Sources, general exact Queries, editing, terrain,
  export, host UI, or automatic recovery;
- final pre-v1 API compatibility, a stable visual baseline, production support,
  beta status, release-candidate status, or a v1 promise; or
- representative visual-quality coverage for sparse, dense, layered,
  high-dynamic-range, classification, large-world, and mixed-LOD conditions.
  v0.21 remains responsible for accepting and measuring that corpus.

## Integration package boundary

`@punctra/viewer` retains four supported package entry points:

- `@punctra/viewer` owns viewer creation, lifecycle-safe viewer operations,
  bounded state/error values, asset resolution, and their public types;
- `@punctra/viewer/input` normalizes bounded pointer, touch, wheel, and keyboard
  facts without selecting navigation policy;
- `@punctra/viewer/exact-query` supplies the narrow immutable-LAS exact-record
  bridge for the accepted deployment profile; and
- `@punctra/viewer/package.json` exposes package metadata to tooling that
  explicitly requests it.

The exact-query entry exports only the bridge factory, structured error, and
types needed to configure and call that bridge. LAS header and point-record
decoders are package-private implementation details. The decoded transfer
layout, module loader, Range validator, Worker protocol, cache implementation,
raw `wasm-bindgen` module, generated declarations, and renderer methods remain
unexported even when their files are present for the package's internal module
graph.

`@punctra/react` remains a thin translation over the same viewer lifecycle. It
does not create a canvas, load a Source, choose a camera, interpret a pick,
decide recovery, or retain a viewer after unmount.

All Rust libraries and both JavaScript packages advance together to
`0.20.0-alpha.1`. Persisted, transfer, cache, diagnostics, Source, index,
renderer, and qualification schemas do not advance solely because the package
version changes.

## Consumer-shaped quickstart

The plain TypeScript example becomes the v0.20 quickstart and remains a clean
consumer:

- it imports only the three supported viewer entry points;
- it owns the canvas, CSS layout, physical viewport calculation, visibility,
  input-to-camera policy, controls, current viewer handle, and disposal;
- it constructs the exact bridge for the same manifest that it streams;
- it exposes visible actions for load/retry, display mode, projection, pick,
  highlight/clear, exact confirmation, pause/resume, and disposal;
- it publishes structured status, Source/generation/Coverage, authority, and
  safe recovery text without presenting provisional display values as exact;
  and
- it has one deterministic acceptance mode that completes load, navigation,
  projection/display changes, provisional pick/highlight/clear, exact
  confirmation, cancellation, retry/recreation where required, and disposal.

The example has no embedded credentials, Source discovery, URL allowlist,
telemetry, application persistence, or general editor policy. The repository's
strict server supplies only the immutable accepted deployment and bounded fault
routes. An adopter replaces the manifest URL, hosting, credentials, and exact
authority according to its own deployment.

## Baseline freeze

The machine-readable `punctra-browser-integration-baseline-v1` record binds:

- package version, supported entry points, required deployable assets, and
  generated API-reference digest;
- exact Source/index/manifest/record byte lengths and SHA-256 identities;
- generated-scene Source identity, Point count, world origin, generation,
  batch identity, and logical resource facts;
- the five display modes, two projections, sampled-Coverage meaning, and the
  unchanged renderer/display-policy versions;
- the exact inherited v0.19 qualified browser lane and each explicitly
  unqualified platform class;
- latency and independent logical resource ceilings plus their observed local
  values; and
- retry-in-place, recreation-required, cancellation, stale-generation, and
  unsupported-initialization dispositions.

The verifier derives these facts from checked-in sources, package artifacts,
fixture bytes, and the browser observation. A recorded pass flag cannot override
a failed derived condition. Changing a frozen input or supported export requires
an explicit later baseline revision rather than silently regenerating hashes.

The existing generated 1,089-Point scene and 70,000-Point immutable LAS/
4,096-Point sampled root remain the v0.20 inputs. They protect functional and
appearance continuity but do not by themselves satisfy the v0.21 representative
visual-corpus activation gate.

## Browser capability and support statement

The v0.20 support authority preserves the exact v0.19 matrix unless a new lane
is actually executed. A platform may be:

- **qualified**: the exact matrix entry passed the packed quickstart and full
  inherited qualification suite;
- **unqualified**: it was not executed, even if initialization may work; or
- **unsupported**: a required secure-context, WebGPU, adapter, surface, renderer,
  viewport, deployment, or Range capability failed before usable continuation.

There is no WebGL, Canvas, software, reduced-feature, silent cache, or visual
fallback. Browser support is not inferred from a user-agent family.

## Performance and resource statement

v0.20 retains the v0.19 fixed workload, timing definitions, 30 settled-frame
sample, and independent ceilings. The performance report records exact observed
values for the new package and quickstart run while keeping these boundaries:

- first Coverage and settled View end at main-thread frame submission, not
  physical GPU completion;
- callback cadence is not display-panel presentation or GPU-completion timing;
- JavaScript heap facts remain nullable and non-standard;
- renderer, decoded-transfer, Worker-staging, response, cache, canvas, and
  transient-texture byte counts remain separate logical facts; and
- process RSS, physical cache allocation, driver/GPU allocation, energy, and
  thermal behavior remain unobserved.

No general performance claim is derived from the loopback fixture.

## Recovery and security statement

The public `ViewerError` remains the one recovery boundary. A recoverable
failure keeps the last safe viewer/frame and lets the host retry only after the
reported condition is corrected. A post-publication, device, or fused renderer/
surface failure destroys the viewer before returning and requires explicit
recreation. Cancellation, Source identity, View generation, exact handoff,
highlight, and scheduled-render state remain viewer-owned and generation-safe.

The v0.16-v0.19 remote-Source security contract remains unchanged: bounded
explicit Range requests, manual redirect behavior, identity encoding, exposed
headers, strong validators, exact lengths, range digests, representation
digests, identity-versioned cache keys, bounded external messages, and no retry
in a partially published viewer. The host still owns authentication,
authorization, credentials, URL policy, CSP, cache consent, telemetry consent,
and issue-data redaction.

## Documentation set

Repository completion publishes and cross-checks:

- a five-minute browser quickstart tied to the clean packed consumer;
- the existing SDK deployment guide as the detailed embedding guide;
- a machine-readable integration baseline and capability/support matrix;
- the exact local performance and resource report;
- the existing browser qualification guide as the recovery and issue-evidence
  guide; and
- one consolidated known-limitations document that distinguishes unsupported,
  unqualified, deferred, and host-owned behavior.

Generated API documentation comes from the packed declarations and fails its
check when the package surface changes without regeneration.

## Verification and completion

Repository completion requires:

- deterministic unit tests for the quickstart controller, navigation policy,
  supported runtime/declaration exports, removed accidental exports, baseline
  derivation, fixture binding, recovery actions, and exact authority labels;
- packed-artifact-only TypeScript and React installs, strict type checking,
  development/production Vite builds, emitted Wasm/Worker graph checks, and the
  generated API-reference check;
- the quickstart build served through the strict local Range host and one exact
  attended WebGPU browser run of its deterministic acceptance mode;
- the inherited Rust, JavaScript, Wasm, fixture, package, documentation, fuzz,
  example, benchmark, and `PUNCTRA_REQUIRE_GPU=1` lanes in `CONTRIBUTING.md`;
- a machine-readable baseline, human-readable release verification record,
  exact implementation pin, and checked-in observed browser facts; and
- explicit audit confirmation that no known release-blocking correctness,
  security, data-mixing, lifecycle, recovery, packaging, or documentation defect
  remains inside the declared repository baseline.

No hosted CI is added. Completion wording will be: **Complete and repository-
verified for the bounded packed browser integration baseline and one exact
local Chromium/macOS/Apple-GPU consumer lane; independent adoption, registry/
CDN publication, other browsers and devices, API stability, visual-quality
completion, support qualification, beta, v1, and release-candidate status
remain outstanding.**
