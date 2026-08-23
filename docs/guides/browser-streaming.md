# Local Browser HTTP Range Streaming Guide

Punctra v0.16 extends the private browser acceptance host with one bounded
immutable-LAS deployment. It proves strict HTTP Range transport, identity-
versioned browser caching, worker decoding, and progressive WebGPU publication
for the checked-in fixture. It does not load arbitrary URLs or define the later
public viewer/SDK.

## What the deployment contains

`apps/browser-demo/web/fixtures/v1` contains four immutable files:

| File | Role |
|---|---|
| `representative.las` | Deterministic LAS 1.2 format-3 representation with 70,000 attributed Points. |
| `source-record.json` | Complete `source-las` verification record that establishes Source identity before deployment. |
| `representative.pidx` | Compatible 172,808-byte disk-v2 inspection index with a 4,096-sample root. |
| `deployment.json` | Trusted v1 binding for URLs, lengths, strong ETags, Source identity, transform, root layout, and SHA-256 range digests. |

The browser requests a 256-byte Source probe, the 408-byte index header/root,
and the 172,032-byte root sample block. It does not request the remaining
2,379,971 Source bytes. Root samples are Sampled Coverage and remain
non-authoritative even though their Point identities retain exact Source
ordinals.

Regenerate every artifact in an isolated temporary directory and compare it
with the committed bytes:

```bash
cargo run -p browser-demo --bin generate_stream_fixture
```

Maintainers intentionally replacing the fixture use the explicit write mode,
then inspect and commit every changed byte and manifest fact:

```bash
cargo run -p browser-demo --bin generate_stream_fixture -- --write
```

## Build and strict local server

Install the pinned `wasm32-unknown-unknown` target and `wasm-bindgen-cli`
0.2.127 as described in the [browser foundation guide](browser-foundation.md),
then run:

```bash
scripts/build-browser-demo.sh
scripts/serve-browser-demo.py --port 8000
```

Open [http://127.0.0.1:8000/](http://127.0.0.1:8000/). Localhost is a browser
secure context. Do not use `file://`, and do not replace the repository server
with a generic static server for qualification. The v0.16 server supplies:

- exact single-range `206` responses and `Content-Range` totals;
- strong SHA-256 ETags matching the deployment manifest;
- `Accept-Ranges: bytes`, exact `Content-Length`, and identity encoding;
- CORS permission and exposure for every inspected response header;
- immutable/no-transform caching for fixture bytes and no-store behavior for
  changing host/build assets; and
- no redirect, suffix-range, multipart-range, or content transformation.

## Automatic acceptance sequence

The page first repeats the v0.15 generated lifecycle checks: WebGPU capability,
deterministic planning/publication, visible render, bounded resize, hidden-frame
suppression, provisional centre pick, fused shutdown, and explicit recreation.

It then runs the v0.16 path:

1. cancel one deliberately delayed manifest Fetch and require a deterministic
   `cancelled` acknowledgement within 1,000 milliseconds;
2. create one module Worker with persistent cache policy and explicit exact-
   namespace invalidation;
3. fetch and validate the bounded manifest;
4. probe 256 Source bytes and verify the strong validator and SHA-256;
5. fetch, hash, and independently decode the disk-v2 header/root record;
6. fetch, hash, decode, and transfer the root sample range in four 1,024-Point
   buffers;
7. reset to the v0.16 View generation, publish each bounded renderer batch, and
   render Sampled Coverage before complete Source transfer;
8. destroy and recreate the viewer and Worker; and
9. repeat with the exact persistent-cache namespace, requiring three verified
   cache hits and zero binary network requests.

A successful document publishes:

```text
PASS — WebGPU lifecycle, bounded remote ranges, worker decode, and warm-cache isolation verified locally.
```

The raw acceptance record includes the complete renderer diagnostics plus
the cancellation acknowledgement plus separate cold and warm transport metrics.
Record the browser, platform, WebGPU
backend/adapter label, surface, viewport, request and byte counts, cache hits,
queue and response high-water, worker staging, transferred batches/Points/
bytes, largest main-thread batch, observed main-thread publication duration,
canvas accounting, renderer logical residency, and transient textures.

Observed milliseconds are local diagnostics, not a stable performance limit.
The deterministic main-thread acceptance limit is one 1,024-Point/24,576-byte
publication per task.

## Cache policy

The private worker accepts exactly `none`, `memory`, or `persistent`. Cache
namespaces and entry URLs include the schema, deployment identity, Source
identity, strong validator, index digest, resource kind, and exact range. A
cached response repeats and revalidates those facts plus its SHA-256 before
decode.

Explicit invalidation deletes only the exact derived namespace. Cache quota or
API failure returns `cache_quota` or `cache_unavailable`; the host must
explicitly retry with `memory` or `none`. The worker never silently downgrades
policy or combines bytes from another Source binding.

## Failure checks

Run the deterministic transport and worker-protocol suite:

```bash
node --test apps/browser-demo/web/streaming-protocol.test.mjs
```

It covers successful cold and warm paths, identity-separated cache keys, a
full `200` response to a Range request, validator drift, truncation, digest
corruption, bounded retry success/exhaustion, offline state, cancellation,
quota failure, worker-failure mapping, and rejection of incomplete/bare
deployments before binary Fetch.

The browser exposes one safe action for each terminal code. Recovery creates a
new operation and, when required, a new Worker/viewer. It never downloads the
complete Source as fallback.

## Evidence boundary

This local result qualifies only the exact recorded browser, operating system,
WebGPU environment, server, fixture, and implementation. It does not establish
browser process/heap or physical cache allocation, general LAS/LAZ decoding,
hostile-server authenticity, credentials or service-worker policy, exact CPU
Queries, editing, public API stability, broad compatibility, adoption, support,
or release-candidate readiness.
