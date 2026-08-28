import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";

import {
  FOOTPRINT_BASELINE_SCHEMA,
  FOOTPRINT_EVIDENCE_SCHEMA,
  FOOTPRINT_EXTERNAL_NONCLAIMS,
  FOOTPRINT_IMPLEMENTATION_PATHS,
  FOOTPRINT_LOCAL_TEST_CASE_IDS,
  FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
  FOOTPRINT_LOCAL_TEST_SCHEMA,
  FOOTPRINT_RELEASE,
  FOOTPRINT_RUNTIME_PATHS,
  FOOTPRINT_UNAVAILABLE_MEASUREMENTS,
  FOOTPRINT_VERIFIER_PATH,
  createFootprintMetricBinding,
  createPointFootprintImageArtifact,
  createPointFootprintResourceEvidence,
  createTopologyMetricBinding,
  derivePointFootprintEvidenceSummary,
  expectedPointFootprintResources,
  projectedDensityDisplayDiameter,
  summarizeFootprintTiming,
  validatePointFootprintBaseline,
  validatePointFootprintLocalTestArtifact,
  validatePointFootprintRunInputs,
  verifyPointFootprintEvidence,
} from "./footprint-evidence.js";

const corpus = JSON.parse(await readFile(
  new URL("./fixtures/footprint-v1/corpus.json", import.meta.url),
  "utf8",
));
const SHA = "a".repeat(64);
const REPOSITORY_ROOT_URL = new URL("../../../", import.meta.url);
const IMPLEMENTATION_PATHS = [
  "Cargo.lock",
  "Cargo.toml",
  "apps/browser-demo/Cargo.toml",
  "apps/browser-demo/src/browser.rs",
  "apps/browser-demo/src/capture.rs",
  "apps/browser-demo/src/diagnostics.rs",
  "apps/browser-demo/src/display.rs",
  "apps/browser-demo/src/host.rs",
  "apps/browser-demo/src/lib.rs",
  "apps/browser-demo/src/scene.rs",
  "apps/browser-demo/src/streaming.rs",
  "apps/browser-demo/web/footprint-corpus.js",
  "apps/browser-demo/web/footprint-corpus.test.mjs",
  "apps/browser-demo/web/footprint-evidence.js",
  "apps/browser-demo/web/footprint-evidence.test.mjs",
  "apps/browser-demo/web/footprint-export.js",
  "apps/browser-demo/web/footprint-export.test.mjs",
  "apps/browser-demo/web/footprint-main.js",
  "apps/browser-demo/web/footprint-runner-core.js",
  "apps/browser-demo/web/footprint-runner-core.test.mjs",
  "apps/browser-demo/web/footprint.css",
  "apps/browser-demo/web/footprint.html",
  "apps/browser-demo/web/visual-archive.js",
  "apps/browser-demo/web/visual-capture.js",
  "apps/browser-demo/web/visual-comparison.js",
  "apps/browser-demo/web/visual-corpus.js",
  "apps/browser-demo/web/visual-corpus.test.mjs",
  "apps/browser-demo/web/visual-footprint-metrics.js",
  "apps/browser-demo/web/visual-footprint-metrics.test.mjs",
  "apps/browser-demo/web/visual-png.js",
  "apps/browser-demo/web/visual-provenance.js",
  "apps/browser-demo/web/visual-rubric.js",
  "apps/browser-demo/web/visual-validation.js",
  "apps/browser-demo/web/fixtures/footprint-v1/corpus.json",
  "apps/renderer-demo/src/appearance.rs",
  "crates/render-wgpu/Cargo.toml",
  "crates/render-wgpu/src/footprint.rs",
  "crates/render-wgpu/src/frame.rs",
  "crates/render-wgpu/src/gpu.rs",
  "crates/render-wgpu/src/eye_dome.wgsl",
  "crates/render-wgpu/src/lib.rs",
  "crates/render-wgpu/src/pick.rs",
  "crates/render-wgpu/src/pipeline.rs",
  "crates/render-wgpu/src/point.wgsl",
  "crates/render-wgpu/src/renderer.rs",
  "crates/render-wgpu/src/targets.rs",
  "crates/render-wgpu/tests/contracts.rs",
  "crates/render-wgpu/tests/offscreen.rs",
  "crates/render-wgpu/test-support/gpu.rs",
  "scripts/build-browser-demo.sh",
  "scripts/serve-browser-demo.py",
  FOOTPRINT_VERIFIER_PATH,
];

