# Local Browser Foundation Guide

Punctra v0.15 includes one private static browser host for locally verifying the
bounded WebAssembly/WebGPU foundation. It renders generated in-memory data; it
does not fetch or decode a LAS/LAZ file in the browser. Remote Source delivery
begins only after a separate accepted v0.16 design.

## What this host proves

The host compiles `render-protocol`, `point-view`, and `render-wgpu` through the
`wasm32-unknown-unknown` target, creates a WebGPU canvas path without WebGL or a
native renderer shim, and checks:

- secure-context and `navigator.gpu` availability;
- compatible canvas surface, adapter, device, format, and FIFO presentation;
- one deterministic View-planner request followed by resident retention;
- generation-safe publication of one 1,089-Point batch;
- bounded physical resize from explicit CSS-size and device-pixel-ratio facts;
- rendering suspension while the host declares the canvas hidden;
- a provisional centre-pixel pick retaining View generation, batch, batch
  version, and Point identity;
- fused shutdown followed only by explicit recreation; and
- separate logical vertex, surface, and renderer transient-texture accounting.

The generated display and its pick are GPU evidence only. A host reading a real
Source must confirm the identity and exact values against its caller-owned CPU
authority before inspection or editing.

## Prerequisites

Install the pinned Rust toolchain and target:

```bash
rustup target add wasm32-unknown-unknown
```

Install the CLI version matching the repository's exact `wasm-bindgen`
dependency:

```bash
cargo install wasm-bindgen-cli --version 0.2.127 --locked
```

Use a WebGPU-capable browser in a secure context. `http://127.0.0.1` and
`http://localhost` are accepted local secure contexts in the declared harness.
Opening `index.html` directly through `file://` is unsupported.

## Build and serve

From the repository root:

```bash
scripts/build-browser-demo.sh
python3 -m http.server 4173 --bind 127.0.0.1 \
  --directory apps/browser-demo/web
```

Open [http://127.0.0.1:4173/](http://127.0.0.1:4173/). The build script writes
generated ES-module bindings and WebAssembly into the ignored
`apps/browser-demo/web/pkg` directory. No npm installation, bundler, framework,
or development server is required.

The page runs its acceptance sequence automatically. A successful run reports:

```text
PASS — browser WebGPU lifecycle and invariants verified locally.
```

The visible instrument record shows the browser, adapter/backend, surface
format, physical viewport, logical residency, separately accounted surface and
transient texture bytes, rendered frames, and provisional-pick state. The raw
diagnostic disclosure contains the complete `punctra-browser-foundation-v1`
record.

## Host ownership

The JavaScript page owns the canvas element, CSS placement, accessibility text,
device-pixel-ratio choice, visibility and animation-frame policy, resize
observations, user controls, error presentation, and the decision to recreate
or shut down.

The private Rust browser adapter acts as the host of the existing public
`render-wgpu` contract. It owns the WebGPU instance, surface, adapter, device,
queue, surface configuration, encoders, submissions, presentations, and pick
readback for that canvas. `WgpuRenderer` still receives a caller-owned device,
records into caller-owned encoders/targets, and never submits the queue itself.

The example `wasm-bindgen` boundary is private and may change. It is not the
supported JavaScript/TypeScript SDK planned by the roadmap.

## Fixed resource ceilings

| Resource family | Acceptance ceiling |
|---|---:|
| Logical resident Points | 2,048 |
| Logical point-vertex bytes | 49,152 |
| Resident batches | 4 |
| Highlight input Points | 32 |
| Generated scene | 1,089 Points / 26,136 bytes / 1 batch |
| Physical canvas dimension | 4,096 pixels per axis |
| Physical canvas area | 8,388,608 pixels |
| Caller-declared device-pixel ratio | `0 < dpr <= 4` |
| Surface byte accounting | 4 bytes per physical pixel |
| Renderer transient textures | 67,108,864 bytes |
| Presentation latency hint | 2 frames |

Logical vertex bytes exclude canvas textures, depth/pick targets, browser heap,
driver allocation, staging, command storage, and allocator padding. Surface
bytes and renderer transient bytes are reported separately. None of these
algorithm-accounted values is a browser-process or GPU-memory measurement.

An invalid or over-limit resize fails before changing canvas dimensions or
surface configuration. The host does not silently reduce device-pixel ratio,
sample the scene, or merge independent resource families into one limit.

## Unsupported and recovery states

Every failure carries a code, explanation, and one safe action. The main
classes are:

| Failure | Safe host action |
|---|---|
| Missing secure context, `navigator.gpu`, compatible adapter, device, format, or FIFO presentation | Keep the canvas unavailable; correct the browser/context/device, then explicitly retry initialization. |
| Invalid or over-limit CSS size or device-pixel ratio | Keep the current surface configuration; choose bounded values and resize again. |
| Hidden or occluded canvas, or surface timeout | Keep the last presented frame; wait until visible and request another frame. |
| Surface or device loss, validation, renderer failure, or readback failure | Destroy the viewer and explicitly create a new viewer. |
| Shutdown viewer | Create a new viewer before any later work. |

There is no WebGL fallback, automatic device recreation, remote retry policy,
cache recovery, or hidden reuse of a partially initialized viewer.

## Evidence boundary

Passing this host establishes one local generated browser path. It does not
establish:

- browser LAS/LAZ loading or HTTP Range/CORS behavior;
- Web Worker decoding, cache policy, offline behavior, or real Source latency;
- observed JavaScript heap, process memory, driver allocation, or energy use;
- broad browser, operating-system, adapter, mobile, or support qualification;
- exact CPU Queries, editing, terrain, or QA in the browser; or
- independent embedding, SDK stability, package publication, production use,
  or release-candidate status.
