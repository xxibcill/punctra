# HTTP Range Streaming, Browser Caching, and Worker Decoding Design (v0.16)

Status: **Active — accepted repository scope; implementation and local
acceptance in progress**

This design is authoritative for the bounded Punctra v0.16 repository slice.
The maintainer's request to continue through v0.16 activates this technical
scope. Repository completion can establish one local immutable LAS deployment
and one browser streaming path; it cannot establish arbitrary-URL support,
broad browser qualification, a stable browser SDK, independent adoption, or a
production hosting promise.

## Outcome

Punctra v0.16 progressively displays one remotely hosted LAS Source from a
compatible prebuilt disk-v2 Punctra Spatial Index. The browser requests only
bounded byte ranges, decodes index samples in one Web Worker, and transfers
bounded batches to the existing WebAssembly/WebGPU renderer without scanning or
downloading the complete Source.

One versioned deployment manifest binds the immutable Source representation to
its Source identity, strong HTTP validator, byte length, compatible index,
inspection-sample recipe, transform, root-node sample range, and integrity
digests. An arbitrary LAS/LAZ URL without that binding fails before a Source
request is made.

## Evidence boundary

Repository completion may prove:

- strict bounded Fetch/HTTP Range behavior for one locally hosted immutable LAS
  fixture and its compatible disk-v2 index;
- worker-owned index validation, sample decoding, color mapping, and transferable
  batch construction under independent queue, request, staging, and cache limits;
- source-aware progressive Coverage and Point Identity at the existing renderer
  boundary;
- explicit none, memory, and persistent cache policies with identity-versioned
  keys and caller-requested invalidation;
- deterministic classifications for missing Range support, validator drift,
  truncation, corruption, offline/network failure, quota failure, cancellation,
  retry exhaustion, and worker failure; and
- a cold/reload/warm-cache local browser acceptance path whose first remote frame
  precedes complete Source transfer.

It does not prove:

- general LAS or LAZ decoding in the browser, arbitrary index traversal, exact
  Source reads, or exact browser Queries;
- hostile-server integrity when a server reuses a strong validator for changed
  bytes, or authenticity of caller-supplied deployment metadata;
- browser process memory, JavaScript heap, cache implementation overhead, GPU
  allocator bytes, energy use, or deterministic wall-clock performance;
- credentials, authorization, service workers, offline-first application
  behavior, a CDN or object-store support matrix, or production hosting; or
- a supported JavaScript/TypeScript SDK, framework integration, broad browser
  support, independent adoption, visual-quality improvement, or release-
  candidate status.

## Containment and ownership

The v0.16 implementation remains in the private `browser-demo` application and
its static host. It adds no public Rust crate and does not change the public
`point-source`, `source-las`, `point-index`, `point-view`, `render-protocol`, or
`render-wgpu` interfaces.

The Browser Host owns:

- the manifest URL, cache mode, explicit invalidation choice, credentials mode,
  retry decision, worker lifetime, canvas lifecycle, and recovery UI;
- starting and cancelling one bounded stream operation; and
- scheduling transferred batches into WebAssembly and requesting frames.

The private streaming worker owns on the host's behalf:

- manifest validation, Range requests, retry/cancellation, response validation,
  byte integrity checks, index-header/root validation, sample decoding, cache
  access, backpressure, and transferable output buffers.

The Rust WebAssembly host continues to own renderer validation and GPU resources
on behalf of the caller. Progressive display remains non-authoritative and
disposable. A later viewer/API design may replace this private boundary.

## Deployment manifest v1

The checked-in fixture uses `punctra-browser-stream-v1`. The manifest is a
trusted deployment statement, not a discoverable Source format. It contains:

- one absolute-or-manifest-relative Source URL, media type, exact byte length,
  strong ETag, SHA-256 content digest, Punctra Source identity, and a small
  independently hashed probe range;
- one absolute-or-manifest-relative disk-v2 index URL, exact byte length,
  SHA-256 content digest, disk/recipe/schema versions, Source identity, Source
  Point count, position transform, and one root-node descriptor;
- the root header/node byte range and root sample byte range with exact offsets,
  lengths, counts, record width, SHA-256 digests, sampled Coverage, and world
  bounds; and
- the fixed display mapping and deployment-profile version.

The worker accepts only disk version 2, inspection recipe version 2, display-
sample schema version 1, 168-byte node records, and 42-byte attributed sample
records. It independently decodes the fetched Punctra header and root record and
requires their magic, versions, identity, Point count, transform, offsets,
counts, Coverage, and bounds to match the manifest. Manifest facts cannot relax
host resource ceilings.