test("qualification binds the exact loaded corpora before running", () => {
  const inputs = validRunInputs();
  const runningPins = validBaseline().pins;
  runningPins.corpus = {
    ...runningPins.corpus,
    byte_length: inputs.footprint.byte_length,
    sha256: inputs.footprint.sha256,
  };
  assert.equal(validatePointFootprintRunInputs(inputs, runningPins), inputs);

  for (const [label, mutate] of [
    ["footprint digest", (candidate) => { candidate.footprint.sha256 = "0".repeat(64); }],
    ["predecessor pins", (candidate, pins) => { pins.predecessor.release = "tampered"; }],
    ["visual URL", (candidate) => { candidate.visual.corpus_url += "?tampered=1"; }],
    ["visual length", (candidate) => { candidate.visual.corpus_byte_length += 1; }],
    ["visual digest", (candidate) => { candidate.visual.corpus_sha256 = "0".repeat(64); }],
  ]) {
    const candidate = structuredClone(inputs);
    const pins = structuredClone(runningPins);
    mutate(candidate, pins);
    assert.throws(
      () => validatePointFootprintRunInputs(candidate, pins),
      new RegExp(label),
    );
  }
});

test("implementation pins close every relative JavaScript import", async () => {
  const importedModules = await relativeJavaScriptImportClosure(
    "apps/browser-demo/web/footprint-main.js",
  );
  const pinnedPaths = new Set([
    ...FOOTPRINT_IMPLEMENTATION_PATHS,
    ...FOOTPRINT_RUNTIME_PATHS,
  ]);
  assert.equal(importedModules.has("apps/browser-demo/web/pkg/browser_demo.js"), true,
    "generated runtime entrypoint is absent from the import closure");
  for (const modulePath of importedModules) {
    assert.equal(pinnedPaths.has(modulePath), true, `${modulePath} is not pinned`);
  }
});

test("baseline closes implementation, verifier, runtime, corpus, predecessor, and nine images", () => {
  const baseline = validBaseline();
  assert.equal(validatePointFootprintBaseline(baseline, corpus), baseline);

  const omittedRenderer = structuredClone(baseline);
  omittedRenderer.pins.implementation.files = omittedRenderer.pins.implementation.files
    .filter(({ path }) => path !== "crates/render-wgpu/src/footprint.rs");
  assert.throws(() => validatePointFootprintBaseline(omittedRenderer, corpus), /omit.*footprint\.rs/);

  const extraImplementation = structuredClone(baseline);
  extraImplementation.pins.implementation.files.push(digest("README.md"));
  assert.throws(() => validatePointFootprintBaseline(extraImplementation, corpus),
    /implementation file paths differ/);

  const predecessorTamper = structuredClone(baseline);
  predecessorTamper.pins.predecessor.release_evidence.sha256 = "0".repeat(64);
  assert.throws(() => validatePointFootprintBaseline(predecessorTamper, corpus), /predecessor pins differ/);
});

test("local renderer artifact has one exact environment and five exact source cases", () => {
  const implementationCommit = "b".repeat(40);
  const artifact = {
    schema: FOOTPRINT_LOCAL_TEST_SCHEMA,
    implementation_commit: implementationCommit,
    producer_command: FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
    environment: {
      operating_system: "test-os",
      adapter_name: "test-adapter",
      backend: "test-backend",
    },
    cases: FOOTPRINT_LOCAL_TEST_CASE_IDS.map((id) => ({
      id,
      source_test: id,
      passed: true,
      facts: {},
    })),
  };
  assert.equal(
    validatePointFootprintLocalTestArtifact(artifact, implementationCommit),
    artifact,
  );

  const extraCase = structuredClone(artifact);
  extraCase.cases.push({ id: "unbound", source_test: "unbound", passed: true, facts: {} });
  assert.throws(
    () => validatePointFootprintLocalTestArtifact(extraCase, implementationCommit),
    /case order or membership differ/,
  );
});

test("timing and exact transient accounting are derived from raw facts", () => {
  assert.deepEqual(summarizeFootprintTiming([4, 1, 3, 2]), {
    count: 4,
    p50: 2,
    p95: 4,
    maximum: 4,
  });
  const resources = expectedPointFootprintResources({
    selected: "multisample4x",
    physicalWidth: 640,
    physicalHeight: 480,
    eyeDomeActive: true,
    pickTargetsRetained: true,
    ceilingBytes: 67_108_864,
  });
  assert.equal(resources.multisample_color_bytes, 4_915_200);
  assert.equal(resources.multisample_depth_bytes, 4_915_200);
  assert.equal(resources.renderer_transient_texture_bytes, 14_745_600);
  assert.equal(resources.renderer_transient_byte_ceiling, 67_108_864);
});

