# Browser Viewer API Design (v0.17)

Status: **Complete and repository-verified for the bounded repository slice**

This design is authoritative for the bounded Punctra v0.17 repository slice.
The maintainer's request to continue through v0.17 activates this technical
scope. Repository completion can establish one coherent framework-neutral
viewer API and one local plain-browser integration; it cannot establish a
packaged SDK, framework support, broad browser qualification, independent
adoption, or a browser-engine release-candidate promise.

## Outcome

Punctra v0.17 exposes one typed browser viewer API for lifecycle, viewport,
camera and projection, five inherited display modes, render scheduling,
provisional picking, highlighting, bounded state observation, and asynchronous
exact Point confirmation.

The API composes the existing private WebAssembly renderer and v0.16 streaming
worker behind one viewer handle. A plain JavaScript host can load the checked-in
immutable LAS deployment, navigate it, switch every inherited display mapping,
pick and highlight a resident sample, clear highlights, and hand the provisional
Point Identity to an independently supplied exact-Query bridge without calling
renderer, planner, worker, cache, or Source-publication internals.

## Evidence boundary

Repository completion may prove:

- a coherent checked-in JavaScript API and matching TypeScript declaration for
  the one bounded browser viewer composition;
- one caller-owned canvas with explicit create, visible/hidden, resize, render,
  scheduled-render, Source-load, state-subscription, and destroy behavior;
- explicit perspective and orthographic camera inputs plus optional normalized
  pointer, wheel, keyboard, and touch events whose policy remains host-owned;
- the inherited neutral, elevation, RGB, intensity, and classification display
  mappings over the v0.16 attributed root samples;
- provisional GPU pick and presentation-only highlight behavior bound to Source
  identity and View generation; and
- exact confirmation of one Point from the immutable uncompressed LAS fixture
  through a separately injected, cancellable, Source-record bridge.

It does not prove:

- npm or registry packaging, bundler asset resolution, framework adapters,
  semantic-version stability, generated API reference, or installability;
- arbitrary LAS/LAZ URLs, LAZ random access, general Spatial Index traversal,
  multi-Source viewing, general exact Query planning, rectangle selection, or a
  browser Workspace;
- that sampled display Coverage is complete, authoritative, or suitable for an
  Edit before exact confirmation;
- automatic interaction policy, terrain, export, editing, annotation,
  collaboration, application UI, service ownership, or authentication; or
- browser/device support outside the exact recorded local environment,
  independent adoption, production performance, support qualification, visual-
  quality expansion, or release-candidate status.

## Containment and ownership

The v0.17 API remains in the `browser-demo` repository application. It is a
public integration boundary for the checked-in browser host but is not yet a
distributed SDK. v0.18 remains responsible for package layout, generated and
published artifacts, bundler/worker URL behavior, framework trials, and any
stability declaration.

The Browser Host owns:

- the canvas element, CSS layout, visibility policy, user-facing controls,
  camera gestures, display-mode choice, credentials choice, cache choice,
  recovery UI, and viewer recreation;
- whether normalized input events become orbit, pan, zoom, projection, reset,
  or another application action; and
- construction and injection of an exact-Query bridge appropriate to its
  authoritative Source and revision model.

The Browser Viewer owns on the host's behalf:

- one WebAssembly viewer handle, WebGPU resources, one active worker operation,
  decoded batch publication, active View generation, recorded-frame pick state,
  complete highlight updates, coalesced animation-frame scheduling, and bounded
  state delivery;
- validation that viewer commands target the active lifecycle and generation;
  and
- cancellation and disposal of its owned animation frame, worker, subscriptions,
  and pending viewer operations.

The exact-Query bridge owns:

- authoritative Source/revision lookup, exact Point existence and values,
  cancellation, and its own resource ceilings;
- returning an exact result whose Source identity and Point ordinal match the
  request; and
- never using a resident display sample or GPU pick payload as exact position or
  Attribute authority.

## Public module boundary

The checked-in framework-neutral modules are:

- `viewer-api.js`: viewer creation, lifecycle, load, render scheduling, camera,
  display mode, pick, highlight, clear, exact handoff, state, and error types;
- `viewer-api.d.ts`: the exact TypeScript declaration for that runtime surface;
- `viewer-input.js`: optional bounded input normalization without camera or
  application policy; and
- `exact-query.js`: the local fixture's bounded immutable-LAS exact bridge and
  its validation helpers.

The generated `wasm-bindgen` module, raw JSON diagnostics, worker protocol,
stream publication calls, cache keys, renderer updates, planner state, and wgpu
objects are implementation details. The plain host imports only the public
viewer modules and the generated module needed to initialize WebAssembly; it
does not coordinate raw viewer methods or worker messages.