Source identity was established when the local `source-las` adapter fully
verified the complete fixture and built the index. Browser display does not
repeat that full verification. Instead, every Source response must retain the
manifest's strong ETag and exact representation length, and the acceptance
probe must match its SHA-256 digest. The profile therefore assumes an immutable
server that obeys strong-validator semantics. Validator reuse after mutation is
server misconduct and outside the proven integrity envelope.

## HTTP Range contract

Every binary request:

- sends one inclusive `Range: bytes=start-end` header, the host-selected
  credentials mode, and an abort signal. Browser Fetch owns the forbidden
  `Accept-Encoding` request header, so the deployment disables transformation
  server-side and the worker validates the response encoding;
- accepts only status `206`, exact `Content-Range`, exact visible
  `Content-Length`, the expected total representation length, the manifest's
  strong ETag, and absent or `identity` content encoding;
- requires `Accept-Ranges: bytes` on the Source probe;
- rejects a `200` full response before reading its body;
- reads at most the requested bounded body, checks the exact returned length,
  and verifies SHA-256 before publication or caching; and
- classifies non-retryable representation/contract failures separately from
  bounded retryable network and server failures.

The declared deployment must expose `Content-Length`, `Content-Range`, `ETag`,
`Accept-Ranges`, and `Content-Encoding` through CORS when it is cross-origin.
The local acceptance server permits `GET`, `HEAD`, and `OPTIONS`, exposes those
headers, sends identity bytes, and has no redirect or content transformation.

At most two retries follow retryable network failures or status 408, 429, 500,
502, 503, or 504. Retry delay is bounded and cancellable. Redirects, status
200, malformed headers, validator drift, over-limit ranges, corruption, and
truncation are terminal for the current operation. Recovery always creates a
new operation; partial decoded batches never change Source identity.

## Worker protocol and backpressure

One dedicated module Worker handles one active stream. Messages use a private
versioned schema and operation identity. The host may send `start` or `cancel`;
the worker publishes bounded `state`, `batch`, `complete`, or `failure`
messages. Late messages for another operation are ignored.

The fixed repository ceilings are:

- one active operation and one in-flight HTTP request;
- two queued range requests and 512 KiB of queued range bytes;
- 256 KiB per HTTP range and 256 KiB of concurrent response bytes;
- 320 KiB of decoded worker staging, including the retained input range and one
  output batch;
- 1,024 Points and 24 encoded bytes per transferred display batch;
- eight transferred batches and 8,192 decoded Points for the accepted root;
- 1,000 milliseconds from an explicit host cancellation request to the
  worker's deterministic `cancelled` acknowledgement;
- 512 KiB of memory-cache response bodies; and
- 4 MiB of logical persistent-cache response bodies for one identity-versioned
  cache namespace.

The worker decodes little-endian `(ordinal, ticks, intensity,
classification, RGB)` disk-v2 samples. It checks sorted unique ordinals, finite
world positions, root bounds, and safe relative `f32` conversion. Output retains
the exact ordinal, relative position, and deterministic RGB8 display value.
Each transferred `ArrayBuffer` is detached from the worker. The main thread
publishes at most one 1,024-Point batch per task and yields before the next
batch. Observed task duration is diagnostic only; the deterministic per-task
Point/byte ceilings are the acceptance limit.

## Cache policy and invalidation

The host explicitly chooses `none`, `memory`, or `persistent`:

- `none` retains no response body after decoding;
- `memory` retains verified response bytes only for the active worker; and
- `persistent` uses the browser Cache API only after response validation and
  digest verification.

Cache namespaces and entry keys include the streaming schema, deployment
identity, Source identity, strong Source validator, index SHA-256, resource
kind, and exact byte range. Cached metadata repeats those facts and the digest;
a hit is revalidated before decode. Bytes from another Source identity,
validator, index digest, or range are unreachable through the key and rejected
if metadata differs.

Explicit invalidation deletes only the exact derived namespace before network
work. Quota or Cache API failure never falls back silently: the operation
returns `cache_quota` or `cache_unavailable`, and the safe action tells the host
to retry with `memory` or `none`. Persistent-cache byte counts are logical
verified response-body bytes, not browser storage allocation measurements.

## Rendering and Coverage

The first remote publication resets to a distinct v0.16 View generation. Each
worker batch becomes one renderer batch with an increasing fixed version, the
manifest Source identity, exact Source ordinals, one stable world origin, and
bounded relative positions. The rendered batches together represent the root
node's Sampled Coverage; they are not a complete Source, exact Query result, or
authorization to Edit.