test("density diameter treats an empty resident set as one point like the host", () => {
  const profile = { physical_width: 640, physical_height: 480 };
  const policy = {
    display_diameter: {
      spacing_fraction: 0.55,
      minimum_physical_pixels: 2,
      maximum_physical_pixels: 6,
    },
  };
  assert.equal(
    projectedDensityDisplayDiameter(profile, 0, policy),
    projectedDensityDisplayDiameter(profile, 1, policy),
  );
  assert.throws(
    () => projectedDensityDisplayDiameter(profile, -1, policy),
    /resident point count is invalid/,
  );
});

test("runner adapters project PNG, metric, and resource facts without carrying its larger record shape", () => {
  const image = imageArtifact({
    kind: "focused_candidate_png",
    trialId: "trial",
    recreationIndex: null,
    profileId: "profile",
    path: "docs/releases/v0.22-browser-point-footprint-artifacts/trial.png",
    width: 8,
    height: 8,
    digestCharacter: "b",
  });
  const runnerMetadata = { ...image, filename: "trial.png", frame_index: null };
  delete runnerMetadata.profile_id;
  assert.deepEqual(createPointFootprintImageArtifact(runnerMetadata, "profile"), image);

  const topology = topologyReport({ x: 0, y: 0, width: 8, height: 8 });
  topology.foreground_threshold = 0.5;
  const topologyBindingResult = createTopologyMetricBinding({
    metricId: "runner/topology",
    artifactPath: image.path,
    backgroundRgba: [19, 20, 19, 255],
    measurement: {
      occupancy_normalization: "maximum_absolute_rgba8_channel_delta_from_clear_color_v1",
      channel_threshold: 2,
      metrics: topology,
    },
  });
  assert.equal(topologyBindingResult.report, topology);

  const footprint = footprintBinding("runner/footprint", image.path, 0.1);
  footprint.report.normalization = {
    foreground_rgba: footprint.foreground_rgba,
    background_rgba: footprint.background_rgba,
  };
  const footprintBindingResult = createFootprintMetricBinding({
    metricId: "runner/footprint",
    artifactPath: image.path,
    measurement: { foreground_rgba: footprint.foreground_rgba, metrics: footprint.report },
  });
  assert.equal(footprintBindingResult.report, footprint.report);

  const resources = resourcesForProfile("multisample4x", corpus.canonical_profile, false);
  assert.deepEqual(createPointFootprintResourceEvidence({
    pointFootprint: footprintFacts(
      "antialiased", "multisample4x", corpus.canonical_profile, 5_808,
    ),
    profile: corpus.canonical_profile,
    pickTargetsRetained: false,
    rendererTransientTextureBytes: resources.renderer_transient_texture_bytes,
    ceilingBytes: corpus.policy.renderer_transient_byte_ceiling,
  }), resources);
});

test("complete evidence derives 27 canonical recreations, nine DPR trials, and three fallback paths", () => {
  const baseline = validBaseline();
  const evidence = validEvidence(baseline);
  evidence.summary = derivePointFootprintEvidenceSummary(evidence, { baseline, corpus });
  const verified = verifyPointFootprintEvidence(evidence, { baseline, corpus });

  assert.deepEqual(verified.summary, {
    passed: true,
    canonical_trials: 9,
    canonical_recreations: 27,
    focused_scale_trials: 9,
    fallback_trials: 3,
    artifacts: evidence.artifacts.png.length + 1,
    metric_reports: 81,
    failures: [],
  });
});

test("host f32 display diameter accepts serde's shortest decimal representation", () => {
  const baseline = validBaseline();
  const evidence = validEvidence(baseline);
  const focusedDpr1 = evidence.focused_trials.find(
    ({ profile_id }) => profile_id === "focused-dpr1",
  );
  focusedDpr1.resident_points = 1_878;
  focusedDpr1.point_footprint.display_size_physical_pixels = 3.5171874;
  evidence.summary = derivePointFootprintEvidenceSummary(evidence, { baseline, corpus });

  assert.equal(evidence.summary.passed, true);
  assert.doesNotThrow(() => verifyPointFootprintEvidence(evidence, { baseline, corpus }));
});

