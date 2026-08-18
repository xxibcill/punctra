# View Your First LAS/LAZ Source

This guide exercises Punctra's v0.10 inspection View without changing the
Source. Allow about five minutes after the pinned Rust toolchain and workspace
dependencies have been built once.

## 1. Choose a Source and two cache paths

Use a local LAS or supported LAZ file that you are permitted to inspect. Keep
the position-only and attributed indexes at different paths:

```bash
SOURCE=/absolute/path/to/survey.laz
CACHE_DIR=/absolute/path/to/cache
mkdir -p "$CACHE_DIR"
POSITION_INDEX="$CACHE_DIR/survey.position-v1.pidx"
INSPECTION_INDEX="$CACHE_DIR/survey.inspection-v2.pidx"
```

Punctra reads LAS point-data record formats 0–10 and LAZ formats 0–8. LAZ
formats 9 and 10 are rejected because their waveform values are not yet
preserved exactly. The Source is immutable; `.pidx`, `.pidx.work`, and owned
`.pidx.samples.*` temporaries are rebuildable local cache state.

Explicit cache paths make ownership clearest. If you omit them, neutral and
elevation append `.pidx` to the Source path, while RGB, intensity, and
classification append `.inspection-v2.pidx`, so the incompatible recipes stay
separate.

## 2. Verify the file before opening a window

Inspect the Source metadata and then run the GPU-free bridge smoke:

```bash
cargo run --release -p source-las --example inspect -- "$SOURCE"
cargo run --release -p renderer-demo -- \
  --smoke "$SOURCE" "$POSITION_INDEX"
```

The smoke command Full-verifies the Source, builds or opens the position-only
disk-v1 index, plans one node, and accepts one atomic CPU-model renderer
Upsert. It does not require a GPU. A successful message does not mean every
Source Point is resident or that an exact Query completed.

## 3. Open the interactive View

Start with elevation and an orthographic camera:

```bash
cargo run --release -p renderer-demo -- \
  --display elevation --projection orthographic \
  "$SOURCE" "$POSITION_INDEX"
```

The available controls are:

- left drag: orbit;
- middle drag: pan;
- mouse wheel: zoom;
- `P`: switch between perspective and orthographic while preserving the
  target-plane scale;
- `R`: reset the camera while keeping the selected projection;
- `H`: toggle stable-identity highlight coloring;
- Space: freeze or resume planner, request, materialization, and retirement
  work while keeping the current resident display visible; and
- Escape: exit.

The on-canvas panel labels resident Coverage as `Sampled`, `Complete`, or a
mix alongside the display, projection, loading state, Point counts, selection,
orientation, scale, cursor, and palette facts. The compact title contains only
the package-derived View name/version. Detailed LOD demand, candidates, issued
requests, retention, retirement, queue/staging, resident-node, timing, and
resource facts are printed as a separate terminal transcript. Pausing freezes
View lifecycle work; it does not turn the current partial display into an exact
result.

## 4. Try Source Attributes

RGB, intensity, and classification use the disk-v2 inspection recipe:

```bash
cargo run --release -p renderer-demo -- \
  --display rgb "$SOURCE" "$INSPECTION_INDEX"
cargo run --release -p renderer-demo -- \
  --display intensity "$SOURCE" "$INSPECTION_INDEX"
cargo run --release -p renderer-demo -- \
  --display classification "$SOURCE" "$INSPECTION_INDEX"
```

The same v2 target can be reused by all three attributed modes. Intensity must
be LAS Attribute 1 as `U16`, classification must be Attribute 6 as `U8`, and
RGB must be the all-or-none `U16` Attributes 16, 17, and 18. RGB mode fails
clearly when all three channels are unavailable. It never substitutes
intensity or neutral color.

Disk v1 and v2 targets are deliberately incompatible. If a target was built
with the other recipe, choose the matching path above or move aside/delete the
whole rebuildable index family yourself before rebuilding. Punctra never
silently replaces an incompatible cache.

The display mapping is exact and presentation-only:

- neutral is `[190,205,220,255]`;
- elevation normalizes exact world Z against the complete Source bounds,
  clamps to `[0,1]`, uses `0.5` for zero extent, and interpolates the fixed
  `[68,1,84]`, `[59,82,139]`, `[33,145,140]`, `[94,201,98]`, and
  `[253,231,37]` stops at quarters with nearest-byte rounding;
- each RGB channel and intensity `U16` value `v` becomes
  `(v * 255 + 32767) / 65535` by integer division; intensity repeats that byte
  across RGB; and
- classification 0–18 uses the [fixed v0.10
  table](../design/field-inspection-view-v0.10.md#implemented-display-mappings-and-cli),
  while 19–255 uses wrapping `u8` arithmetic
  `(73c+41, 151c+97, 199c+17)`.

Alpha is always `255`. None of these bytes assigns new Source semantics or
changes Point Identity, exact ticks, geometry, or Coverage.

## 5. Interpret colors and failures correctly

`neutral`, `elevation`, `rgb`, `intensity`, and `classification` change only
disposable display bytes. Point Identity, exact Source ticks, geometry,
Coverage, Query, Workspace Edits, terrain, QA, and export remain
CPU-authoritative.

A View failure is printed as a stable `PVIEW_*` code, an owning phase, bounded
detail, and exactly one safe action. For example, use `--smoke` to isolate a
GPU setup failure, but do not bypass a Source or index validation failure.

A conflicting corpus report is never replaced. Preserve it, choose a fresh
report path and fresh index target for every manifest entry, then rerun; the
completed timed entry cannot be honestly replayed as another cold build against
its now-populated index path.

## Optional: record a local viewing run

Copy the [example field-corpus manifest](field-corpus.example.json), replace
every placeholder, and leave both permission fields true only when you have
explicit authority to inspect and measure that Source. Then run:

```bash
cargo run --release -p renderer-demo -- corpus \
  --manifest /private/path/to/field-corpus.json \
  --report /private/path/to/viewing-report.json
```

The corpus runner requires a local wgpu adapter. Its bounded no-replace report
records declared machine labels, observed adapter/backend, Source identity and
point count, index versions/disposition, operation timings, resources,
Coverage, navigation-trace facts, failures, and explicit nonclaims. It omits
Source/index paths and opaque project/firm identifiers, but it retains Source
identity and machine facts that may still be sensitive. Do not publish a
manifest or report without permission.
Use a fresh absent index path for every entry: the runner requires a real cold
build, then records a separate immediate warm open. It preserves and rejects an
existing or resumable index rather than deleting or mislabeling it.
Use a new report target for a new timed run; a different existing report is
never overwritten.

For the pre-v0.13 field lane, set `pre_v0_13_qualification` to `true` only in a
private manifest containing at least five permitted projects from three firms,
all five display modes in both projections, and the complete bounded
known-feature outcome set. The runner then ignores the short fixed-frame
observation count as a stopping condition: every pose must settle within
`settlement_frame_ceiling` and remain quiet for 300 rendered frames. A report
records manifest outcomes under `declared_known_feature_outcomes` and reports
their counts with `declared_*` keys. Its explicit
`declared_known_feature_outcomes_verified: false` nonclaim prevents those
inputs from being mistaken for observed acceptance.

This local runner is a reproducible measurement path, not evidence of a
production corpus, professional preference, terrain capacity, partner
acceptance, downstream interoperability, or human-time savings.
