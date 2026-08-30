import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  FOOTPRINT_BASELINE_SCHEMA,
  FOOTPRINT_EVIDENCE_SCHEMA,
  FOOTPRINT_EXTERNAL_NONCLAIMS,
  FOOTPRINT_IMPLEMENTATION_PATHS,
  FOOTPRINT_LOCAL_TEST_CASE_IDS,
  FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
  FOOTPRINT_RUNTIME_PATHS,
  FOOTPRINT_VERIFIER_PATH,
  expectedPointFootprintResources,
  projectedDensityDisplayDiameter,
} from "./footprint-evidence.js";
import {
  createPointFootprintBaselineRecord,
  createPointFootprintEvidenceRecord,
  pointFootprintLocalTestCase,
} from "./footprint-records.js";

const corpus = JSON.parse(await readFile(
  new URL("./fixtures/footprint-v1/corpus.json", import.meta.url),
  "utf8",
));
const ARTIFACT_ROOT = "docs/releases/v0.22-browser-point-footprint-artifacts";
const BACKGROUND_RGBA = Object.freeze([19, 20, 19, 255]);
const COMPLETED_AT = "2026-08-29T02:00:00.000Z";
const RESIDENT_POINTS = 5_808;
const SHA = "a".repeat(64);

test("record builders import without browser globals and local cases are cloned", async () => {
  const source = await readFile(new URL("./footprint-records.js", import.meta.url), "utf8");
  assert.doesNotMatch(source, /\b(?:document|navigator|window)\b|new Date/);

  const id = FOOTPRINT_LOCAL_TEST_CASE_IDS[0];
  const localTests = {
    cases: [{ id, source_test: id, passed: true, facts: { bounded: true } }],
  };
  const selected = pointFootprintLocalTestCase(localTests, id);
  assert.deepEqual(selected, localTests.cases[0]);
  selected.facts.bounded = false;
  assert.equal(localTests.cases[0].facts.bounded, true);
  assert.throws(
    () => pointFootprintLocalTestCase(localTests, "unbound-case"),
    /outside the closed contract/,
  );
});

test("baseline records project canonical and focused runner artifacts", () => {
  const pins = validPins();
  const baseline = validBaselineRecord(pins);

  assert.equal(baseline.schema, FOOTPRINT_BASELINE_SCHEMA);
  assert.equal(baseline.release, corpus.release);
  assert.notEqual(baseline.pins, pins);
  assert.equal(baseline.candidate_images.length, corpus.canonical_trials.length);
  assert.equal(
    baseline.focused_images.length,
    corpus.focused_trials.length * (1 + corpus.scale_profiles.length),
  );
  assert.deepEqual(baseline.environment, validEnvironment());
  assert.deepEqual(baseline.external_evidence, FOOTPRINT_EXTERNAL_NONCLAIMS);
});

test("evidence records project deterministic explicit inputs", () => {
  const options = validEvidenceRecordOptions();
  const evidence = createPointFootprintEvidenceRecord(options);

  assert.equal(evidence.schema, FOOTPRINT_EVIDENCE_SCHEMA);
  assert.equal(evidence.started_at, options.startedAt);
  assert.equal(evidence.completed_at, COMPLETED_AT);
  assert.equal(evidence.environment.browser_user_agent, options.browserUserAgent);
  assert.equal(evidence.environment.browser_platform, options.browserPlatform);
  assert.deepEqual(evidence.baseline, options.baselineIdentity);
  assert.deepEqual(evidence.pick_identity_reference.pick_probes, preferredPickProbes());
  assert.equal(evidence.summary.canonical_recreations, 27);
  assert.equal(evidence.summary.focused_scale_trials, 9);
  assert.equal(evidence.summary.passed, true);
});

test("evidence records reject local results from another implementation commit", () => {
  const options = validEvidenceRecordOptions();
  options.localTests.implementation_commit = "2".repeat(40);

  assert.throws(
    () => createPointFootprintEvidenceRecord(options),
    /local test evidence implementation commit differs from the browser implementation pin/,
  );
});

function validBaselineRecord(pins = validPins()) {
  return createPointFootprintBaselineRecord({
    footprint: corpus,
    pins,
    canonicalTrials: corpus.canonical_trials.map(() => ({ passed: true })),
    focusedTrials: corpus.focused_trials.map(() => ({ passed: true })),
    fallback: { passed: true },
    baselineArtifacts: baselineArtifacts(),
    environment: validEnvironment(),
  });
}

function validEnvironment() {
  return {
    browser_user_agent: "test-browser",
    browser_platform: "test-platform",
    operating_system: "test-os",
    adapter_name: "test-adapter",
    backend: "test-backend",
    same_adapter_for_scale_trials: true,
    physical_display_observed: false,
  };
}