test("pass flags, samples, metrics, resources, status, identities, and browser provenance cannot be forged", () => {
  const baseline = validBaseline();
  const evidence = validEvidence(baseline);
  evidence.summary = derivePointFootprintEvidenceSummary(evidence, { baseline, corpus });

  for (const mutate of [
    (value) => { value.summary.passed = false; },
    (value) => { value.canonical_trials[0].recreations[0].timing.frame_interval.p95 = 2; },
    (value) => { value.focused_trials[0].isolated_footprints[0].candidate.report.coverage.root_mean_square_error = 0.19; },
    (value) => { value.canonical_trials[0].recreations[0].resources.multisample_color_bytes -= 4; },
    (value) => { value.canonical_trials[0].recreations[0].point_footprint.selected = "multisample_4x"; },
    (value) => { value.canonical_trials[0].recreations[0].point_footprint.display_size_physical_pixels += 0.01; },
    (value) => { value.fallback_trials[2].pick_probes[0].point_ordinal = "1867"; },
    (value) => { value.fallback_trials[2].selection.multisample_target_allocated = true; },
    (value) => { value.fallback_trials[0].selection.multisample_pipeline_created = true; },
    (value) => { value.fallback_trials[0].hard_circle_mask.observed_sha256 = "5".repeat(64); },
    (value) => { value.local_gpu_fixture.resource_fallback.nominal_pick_identity.matched = false; },
    (value) => { value.fallback_trials[0].browser_observation = {}; },
    (value) => { value.artifacts.local_test_results[0].producer_command += " "; },
    (value) => { value.artifacts.local_test_results.push(structuredClone(value.artifacts.local_test_results[0])); },
    (value) => { value.local_gpu_fixture.environment.adapter_name = ""; },
    (value) => { value.external_evidence.cross_browser = true; },
  ]) {
    const tampered = structuredClone(evidence);
    mutate(tampered);
    assert.throws(() => verifyPointFootprintEvidence(tampered, { baseline, corpus }));
  }
});

test("offline recomputation and predecessor timing maps are exact injected dependencies", () => {
  const baseline = validBaseline();
  const evidence = validEvidence(baseline);
  const recomputedMetrics = collectMetricReports(evidence);
  const predecessorTiming = new Map();
  for (const trial of evidence.canonical_trials) {
    for (const recreation of trial.recreations) {
      predecessorTiming.set(`${trial.trial_id}:${recreation.index}`, {
        frame_interval_p95_milliseconds:
          corpus.canonical_trials.find(({ id }) => id === trial.trial_id)
            .predecessor_timing.maximum_recreation_frame_interval_p95_milliseconds,
        frame_submission_p95_milliseconds:
          corpus.canonical_trials.find(({ id }) => id === trial.trial_id)
            .predecessor_timing.maximum_recreation_frame_submission_p95_milliseconds,
      });
    }
  }
  evidence.summary = derivePointFootprintEvidenceSummary(evidence, {
    baseline, corpus, recomputedMetrics, predecessorTiming,
  });
  verifyPointFootprintEvidence(evidence, { baseline, corpus, recomputedMetrics, predecessorTiming });

  recomputedMetrics.set(recomputedMetrics.keys().next().value, { tampered: true });
  assert.throws(
    () => verifyPointFootprintEvidence(evidence, { baseline, corpus, recomputedMetrics, predecessorTiming }),
    /recomputed metric/,
  );
});

function validBaseline() {
  return {
    schema: FOOTPRINT_BASELINE_SCHEMA,
    release: FOOTPRINT_RELEASE,
    pins: {
      implementation: {
        commit: "1".repeat(40),
        files: IMPLEMENTATION_PATHS.map((path) => digest(path)),
      },
      verifier: digest(FOOTPRINT_VERIFIER_PATH),
      runtime: {
        package_name: "@punctra/viewer",
        package_version: FOOTPRINT_RELEASE,
        artifacts: FOOTPRINT_RUNTIME_PATHS.map((path) => digest(path)),
      },
      corpus: digest("apps/browser-demo/web/fixtures/footprint-v1/corpus.json"),
      predecessor: structuredClone(corpus.predecessor),
    },
    candidate_images: corpus.canonical_trials.map((trial, index) => imageArtifact({
      kind: "candidate_baseline_png",
      trialId: trial.id,
      recreationIndex: null,
      profileId: corpus.canonical_profile.id,
      path: `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}.png`,
      width: corpus.canonical_profile.physical_width,
      height: corpus.canonical_profile.physical_height,
      digestCharacter: String((index + 1) % 10),
    })),
    focused_images: corpus.focused_trials.flatMap((trial, trialIndex) => (
      [corpus.canonical_profile, ...corpus.scale_profiles].map((profile, profileIndex) => {
        const suffix = profile.id === corpus.canonical_profile.id ? "" : `-${profile.id}`;
        return imageArtifact({
          kind: "focused_baseline_png",
          trialId: trial.id,
          recreationIndex: null,
          profileId: profile.id,
          path: `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}${suffix}.png`,
          width: profile.physical_width,
          height: profile.physical_height,
          digestCharacter: focusedDigestCharacter(trial.id, trialIndex, profileIndex),
        });
      })
    )),
    external_evidence: structuredClone(FOOTPRINT_EXTERNAL_NONCLAIMS),
  };
}

