# Browser SDK installation and deployment

Punctra `0.22.0-alpha.1` packages the framework-neutral browser viewer as
`@punctra/viewer` and the one qualified lifecycle adapter as `@punctra/react`.
The repository verifies locally packed npm tarballs; it does not claim that
either package has been published to a registry.

## Install the packed artifact

Build the Wasm bindings and both package tarballs:

```bash
scripts/build-browser-sdk.sh
```

The artifacts are written under `target/npm/`. A clean application can install
the exact viewer artifact directly:

```bash
npm install /absolute/path/to/target/npm/punctra-viewer-0.22.0-alpha.1.tgz
```

The checked-in `examples/browser-typescript` consumer is the v0.22 package-
version continuation of the v0.20
[five-minute quickstart](browser-quickstart.md); `examples/browser-react`
remains the thin React trial. Both intentionally contain no repository-relative
Punctra dependency.
`scripts/verify-browser-sdk.mjs` copies them to fresh temporary directories,
installs only the packed artifacts, type-checks them, and executes development
and production builds.

## Plain TypeScript

Create and own the canvas, then dynamically import the SDK when the view is
needed:

```ts
import type { BrowserViewer } from "@punctra/viewer";

const { createViewer } = await import("@punctra/viewer");
const viewer: BrowserViewer = await createViewer({
  canvas,
  viewport: {
    cssWidth: canvas.clientWidth,
    cssHeight: canvas.clientHeight,
    devicePixelRatio: window.devicePixelRatio,
  },
});

viewer.render();
viewer.pause();
viewer.resume();
viewer.resize(nextViewport);
viewer.dispose();
```

`loadSource()` returns immutable `timings` for first sampled Coverage, settled
View, and main-thread batch high-water. The older
`mainThreadMillisecondsHighWater` field remains a deprecated alias for the
last value during this pre-1 release. These timings end at main-thread frame
submission, not physical GPU completion.

Creation is asynchronous. Do not publish the handle before the promise
resolves. Always call `dispose()` during host teardown. `pause()` preserves the
viewer and Source load; `resume()` does not start an animation loop, so request
or render the next frame deliberately.

The host still owns CSS layout, resize policy, visibility policy, camera
gestures, display-mode choice, credentials, recovery UI, and exact-Query
authority. Optional input normalization is available from
`@punctra/viewer/input`. The immutable-LAS fixture bridge is available from
`@punctra/viewer/exact-query`; it is not a general browser Source or Query API.

## React

Install both packed artifacts plus a qualified React version:

```bash
npm install \
  /absolute/path/to/target/npm/punctra-viewer-0.22.0-alpha.1.tgz \
  /absolute/path/to/target/npm/punctra-react-0.22.0-alpha.1.tgz \
  react react-dom
```

The hook binds a caller-owned canvas to the same viewer API:

```tsx
const [canvas, setCanvas] = useState<HTMLCanvasElement | null>(null);
const binding = usePunctraViewer({
  canvas,
  active,
  viewport: { cssWidth, cssHeight, devicePixelRatio },
  mountKey: deploymentIdentity,
});

return <canvas ref={setCanvas} />;
```

Strict Mode replay and an asynchronous unmount are safe: a viewer resolving
after cleanup is immediately disposed. Viewport and `active` updates reuse the
viewer. Change `mountKey` only when a creation option intentionally requires a
new viewer. The hook renders no UI and makes no camera, Source, input, or error-
recovery decision.

## Wasm and Worker assets

With the qualified Vite path, the default assets are discovered from
`import.meta.url`. Production builds emit a content-hashed Wasm file and a
content-hashed module Worker with its dependencies. No global public path is
required. The qualified Vite 8.2.2 configuration keeps the module Worker in ESM
format:

```ts
export default defineConfig({
  optimizeDeps: {
    exclude: ["@punctra/viewer"],
  },
  worker: { format: "es" },
});
```

