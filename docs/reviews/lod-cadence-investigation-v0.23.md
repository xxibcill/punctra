# v0.23 browser cadence investigation

The original historical-cadence gate rejected two final-pin full runs for
generated-elevation-layered-orthographic, recreation index 1: 34.30 and 34.40 ms
versus twice the historical 17.10-ms p95. Neither run exported an accepted
archive. An earlier final attempt also failed a historical submission ratio.

The [raw diagnostic record](lod-cadence-investigation-v0.23.json) alternates
three frozen v0.22 and three v0.23 renderers after a no-renderer scheduling
control. Each observes 150 callbacks with the same Source, orthographic camera,
640×480 physical viewport, DPR 2, elevation mapping and sampling loop. The
frozen loaded JS and Wasm bytes match the immutable accepted v0.22 runtime
pins. Every renderer capture has decoded SHA-256
`d5f90a90519c1bee5d9ea843b30d0665195877a3bff91d4eb2e458a275980a96`,
matching the historical elevation canonical image.

| Run | Callback median / p95 (ms) | Submission p95 (ms) |
|---|---|---|
| No renderer | 33.30 / 35.20 | No rendering |
| v0.22, recreation 0 | 33.30 / 35.10 | 0.30 |
| v0.23, recreation 0 | 33.30 / 34.70 | 0.30 |
| v0.22, recreation 1 | 33.30 / 34.50 | 0.30 |
| v0.23, recreation 1 | 33.30 / 35.00 | 0.30 |
| v0.22, recreation 2 | 33.30 / 34.30 | 0.30 |
| v0.23, recreation 2 | 33.30 / 34.40 | 0.30 |

The shared scheduling cadence and equal submission p95 support using matched
session controls rather than attributing the historic callback-rate difference
to the renderer change. The cause of the browser scheduler's changed cadence
is not established. Screen facts are browser observations, not physical panel
measurements. This diagnostic is not accepted release qualification.

The accepted design now requires a pinned same-session v0.22 control beside
every canonical recreation, with unchanged 50-ms/16.7-ms absolute p95 ceilings
and a two-times paired ratio. Historical submission ratios remain enforced.
Both record and independent verify stages reproduce all paired captures.