function validEvidence(baseline) {
  const png = [];
  const canonicalTrials = corpus.canonical_trials.map((trial, trialIndex) => {
    const predecessorTopology = topologyBinding(
      `canonical/${trial.id}/predecessor`,
      trial.predecessor_baseline.path,
      { x: 0, y: 0, width: 640, height: 480 },
    );
    return {
      trial_id: trial.id,
      predecessor_topology: predecessorTopology,
      recreations: Array.from({ length: 3 }, (_, recreationIndex) => {
        const artifact = imageArtifact({
          kind: "canonical_recreation_png",
          trialId: trial.id,
          recreationIndex,
          profileId: corpus.canonical_profile.id,
          path: `docs/releases/v0.22-browser-point-footprint-artifacts/${trial.id}-r${recreationIndex}.png`,
          width: 640,
          height: 480,
          digestCharacter: String((trialIndex + 1) % 10),
        });
        png.push(artifact);
        return {
          index: recreationIndex,
          adapter: observedAdapter(),
          resident_points: 5_808,
          point_footprint: footprintFacts(
            "antialiased", "multisample4x", corpus.canonical_profile, 5_808,
          ),
          timing: timing(trial),
          resources: resources("multisample4x", corpus.canonical_profile),
          capture_artifact_path: artifact.path,
          candidate_topology: topologyBinding(
            `canonical/${trial.id}/r${recreationIndex}`,
            artifact.path,
            { x: 0, y: 0, width: 640, height: 480 },
          ),
          feature_checks: [{
            id: "bound-feature",
            predecessor_foreground_pixels: 1,
            candidate_foreground_pixels: 1,
            centroid_distance_pixels: 0,
          }],
          dense_region_checks: (
            corpus.focused_trials.find(({ id }) => id === trial.id)?.dense_regions ?? []
          ).map((rectangle, regionIndex) => ({
            rectangle,
            predecessor: topologyBinding(
              `canonical/${trial.id}/r${recreationIndex}/dense/${regionIndex}/predecessor`,
              trial.predecessor_baseline.path,
              rectangle,
            ),
            candidate: topologyBinding(
              `canonical/${trial.id}/r${recreationIndex}/dense/${regionIndex}/candidate`,
              artifact.path,
              rectangle,
            ),
          })),
        };
      }),
    };
  });

  const profiles = [corpus.canonical_profile, ...corpus.scale_profiles];
  const focusedTrials = corpus.focused_trials.flatMap((expectedTrial, trialIndex) => profiles.map((profile, profileIndex) => {
    let candidate;
    if (profile.id === corpus.canonical_profile.id) {
      candidate = png.find((artifact) => artifact.trial_id === expectedTrial.id
        && artifact.kind === "canonical_recreation_png" && artifact.recreation_index === 0);
    } else {
      candidate = imageArtifact({
        kind: "focused_candidate_png",
        trialId: expectedTrial.id,
        recreationIndex: null,
        profileId: profile.id,
        path: `docs/releases/v0.22-browser-point-footprint-artifacts/${expectedTrial.id}-${profile.id}-candidate.png`,
        width: profile.physical_width,
        height: profile.physical_height,
        digestCharacter: focusedDigestCharacter(expectedTrial.id, trialIndex, profileIndex),
      });
      png.push(candidate);
    }
    const prefix = `focused/${expectedTrial.id}/${profile.id}`;
    const suffix = profile.id === corpus.canonical_profile.id ? "" : `-${profile.id}`;
    return {
      trial_id: expectedTrial.id,
      profile_id: profile.id,
      adapter: observedAdapter(),
      resident_points: 5_808,
      point_footprint: footprintFacts("antialiased", "multisample4x", profile, 5_808),
      resources: resources("multisample4x", profile),
      candidate_artifact_path: candidate.path,
      baseline_artifact_path:
        `apps/browser-demo/web/fixtures/footprint-v1/baselines/${expectedTrial.id}${suffix}.png`,
      isolated_footprints: expectedTrial.isolated_ordinals.map((ordinal) => ({
        ordinal,
        center_foreground: true,
        candidate: footprintBinding(`${prefix}/point/${ordinal}/candidate`, candidate.path, 0.1),
      })),
    };
  }));

  const pickMask = imageArtifact({
    kind: "nominal_pick_mask_png",
    trialId: corpus.focused_trials[2].id,
    recreationIndex: null,
    profileId: corpus.canonical_profile.id,
    path: "docs/releases/v0.22-browser-point-footprint-artifacts/nominal-pick-mask.png",
    width: 16,
    height: 16,
    digestCharacter: "f",
  });
  png.push(pickMask);
  const probes = [{
    ordinal: 1866,
    generation: 1,
    source_identity: "2".repeat(64),
    batch_key: 4,
    batch_version: 2,
    point_ordinal: "1866",
  }, {
    ordinal: 2005,
    generation: 1,
    source_identity: "2".repeat(64),
    batch_key: 4,
    batch_version: 2,
    point_ordinal: "2005",
  }];
  const localTestResults = [
    localResult("docs/releases/v0.22-browser-point-footprint-artifacts/local-test-evidence.json"),
  ];
  const fallbackTrials = [
    localFallback(
      "single_sample", "single_sample", "single_sample",
      "single_sample_request_never_becomes_a_fallback", localTestResults[0].path,
    ),
    localFallback(
      "unsupported_fallback", "antialiased", "unsupported_fallback",
      "capability_fallback_precedes_the_viewport_resource_check", localTestResults[0].path,
    ),
    browserFallback(probes),
  ];
  return {
    schema: FOOTPRINT_EVIDENCE_SCHEMA,
    release: FOOTPRINT_RELEASE,
    mode: "verify",
    started_at: "2026-08-29T01:00:00.000Z",
    completed_at: "2026-08-29T02:00:00.000Z",
    baseline: digest("docs/releases/v0.22-browser-point-footprint-baseline.json"),
    pins: structuredClone(baseline.pins),
    environment: {
      browser_user_agent: "test-browser",
      browser_platform: "test-platform",
      operating_system: "test-os",
      adapter_name: "test-adapter",
      backend: "test-backend",
      same_adapter_for_scale_trials: true,
      physical_display_observed: false,
    },
    artifacts: { png, local_test_results: localTestResults },
    canonical_trials: canonicalTrials,
    focused_trials: focusedTrials,
    local_gpu_fixture: {
      evidence_source: "local_renderer_gpu_test",
      browser_observation: null,
      environment: {
        operating_system: "test-local-os",
        adapter_name: "test-local-adapter",
        backend: "test-local-backend",
      },
      local_test_evidence: {
        quality: localTestEvidence(
          localTestResults[0].path,
          "antialiased_footprint_quality_matrix",
        ),
        pick_independence: localTestEvidence(
          localTestResults[0].path,
          "four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking",
        ),
        resource_accounting: localTestEvidence(
          localTestResults[0].path,
          "exact_high_water_accounts_for_pick_and_eye_dome_targets",
        ),
      },
      diameters_physical_pixels: [2, 3, 4, 5, 6],
      subpixel_center_phases: [
        [0, 0], [0.25, 0], [0.5, 0], [0.75, 0],
        [0, 0.5], [0.25, 0.5], [0.5, 0.5], [0.75, 0.5],
      ],
      preferred: {
        maximum_coverage_rmse: 0.1,
        maximum_exact_distance_outer_leakage_pixels: 0,
        all_centers_foreground: true,
        all_quad_corners_clear: true,
      },
      single_sample: { coverage_rmse_at_preferred_worst_case: 0.2 },
      pick_independence: {
        display_diameter_physical_pixels: 18,
        nominal_pick_diameter_physical_pixels: 2.4,
        visual_only_probe_offset_physical_pixels: [5, 0],
        visual_only_probe_result: "miss",
        nominal_probe_result: "expected_identity",
      },
      transient_bounds: {
        preferred_non_edl_bytes_per_pixel: 40,
        preferred_edl_bytes_per_pixel: 48,
        fallback_bytes_per_pixel: 8,
        maximum_preferred_physical_pixels: 1_310_720,
        maximum_preferred_transient_bytes: 62_914_560,
        renderer_transient_byte_ceiling: 67_108_864,
      },
      resource_fallback: localFallbackProof(2401),
    },
    pick_identity_reference: {
      profile_id: corpus.canonical_profile.id,
      resident_points: 5_808,
      point_footprint: footprintFacts(
        "antialiased", "multisample4x", corpus.canonical_profile, 5_808,
      ),
      pick_probes: probes,
      pick_mask_artifact_path: pickMask.path,
    },
    fallback_trials: fallbackTrials,
    summary: {},
    external_evidence: structuredClone(FOOTPRINT_EXTERNAL_NONCLAIMS),
    unavailable_measurements: [...FOOTPRINT_UNAVAILABLE_MEASUREMENTS],
    fatal_error: null,
  };
}