The viewer is already valid ESM. Excluding it from development dependency
pre-bundling lets Vite transform the package-local Worker URL suffix directly,
including on a cold optimizer start. Production builds still bundle and hash
the complete Worker module graph.

The clean trials verify the emitted SDK and bundled-Worker module graph so a
build fails when a relative production dependency is absent.

For an explicit copied-asset deployment:

```ts
const viewer = await createViewer({
  canvas,
  viewport,
  assets: {
    wasmUrl: new URL("/assets/punctra/browser_demo_bg.wasm", location.origin),
    workerUrl: new URL("/assets/punctra/stream-worker.js", location.origin),
    cacheKey: "0.22.0-alpha.1-build-7",
  },
});
```

Copy the exact same-version Wasm file. When overriding `workerUrl`, also copy
the Worker's relative module graph from the package: `module-loader.js`,
`streaming-protocol.js`, `worker-protocol.js`, and `range-response.js`. Keep
those files together and immutable. The explicit Worker is currently qualified
only from the host's origin. Existing query parameters are preserved when
`cacheKey` adds the bounded `punctra-v` token.

Serve Wasm as `application/wasm` and JavaScript as a JavaScript MIME type. A
cross-origin Wasm or Source request needs CORS. Remote Source delivery still
requires the exact Range and validator behavior in the
[browser streaming guide](browser-streaming.md).

## Content Security Policy and isolation

The v0.22 path uses a module Worker and WebAssembly, but no inline/evaluated
JavaScript, `blob:` Worker, `SharedArrayBuffer`, or service worker. It does not
require COOP/COEP cross-origin isolation. A restrictive same-origin starting
policy is:

```text
default-src 'self';
script-src 'self' 'wasm-unsafe-eval';
worker-src 'self';
connect-src 'self' https://declared-source.example;
```

Some browser CSP implementations require `wasm-unsafe-eval` for WebAssembly
compilation. Extend `connect-src` only for the explicitly declared Wasm or
Source origins. Do not add `unsafe-eval`, `blob:`, broad wildcards, or credential
policy merely to make the SDK silent; unsupported deployment must fail visibly.

## Verification and support boundary

Run:

```bash
scripts/build-browser-sdk.sh
node scripts/verify-browser-sdk.mjs
node scripts/verify-browser-integration-baseline.mjs
node scripts/verify-browser-visual-baseline.mjs
node scripts/generate-browser-sdk-reference.mjs --check
```

The [generated API reference](../api/browser-sdk.md) is derived from the exact
packed declarations. The v0.20 [integration
baseline](../releases/v0.20-browser-baseline.json) and [browser
matrix](../releases/v0.20-browser-matrix.json) remain immutable historical
evidence. The v0.21 [integration
baseline](../releases/v0.21-browser-baseline.json), [browser
matrix](../releases/v0.21-browser-matrix.json), and [verification
record](../releases/v0.21.0.md) remain immutable predecessor evidence. The
v0.22 continuation must publish fresh `v0.22-browser-baseline.json`,
`v0.22-browser-quickstart.json`, and `v0.22-browser-matrix.json` records only
after rebuilding the exact pinned package and repeating the attended functional
lane. Those files and observations are pending; v0.21 values must not be copied
forward as if newly observed.

The v0.21 visual corpus and the v0.22 Point-footprint capture, PNG, metric, and
evidence runner remain repository-private and add no package export. See the
[browser visual-quality guide](browser-visual-quality.md) for the immutable
predecessor and the [point-footprint guide](browser-point-footprint.md) for the
active record-to-verify continuation. Final v0.22 evidence is not eligible
until the inherited packed quickstart and qualification pass on the pinned
rebuild. Neither lane establishes cross-browser equivalence,
independent-human interpretation, or a physical-display claim.
The LAS header and Point-record decoders used by `exact-query` are deliberately
package-private and are not supported exports. Other bundlers,
frameworks, browsers, devices, hosting stacks, and CSP deployments require
their own evidence and are not implied by ESM compatibility. The completed
repository lane does not establish API stability, support qualification, beta,
v1, or release-candidate status.
