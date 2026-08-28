# Browser integration known limitations

Punctra `0.20.0-alpha.1` is complete only for the bounded packed integration
baseline and one exact local Chromium/macOS/Apple-GPU consumer lane. The
[machine-readable baseline](../releases/v0.20-browser-baseline.json) and
[browser matrix](../releases/v0.20-browser-matrix.json) are the authorities.

## Unsupported

These conditions cannot create a usable viewer in v0.20:

- insecure context, unavailable WebGPU/adapter, unsupported surface format,
  presentation mode, alpha mode, renderer limit, or invalid physical viewport;
- Source hosting without the required bounded byte ranges, exact lengths,
  identity encoding, strong validators, range digests, and exposed response
  headers;
- arbitrary LAS/LAZ URLs, LAZ decompression in the browser, arbitrary hierarchy
  traversal, multiple active Sources, general exact Queries, WebGL, Canvas,
  software rendering, or a reduced-feature fallback; and
- continuing after partial publication, device loss, surface loss, or another
  fused renderer failure without explicitly disposing and recreating.

Unsupported initialization returns a structured failure. Hosts must explain it;
they must not silently substitute a different renderer, Source, or cache mode.

## Unqualified

Only the exact entry in `v0.20-browser-matrix.json` is repository-qualified.
Installed Chrome, Safari, other Chromium builds, other screens, operating
systems, GPUs, adapters, mobile devices, bundlers, framework versions, CDNs,
authentication stacks, CSP variants, and production networks remain
unqualified until their own attended evidence is recorded.

Initialization success does not promote an unqualified platform to supported.

## Deferred

The following are intentionally outside this integration baseline:

- complete Source Coverage, dense/sparse/layered/mixed-LOD visual corpus,
  stable visual metrics, final display policy, and visual-quality sign-off;
- editing, terrain workflows, export, host UI, offline-first behavior, service
  workers, telemetry, application persistence, and automatic recovery;
- registry or CDN publication, independent-adopter completion, setup-time and
  adopter-friction evidence, API stability, support operations, beta,
  release-candidate, v1, and compatibility promises; and
- physical GPU completion, process RSS, physical cache allocation, driver/GPU
  allocation, energy, thermals, or general remote-network performance claims.

The fixed generated scene and sampled LAS root preserve functional and
appearance continuity; they are not the representative visual corpus planned
for v0.21.

## Host-owned

Applications remain responsible for canvas/layout, DPR and resize decisions,
visibility, camera/navigation policy, credentials, Source allowlists, CSP,
authentication/authorization, cache and telemetry consent, retry UI, issue-data
redaction, and teardown. Presentation picks and highlights remain provisional;
only a configured exact authority may confirm a Source record.

See the [recovery playbook](browser-qualification.md#host-recovery-playbook) for
the retry-in-place and recreation-required boundary.
