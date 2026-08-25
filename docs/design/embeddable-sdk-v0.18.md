# Embeddable SDK and Framework Integration Design (v0.18)

Status: **Complete and repository-verified for the bounded packed-artifact slice**

This design is authoritative for the bounded Punctra v0.18 repository slice.
The maintainer's request to continue through v0.18 activates implementation of
the package and embedding work below. The two clean repository embedding
trials satisfy the technical activation gate by exposing concrete TypeScript,
Vite, Worker, Wasm-URL, React Strict Mode, and teardown requirements. They are
maintainer-run trials, not independent adoption, registry publication, or broad
bundler/browser qualification.

The completed local evidence is pinned to an exact implementation commit in the
[v0.18 repository verification record](../releases/v0.18.0.md).

## Outcome

Punctra v0.18 packages the v0.17 browser viewer as the versioned
`@punctra/viewer` ES-module/Wasm artifact. A clean TypeScript application can
install its npm tarball, dynamically import the SDK, create one viewer with a
caller-owned canvas, resolve its packaged Wasm and module Worker, pause,
resume, resize, and dispose it without repository-relative imports.

One clean React application additionally installs the packed
`@punctra/react` artifact. Its single hook translates React mount, viewport,
active, unmount, Strict Mode replay, and hot-replacement behavior into the same
framework-neutral viewer API. It owns no canvas markup, camera policy, Source
choice, authentication, UI, or recovery policy.

## Activation observations

The two clean trials fixed the following requirements before acceptance:

- the package must expose a standards-based ESM entry plus declarations and
  retain the generated Wasm module as an explicit deployable asset;
- Vite Worker discovery requires the literal
  `new Worker(new URL(..., import.meta.url), { type: "module" })` form with
  statically readable options, while explicit host-owned Worker URLs remain a
  separate supported path;
- the default Wasm and Worker locations must derive from `import.meta.url`, not
  the page URL or a repository-relative path;
- an explicit cache token must preserve caller query parameters for copied
  assets, while bundler production output uses content-hashed assets;
- the plain TypeScript entry must type-check separately and the SDK dynamic
  import must remain a production code-split boundary; and
- React cleanup must exist before asynchronous creation completes, dispose a
  late viewer, unsubscribe before disposal, tolerate repeated cleanup, update
  viewport/active state without recreation, and use an explicit `mountKey`
  when creation options intentionally change.

The repository trials qualify Vite 8.2.2 and TypeScript 7.0.2 on the recorded
local Node environment only. Other bundlers may consume the standards-based
package but are not repository-qualified in v0.18.

## Evidence boundary

Repository completion may prove:

- exact npm tarball contents, package version, exports, TypeScript declaration
  consumption, generated API-reference freshness, and packed-artifact-only
  installation;
- development and production Vite resolution of the ESM entry, code-split SDK
  chunk, content-hashed Wasm, bundled module Worker, and Worker dependencies;
- explicit copied Wasm/Worker URL resolution with bounded cache tokens;
- the inherited v0.17 public viewer behavior through the packaged SDK;
- repeated asynchronous React mount abandonment and mounted teardown without
  retained subscription or viewer ownership; and
- one local browser execution through the SDK rather than the private raw
  Wasm binding.

It does not prove:

- npm-registry publication, CDN publication, semantic-version stability, a
  Git tag, or a hosted deployment;
- Webpack, Rollup, Parcel, esbuild, Next.js, Remix, Angular, Vue, Svelte, or
  another unexecuted bundler/framework integration;
- independent adoption, package popularity, production support, or a stable
  browser-engine release candidate;
- arbitrary LAS/LAZ URLs, general Source discovery, multi-Source viewing,
  general exact Queries, a browser Workspace, editing, terrain, export, or UI;
  or
- browser, operating-system, adapter, mobile, CSP-host, credential, cache,
  device-loss, backgrounding, memory, or performance qualification outside
  the exact recorded acceptance environment.

## Package boundary

`@punctra/viewer` has four supported entry points:

- `@punctra/viewer`: viewer creation, public state/error constants, package
  asset resolution, and the public viewer types;
- `@punctra/viewer/input`: optional policy-free input normalization;
- `@punctra/viewer/exact-query`: the bounded immutable-LAS exact record bridge;
  and
- `@punctra/viewer/package.json`: package metadata for tooling that explicitly
  requests it.