Diagnostics separately report Source bytes requested/received, index bytes
requested/received, network and cache hits, retries, queue high-water, decoded
staging high-water, transferred batches/bytes/Points, main-thread publication
high-water, cancellation latency when exercised, logical cache bytes, and the
renderer facts inherited from v0.15. The accepted fixture must render before
its complete Source byte length has been transferred.

## Deterministic failures and safe actions

The private protocol fixes these terminal categories:

- `manifest_invalid` / `unsupported_deployment`: repair or select a compatible
  deployment before retrying;
- `range_unsupported` / `cors_headers_hidden` / `content_encoding`: repair the
  host response contract;
- `source_changed`: discard the old deployment/cache binding and publish a new
  manifest after rebuilding the compatible index;
- `range_truncated` / `range_corrupt` / `index_incompatible`: discard the bad
  response or index deployment and retry only after repair;
- `offline` / `retry_exhausted`: wait for connectivity or server recovery and
  start a new operation;
- `cache_quota` / `cache_unavailable`: explicitly select `memory` or `none`, or
  free origin storage, then start a new operation;
- `cancelled`: start a new operation only if the caller still wants the View;
  and
- `worker_failed`: terminate the worker, keep the current rendered frame, and
  create a new worker before retrying.

No failure silently downloads the full Source, combines partial operations, or
changes cache policy.

## Local acceptance fixture and harness

The repository generator deterministically produces one attributed LAS 1.2
fixture with more than one index leaf, its fully verified `SourceRecord`, one
compatible disk-v2 inspection index, and the deployment manifest. Verification
regenerates these artifacts in an isolated temporary directory and compares all
committed bytes and manifest facts.

The local static server implements the exact Range/CORS/validator contract and
bounded fault routes used by protocol tests. The browser harness:

1. completes the inherited v0.15 WebGPU lifecycle smoke;
2. cancels one delayed manifest Fetch and receives `cancelled` within 1,000
   milliseconds;
3. starts a cold persistent-cache remote operation;
4. proves the Source probe and index header/root/sample requests are bounded;
5. renders Sampled Coverage before complete Source transfer;
6. shuts down and recreates the viewer and worker;
7. completes a warm-cache operation with identical Source identity and Point
   ordinals without refetching cached index bytes; and
8. publishes `PASS`, `UNSUPPORTED`, or `FAIL` plus inspectable streaming and
   renderer diagnostics.

Native Rust tests cover manifest/batch/resource/render publication rules.
JavaScript module tests cover response validation, retry classification,
backpressure, cache-key isolation, corruption, cancellation, and failure
mapping without requiring a browser. Browser execution remains required for
real Worker, Cache API, Fetch/CORS, WebAssembly, WebGPU, and lifecycle evidence.

## Explicit non-goals

v0.16 does not add:

- arbitrary raw LAS/LAZ URL loading, full-file fallback, sequential LAZ replay,
  COPC, EPT, 3D Tiles, a general remote `point-source` adapter, or Source
  rewriting;
- arbitrary index traversal, leaf Source decoding, exact CPU Query, picking
  confirmation, editing, multiple Sources, or Workspace persistence;
- service-worker ownership, cache eviction policy, background sync, credentials
  storage, signed URLs, authentication, telemetry, CDN configuration, or hosted
  infrastructure;
- a public viewer or networking API, npm package, bundler/framework adapter,
  compatibility promise, or automatic renderer/device recovery; or
- visual-quality changes, navigation policy, application UI, broad browser or
  device qualification, production performance, independent adoption, support
  qualification, or browser-engine release-candidate claims.

## Verification and completion

Repository completion requires:

- deterministic native tests for streaming manifest, decoded batch, renderer
  publication, identity, Coverage, and all fixed resource ceilings;
- deterministic JavaScript tests for HTTP, retry, cancellation, cache,
  backpressure, decode, and failure contracts;
- fixture regeneration equivalence and local Range-server contract tests;
- `wasm32-unknown-unknown` checks and a clean `wasm-bindgen` browser build;
- the cold/recreation/warm-cache browser harness passing on the exact recorded
  browser, operating system, adapter, and local server;
- all existing workspace formatting, linting, tests, rustdoc, package, fuzz,
  benchmark, example, and forced-GPU commands from `CONTRIBUTING.md`;
- updated architecture, package, guide, changelog, roadmap, context, and
  contribution documentation; and
- a verification record separating deterministic byte/work ceilings and local
  observations from unsupported external evidence.

No hosted CI is added. The completed release wording will be: **Complete and
repository-verified for one bounded immutable-LAS HTTP Range, browser-cache,
and worker-decoding slice; arbitrary Source delivery, exact browser Queries,
broad browser qualification, independent adoption, SDK stability, and support
qualification outstanding.**