function localFallback(id, requested, selected, testCase, artifactPath) {
  return {
    id,
    evidence_source: "local_renderer_test",
    physical_width: corpus.canonical_profile.physical_width,
    physical_height: corpus.canonical_profile.physical_height,
    selection: {
      requested, selected, sample_count: 1, multisample_pipeline_created: false,
    },
    resources: null,
    pick_probes: null,
    ...localFallbackProof(id === "single_sample" ? 2201 : 2301),
    browser_observation: null,
    local_test_evidence: {
      artifact_path: artifactPath,
      case: testCase,
      source_test: testCase,
      result: "passed",
    },
  };
}

function browserFallback(probes) {
  const residentPoints = 5_808;
  const pointFootprint = footprintFacts(
    "antialiased", "resource_fallback", corpus.fallback_profile, residentPoints,
  );
  const exactResources = resources("resource_fallback", corpus.fallback_profile);
  return {
    id: "resource_fallback",
    evidence_source: "attended_browser",
    physical_width: corpus.fallback_profile.physical_width,
    physical_height: corpus.fallback_profile.physical_height,
    selection: {
      requested: "antialiased",
      selected: "resource_fallback",
      sample_count: 1,
      multisample_target_allocated: false,
    },
    resources: exactResources,
    pick_probes: structuredClone(probes),
    hard_circle_mask: null,
    nominal_pick_identity: null,
    browser_observation: {
      profile_id: corpus.fallback_profile.id,
      capture_performed: false,
      adapter: observedAdapter(),
      resident_points: residentPoints,
      point_footprint: structuredClone(pointFootprint),
      resources: structuredClone(exactResources),
    },
    local_test_evidence: null,
  };
}