function validEvidenceRecordOptions() {
  const pins = validPins();
  const baseline = validBaselineRecord(pins);
  const canonicalTrials = canonicalRunnerTrials();
  const focusedTrials = focusedRunnerTrials(canonicalTrials);
  const localTests = localTestArtifact(pins.implementation.commit);
  return {
    startedAt: "2026-08-29T01:00:00.000Z",
    readCompletedAt: () => COMPLETED_AT,
    browserUserAgent: "test-browser",
    browserPlatform: "test-platform",
    backgroundRgba: BACKGROUND_RGBA,
    footprint: corpus,
    pins,
    host: { operating_system: { name: "test-os" } },
    baseline,
    baselineIdentity: digest("docs/releases/v0.22-browser-point-footprint-baseline.json"),
    localTests,
    localTestMetadata: {
      path: `${ARTIFACT_ROOT}/local-test-evidence.json`,
      encoded_byte_length: 1,
      encoded_sha256: SHA,
    },
    canonicalTrials,
    focusedTrials,
    fallback: fallbackRunnerRecord(localTests),
  };
}

function canonicalRunnerTrials() {
  return corpus.canonical_trials.map((trial, trialIndex) => {
    const rectangle = fullProfileRectangle(corpus.canonical_profile);
    const predecessorTopology = topologyMeasurement(rectangle);
    return {
      trial_id: trial.id,
      recreations: Array.from(
        { length: corpus.canonical_profile.recreations },
        (_, recreationIndex) => {
          const picks = nominalPicksForTrial(trial.id);
          const artifact = imageMetadata({
            kind: "canonical_candidate_png",
            trialId: trial.id,
            profile: corpus.canonical_profile,
            path: `${ARTIFACT_ROOT}/${trial.id}-r${recreationIndex}.png`,
            digestCharacter: String(trialIndex + 1),
            recreationIndex,
          });
          return {
            index: recreationIndex,
            adapter: observedAdapter(),
            point_footprint: pointFootprintFacts(corpus.canonical_profile),
            lifecycle_timing: {
              first_coverage_milliseconds: 1,
              settled_view_milliseconds: 1,
            },
            representative_timing: representativeTiming(),
            resources: runnerResources(corpus.canonical_profile, "multisample4x", picks.length > 0),
            nominal_picks: picks,
            capture: { artifact },
            predecessor_topology: predecessorTopology,
            candidate_topology: topologyMeasurement(rectangle),
            component_bridges: componentBridgeMeasurement(rectangle),
            feature_comparisons: [{
              feature_id: "bound-feature",
              predecessor: { foreground_pixels: 1 },
              candidate: { foreground_pixels: 1 },
              centroid_distance_pixels: 0,
            }],
            dense_region_comparisons: (
              corpus.focused_trials.find(({ id }) => id === trial.id)?.dense_regions ?? []
            ).map((denseRectangle) => ({
              rectangle: denseRectangle,
              predecessor: topologyMeasurement(denseRectangle),
              candidate: topologyMeasurement(denseRectangle),
            })),
          };
        },
      ),
      passed: true,
    };
  });
}

function focusedRunnerTrials(canonicalTrials) {
  const profiles = [corpus.canonical_profile, ...corpus.scale_profiles];
  return corpus.focused_trials.flatMap((trial, trialIndex) => profiles.map((profile) => {
    const profileIndex = corpus.scale_profiles.findIndex(({ id }) => id === profile.id);
    const artifact = profile.id === corpus.canonical_profile.id
      ? canonicalTrials.find(({ trial_id: trialId }) => trialId === trial.id)
        .recreations[0].capture.artifact
      : imageMetadata({
        kind: "focused_candidate_png",
        trialId: trial.id,
        profile,
        path: `${ARTIFACT_ROOT}/${trial.id}-${profile.id}.png`,
        digestCharacter: "abcdef"[trialIndex * corpus.scale_profiles.length + profileIndex],
        recreationIndex: null,
      });
    const picks = nominalPicksForTrial(trial.id);
    return {
      trial_id: trial.id,
      profile_id: profile.id,
      adapter: observedAdapter(),
      resident_points: RESIDENT_POINTS,
      point_footprint: pointFootprintFacts(profile),
      resources: runnerResources(profile, "multisample4x", picks.length > 0),
      nominal_picks: picks,
      capture: { artifact },
      measurements: trial.isolated_ordinals.map((ordinal) => ({
        ordinal,
        center_foreground: true,
        ...footprintMeasurement(),
      })),
      passed: true,
    };
  }));
}

