# Adaptive View Planning v0.2

Status: accepted implementation scope

Punctra v0.2 adds a renderer-neutral **point-view** module between a host's
spatial hierarchy and **render-protocol**. Its one job is to turn a frozen
camera, viewport, hierarchy snapshot, and hard residency budget into a
deterministic `ViewPlan`.

The module performs no file I/O, starts no background work, and owns no GPU
resources. A host remains responsible for materializing requested nodes and
for explicitly applying batches and retirements to a renderer.

## Primary interface

The public seam has one stateful operation:

```rust,ignore
let plan = planner.plan(&camera, viewport, available_nodes, budget)?;

for retirement in plan.retirements() {
    renderer.apply(&retirement.render_update())?;
}
for request in plan.requests() {
    host.request(request.node());
}
```

`ViewPlanner` retains only hysteresis history. Hierarchy topology, materialized
data, in-flight requests, renderer residency, and scheduling remain host-owned
and are reported back in the next immutable node snapshot.

`ViewPlan` contains:

- missing nodes to request, ordered by descending visual priority with a stable
  node-key tie break;
- resident nodes that must be retained for selected detail or fallback
  Coverage; and
- resident batches safe to retire, each carrying the exact View generation,
  batch key, and expected batch version required by `render-protocol`.

The retained and retirement lists use stable key order. Input slice order,
worker completion order, and hash iteration cannot affect a plan.

## Planning rules

### Visibility

Node bounds are tested against the perspective camera frustum in 64-bit world
coordinates. A node outside any frustum plane is excluded. Intersecting nodes
remain candidates so culling cannot introduce a false-negative hole at a clip
or side plane.

### Level of detail

Projected geometric error is measured in physical viewport pixels. A visible
node refines when its screen-space error crosses the upper hysteresis threshold
and coarsens only after crossing the lower threshold. Decisions inside the
dead band retain the previous accepted refinement state.

The target cut begins at visible hierarchy roots. Refinements are considered
in descending screen-space error order with a stable node-key tie break.

### Hard budgets

Every node declares its point and byte cost and represents one render batch.
The planner checks point, byte, and batch limits independently before emitting
a request. It reserves costs for already requested work as well as newly
requested work.

A refinement must fit its transition footprint, not only its final footprint.
Until all visible replacement descendants are resident, that footprint
includes the resident ancestor that still supplies Coverage. This prevents a
plan that fits after replacement but cannot be applied through the renderer's
atomic single-batch update interface.

If retained or in-flight work already exceeds a newly lowered budget, the
planner emits no request that increases the excess. It does not discard the
last visible Coverage merely to force current residency under the new limit;
the host must choose a new generation or otherwise resolve that pressure.

### Progressive Coverage

A resident parent remains retained while any selected visible replacement is
missing or merely requested. It becomes safe to retire only after every such
replacement is resident. The inverse rule applies while coarsening: resident
descendants remain until the selected parent is resident.

Invisible and redundant resident batches may retire immediately. Retirement
is advisory and explicit; **point-view** never mutates renderer state.

### Generations

One node snapshot belongs to exactly one `ViewGenerationKey`. Changing that key
clears hysteresis history. Every request and retained/retirement token copies
the snapshot generation. A retirement also copies the observed resident batch
version, so a delayed plan cannot remove a newer replacement.

## Validation and errors

The shared `render-protocol::Viewport` contract rejects empty physical extents.
Planning rejects malformed hierarchy snapshots before updating hysteresis:

- duplicate node or batch keys;
- missing, self-referential, or cyclic parent links;
- child bounds outside their parent;
- non-finite or inverted bounds;
- negative or non-finite geometric error;
- invalid node costs; and
- resource-accounting overflow.

An empty hierarchy produces no requests, retained nodes, or retirements. A
fully culled hierarchy produces no requests or retention; resident batches are
returned as conditional retirements, while already requested work remains in
the reported resource usage.

## Demo and verification

The renderer demo uses a deterministic synthetic quadtree representing more
than 10 million logical Points. It materializes only planner requests and keeps
the renderer at fixed point, byte, and batch limits while orbiting and zooming.

Local verification includes:

- interface-level CPU tests for culling, screen-space error, all three budgets,
  fallback Coverage, coarsening, hysteresis, deterministic ordering, and exact
  retirement tokens;
- an optimized CPU planner benchmark over a multi-level synthetic hierarchy;
  and
- a headless planner-to-renderer GPU acceptance test that exercises coarse
  Coverage, child replacement, exact retirement, and bounded rendering.

GPU acceptance uses `PUNCTRA_REQUIRE_GPU=1`, as documented in
[`CONTRIBUTING.md`](../../CONTRIBUTING.md), so a missing local adapter fails the
required gate.

## Out of scope

Punctra v0.2 does not add:

- LAS or LAZ decoding;
- Spatial Index construction or persistence;
- network transports, caches, or request cancellation;
- automatic renderer eviction;
- occlusion culling or exact visible-only selection; or
- visual effects such as eye-dome lighting.

Those concerns remain in hosts or future modules. A View remains progressive
display Coverage, never an exact Query or authoritative Point geometry.