function localTestEvidence(artifactPath, testCase) {
  return {
    artifact_path: artifactPath,
    case: testCase,
    source_test: testCase,
    result: "passed",
  };
}

function footprintFacts(requested, selected, profile, residentPoints) {
  return {
    requested,
    selected,
    nominal_pick_size_physical_pixels: 7,
    display_size_physical_pixels: projectedDensityDisplayDiameter(
      profile,
      residentPoints,
      corpus.policy,
    ),
  };
}

function observedAdapter() {
  return { name: "test-adapter", backend: "test-backend" };
}

function localFallbackProof(pointOrdinal) {
  const expected = {
    generation: 1,
    source_identity: "4".repeat(64),
    batch_key: 1,
    batch_version: 1,
    point_ordinal: pointOrdinal,
  };
  return {
    hard_circle_mask: {
      width: 16,
      height: 16,
      byte_length: 256,
      reference_sha256: "3".repeat(64),
      observed_sha256: "3".repeat(64),
      equivalent: true,
    },
    nominal_pick_identity: {
      expected,
      observed: structuredClone(expected),
      matched: true,
    },
  };
}

function resources(selected, profile) {
  return resourcesForProfile(selected, profile, true);
}

function resourcesForProfile(selected, profile, pickTargetsRetained) {
  return expectedPointFootprintResources({
    selected,
    physicalWidth: profile.physical_width,
    physicalHeight: profile.physical_height,
    eyeDomeActive: false,
    pickTargetsRetained,
    ceilingBytes: corpus.policy.renderer_transient_byte_ceiling,
  });
}

function timing(trial) {
  const intervalSamples = Array(30).fill(1);
  const submissionSamples = Array(30).fill(0.1);
  return {
    frame_interval_samples_milliseconds: intervalSamples,
    frame_submission_samples_milliseconds: submissionSamples,
    frame_interval: summarizeFootprintTiming(intervalSamples),
    frame_submission: summarizeFootprintTiming(submissionSamples),
    predecessor_frame_interval_p95_milliseconds:
      trial.predecessor_timing.maximum_recreation_frame_interval_p95_milliseconds,
    predecessor_frame_submission_p95_milliseconds:
      trial.predecessor_timing.maximum_recreation_frame_submission_p95_milliseconds,
    first_coverage_milliseconds: 1,
    settled_view_milliseconds: 1,
  };
}

function footprintBinding(metricId, artifactPath, rmse) {
  const rectangle = { x: 0, y: 0, width: 8, height: 8 };
  const center = [4, 4];
  const radius = 2;
  return {
    kind: "known_endpoint_disk_v1",
    metric_id: metricId,
    artifact_path: artifactPath,
    rectangle,
    center,
    radius_pixels: radius,
    foreground_rgba: [255, 255, 255, 255],
    background_rgba: [0, 0, 0, 255],
    report: {
      schema: "punctra-browser-point-footprint-metrics-v1",
      rectangle,
      center,
      radius_pixels: radius,
      coverage: { root_mean_square_error: rmse, partial_edge_pixels: rmse === 0.1 ? 8 : 0 },
      centroid: { error_pixels: 0 },
      corner_leakage: {
        exact_distance_outer: { margin_physical_pixels: 0.75, pixel_count: 0, coverage: 0 },
      },
    },
  };
}