function fallbackRunnerRecord(localTests) {
  const profile = corpus.fallback_profile;
  const resources = runnerResources(profile, "resource_fallback", true);
  const localCase = (id) => localTests.cases.find((testCase) => testCase.id === id);
  return {
    browser_resource_fallback: {
      resident_points: RESIDENT_POINTS,
      adapter: observedAdapter(),
      point_footprint: pointFootprintFacts(profile, "resource_fallback"),
      frame: { transient_texture_bytes: resources.transient_texture_bytes },
      nominal_picks: nominalPicksForTrial(preferredPickTrial().id),
      capture_performed: false,
      multisample_target_allocated: false,
    },
    single_sample_fixture: localCase("single_sample_request_never_becomes_a_fallback"),
    unsupported_fixture: localCase(
      "capability_fallback_precedes_the_viewport_resource_check",
    ),
    passed: true,
  };
}

function localTestArtifact(implementationCommit) {
  const facts = {
    single_sample_request_never_becomes_a_fallback:
      localFallbackFacts("single_sample", "single_sample", 2_201),
    capability_fallback_precedes_the_viewport_resource_check:
      localFallbackFacts("antialiased", "unsupported_fallback", 2_301),
    antialiased_footprint_quality_matrix: {
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
    },
    four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking: {
      pick_independence: {
        display_diameter_physical_pixels: 18,
        nominal_pick_diameter_physical_pixels: 2.4,
        visual_only_probe_offset_physical_pixels: [5, 0],
        visual_only_probe_result: "miss",
        nominal_probe_result: "expected_identity",
      },
    },
    exact_high_water_accounts_for_pick_and_eye_dome_targets: {
      transient_bounds: {
        preferred_non_edl_bytes_per_pixel: 40,
        preferred_edl_bytes_per_pixel: 48,
        fallback_bytes_per_pixel: 8,
        maximum_preferred_physical_pixels: corpus.policy.maximum_multisample_physical_pixels,
        maximum_preferred_transient_bytes:
          corpus.policy.maximum_multisample_physical_pixels * 48,
        renderer_transient_byte_ceiling: corpus.policy.renderer_transient_byte_ceiling,
      },
      resource_fallback: localFallbackProof(2_401),
    },
  };
  return {
    implementation_commit: implementationCommit,
    producer_command: FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
    environment: {
      operating_system: "test-local-os",
      adapter_name: "test-local-adapter",
      backend: "test-local-backend",
    },
    cases: FOOTPRINT_LOCAL_TEST_CASE_IDS.map((id) => ({
      id,
      source_test: id,
      passed: true,
      facts: facts[id],
    })),
  };
}

