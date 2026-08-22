# WebAssembly and WebGPU Browser Foundation Design (v0.15)

Status: **Complete and repository-verified for one bounded local
WebAssembly/WebGPU browser-foundation slice; remote Source delivery, broad
browser qualification, independent adoption, SDK stability, and support
qualification outstanding**

This design is authoritative for the bounded Punctra v0.15 repository slice.
The maintainer's request to continue through v0.15 activates this technical
scope. Repository completion can establish one local browser path; it cannot
establish broad browser support, remote LAS/LAZ delivery, independent adoption,
or browser-product release-candidate status.

## Outcome

Punctra v0.15 renders one deterministic generated point-cloud scene inside a
browser canvas through the existing `render-protocol`, `point-view`, and
`render-wgpu` contracts compiled to WebAssembly. One private static host proves
the browser lifecycle without making a framework or SDK commitment.

The host reports the selected adapter and surface facts, bounded logical GPU
residency, renderer-owned transient texture bytes, canvas allocation bytes,
progressive Coverage, provisional picking authority, and explicit unsupported
states. These are repository diagnostics, not CPU-authoritative Source values,
browser heap measurements, production performance, or a support matrix.

## Evidence boundary

Repository completion may prove:

- `wasm32-unknown-unknown` compilation of the renderer-neutral protocol, math,
  View planner, wgpu renderer, and browser host;
- asynchronous WebGPU initialization in one declared local browser;
- deterministic generated planning, publication, canvas rendering, resizing,
  device-pixel-ratio handling, visibility suspension, provisional picking, and
  shutdown;
- explicit capability and resource-limit diagnostics before renderer state is
  published; and
- a repeatable local static-host and browser smoke path without a JavaScript
  framework or native-only renderer shim.

It does not prove:

- HTTP Range delivery, LAS/LAZ decoding in a browser, caching, Web Workers, or
  main-thread latency for a real remote Source;
- compatibility outside the exact recorded browser, operating system, and
  WebGPU adapter;
- browser process memory, JavaScript heap, GPU allocator padding, or energy use;
- independent embedding, package-registry publication, API stability, or
  production support; or
- visual-quality improvements, photorealism, or release-candidate readiness.

## Build and packaging path

The selected browser build path is:

- the repository-pinned Rust 1.90 toolchain;
- Cargo's `wasm32-unknown-unknown` target;
- `wgpu` 30's WebGPU backend with WGSL shaders;
- `wasm-bindgen` 0.2.127 and the matching CLI for ES-module bindings; and
- a checked-in static HTML/CSS/JavaScript host with generated bindings written
  to an ignored `web/pkg` directory.

The browser host has no npm dependency, bundler, framework, development server,
or WebGL fallback. A repository script builds the WebAssembly artifact and
bindings. Any static HTTP server may serve the host on localhost; opening it as
`file://` is unsupported because browser module and WebGPU security rules differ.

`browser-demo` is a private application package. Its generated JavaScript and
TypeScript declarations are acceptance-host artifacts, not the v0.18 supported
SDK. The existing publishable Rust crates remain independently usable.

## JavaScript boundary

The example-only `wasm-bindgen` boundary contains:

- one asynchronous factory that accepts a caller-owned `HTMLCanvasElement` and
  returns a viewer only after capability validation, adapter/device creation,
  surface configuration, renderer creation, deterministic View planning, and
  complete initial batch publication succeed;
- explicit `resize(css_width, css_height, device_pixel_ratio)`,
  `set_visible(visible)`, `render()`, `begin_pick(x, y)`, `poll_pick()`,
  `diagnostics()`, and `shutdown()` operations; and
- structured JSON diagnostics and errors with one safe host action.

The boundary does not accept Source URLs, credentials, arbitrary point arrays,
shaders, callbacks, framework objects, persistent storage, or host policy. A
future SDK design may replace this example boundary rather than preserving it.

## Canvas, device, and lifecycle ownership

The JavaScript host owns:

- the canvas element and its placement, CSS size, accessibility text, input
  event policy, and removal;
- the decision to initialize, resize, render, suspend, resume, retry, or destroy;
- the CSS-size and device-pixel-ratio values passed at each resize;
- the animation-frame schedule and document-visibility policy; and
- presentation of diagnostics, errors, and recovery actions.

The private Rust browser adapter owns on that host's behalf:

- one WebGPU `Instance`, canvas `Surface`, selected `Adapter`, `Device`, and
  `Queue`;
- surface format, FIFO presentation, configuration, command encoders,
  submission, presentation, and asynchronous pick readback; and
- one existing `WgpuRenderer` whose resource publication remains governed by
  `render-protocol`.

This composition preserves the public renderer contract: `render-wgpu` does
not create a device, submit a queue, own a browser canvas, or take application
policy. The private adapter is the host in that contract. Shutdown drops its
renderer and GPU handles and makes every later operation fail explicitly. A
surface-loss or device-loss result instructs JavaScript to destroy the viewer
and call the asynchronous factory again; the example does not silently retry or
recreate resources.

## WebGPU capability floor

Initialization requires all of the following before publishing the initial
View generation or point batch:

- a secure browser context exposing `navigator.gpu`;
- one adapter compatible with the caller's canvas surface;
- WebGPU core features only (`required_features` is empty);
- the WebGPU default limit profile accepted by `wgpu`, including the buffer,
  bind-group, vertex-buffer, texture-dimension, and attachment support required
  by `WgpuRenderer`;