function topologyBinding(metricId, artifactPath, rectangle) {
  return {
    kind: "background_difference_topology_v1",
    metric_id: metricId,
    artifact_path: artifactPath,
    rectangle,
    background_rgba: [19, 20, 19, 255],
    maximum_background_channel_delta: 2,
    foreground_threshold: 0.5,
    report: topologyReport(rectangle),
  };
}

function topologyReport(rectangle) {
  return {
    schema: "punctra-browser-region-topology-metrics-v1",
    rectangle,
    foreground_pixels: 1,
    foreground_fraction: 0.5,
    solid_2x2_blocks: 1,
    foreground: {
      connectivity: 4,
      component_count: 1,
      left_right_bridge_components: 0,
      top_bottom_bridge_components: 0,
    },
    background: {
      connectivity: 4,
      component_count: 1,
      left_right_bridge_components: 0,
      top_bottom_bridge_components: 0,
    },
  };
}

function validRunInputs() {
  const footprintUrl = new URL("./fixtures/footprint-v1/corpus.json", import.meta.url);
  const predecessorCorpus = corpus.predecessor.corpus;
  return {
    footprint: {
      corpus: structuredClone(corpus),
      url: footprintUrl.href,
      byte_length: 12_345,
      sha256: SHA,
    },
    visual: {
      corpus: {},
      corpus_url: new URL(predecessorCorpus.path, footprintUrl).href,
      corpus_byte_length: predecessorCorpus.byte_length,
      corpus_sha256: predecessorCorpus.sha256,
    },
  };
}

async function relativeJavaScriptImportClosure(entryPath) {
  const visited = new Set();
  const pending = [entryPath];
  while (pending.length > 0) {
    const modulePath = pending.pop();
    if (visited.has(modulePath)) continue;
    visited.add(modulePath);
    let source;
    try {
      source = await readFile(new URL(modulePath, REPOSITORY_ROOT_URL), "utf8");
    } catch (error) {
      if (error?.code === "ENOENT" && FOOTPRINT_RUNTIME_PATHS.includes(modulePath)) continue;
      throw error;
    }
    for (const specifier of relativeJavaScriptImports(source)) {
      const dependency = path.posix.normalize(path.posix.join(
        path.posix.dirname(modulePath),
        specifier,
      ));
      if (!visited.has(dependency)) pending.push(dependency);
    }
  }
  return visited;
}

function relativeJavaScriptImports(source) {
  const imports = [];
  const staticImport = /(?:^|\n)\s*(?:import|export)\s+(?:[^;]*?\s+from\s+)?["'](\.[^"']+\.js)["']\s*;/g;
  for (const match of source.matchAll(staticImport)) imports.push(match[1]);
  const dynamicImport = /\bimport\(\s*["'](\.[^"']+\.js)["']\s*\)/g;
  for (const match of source.matchAll(dynamicImport)) imports.push(match[1]);
  return imports;
}

function imageArtifact({
  kind, trialId, recreationIndex, profileId, path, width, height, digestCharacter,
}) {
  return {
    kind,
    trial_id: trialId,
    recreation_index: recreationIndex,
    profile_id: profileId,
    path,
    mime_type: "image/png",
    encoding: "png-rgba8-filter-0",
    width,
    height,
    encoded_byte_length: 1,
    encoded_sha256: digestCharacter.repeat(64),
    decoded_byte_length: width * height * 4,
    decoded_sha256: digestCharacter.repeat(64),
    authority: "presentation_only",
  };
}

function digest(path) {
  return { path, byte_length: 1, sha256: SHA };
}

function localResult(path) {
  return {
    ...digest(path),
    media_type: "application/json",
    producer_command: FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
  };
}

function focusedDigestCharacter(trialId, trialIndex, profileIndex) {
  if (profileIndex === 0) {
    return String((corpus.canonical_trials.findIndex(({ id }) => id === trialId) + 1) % 10);
  }
  return String((trialIndex * 3 + profileIndex + 1) % 10);
}

function collectMetricReports(evidence) {
  const reports = new Map();
  const add = (binding) => reports.set(binding.metric_id, structuredClone(binding.report));
  for (const trial of evidence.canonical_trials) {
    add(trial.predecessor_topology);
    for (const recreation of trial.recreations) add(recreation.candidate_topology);
    for (const recreation of trial.recreations) {
      for (const region of recreation.dense_region_checks) {
        add(region.predecessor);
        add(region.candidate);
      }
    }
  }
  for (const trial of evidence.focused_trials) {
    for (const footprint of trial.isolated_footprints) {
      add(footprint.candidate);
    }
  }
  return reports;
}