function localFallbackFacts(requested, selected, pointOrdinal) {
  return {
    physical_width: corpus.canonical_profile.physical_width,
    physical_height: corpus.canonical_profile.physical_height,
    selection: {
      requested,
      selected,
      sample_count: 1,
      multisample_pipeline_created: false,
    },
    resources: null,
    pick_probes: preferredPickProbes(),
    ...localFallbackProof(pointOrdinal),
  };
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

function preferredPickTrial() {
  return corpus.focused_trials.find(({ nominal_pick_ordinals: ordinals }) => (
    Array.isArray(ordinals)
  ));
}

function preferredPickProbes() {
  return preferredPickTrial().nominal_pick_ordinals.map((ordinal) => ({
    ordinal,
    generation: 1,
    source_identity: "21".repeat(32),
    batch_key: 4,
    batch_version: 2,
    point_ordinal: String(ordinal),
  }));
}

function nominalPicksForTrial(trialId) {
  if (trialId !== preferredPickTrial().id) return [];
  return preferredPickProbes().map(({ ordinal: _ordinal, ...expected }) => ({
    expected,
    matched: true,
  }));
}

function observedAdapter() {
  return { name: "test-adapter", backend: "test-backend" };
}

function pointFootprintFacts(profile, selected = "multisample4x") {
  return {
    requested: "antialiased",
    selected,
    nominal_pick_size_physical_pixels: corpus.policy.nominal_pick_diameter_physical_pixels,
    display_size_physical_pixels: projectedDensityDisplayDiameter(
      profile,
      RESIDENT_POINTS,
      corpus.policy,
    ),
  };
}

function runnerResources(profile, selected, pickTargetsRetained) {
  const exact = expectedPointFootprintResources({
    selected,
    physicalWidth: profile.physical_width,
    physicalHeight: profile.physical_height,
    eyeDomeActive: false,
    pickTargetsRetained,
    ceilingBytes: corpus.policy.renderer_transient_byte_ceiling,
  });
  return {
    resident_points: RESIDENT_POINTS,
    transient_texture_bytes: exact.renderer_transient_texture_bytes,
  };
}

function representativeTiming() {
  return {
    frame_interval_samples_milliseconds: Array(30).fill(1),
    frame_submission_samples_milliseconds: Array(30).fill(0.1),
  };
}

function topologyMeasurement(rectangle) {
  return {
    occupancy_normalization: "maximum_absolute_rgba8_channel_delta_from_clear_color_v1",
    channel_threshold: 2,
    metrics: {
      schema: "punctra-browser-region-topology-metrics-v1",
      rectangle: structuredClone(rectangle),
      foreground_threshold: 0.5,
      foreground_pixels: 1,
      partial_edge_pixels: 0,
      foreground_fraction: 0.5,
      solid_2x2_blocks: 1,
      foreground: componentFacts(),
      background: componentFacts(),
    },
  };
}

function componentBridgeMeasurement(rectangle) {
  return {
    occupancy_normalization: "maximum_absolute_rgba8_channel_delta_from_clear_color_v1",
    channel_threshold: 2,
    metrics: {
      schema: "punctra-browser-component-bridge-metrics-v1",
      rectangle: structuredClone(rectangle),
      connectivity: 4,
      minimum_clear_separation_pixels:
        corpus.metric_limits.minimum_component_clear_separation_pixels,
      predecessor_component_count: 1,
      candidate_component_count: 1,
      bridging_candidate_component_count: 0,
      first_bridge: null,
    },
  };
}

function componentFacts() {
  return {
    connectivity: 4,
    component_count: 1,
    left_right_bridge_components: 0,
    top_bottom_bridge_components: 0,
  };
}

function footprintMeasurement() {
  const rectangle = { x: 0, y: 0, width: 8, height: 8 };
  const center = [4, 4];
  return {
    foreground_rgba: [255, 255, 255, 255],
    metrics: {
      schema: "punctra-browser-point-footprint-metrics-v1",
      rectangle,
      center,
      radius_pixels: 2,
      normalization: { background_rgba: [0, 0, 0, 255] },
      coverage: { root_mean_square_error: 0.1, partial_edge_pixels: 8 },
      centroid: { error_pixels: 0 },
      corner_leakage: {
        all_quad_corners_clear: true,
        exact_distance_outer: {
          margin_physical_pixels: 0.75,
          pixel_count: 0,
          coverage: 0,
        },
      },
    },
  };
}

function fullProfileRectangle(profile) {
  return { x: 0, y: 0, width: profile.physical_width, height: profile.physical_height };
}

function validPins() {
  return {
    implementation: {
      commit: "1".repeat(40),
      files: FOOTPRINT_IMPLEMENTATION_PATHS.map((path) => digest(path)),
    },
    verifier: digest(FOOTPRINT_VERIFIER_PATH),
    runtime: {
      package_name: "@punctra/viewer",
      package_version: corpus.release,
      artifacts: FOOTPRINT_RUNTIME_PATHS.map((path) => digest(path)),
    },
    corpus: digest("apps/browser-demo/web/fixtures/footprint-v1/corpus.json"),
    predecessor: structuredClone(corpus.predecessor),
  };
}

function baselineArtifacts() {
  const artifacts = corpus.canonical_trials.map((trial, index) => ({
    kind: "canonical",
    trial_id: trial.id,
    profile_id: corpus.canonical_profile.id,
    artifact: imageMetadata({
      kind: "canonical_baseline_png",
      trialId: trial.id,
      profile: corpus.canonical_profile,
      path: `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}.png`,
      digestCharacter: String(index + 1),
    }),
  }));
  for (const [trialIndex, trial] of corpus.focused_trials.entries()) {
    for (const [profileIndex, profile] of corpus.scale_profiles.entries()) {
      artifacts.push({
        kind: "focused",
        trial_id: trial.id,
        profile_id: profile.id,
        artifact: imageMetadata({
          kind: "focused_baseline_png",
          trialId: trial.id,
          profile,
          path: `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}-${profile.id}.png`,
          digestCharacter: "abcdef"[trialIndex * corpus.scale_profiles.length + profileIndex],
        }),
      });
    }
  }
  return artifacts;
}

function imageMetadata({
  kind,
  trialId,
  profile,
  path,
  digestCharacter,
  recreationIndex = null,
}) {
  return {
    kind,
    trial_id: trialId,
    recreation_index: recreationIndex,
    path,
    filename: path.split("/").at(-1),
    mime_type: "image/png",
    encoding: "png-rgba8-filter-0",
    width: profile.physical_width,
    height: profile.physical_height,
    encoded_byte_length: 1,
    encoded_sha256: digestCharacter.repeat(64),
    decoded_byte_length: profile.physical_width * profile.physical_height * 4,
    decoded_sha256: digestCharacter.repeat(64),
    authority: "presentation_only",
  };
}

function digest(path) {
  return { path, byte_length: 1, sha256: SHA };
}
