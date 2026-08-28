import { sha256Hex } from "./visual-png.js";
import { createVisualValidator } from "./visual-validation.js";

export const FOOTPRINT_CORPUS_SCHEMA = "punctra-browser-point-footprint-corpus-v1";
export const FOOTPRINT_RELEASE = "0.22.0-alpha.1";

const { requireCondition, requireRecord } = createVisualValidator("Point-footprint corpus invalid");

const PREDECESSOR_ARTIFACTS = Object.freeze({
  corpus: Object.freeze({
    path: "../visual-v1/corpus.json",
    byte_length: 28_119,
    sha256: "f8d78105861a6822523be62c914469c98072201f2f343f290b288e607061e580",
  }),
  baseline_inputs: Object.freeze({
    path: "../visual-v1/baseline-inputs.json",
    byte_length: 5_100,
    sha256: "f4f2a5714f17dbb1d285308d74886ea7aee63e0aa52df4dca9b01d7cb7d0993b",
  }),
  release_baseline: Object.freeze({
    path: "../../../../../docs/releases/v0.21-browser-visual-baseline.json",
    byte_length: 33_876,
    sha256: "ce17514938a1259924e341220ec5bb90b705ede4b76668944f2a2fe95ad4b7a5",
  }),
  release_evidence: Object.freeze({
    path: "../../../../../docs/releases/v0.21-browser-visual-evidence.json",
    byte_length: 22_737_391,
    sha256: "eaee2981f4bab9a8bc36f50b7b05e453257637aa50ad82980828514d503b4bfe",
  }),
});

const PREDECESSOR_BASELINES = Object.freeze({
  "generated-neutral-mixed-lod-perspective": Object.freeze([7_391, "200422ed92fc8aff6dcb013d5957f2c1915aa88a816c0f69f0abdbc3d52c37bd", 34, 0.6000000014901161]),
  "generated-elevation-layered-orthographic": Object.freeze([18_869, "a63dd46aa271f7440dd1787d059e0a2826a4a6b2d276654c9f2b95d804812148", 34.19999999552965, 0.6999999955296516]),
  "generated-rgb-hdr-perspective": Object.freeze([15_810, "d5afb1249999329121ecf41eca73ae29c51cac29035e81f6e6a71bb4bde9eca7", 34.29999999701977, 0.6000000014901161]),
  "generated-intensity-sparse-orthographic": Object.freeze([25_047, "4b947d9becc14f451c705ec7448e3349507a20623880e34059f4d6e324577479", 34.100000001490116, 0.30000000447034836]),
  "generated-classification-selection-perspective": Object.freeze([16_982, "ff2d787546a3f568123152ac2da58b48dae59245f7eaadf0d30ee5a30dc0e335", 34.29999999701977, 0.6000000014901161]),
  "autzen-rgb-perspective": Object.freeze([15_337, "7cdb8f7e2b8730e60504d9a4d45f772111ed3ec0fe05740ab1c444eba71a8cc2", 33.899999998509884, 0.6000000014901161]),
  "autzen-classification-perspective": Object.freeze([5_387, "5bbf4163a7f8f79aa2dbc45b1b619f9dfab6531326acd5debab7aa5bf2ab4437", 33.600000001490116, 0.6999999955296516]),
  "autzen-intensity-perspective": Object.freeze([13_999, "d13be19a007924e543be1c217e2515980d12bde7ab0c49efa30657c16e9bb166", 34.29999999701977, 0.6999999955296516]),
  "autzen-elevation-perspective": Object.freeze([10_516, "22a72d45da0b2fd75314356b9c65d1604851245307dca988e1e22d87636f8c0b", 33.900000005960464, 0.6999999955296516]),
});

