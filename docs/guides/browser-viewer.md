# Local Browser Viewer API Guide

Punctra v0.17 adds one framework-neutral browser viewer boundary inside the
private `browser-demo` application. It composes the existing WebAssembly/WebGPU
viewer and v0.16 HTTP Range worker without claiming an installable SDK, stable
package surface, arbitrary Source support, or broad browser qualification.

The accepted scope is recorded in the [v0.17 Browser Viewer API
design](../design/browser-viewer-api-v0.17.md).

## Public modules

The plain host imports these checked-in modules:

- `viewer-api.js` and `viewer-api.d.ts` define viewer creation, lifecycle,
  viewport, camera, display mode, render scheduling, Source load, bounded state,
  provisional picking, highlights, exact handoff, destruction, and structured
  errors;
- `viewer-input.js` and `viewer-input.d.ts` optionally normalize pointer,
  two-touch, wheel, and keyboard input without choosing navigation policy; and
- `exact-query.js` and `exact-query.d.ts` implement and declare the local
  fixture's separately injected exact-Point bridge. The public surface is
  `ExactQueryError`, `createLasExactQueryBridge`, `decodeLasLayout`, and
  `decodeLasPointRecord`; the two decoder helpers support bounded fixture and
  integration validation without widening the accepted Source profile.

Generated `wasm-bindgen` methods, worker messages, transfer records, cache keys,
renderer updates, and raw diagnostics remain implementation details.

## Minimal lifecycle

Initialize the generated Wasm module, then create one viewer with the caller's
canvas and physical-size policy:

```js
import init, * as bindings from "./pkg/browser_demo.js";
import { createBrowserViewer } from "./viewer-api.js";
import { createLasExactQueryBridge } from "./exact-query.js";

await init();
const exactQueryBridge = createLasExactQueryBridge({
  manifestUrl: "./fixtures/v1/deployment.json",
  credentials: "same-origin",
});
const viewer = await createBrowserViewer({
  bindings,
  canvas,
  viewport: {
    cssWidth: canvas.clientWidth,
    cssHeight: canvas.clientHeight,
    devicePixelRatio: window.devicePixelRatio,
  },
  workerUrl: new URL("./stream-worker.js", import.meta.url),
  exactQueryBridge,
});

const unsubscribe = viewer.subscribe((state) => publishStatus(state));
await viewer.loadSource({
  manifestUrl: "./fixtures/v1/deployment.json",
  cacheMode: "persistent",
  credentials: "same-origin",
});
viewer.render();

unsubscribe();
viewer.destroy();
```

`destroy()` is idempotent. Every later operation fails as
`viewer_destroyed`; recreation is explicit. Only one Source load and one exact
confirmation may be active per viewer. An `AbortSignal` cancels either without
changing the last complete presentation.

## Camera, rendering, and input

`setCamera()` accepts one complete perspective or orthographic camera. The host
owns orbit, pan, zoom, reset, key binding, accessibility, and animation policy.
`createInputNormalizer()` emits only bounded input facts and removes all of its
listeners when disposed.

Call `render()` for immediate visible presentation or `requestRender()` to
coalesce work before the next animation frame. `resize()` accepts CSS width,
CSS height, and device-pixel ratio and reports the resulting physical viewport.
`setVisible(false)` suppresses presentation without destroying resident state.

## Display, pick, highlight, and authority

The supported display modes are `neutral`, `elevation`, `rgb`, `intensity`, and
`classification`. A mode change replaces every retained renderer batch at a
higher batch version while preserving Source identity, Point ordinal,
generation, position, and Sampled Coverage.

`pick({x, y})` returns either `null` or a provisional GPU hint containing the
recorded generation, Source identity, Point ordinal, batch key, and batch
version. `setHighlights()` publishes one complete presentation-only set of at
most 32 unique Points for the active Source and generation. `clearHighlights()`
publishes the empty set.

`confirmPoint()` passes the provisional identity to the separately injected
exact bridge. The checked-in fixture bridge validates the manifest, Source
probe, strong ETag, byte layout, and one exact HTTP Range response before it
decodes the requested 34-byte LAS record. Exact authority is
`exact_source_record`; resident positions, colors, and Attributes are never
used as exact values.

## Build and run

From the repository root:

```bash
cargo run -p browser-demo --bin generate_stream_fixture
node --test apps/browser-demo/web/*.test.mjs
scripts/build-browser-demo.sh
scripts/serve-browser-demo.py --port 8000
```

Open `http://127.0.0.1:8000/` in a secure-context WebGPU browser. Use the strict
server rather than a generic static server: exact `206`, strong validator,
identity encoding, range length, and exposed CORS headers are part of both the
streaming and exact-query contracts.

The page must report `PASS` after it verifies:

1. create, render, resize, hide/show, destroy, and recreate;
2. cancelled load, cold persistent load, recreation, and zero-binary-request
   warm load;
3. all five display modes and both projection types;
4. provisional pick, one complete highlight, exact confirmation of the same
   Source identity and Point ordinal, and complete clear; and
5. cancelled exact confirmation plus stale-generation rejection after a new
   Source generation.

Record the exact browser, OS, adapter, surface, viewport, public state,
transport/cache/worker limits, provisional identity, and exact record facts.
Passing one local environment does not qualify another browser or device.

## Failure and resource boundary

`ViewerError` uses the closed `punctra-viewer-error-v1` code union declared in
`viewer-api.d.ts`, with a bounded message, one safe host action, and a
recoverability flag. Tests require the runtime code set and TypeScript union to
agree. Fused renderer/device failures destroy the viewer before returning;
recoverable pre-publication network, cancellation, and exact-query failures
retain the last complete frame.

The checked-in host caps resident display Points at 8,192, renderer batches at
8, highlights at 32 Points, retained transfer-v2 records at 262,144 bytes, and
worker staging at 327,680 bytes. State reports these limits independently from
surface and transient texture bytes.
