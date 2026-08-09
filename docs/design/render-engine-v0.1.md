# Punctra render engine v0.1

Status: implemented

## Target

Punctra v0.1 is an embeddable Rust and wgpu module for progressive display of
very large point sets. It accepts renderer-neutral, generation-safe updates,
keeps logical point-vertex residency within caller-selected hard limits, and
records drawing commands into a host-owned command encoder.

## Interface

The renderer has three operations:

1. apply a complete logical update;
2. record one frame into a caller-owned command encoder; and
3. encode an asynchronous pick request tied to that exact frame.

The interface does not expose buffers, bind groups, pipelines, shaders, or
residency tables. Updates use one validated point representation in v0.1.
Runtime schemas, custom shaders, and alternative residency policies require a
second real caller before they become public seams.

## Invariants

- A View generation begins with exactly one reset.
- Updates for stale View generations are rejected.
- A batch version increases monotonically within one View generation.
- A conditional removal cannot remove a newer batch version.
- An upsert is atomic: a frame sees either the previous complete batch or the
  replacement complete batch.
- Absolute positions are represented by one finite 64-bit world origin and
  finite origin-relative 32-bit positions.
- Resident point vertices never exceed the configured estimated-byte, point,
  or batch limits. Per-batch uniforms, depth and pick targets, allocator
  padding, and CPU bookkeeping are outside this point-residency model.
- Protocol or device buffer-limit violations are reported as errors; active
  data is not silently evicted.
- Rendering records GPU commands but performs no file access, decoding, or
  command submission.
- Picking reports the View generation and batch version that produced the hit.

## v0.1 acceptance

- Synthetic point batches render through wgpu with depth and circular splats.
- Reset, upsert, replacement, conditional removal, and stale-update behavior
  pass through both the CPU reference state and the GPU renderer interface.
- Large world origins preserve nearby point separation after rebasing.
- Configured point-residency limits are enforced before publication.
- Highlighting and asynchronous point picking retain caller point identities.
- An interactive demo streams data, moves the camera, and reports resident
  points, resident bytes, upload bytes, draw calls, and frame encoding time.
- Offscreen tests cover empty input, one point, occlusion, generation changes,
  highlighting, and large-coordinate precision on an available GPU adapter.

## Non-goals

- file decoding or Source identity;
- spatial indexing or LOD selection;
- editing and Workspace persistence;
- terrain derivation or export;
- window or event-loop ownership;
- source-scale I/O during rendering;
- runtime point schemas, user shaders, or a public plugin interface; and
- mesh rendering.