## Viewer lifecycle and errors

Viewer creation is asynchronous and publishes no usable viewer until the
existing v0.15 capability, canvas, device, renderer, and initial-scene checks
complete. `destroy()` is idempotent. Every other operation against a destroyed
viewer fails with `viewer_destroyed` and cannot recreate resources implicitly.

The viewer exposes structured `ViewerError` values with:

- a versioned schema;
- a closed error code declared by the TypeScript union;
- a bounded human-readable message;
- one safe host action; and
- whether the current viewer may be retained.

Rust/Wasm failures, worker failures, exact-bridge failures, cancellation, stale
generation, invalid caller input, and lifecycle errors cross the same public
error boundary. Tests compare the runtime code list with the declaration. Raw
exceptions and unbounded external messages are not published directly.

Device loss, stale surface state that requires recreation, partial renderer
publication failure, or another fused renderer error destroys the viewer before
the error is returned. A recoverable pre-publication network, cancellation, or
exact-Query failure keeps the last presented frame and viewer state.

## View generation and state

Every loaded Source receives a monotonically advancing generation within the
existing stream View identity. A provisional pick contains:

- the exact generation of its recorded frame;
- Source identity and Point ordinal;
- producing batch key and version; and
- the explicit authority `provisional_gpu_hint`.

Camera or viewport changes discard the recorded pick frame. A new Source
generation clears recorded-frame and highlight presentation. Exact confirmation
checks the request generation both before and after the asynchronous bridge
call. A result that finishes after another Source generation, destruction, or
cancellation fails and is never presented as current.

`state()` returns a bounded immutable snapshot. `subscribe()` delivers only
complete snapshots and returns an unsubscribe function. The state includes
lifecycle, viewport, camera, projection, display mode, active Source/generation,
Coverage, resident point/batch/resource facts, render scheduling, provisional
pick state, highlight count, and the most recent safe public failure. It does
not expose mutable renderer, worker, cache, or Source internals.

## Camera, viewport, and rendering

The camera accepts finite eye, target, and up triples; a perspective vertical
field of view or orthographic vertical world height; and positive ordered near
and far clipping distances. The existing renderer camera validation remains the
final numeric authority.

The host supplies finite positive CSS width/height and device-pixel ratio. The
inherited physical dimension, pixel-count, DPR, surface-byte, and transient-
texture ceilings remain unchanged. Resize is atomic from the public API's
perspective and invalidates the recorded frame.

`render()` records and presents immediately when visible. `requestRender()`
coalesces all requests before the next animation frame into one render and one
shared completion. Hiding, destruction, generation change, or a fused error
cancels stale scheduled presentation. The viewer never installs a perpetual
render loop.

## Input normalization

The optional input helper observes one caller-selected event target and emits a
small closed union:

- pointer orbit delta;
- pointer or two-touch pan delta;
- wheel or two-touch zoom delta; and
- bounded keyboard facts.

Coordinates and deltas are finite CSS pixels or normalized wheel lines. At most
two active pointers are retained. The host chooses event prevention, pointer
capture, key bindings, camera transformations, animation, and accessibility
alternatives. Disposing the helper removes every installed listener and clears
retained pointers.

## Display modes and decoded sample record

The default remote display mode remains RGB. Worker-to-Wasm transfer record v2
is a fixed 32 bytes:

| Offset | Bytes | Meaning |
|---:|---:|---|
| 0 | 8 | little-endian Source Point ordinal |
| 8 | 12 | three finite relative `f32` positions |
| 20 | 2 | raw intensity `u16` |
| 22 | 1 | raw classification `u8` |
| 23 | 1 | zero reserved byte |
| 24 | 6 | raw red, green, and blue `u16` values |
| 30 | 2 | zero reserved bytes |

The worker validates and transfers attributes but does not select presentation
color. The Wasm viewer retains at most eight decoded display batches and derives
complete renderer replacement batches for the chosen mode. A display change
increments every affected batch version and preserves Point Identity,
generation, position, Coverage, and highlight membership.

The mappings remain exactly those inherited from v0.10:

- neutral `[190, 205, 220, 255]`;
- elevation normalized across the complete Source Z bounds and interpolated
  across the fixed five-stop palette;
- RGB and intensity mapped by integer `(value * 255 + 32767) / 65535`;
- classification 0 through 18 from the fixed table and 19 through 255 from the
  existing deterministic wrapping fallback; and
- alpha always 255.