The raw `wasm-bindgen` module, renderer handle, Worker messages, streaming
protocol, range validator, decoded record layout, and cache internals are
packed implementation assets rather than supported imports. The package does
not export their subpaths.

`createViewer()` initializes one imported Wasm module at most once and creates
independently disposable viewer handles from it. Concurrent callers share the
same initialization promise. A failed initialization clears the promise so a
corrected retry can proceed. One imported SDK module rejects a second,
different Wasm URL after successful initialization; callers that need a
different immutable version import a separately versioned SDK module.

## Asset resolution

`resolveViewerAssets()` returns immutable absolute Wasm and Worker URLs.
Defaults use static `new URL(relative, import.meta.url)` expressions so a
supported bundler can emit content-hashed assets without a global public-path
setting. `createViewer()` uses a statically discoverable module Worker when the
default Worker is selected.

Hosts that copy assets outside the package graph may provide `wasmUrl` and
`workerUrl`. The Wasm response must be the matching immutable package bytes and
use `application/wasm`. An explicit Worker URL opts out of bundler Worker
discovery; the host must deploy `stream-worker.js` and its same-version relative
module dependencies together. The module Worker must be same-origin with the
host under the currently qualified path. A bounded `cacheKey` adds
`punctra-v` without dropping existing query parameters.

Source manifest, LAS, and index URLs remain independent caller/Remote
Deployment choices. The SDK does not rewrite them relative to the package.

## Lifecycle

The inherited `BrowserViewer` adds intention-revealing lifecycle aliases:

- `pause()` maps to hidden presentation, cancels stale scheduled rendering,
  and preserves an active Source load and viewer state;
- `resume()` returns to visible presentation without starting a perpetual
  render loop; and
- `dispose()` is the idempotent supported teardown spelling over `destroy()`.

Resize remains explicit and atomic. Disposal cancels the owned animation
frame, active load, exact confirmation, provisional pick, Worker, state
subscriptions, Wasm viewer, and GPU state through the same v0.17 ownership
path. A disposed viewer never recreates itself.

The React hook creates no viewer until its canvas exists. Cleanup marks the
mount abandoned before releasing a current viewer. A viewer that resolves
after cleanup is disposed without subscription or publication. Current
subscriptions are removed before disposal. Viewport and `active` changes call
`resize()`, `pause()`, or `resume()`; they do not remount. Non-viewport creation
changes require a caller-selected `mountKey`, keeping recreation explicit.

## Deployment and security

The SDK uses no `eval`, inline Worker source, `blob:` Worker, `SharedArrayBuffer`,
or service worker. Cross-origin isolation is therefore not required by the
v0.18 implementation. Hosts remain responsible for testing their exact policy.

A same-origin deployment normally needs policy equivalent to:

```text
default-src 'self';
script-src 'self' 'wasm-unsafe-eval';
worker-src 'self';
connect-src 'self' <declared-source-or-asset-origins>;
```

`wasm-unsafe-eval` is required where the selected browser applies that CSP token
to WebAssembly compilation. Source and cross-origin Wasm Fetches require
appropriate CORS. Remote immutable Source responses retain all v0.16 Range,
identity encoding, validator, exposed-header, and no-transform requirements.
The SDK does not own credentials, authorization, URL allowlists, telemetry,
cache consent, or CSP relaxation.

## Verification

The v0.18 verification adds:

- Node unit tests for SDK exports, URL/cache resolution, lifecycle aliases,
  bundler-aware Worker construction, and repeated React lifecycle races;
- `scripts/build-browser-sdk.sh` for release Wasm generation, exact npm tarball
  creation, and API-reference generation;
- `scripts/verify-browser-sdk.mjs` for exact tarball inspection, clean npm
  installation, strict TypeScript, development/production Vite builds,
  code-split output, content-hashed Wasm/Worker output, and development-server
  transforms in both trials; and
- the inherited browser acceptance host importing only `sdk.js`, exercising
  packaged lifecycle and public behavior, and reporting package version
  `0.18.0-alpha.1`.

All prior native, Wasm, fixture, documentation, fuzz, benchmark, example, and
required GPU lanes remain inherited. v0.18 changes packaging and lifecycle
translation, not native geometry, authority, rendering appearance, persistence,
Terrain, or Query semantics.
