# v0.22 closeout investigation

Status: **Qualification failed; corrective scope proposed**

PR #23 merged the implementation, but the mandatory record stage still fails.
The v0.22 roadmap remains Active and its release record remains Pending.
This investigation is diagnostic work, not a substitute for the pinned TAR
record/verify workflow or final release evidence.

## Reproduction

The follow-up branch starts at merge `e3a324e542dd02b9ddbef2f889ab1ac980eb99fc`
and preserves the local fixture-layout correction and eye-dome pass refactor.
The first record attempt used `dc07391`; the diagnostic repeat used `e760c5d`.
Both were built locally and used a fresh local GPU artifact from their clean
implementation commit, the strict server, and the visible trusted Run button.
The second attempt's session label was `codex-local-20261002-diagnostics`.
The server was restarted to bind that second implementation commit.

The host reports macOS 26.6.2 (25G83), Apple M5 Pro, 15 CPU cores and 16 GPU
cores. The browser reported the generic WebGPU adapter / BrowserWebGpu lane.
Canonical capture was 640 × 480 physical pixels, requested DPR 2.

The initial run had intermittent timing failures. The diagnostic repeat had no
canonical timing failures, but reproduced all five feature-centroid failures
in all three recreations. Four canonical trials passed. All nine focused
scale cases failed. The attended resource fallback passed. No accepted
baseline manifest or final verify evidence was produced.

## Observed failures

The accepted design requires the foreground centroid of each inherited
feature to remain within one physical pixel of v0.21. The diagnostic repeat
reported these distances identically across all three recreations:

| Trial | Distance in physical pixels | Display diameter |
|---|---:|---:|
| generated-rgb-hdr-perspective | 1.213914586 | 6 |
| autzen-rgb-perspective | 2.041961044 | 4.7631397 |
| autzen-classification-perspective | 2.041961044 | 4.7631397 |
| autzen-intensity-perspective | 2.551914786 | 4.7631397 |
| autzen-elevation-perspective | 2.041961044 | 4.7631397 |

For Autzen RGB, the bound region changed from 7,028 foreground pixels with
centroid (292.863546, 269.280876) to 5,137 with centroid
(290.891960, 269.812342). The foreground ratio remains inside the accepted
range. The centroid failure is independently gating and is not waived.
Shrinking overlapping footprints can move a binary occupancy centroid without
moving Point centers; that is a hypothesis to test with a matched-footprint
control, not evidence that the existing gate passed.

The focused corpus labels Points as isolated although their neighbors overlap
the measurement region. A CPU projection of the unchanged authored positions
and camera establishes, at canonical DPR 2:

| Trial / measured Point | Nearest other Point | Separation in physical pixels |
|---|---:|---:|
| neutral / 1866 | 1867 | 2.938772398 |
| neutral / 1913 | 2006 | 1.541725052 |
| neutral / 1961 | 1960 | 3.281207788 |
| intensity / 1913 | 2008 | 2.674745039 |
| intensity / 1962 | 1963 | 5.439713859 |
| classification / 1866 | 1867 | 2.938772398 |
| classification / 2005 | 2007 | 1.674414235 |

The display diameter in these canonical trials is six pixels. A measurement
against a single ideal disk therefore also contains neighboring disks. For
neutral Point 1913, observed integrated coverage was 122.510114 pixels versus
28.289063 for the ideal disk, with RMSE 0.720562 against the 0.18 ceiling.
These observations cannot qualify an isolated-footprint kernel.

At DPR 1, the fixed seven-pixel pick footprint overlaps additional nearer
Points. The classification probe expected 1866 but observed 1867–1869 across
its nine sampled pixels; the probe expected 2005 but observed 2000, 2001, or
2003. This does not by itself show a picking regression. The fixture must
establish the visible depth winner at the tested pixel.

## Repairs in this follow-up

- Preserve failed canonical, focused, and fallback measurements in the
  visible error record, explicitly labelled `diagnostic_only`. A failed run
  still cannot create an accepted baseline.
- Compare every host density diameter at f32 precision. Rust serialized
  3.5171874 for the DPR 1 case; its exact f32 value is represented differently
  in JavaScript. The static verifier already rounded before comparison, but
  one browser-runner branch used direct number equality. A shared comparison
  now covers all three call sites and rejects a different adjacent f32 value.

## Local verification

The full 68-command `CONTRIBUTING.md` matrix ran locally from October 2 into
October 3, 2026 (Asia/Bangkok), with `PUNCTRA_REQUIRE_GPU=1` inherited by every
command. The [command results](point-footprint-closeout-local-checks-2026-10-02.json)
record 65 successful commands and three failures against executable source
`58bb99870876865411176a2ec8714d648283e0bc`:

- The full JavaScript suite passed 265 tests and failed 12 integration or
  qualification tests because the existing evidence names an older
  implementation or packed artifact.
- `verify-browser-qualification.mjs` rejects qualified-file changes after the
  recorded implementation `1ea86a9c65f4d85630e57173c6a3bb68a22b3e17`.
- `verify-browser-integration-baseline.mjs` rejects the rebuilt viewer artifact
  digest, which differs from the earlier quickstart observation.

Workspace formatting, linting, all 891 workspace tests (three ignored),
rustdoc, 12-library package verification, fuzz checks, WASM checks, fixture
generation, SDK build/packed consumers/reference, immutable v0.21 visual
verification, all nine benchmark commands, examples, explicit forced-GPU
checks, documentation/JSON checks, and diff checks passed. Benchmark exit
status is not a cross-revision performance claim.

The new diagnostics and precision regression tests pass. Investigation and
status documentation were edited during the command run, so these results
are regression-check observations, not a clean-pin acceptance record. The
earlier functional records remain intact; no digest or pin was replaced to
make a failed qualification check appear successful.

## Proposed corrective scope

Keep the v0.21 corpus, images, evidence, geometry, colors, camera, and authority
contracts immutable. Preserve the existing metric thresholds while diagnosing
the failures. Do not mark v0.22 Complete from this investigation.

1. Add a separate v0.22 generated browser fixture with provably isolated
   footprints at DPR 1, 2, and 4. Validate separation, capture margins, and
   nominal-pick visibility before GPU execution. Retain the inherited mixed
   scenes for thin-feature, density, component-bridge, and resource checks.
2. Bind nominal-pick probes to CPU-derived visible depth winners or dedicated
   unobstructed targets. Preserve the seven-pixel nominal pick contract and
   verify identical identity across single-sample and antialiased rendering.
3. Run a matched-footprint control to distinguish occupancy redistribution
   from displacement. If the accepted size policy cannot satisfy the inherited
   one-pixel occupancy-centroid limit, explicitly revise the design before
   changing either policy or acceptance metric. Do not merely raise the limit
   to fit these results.
4. Restart the complete clean-commit GPU, record, image-pin, repeated record,
   functional qualification, and verify sequence after the corrective design
   and implementation are settled.

The immediate decision is whether to extend this follow-up into that
corrective design and implementation, or keep it as a diagnostic/precision
repair while v0.22 acceptance remains open.