Transfer remains capped at 1,024 Points and 32,768 bytes per batch, eight
batches, and 8,192 Points. The existing 320-KiB worker staging ceiling still
covers one 172,032-byte root sample range plus one maximum output buffer.
Decoded batch retention is reported independently and capped at 262,144 logical
record bytes.

## Picking and highlighting

Picking is nonblocking and always targets the exact last recorded frame.
Beginning a pick validates a physical pixel inside the current viewport.
Polling remains bounded and never blocks the browser event loop. A hit is
accepted only when its generation is active and its Point Source matches the
active streamed Source or the deterministic initial fixture.

Highlights are complete replacement sets, never incremental hidden state. The
API accepts at most the existing 32 Point limit, one active Source, unique
ordinals, and the active generation. Empty input clears highlights. Highlighting
is presentation only and does not establish exact Query membership, visibility,
classification, or Edit eligibility.

## Exact Point bridge

The public viewer calls an injected asynchronous bridge with Source identity,
Point ordinal, active generation, and `AbortSignal`. The checked-in plain host
uses a fixture bridge limited to uncompressed LAS 1.2 point format 3:

- the deployment manifest and 256-byte Source probe establish exact Source
  length, strong ETag, Source identity, point count, point-data offset, 34-byte
  record width, format, scale, and offset;
- one confirmation requests exactly one 34-byte Source record by HTTP Range;
- response status, `Content-Range`, length, identity content encoding, and strong
  ETag must match before decoding; and
- exact ticks, world position, intensity, classification, and RGB are returned
  with `exact_source_record` authority.

The bridge rejects out-of-range ordinals, format or layout drift, validator
drift, truncation, full responses, cancellation, non-finite decoded coordinates,
and mismatched Point identity. It performs no LAZ decompression, arbitrary Query,
Workspace lookup, or Edit.

## Plain-host acceptance

The local host must complete the inherited v0.15 lifecycle and v0.16 cold/warm
stream checks through the public viewer API, then additionally:

- render all five display modes at one active Source generation;
- exercise both perspective and orthographic camera inputs and optional
  normalized input wiring;
- obtain a streamed provisional pick, publish one highlight, clear it, and
  exactly confirm that same Point from the immutable Source record;
- reject a stale generation, a cancelled exact confirmation, and work after
  destruction without stale presentation; and
- expose the bounded public state and TypeScript/runtime contract evidence.

The harness records the exact browser, operating system, adapter, surface,
viewport, transport/cache/worker/main-thread limits, View generation, display
modes, pick/highlight facts, exact record facts, and explicit nonclaims.

## Non-goals

v0.17 does not add:

- an npm package, package registry publication, bundler integration, worker or
  Wasm asset resolver, framework adapter, hot-reload guarantee, or API stability
  promise;
- arbitrary Source discovery, LAZ/COPC/EPT/3D Tiles, multi-Source composition,
  complete remote hierarchy traversal, rectangle/polygon Query, or bulk exact
  reads;
- Workspace creation, selection storage, classification Edit, Revert, terrain,
  export, annotations, collaboration, or application UI policy;
- gesture bindings owned by the viewer, a perpetual render loop, automatic
  device recreation, telemetry, authentication, or service infrastructure; or
- browser/device qualification, visual-quality claims, independent adoption,
  support qualification, or release-candidate language.

## Verification and completion

Repository completion requires:

- deterministic Rust tests for camera, display mappings, transfer-v2 decoding,
  batch-version replacement, generation-aware pick/highlight, and every new
  independent resource limit;
- deterministic JavaScript tests for the viewer façade, structured errors,
  scheduled-render coalescing, Source-load ownership, stale/cancelled exact
  handoff, exact LAS record decoding, input normalization, and declaration/runtime
  agreement;
- fixture regeneration equivalence, local Range-server checks, `wasm32` checks,
  and a clean `wasm-bindgen` browser build;
- the plain-host local browser acceptance path passing on the exact recorded
  browser, operating system, adapter, and local server;
- all formatting, linting, tests, rustdoc, package, fuzz, benchmark, example, and
  forced-GPU commands required by `CONTRIBUTING.md`;
- updated architecture, package, guide, changelog, roadmap, context, and
  contribution documentation; and
- a verification record that separates deterministic repository evidence from
  every unsupported external exit.

No hosted CI is added. The completed release wording will be: **Complete and
repository-verified for one bounded framework-neutral browser viewer API and
immutable-LAS exact-Point bridge; SDK packaging, arbitrary Sources and Queries,
broad browser qualification, independent adoption, API stability, and support
qualification outstanding.**