- a surface format accepted by the renderer as render-attachable and blendable;
- FIFO presentation and a supported composite alpha mode; and
- physical canvas dimensions inside the independent host ceilings below.

There is no WebGL fallback. Unsupported initialization returns a bounded error
with the safe action: keep the canvas unavailable and use a secure context with
a WebGPU-capable browser/device before explicitly retrying.

## Deterministic scene and planning

The generated scene contains one root hierarchy node and one point batch with a
fixed Source identity, View generation, batch key, batch version, world origin,
positions, colors, and centre Point identity. The private host runs
`ViewPlanner` twice:

1. the missing root produces exactly one bounded request; and
2. after complete batch publication, the resident root is retained with no new
   request or retirement.

The renderer then records that exact generation and batch into the canvas.
Picking is evaluated against the most recently recorded frame. A successful hit
must retain the fixed View generation, batch key, batch version, and Point
identity, but it remains explicitly provisional. A miss is not an exact empty
Query, and the host offers no Edit operation.

## Independent resource limits

The acceptance host fixes and reports these separate ceilings:

- protocol residency: 2,048 Points, 49,152 estimated vertex bytes, four batches,
  and 32 highlighted Point identities;
- deterministic scene: 1,089 generated Points in one batch;
- physical canvas: width and height at most 4,096 pixels and total area at most
  8,388,608 pixels;
- caller-declared device-pixel ratio: finite and inside `0 < dpr <= 4`;
- host surface allocation accounting: four bytes per physical pixel;
- renderer transient texture accounting: reported exactly by each recorded
  frame and independently checked against a conservative eight-bytes-per-
  physical-pixel, 67,108,864-byte ceiling;
  and
- presentation latency hint: two frames.

The logical protocol byte model does not include surface textures, depth/pick
targets, staging buffers, browser heap, GPU allocator padding, command storage,
or driver memory. Canvas bytes and renderer transient bytes are therefore
reported separately and are not described as observed process or GPU memory.
An over-limit resize fails before changing the canvas or surface configuration.

## Local browser acceptance harness

The checked-in static host is the browser harness. Its deterministic smoke path:

1. detects secure-context and `navigator.gpu` availability;
2. imports the generated ES module and initializes against the visible canvas;
3. verifies capability, planner, generation, batch-version, scene-count, and
   resource diagnostics;
4. renders, resizes with an explicit CSS size and device-pixel ratio, suspends
   while hidden, resumes, and renders again;
5. resolves the centre-pixel provisional pick and verifies its generation,
   batch, version, and Point identity;
6. shuts down and proves later rendering fails without publishing work; and
7. publishes `passed`, `unsupported`, or `failed` state in the document so the
   browser result is inspectable without relying on console output.

The host also exposes a manual restart action and readable diagnostic facts.
Browser-only failure coverage includes missing WebGPU/secure context, canvas
surface creation, adapter compatibility, device request, surface format,
device-pixel-ratio conversion, resize/reconfiguration, surface acquisition,
presentation, visibility scheduling, asynchronous buffer mapping, and browser
module loading. Native tests remain necessary regression evidence but cannot
substitute for this path.

## Interface presentation

The static host is designed for web developers integrating LAS/LAZ rendering.
It uses the repository's restrained grey, engineering-workstation design
context: compact factual labels, explicit state words, strong alignment,
accessible contrast, visible keyboard focus, no decorative animation, and no
color-only status. The canvas is labelled as progressive and non-authoritative.
Capability failures state what happened, why rendering is unavailable, and the
single safe action.

## Explicit non-goals

v0.15 does not add:

- remote Source URLs, Fetch, HTTP Range, CORS policy, LAS/LAZ browser decoding,
  Web Workers, caching, IndexedDB, offline support, credentials, or retry policy;
- a supported JavaScript/TypeScript SDK, npm package, framework adapter, custom
  element, or compatibility promise for the example boundary;
- orbit controls, arbitrary application UI, editing, exact CPU Queries, exact
  pick confirmation, selection, persistent Workspace state, terrain, or QA;
- multiple canvases per viewer, worker-owned `OffscreenCanvas`, WebGL fallback,
  native browser shims, or automatic device/surface recovery;
- visual-quality changes, new shaders, tone mapping, eye-dome enablement,
  general materials, plugins, annotations, measurements, or screenshots as
  accuracy evidence; or
- broad browser, mobile, field, partner, adopter, support, or product-release
  claims from one local generated harness.

## Verification and completion

Repository completion requires:

- native unit/interface tests for deterministic scene planning, protocol
  generation, batch-version, resource-limit, lifecycle, and diagnostic rules;
- `wasm32-unknown-unknown` checks for `render-protocol`, `point-view`,
  `render-wgpu`, and `browser-demo`;
- a clean release build plus matching `wasm-bindgen` ES-module bindings;
- the local browser harness passing on the exact recorded browser/adapter;
- existing workspace formatting, linting, tests, rustdoc, package, fuzz,
  benchmark, example, and forced-GPU commands from `CONTRIBUTING.md`;
- updated architecture, package, guide, changelog, roadmap, context, and
  contribution documentation; and
- a repository verification record that separates algorithm-accounted resource
  facts from unsupported browser-process measurements and external evidence.

No hosted CI is added. The completed release wording will be: **Complete and
repository-verified for one bounded local WebAssembly/WebGPU browser-foundation
slice; remote Source delivery, broad browser qualification, independent
adoption, SDK stability, and support qualification outstanding.**