export async function loadFootprintCorpus(url, fetchImpl = globalThis.fetch) {
  requireCondition(typeof fetchImpl === "function", "fetch implementation is unavailable");
  const response = await fetchImpl(url, { cache: "no-store" });
  requireCondition(response?.ok === true, `could not load corpus: HTTP ${response?.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  const corpus = JSON.parse(new TextDecoder().decode(bytes));
  validateFootprintCorpus(corpus);
  return Object.freeze({
    corpus,
    url: new URL(url, globalThis.location?.href),
    byte_length: bytes.byteLength,
    sha256: await sha256Hex(bytes),
  });
}

export function validateFootprintCorpus(corpus) {
  requireRecord(corpus, "corpus");
  requireCondition(corpus.schema === FOOTPRINT_CORPUS_SCHEMA, "schema differs");
  requireCondition(corpus.release === FOOTPRINT_RELEASE, "release differs");
  validatePredecessor(corpus.predecessor);
  validatePolicy(corpus.policy);
  validateProfile(corpus.canonical_profile, "canonical profile");
  requireCondition(corpus.canonical_profile.recreations === 3, "canonical recreation count differs");
  requireCondition(corpus.canonical_profile.quiet_frames === 30, "canonical quiet-frame count differs");
  requireCondition(Array.isArray(corpus.scale_profiles) && corpus.scale_profiles.length === 2, "scale profiles differ");
  corpus.scale_profiles.forEach((profile) => validateProfile(profile, "scale profile"));
  requireCondition(corpus.scale_profiles.map(({ requested_device_pixel_ratio: value }) => value).join(",") === "1,4", "scale DPR set differs");
  validateProfile(corpus.fallback_profile, "fallback profile");
  requireCondition(corpus.fallback_profile.capture === false, "fallback capture policy differs");
  requireCondition(corpus.fallback_profile.physical_width * corpus.fallback_profile.physical_height > corpus.policy.maximum_multisample_physical_pixels, "fallback profile does not exceed the preferred-path area");
  requireCondition(Array.isArray(corpus.canonical_trials) && corpus.canonical_trials.length === 9, "canonical trial count differs");
  const trialIds = new Set();
  for (const trial of corpus.canonical_trials) {
    requireRecord(trial, "canonical trial");
    requireCondition(typeof trial.id === "string" && trial.id.length > 0, "canonical trial id is invalid");
    requireCondition(!trialIds.has(trial.id), "canonical trial id is duplicated");
    trialIds.add(trial.id);
    validateArtifact(trial.predecessor_baseline, "predecessor baseline");
    const expected = PREDECESSOR_BASELINES[trial.id];
    requireCondition(expected !== undefined, `canonical trial ${trial.id} is not closed`);
    requireCondition(
      trial.predecessor_baseline.path === `../visual-v1/baselines/${trial.id}.png`
        && trial.predecessor_baseline.byte_length === expected[0]
        && trial.predecessor_baseline.sha256 === expected[1],
      `canonical trial ${trial.id} predecessor baseline differs`,
    );
    requireRecord(trial.predecessor_timing, `canonical trial ${trial.id} predecessor timing`);
    requireCondition(
      trial.predecessor_timing.maximum_recreation_frame_interval_p95_milliseconds === expected[2]
        && trial.predecessor_timing.maximum_recreation_frame_submission_p95_milliseconds === expected[3],
      `canonical trial ${trial.id} predecessor timing differs`,
    );
  }
  requireCondition(Array.isArray(corpus.focused_trials) && corpus.focused_trials.length === 3, "focused trial count differs");
  for (const trial of corpus.focused_trials) validateFocusedTrial(trial, trialIds);
  validateLimits(corpus.metric_limits, corpus.timing_limits);
  return corpus;
}

function validatePredecessor(predecessor) {
  requireRecord(predecessor, "predecessor");
  requireCondition(predecessor.release === "0.21.0-alpha.1", "predecessor release differs");
  for (const field of ["corpus", "baseline_inputs", "release_baseline", "release_evidence"]) {
    validateArtifact(predecessor[field], `predecessor ${field}`);
    requireCondition(
      predecessor[field].path === PREDECESSOR_ARTIFACTS[field].path
        && predecessor[field].byte_length === PREDECESSOR_ARTIFACTS[field].byte_length
        && predecessor[field].sha256 === PREDECESSOR_ARTIFACTS[field].sha256,
      `predecessor ${field} differs`,
    );
  }
}

function validatePolicy(policy) {
  requireRecord(policy, "policy");
  requireCondition(policy.requested_footprint === "antialiased", "footprint request differs");
  requireCondition(policy.preferred_status === "multisample4x", "preferred status differs");
  requireCondition(policy.multisample_count === 4, "sample count differs");
  requireCondition(policy.maximum_multisample_physical_pixels === 1_310_720, "multisample area differs");
  requireCondition(policy.renderer_transient_byte_ceiling === 67_108_864, "transient byte ceiling differs");
  requireCondition(policy.nominal_pick_diameter_physical_pixels === 7, "nominal pick diameter differs");
  const size = policy.display_diameter;
  requireRecord(size, "display diameter");
  requireCondition(size.kind === "projected_density_v1", "display-size policy differs");
  requireCondition(size.spacing_fraction === 0.55, "display-size spacing fraction differs");
  requireCondition(size.minimum_physical_pixels === 2 && size.maximum_physical_pixels === 6, "display-size bounds differ");
  requireCondition(size.count_authority === "non_retired_resident_points", "display-size count authority differs");
}

function validateProfile(profile, label) {
  requireRecord(profile, label);
  requireCondition(typeof profile.id === "string" && profile.id.length > 0, `${label} id is invalid`);
  for (const field of ["css_width", "css_height", "requested_device_pixel_ratio", "physical_width", "physical_height"]) {
    requireCondition(Number.isFinite(profile[field]) && profile[field] > 0, `${label} ${field} is invalid`);
  }
  requireCondition(Math.round(profile.css_width * profile.requested_device_pixel_ratio) === profile.physical_width, `${label} physical width differs`);
  requireCondition(Math.round(profile.css_height * profile.requested_device_pixel_ratio) === profile.physical_height, `${label} physical height differs`);
  requireCondition(typeof profile.expected_status === "string" && profile.expected_status.length > 0, `${label} status is invalid`);
}

function validateFocusedTrial(trial, canonicalIds) {
  requireRecord(trial, "focused trial");
  requireCondition(canonicalIds.has(trial.id), "focused trial is not canonical");
  requireCondition(Array.isArray(trial.isolated_ordinals) && trial.isolated_ordinals.length >= 2, "isolated ordinals are incomplete");
  requireCondition(trial.isolated_ordinals.every((value) => Number.isSafeInteger(value) && value >= 0), "isolated ordinal is invalid");
  for (const field of ["dense_regions", "thin_feature_regions"]) {
    if (trial[field] === undefined) continue;
    requireCondition(Array.isArray(trial[field]) && trial[field].length > 0, `${field} is invalid`);
    trial[field].forEach((region) => validateRegion(region, field));
  }
  if (trial.nominal_pick_ordinals !== undefined) {
    requireCondition(trial.nominal_pick_ordinals.every((ordinal) => trial.isolated_ordinals.includes(ordinal)), "pick ordinal is not isolated");
  }
}

function validateRegion(region, label) {
  requireRecord(region, label);
  for (const field of ["x", "y", "width", "height"]) {
    requireCondition(Number.isSafeInteger(region[field]) && region[field] >= (field === "width" || field === "height" ? 1 : 0), `${label} ${field} is invalid`);
  }
}

function validateArtifact(artifact, label) {
  requireRecord(artifact, label);
  requireCondition(typeof artifact.path === "string" && artifact.path.length > 0, `${label} path is invalid`);
  requireCondition(Number.isSafeInteger(artifact.byte_length) && artifact.byte_length > 0, `${label} byte length is invalid`);
  requireCondition(/^[0-9a-f]{64}$/.test(artifact.sha256), `${label} SHA-256 is invalid`);
}

function validateLimits(metric, timing) {
  requireRecord(metric, "metric limits");
  requireCondition(metric.ideal_supersample_axis === 16, "ideal supersample axis differs");
  requireCondition(metric.coverage_rmse === 0.18, "coverage RMSE differs");
  requireCondition(metric.minimum_predecessor_rmse_improvement_fraction === 0.2, "RMSE improvement differs");
  requireCondition(metric.minimum_component_clear_separation_pixels === 2,
    "component clear separation differs");
  requireCondition(metric.maximum_dense_solid_2x2_fraction === 0.8,
    "dense solid-block accepted fraction differs");
  requireRecord(metric.foreground_fraction_predecessor_ratio, "foreground ratio");
  requireCondition(metric.foreground_fraction_predecessor_ratio.minimum === 0.5 && metric.foreground_fraction_predecessor_ratio.maximum === 1.05, "foreground ratio differs");
  requireRecord(timing, "timing limits");
  requireCondition(timing.frame_interval_p95_milliseconds === 50, "frame interval differs");
  requireCondition(timing.frame_submission_p95_milliseconds === 16.7, "frame submission differs");
  requireCondition(timing.maximum_predecessor_ratio === 2, "timing ratio differs");
}
