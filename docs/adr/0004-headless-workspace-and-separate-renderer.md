# ADR-0004: The Workspace is headless

## Status

Proposed

## Context

The foundation should support a desktop viewer, CLI, tests, bindings, and other software. If Workspace behavior owns GPU devices, shaders, window state, or rendering caches, every non-graphical caller inherits those dependencies and graphics failures can contaminate document lifecycle.

## Decision

Keep **point-workspace** headless. Its job is coherent Source, Spatial Index, Revision, Snapshot, and Query lifecycle.

Keep View preparation in **point-view**, generation-safe scene values in **render-protocol**, and GPU resource ownership in **render-wgpu**. Application adapters compose these modules.

## Consequences

- CLI, tests, servers, and bindings use the Workspace without compiling a graphics stack.
- Render device loss cannot corrupt logical state.
- A CPU or different graphics adapter can consume the same View Batches later.
- The desktop adapter performs more explicit composition.
- Some zero-copy renderer specialization is sacrificed to preserve a stable, reusable seam.
