import { validateFootprintCorpus } from "./footprint-corpus.js";
import {
  COMPONENT_BRIDGE_METRICS_SCHEMA,
  POINT_FOOTPRINT_METRICS_SCHEMA,
  REGION_TOPOLOGY_METRICS_SCHEMA,
} from "./visual-footprint-metrics.js";
import { createVisualValidator, jsonEqual } from "./visual-validation.js";

export const FOOTPRINT_BASELINE_SCHEMA = "punctra-browser-point-footprint-baseline-v1";
export const FOOTPRINT_EVIDENCE_SCHEMA = "punctra-browser-point-footprint-evidence-v1";
export const FOOTPRINT_RELEASE = "0.22.0-alpha.1";
export const FOOTPRINT_VERIFIER_PATH = "scripts/verify-browser-point-footprint.mjs";
export const FOOTPRINT_LOCAL_TEST_SCHEMA = "punctra-render-wgpu-point-footprint-test-evidence-v1";
export const FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND = "PUNCTRA_REQUIRE_GPU=1 PUNCTRA_POINT_FOOTPRINT_EVIDENCE_PATH=apps/browser-demo/web/fixtures/footprint-v1/local-test-evidence.json cargo test -p render-wgpu --test offscreen write_point_footprint_test_evidence -- --ignored --exact";
export const FOOTPRINT_LOCAL_TEST_CASE_IDS = Object.freeze([
  "single_sample_request_never_becomes_a_fallback",
  "capability_fallback_precedes_the_viewport_resource_check",
  "antialiased_footprint_quality_matrix",
  "four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking",
  "exact_high_water_accounts_for_pick_and_eye_dome_targets",
]);

export const FOOTPRINT_RUNTIME_PATHS = Object.freeze([
  "apps/browser-demo/web/package.json",
  "apps/browser-demo/web/pkg/browser_demo.js",
  "apps/browser-demo/web/pkg/browser_demo_bg.wasm",
]);

export const FOOTPRINT_EXTERNAL_NONCLAIMS = Object.freeze({
  cross_browser: false,
  cross_operating_system: false,
  cross_adapter: false,
  cross_device: false,
  physical_display_presentation: false,
  general_browser_zoom: false,
  responsive_composition: false,
  independent_human: false,
  independent_adopter: false,
  professional_usability: false,
  registry_or_cdn_publication: false,
  final_visual_quality: false,
  support_qualified: false,
  api_stable: false,
  beta: false,
  release_candidate: false,
  v1: false,
});

export const FOOTPRINT_UNAVAILABLE_MEASUREMENTS = Object.freeze([
  "driver_gpu_memory_bytes",
  "energy",
  "gpu_completion_time",
  "physical_display_panel_presentation",
  "process_resident_memory_bytes",
  "thermal_state",
]);

const CORPUS_PATH = "apps/browser-demo/web/fixtures/footprint-v1/corpus.json";
export const FOOTPRINT_IMPLEMENTATION_PATHS = Object.freeze([
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
  "apps/browser-demo/web/footprint-artifacts.js",
  "apps/browser-demo/web/footprint-artifacts.test.mjs",
  "apps/browser-demo/web/footprint-corpus.js",
  "apps/browser-demo/web/footprint-corpus.test.mjs",
  "apps/browser-demo/web/footprint-evidence.js",
  "apps/browser-demo/web/footprint-evidence.test.mjs",
  "apps/browser-demo/web/footprint-export.js",
  "apps/browser-demo/web/footprint-export.test.mjs",
  "apps/browser-demo/web/footprint-main.js",
  "apps/browser-demo/web/footprint-qualification.js",
  "apps/browser-demo/web/footprint-records.js",
  "apps/browser-demo/web/footprint-records.test.mjs",
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
]);
const SHA256 = /^[0-9a-f]{64}$/;
const COMMIT = /^[0-9a-f]{40}$/;
const SOURCE_IDENTITY = /^[0-9a-f]{64}$/;
const { requireCondition, requireRecord } = createVisualValidator("Point-footprint evidence invalid");

/** Validates the immutable v0.22 baseline and its exact implementation boundary. */
export function validatePointFootprintBaseline(baseline, corpus) {
  validateFootprintCorpus(corpus);
  requireRecord(baseline, "baseline");
  requireExactKeys(baseline, [
    "schema", "release", "pins", "environment", "candidate_images", "focused_images",
    "external_evidence",
  ], "baseline");
  requireCondition(baseline.schema === FOOTPRINT_BASELINE_SCHEMA, "baseline schema differs");
  requireCondition(baseline.release === FOOTPRINT_RELEASE, "baseline release differs");
  validatePins(baseline.pins, corpus);
  validatePointFootprintEnvironment(baseline.environment, corpus);
  validateCandidateImages(baseline.candidate_images, corpus);
  validateFocusedImages(baseline.focused_images, baseline.candidate_images, corpus);
  requireJsonEqual(baseline.external_evidence, FOOTPRINT_EXTERNAL_NONCLAIMS, "baseline external nonclaims");
  return baseline;
}

/** Binds the corpus bytes loaded by the browser to the running qualification pins. */
export function validatePointFootprintRunInputs(inputs, runningPins) {
  requireRecord(inputs, "run inputs");
  requireExactKeys(inputs, ["footprint", "visual"], "run inputs");
  const { footprint, visual } = inputs;
  requireRecord(footprint, "loaded footprint corpus");
  requireRecord(visual, "loaded predecessor visual corpus");
  requireRecord(runningPins, "running pins");
  validateFootprintCorpus(footprint.corpus);
  validateDigestRecord(runningPins.corpus, "running corpus pin");
  requireRecord(runningPins.predecessor, "running predecessor pins");

  requireJsonEqual({
    path: CORPUS_PATH,
    byte_length: footprint.byte_length,
    sha256: footprint.sha256,
  }, runningPins.corpus, "footprint digest");
  requireJsonEqual(
    runningPins.predecessor,
    footprint.corpus.predecessor,
    "predecessor pins",
  );

  const predecessorCorpus = footprint.corpus.predecessor.corpus;
  const expectedVisualUrl = new URL(predecessorCorpus.path, footprint.url).href;
  requireCondition(visual.corpus_url === expectedVisualUrl, "visual URL differs");
  requireCondition(
    visual.corpus_byte_length === predecessorCorpus.byte_length,
    "visual length differs",
  );
  requireCondition(
    visual.corpus_sha256 === predecessorCorpus.sha256,
    "visual digest differs",
  );
  return inputs;
}

/**
 * Derives every v0.22 gate from raw evidence. Optional recomputed maps let an
 * offline verifier bind reports to decoded PNG bytes and v0.21 timing bytes.
 */
export function derivePointFootprintEvidenceSummary(evidence, options) {
  requireRecord(options, "verification options");
  const { baseline, corpus } = options;
  validatePointFootprintBaseline(baseline, corpus);
  validateEvidenceEnvelope(evidence, baseline, corpus, options.baselineIdentity);
  const artifacts = validateArtifactRegistry(evidence.artifacts);
  const metricBindings = new Map();
  const failures = [];

  validateCanonicalTrials(
    evidence.canonical_trials,
    baseline,
    corpus,
    artifacts,
    metricBindings,
    failures,
    options.predecessorTiming,
    evidence.environment,
  );
  validateFocusedTrials(
    evidence.focused_trials,
    baseline,
    corpus,
    artifacts,
    metricBindings,
    failures,
    evidence.environment,
  );
  validateLocalGpuFixture(
    evidence.local_gpu_fixture,
    corpus,
    artifacts,
    failures,
  );
  validateFallbackTrials(
    evidence.pick_identity_reference,
    evidence.fallback_trials,
    corpus,
    artifacts,
    failures,
    evidence.environment,
  );
  validateRecomputedMetrics(metricBindings, options.recomputedMetrics);

  const usedArtifactPaths = new Set();
  for (const binding of metricBindings.values()) usedArtifactPaths.add(binding.artifact_path);
  collectEvidenceArtifactPaths(evidence, usedArtifactPaths);
  for (const artifactPath of artifacts.keys()) {
    gate(usedArtifactPaths.has(artifactPath), failures, `artifact ${artifactPath} is unbound`);
  }

  return {
    passed: failures.length === 0,
    canonical_trials: corpus.canonical_trials.length,
    canonical_recreations: corpus.canonical_trials.length * corpus.canonical_profile.recreations,
    focused_scale_trials: corpus.focused_trials.length * 3,
    fallback_trials: 3,
    artifacts: artifacts.size,
    metric_reports: metricBindings.size,
    failures,
  };
}

/** Verifies the recorded summary and rejects any failed or tampered gate. */
export function verifyPointFootprintEvidence(evidence, options) {
  const derived = derivePointFootprintEvidenceSummary(evidence, options);
  requireJsonEqual(evidence.summary, derived, "derived evidence summary");
  requireCondition(derived.passed, `derived gates failed: ${derived.failures.join("; ")}`);
  return Object.freeze({ evidence, summary: derived });
}

/** Validates the exact local renderer artifact envelope consumed by the runner. */
export function validatePointFootprintLocalTestArtifact(artifact, implementationCommit) {
  requireRecord(artifact, "local renderer test artifact");
  requireExactKeys(artifact, [
    "schema", "implementation_commit", "producer_command", "environment", "cases",
  ], "local renderer test artifact");
  requireCondition(artifact.schema === FOOTPRINT_LOCAL_TEST_SCHEMA,
    "local renderer test artifact schema differs");
  requireCondition(COMMIT.test(implementationCommit), "local renderer implementation commit is invalid");
  requireCondition(artifact.implementation_commit === implementationCommit,
    "local renderer test artifact implementation commit differs");
  requireCondition(artifact.producer_command === FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
    "local renderer test artifact producer command differs");
  validateLocalEnvironment(artifact.environment, "local renderer test artifact");
  requireArray(artifact.cases, "local renderer test artifact cases");
  requireJsonEqual(artifact.cases.map(({ id }) => id), FOOTPRINT_LOCAL_TEST_CASE_IDS,
    "local renderer test artifact case order or membership");
  for (let index = 0; index < artifact.cases.length; index += 1) {
    const testCase = artifact.cases[index];
    const id = FOOTPRINT_LOCAL_TEST_CASE_IDS[index];
    requireExactKeys(testCase, ["id", "source_test", "passed", "facts"],
      `local renderer test artifact case ${id}`);
    requireCondition(testCase.id === id && testCase.source_test === id,
      `local renderer test artifact case ${id} source differs`);
    requireCondition(testCase.passed === true,
      `local renderer test artifact case ${id} did not pass`);
    requireRecord(testCase.facts, `local renderer test artifact case ${id} facts`);
  }
  return artifact;
}

export function summarizeFootprintTiming(samples) {
  requireArray(samples, "timing samples");
  requireCondition(samples.length > 0, "timing samples are empty");
  const ordered = [...samples];
  for (const sample of ordered) requireFiniteNonnegative(sample, "timing sample");
  ordered.sort((left, right) => left - right);
  const percentile = (value) => ordered[Math.max(0, Math.ceil(ordered.length * value / 100) - 1)];
  return {
    count: ordered.length,
    p50: percentile(50),
    p95: percentile(95),
    maximum: ordered.at(-1),
  };
}

/** Mirrors the host's f64 density calculation followed by its serialized f32 cast. */
export function projectedDensityDisplayDiameter(profile, residentPoints, policy) {
  requireRecord(profile, "viewport profile");
  requireRecord(policy, "point-footprint policy");
  const physicalPixels = positiveInteger(profile.physical_width, "physical width")
    * positiveInteger(profile.physical_height, "physical height");
  nonnegativeSafeInteger(residentPoints, "resident point count");
  const diameter = Math.sqrt(physicalPixels / Math.max(residentPoints, 1))
    * policy.display_diameter.spacing_fraction;
  return Math.fround(Math.min(
    policy.display_diameter.maximum_physical_pixels,
    Math.max(policy.display_diameter.minimum_physical_pixels, diameter),
  ));
}

/** Applies the accepted dense-region bound and its conditional predecessor budget. */
export function evaluateDenseSolidBlockBudget(predecessor, candidate, limits) {
  requireRecord(predecessor, "predecessor dense-region report");
  requireRecord(candidate, "candidate dense-region report");
  requireRecord(limits, "dense-region limits");
  requireJsonEqual(candidate.rectangle, predecessor.rectangle, "dense-region rectangles");
  const rectangle = predecessor.rectangle;
  validateRectangle(rectangle, "dense-region budget");
  const possibleBlocks = Math.max(0, rectangle.width - 1) * Math.max(0, rectangle.height - 1);
  requireCondition(possibleBlocks > 0, "dense-region budget has no two-by-two cells");
  for (const [label, value] of [
    ["predecessor", predecessor.solid_2x2_blocks],
    ["candidate", candidate.solid_2x2_blocks],
  ]) {
    requireCondition(Number.isSafeInteger(value) && value >= 0 && value <= possibleBlocks,
      `${label} dense-region solid blocks are invalid`);
  }
  const acceptedFraction = limits.maximum_dense_solid_2x2_fraction;
  const predecessorRatio = limits.dense_solid_block_predecessor_ratio;
  requireCondition(Number.isFinite(acceptedFraction)
    && acceptedFraction >= 0 && acceptedFraction <= 1,
  "dense-region accepted fraction is invalid");
  requireCondition(Number.isFinite(predecessorRatio) && predecessorRatio >= 1,
    "dense-region predecessor ratio is invalid");

  const predecessorFraction = predecessor.solid_2x2_blocks / possibleBlocks;
  const alreadyWithinAcceptedBound = predecessorFraction <= acceptedFraction;
  const maximumCandidateBlocks = alreadyWithinAcceptedBound
    ? predecessor.solid_2x2_blocks * predecessorRatio
    : predecessor.solid_2x2_blocks - 1;
  return Object.freeze({
    passed: candidate.solid_2x2_blocks <= maximumCandidateBlocks,
    rule: alreadyWithinAcceptedBound ? "retain_within_predecessor_ratio" : "strict_reduction",
    predecessor_solid_2x2_fraction: predecessorFraction,
    maximum_candidate_solid_2x2_blocks: maximumCandidateBlocks,
  });
}

/** Projects browser PNG metadata onto the closed evidence artifact shape. */
export function createPointFootprintImageArtifact(metadata, profileId) {
  requireRecord(metadata, "browser PNG metadata");
  const artifact = {
    kind: metadata.kind,
    trial_id: metadata.trial_id,
    recreation_index: metadata.recreation_index,
    profile_id: profileId,
    path: metadata.path,
    mime_type: metadata.mime_type,
    encoding: metadata.encoding,
    width: metadata.width,
    height: metadata.height,
    encoded_byte_length: metadata.encoded_byte_length,
    encoded_sha256: metadata.encoded_sha256,
    decoded_byte_length: metadata.decoded_byte_length,
    decoded_sha256: metadata.decoded_sha256,
    authority: metadata.authority,
  };
  validateImageArtifact(artifact, "browser PNG artifact");
  return artifact;
}

/** Binds a runner occupancy measurement to one artifact and stable metric id. */
export function createTopologyMetricBinding({
  metricId,
  artifactPath,
  backgroundRgba,
  measurement,
}) {
  requireRecord(measurement, "runner topology measurement");
  requireCondition(
    measurement.occupancy_normalization
      === "maximum_absolute_rgba8_channel_delta_from_clear_color_v1",
    "runner topology normalization differs",
  );
  const binding = {
    kind: "background_difference_topology_v1",
    metric_id: metricId,
    artifact_path: artifactPath,
    rectangle: measurement.metrics?.rectangle,
    background_rgba: backgroundRgba,
    maximum_background_channel_delta: measurement.channel_threshold,
    foreground_threshold: measurement.metrics?.foreground_threshold,
    report: measurement.metrics,
  };
  validateTopologyBinding(binding, artifactPath, new Map(), "runner topology binding");
  return binding;
}

/** Binds a paired predecessor/candidate component-bridge measurement. */
export function createComponentBridgeMetricBinding({
  metricId,
  predecessorArtifactPath,
  candidateArtifactPath,
  backgroundRgba,
  measurement,
}) {
  requireRecord(measurement, "runner component-bridge measurement");
  requireCondition(
    measurement.occupancy_normalization
      === "maximum_absolute_rgba8_channel_delta_from_clear_color_v1",
    "runner component-bridge normalization differs",
  );
  const binding = {
    kind: "background_difference_component_bridges_v1",
    metric_id: metricId,
    predecessor_artifact_path: predecessorArtifactPath,
    candidate_artifact_path: candidateArtifactPath,
    rectangle: measurement.metrics?.rectangle,
    background_rgba: backgroundRgba,
    maximum_background_channel_delta: measurement.channel_threshold,
    foreground_threshold: 0.5,
    minimum_clear_separation_pixels:
      measurement.metrics?.minimum_clear_separation_pixels,
    report: measurement.metrics,
  };
  validateComponentBridgeBinding(
    binding,
    predecessorArtifactPath,
    candidateArtifactPath,
    binding.minimum_clear_separation_pixels,
    new Map(),
    "runner component-bridge binding",
  );
  return binding;
}

/** Binds one runner isolated-footprint measurement to its captured PNG. */
export function createFootprintMetricBinding({ metricId, artifactPath, measurement }) {
  requireRecord(measurement, "runner footprint measurement");
  const report = measurement.metrics;
  requireRecord(report, "runner footprint metrics");
  const binding = {
    kind: "known_endpoint_disk_v1",
    metric_id: metricId,
    artifact_path: artifactPath,
    rectangle: report.rectangle,
    center: report.center,
    radius_pixels: report.radius_pixels,
    foreground_rgba: measurement.foreground_rgba,
    background_rgba: report.normalization?.background_rgba,
    report,
  };
  validateFootprintBinding(binding, artifactPath, new Map(), "runner footprint binding");
  return binding;
}

/** Derives target components and rejects a runner total that differs. */
export function createPointFootprintResourceEvidence({
  pointFootprint,
  profile,
  eyeDomeActive = false,
  pickTargetsRetained,
  rendererTransientTextureBytes,
  ceilingBytes,
}) {
  requireRecord(pointFootprint, "runner point-footprint facts");
  requireRecord(profile, "runner viewport profile");
  const resources = expectedPointFootprintResources({
    selected: pointFootprint.selected,
    physicalWidth: profile.physical_width,
    physicalHeight: profile.physical_height,
    eyeDomeActive,
    pickTargetsRetained,
    ceilingBytes,
  });
  requireCondition(resources.renderer_transient_texture_bytes === rendererTransientTextureBytes,
    "runner renderer transient total differs from exact target components");
  return resources;
}

/** Returns the closed transient-target accounting expected from one observation. */
export function expectedPointFootprintResources({
  selected,
  physicalWidth,
  physicalHeight,
  eyeDomeActive,
  pickTargetsRetained,
  ceilingBytes,
}) {
  const pixels = positiveInteger(physicalWidth, "physical width")
    * positiveInteger(physicalHeight, "physical height");
  requireCondition(typeof eyeDomeActive === "boolean", "eye-dome fact is invalid");
  requireCondition(typeof pickTargetsRetained === "boolean", "pick-retention fact is invalid");
  positiveInteger(ceilingBytes, "renderer transient ceiling");
  const multisample = selected === "multisample4x";
  requireCondition(
    multisample || ["single_sample", "unsupported_fallback", "resource_fallback"].includes(selected),
    "selected footprint status is invalid",
  );
  const bytes = (bytesPerPixel) => pixels * bytesPerPixel;
  const resources = {
    physical_pixels: pixels,
    eye_dome_active: eyeDomeActive,
    pick_targets_retained: pickTargetsRetained,
    multisample_color_bytes: multisample ? bytes(16) : 0,
    multisample_depth_bytes: multisample ? bytes(16) : 0,
    single_sample_depth_bytes: !multisample || eyeDomeActive ? bytes(4) : 0,
    resolved_eye_dome_color_bytes: eyeDomeActive ? bytes(4) : 0,
    pick_color_bytes: pickTargetsRetained ? bytes(4) : 0,
    separate_pick_depth_bytes: multisample && pickTargetsRetained ? bytes(4) : 0,
    renderer_transient_texture_bytes: 0,
    renderer_transient_byte_ceiling: ceilingBytes,
  };
  resources.renderer_transient_texture_bytes = Object.entries(resources)
    .filter(([name]) => name.endsWith("_bytes") && name !== "renderer_transient_texture_bytes"
      && name !== "renderer_transient_byte_ceiling")
    .reduce((total, [, value]) => total + value, 0);
  return resources;
}

function validatePins(pins, corpus) {
  requireRecord(pins, "baseline pins");
  requireExactKeys(pins, ["implementation", "verifier", "runtime", "corpus", "predecessor"], "baseline pins");
  requireRecord(pins.implementation, "implementation pin");
  requireExactKeys(pins.implementation, ["commit", "files"], "implementation pin");
  requireCondition(COMMIT.test(pins.implementation.commit), "implementation commit is not a full Git object id");
  validateDigestRecords(pins.implementation.files, "implementation files");
  const implementationPaths = pins.implementation.files.map(({ path }) => path);
  for (const required of FOOTPRINT_IMPLEMENTATION_PATHS) {
    requireCondition(implementationPaths.includes(required), `implementation files omit ${required}`);
  }
  requireJsonEqual(implementationPaths, FOOTPRINT_IMPLEMENTATION_PATHS,
    "implementation file paths");
  validateDigestRecord(pins.verifier, "verifier pin");
  requireCondition(pins.verifier.path === FOOTPRINT_VERIFIER_PATH, "verifier path differs");
  requireRecord(pins.runtime, "runtime pin");
  requireExactKeys(pins.runtime, ["package_name", "package_version", "artifacts"], "runtime pin");
  requireCondition(pins.runtime.package_name === "@punctra/viewer", "runtime package name differs");
  requireCondition(pins.runtime.package_version === FOOTPRINT_RELEASE, "runtime package version differs");
  validateDigestRecords(pins.runtime.artifacts, "runtime artifacts");
  requireJsonEqual(pins.runtime.artifacts.map(({ path }) => path), FOOTPRINT_RUNTIME_PATHS, "runtime artifact paths");
  validateDigestRecord(pins.corpus, "corpus pin");
  requireCondition(pins.corpus.path === CORPUS_PATH, "corpus path differs");
  requireRecord(pins.predecessor, "predecessor pins");
  requireJsonEqual(pins.predecessor, corpus.predecessor, "predecessor pins");
}

function validateCandidateImages(images, corpus) {
  requireArray(images, "candidate images");
  requireCondition(images.length === corpus.canonical_trials.length, "candidate image count differs");
  for (let index = 0; index < images.length; index += 1) {
    const trial = corpus.canonical_trials[index];
    const image = images[index];
    validateImageArtifact(image, `candidate image ${trial.id}`);
    requireCondition(image.trial_id === trial.id, `candidate image ${trial.id} trial differs`);
    requireCondition(
      image.path === `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}.png`,
      `candidate image ${trial.id} path differs`,
    );
    requireCondition(
      image.width === corpus.canonical_profile.physical_width
        && image.height === corpus.canonical_profile.physical_height,
      `candidate image ${trial.id} dimensions differ`,
    );
  }
}

function validateFocusedImages(images, candidateImages, corpus) {
  requireArray(images, "focused baseline images");
  const profiles = [corpus.canonical_profile, ...corpus.scale_profiles];
  const expectedPairs = corpus.focused_trials.flatMap((trial) => (
    profiles.map((profile) => [trial, profile])
  ));
  requireCondition(images.length === expectedPairs.length, "focused baseline image count differs");
  for (let index = 0; index < expectedPairs.length; index += 1) {
    const [trial, profile] = expectedPairs[index];
    const image = images[index];
    validateImageArtifact(image, `focused baseline image ${trial.id}/${profile.id}`);
    const suffix = profile.id === corpus.canonical_profile.id ? "" : `-${profile.id}`;
    requireCondition(image.trial_id === trial.id && image.profile_id === profile.id,
      `focused baseline image ${trial.id}/${profile.id} identity differs`);
    requireCondition(
      image.path === `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}${suffix}.png`,
      `focused baseline image ${trial.id}/${profile.id} path differs`,
    );
    requireCondition(image.width === profile.physical_width && image.height === profile.physical_height,
      `focused baseline image ${trial.id}/${profile.id} dimensions differ`);
    if (profile.id === corpus.canonical_profile.id) {
      const canonical = candidateImages.find(({ trial_id: trialId }) => trialId === trial.id);
      requireCondition(canonical !== undefined, `canonical baseline image ${trial.id} is absent`);
      requireCondition(image.encoded_byte_length === canonical.encoded_byte_length
        && image.encoded_sha256 === canonical.encoded_sha256
        && image.decoded_byte_length === canonical.decoded_byte_length
        && image.decoded_sha256 === canonical.decoded_sha256,
      `focused baseline image ${trial.id}/${profile.id} differs from its canonical baseline`);
    }
  }
}

function validateEvidenceEnvelope(evidence, baseline, corpus, baselineIdentity) {
  requireRecord(evidence, "evidence");
  requireExactKeys(evidence, [
    "schema", "release", "mode", "started_at", "completed_at", "baseline", "pins",
    "environment", "artifacts", "canonical_trials", "focused_trials",
    "local_gpu_fixture", "pick_identity_reference", "fallback_trials", "summary",
    "external_evidence", "unavailable_measurements", "fatal_error",
  ], "evidence");
  requireCondition(evidence.schema === FOOTPRINT_EVIDENCE_SCHEMA, "evidence schema differs");
  requireCondition(evidence.release === FOOTPRINT_RELEASE, "evidence release differs");
  requireCondition(evidence.mode === "verify", "evidence mode must be verify");
  const started = timestamp(evidence.started_at, "evidence start");
  const completed = timestamp(evidence.completed_at, "evidence completion");
  requireCondition(completed >= started, "evidence completion precedes its start");
  requireCondition(evidence.fatal_error === null, "evidence contains a fatal error");
  validateDigestRecord(evidence.baseline, "evidence baseline pin");
  if (baselineIdentity !== undefined) requireJsonEqual(evidence.baseline, baselineIdentity, "evidence baseline pin");
  requireJsonEqual(evidence.pins, baseline.pins, "evidence pins");
  validatePointFootprintEnvironment(evidence.environment, corpus);
  requireJsonEqual(evidence.environment, baseline.environment, "evidence environment differs from baseline");
  requireJsonEqual(evidence.external_evidence, FOOTPRINT_EXTERNAL_NONCLAIMS, "evidence external nonclaims");
  requireJsonEqual(evidence.unavailable_measurements, FOOTPRINT_UNAVAILABLE_MEASUREMENTS,
    "evidence unavailable measurements");
}

export function validatePointFootprintEnvironment(environment, corpus) {
  requireRecord(environment, "environment");
  requireExactKeys(environment, [
    "browser_user_agent", "browser_platform", "operating_system", "adapter_name", "backend",
    "same_adapter_for_scale_trials", "physical_display_observed",
  ], "environment");
  for (const field of ["browser_user_agent", "browser_platform", "operating_system", "adapter_name", "backend"]) {
    requireCondition(typeof environment[field] === "string" && environment[field].length > 0, `environment ${field} is invalid`);
  }
  requireCondition(environment.same_adapter_for_scale_trials === true, "scale trials do not bind one adapter");
  requireCondition(environment.physical_display_observed === false, "offscreen evidence cannot claim physical presentation");
  requireCondition(corpus.canonical_profile.expected_status === "multisample4x", "canonical corpus status differs");
}

function validateArtifactRegistry(registry) {
  requireRecord(registry, "artifact registry");
  requireExactKeys(registry, ["png", "local_test_results"], "artifact registry");
  const artifacts = new Map();
  requireArray(registry.png, "PNG artifacts");
  for (const artifact of registry.png) {
    validateImageArtifact(artifact, "PNG artifact");
    addArtifact(artifacts, artifact, "PNG artifact");
  }
  requireArray(registry.local_test_results, "local test-result artifacts");
  requireCondition(registry.local_test_results.length === 1,
    "artifact registry must bind exactly one local test-result artifact");
  for (const artifact of registry.local_test_results) {
    requireExactKeys(artifact, [
      "path", "byte_length", "sha256", "media_type", "producer_command",
    ], "local test-result artifact");
    validateDigestRecord(artifact, "local test-result artifact");
    requireCondition(artifact.media_type === "application/json",
      "local test-result media type must be application/json");
    requireCondition(artifact.producer_command === FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
      "local test-result producer command differs");
    addArtifact(artifacts, artifact, "local test-result artifact");
  }
  return artifacts;
}

function validateCanonicalTrials(
  trials,
  baseline,
  corpus,
  artifacts,
  metricBindings,
  failures,
  predecessorTiming,
  environment,
) {
  requireArray(trials, "canonical trials");
  requireCondition(trials.length === corpus.canonical_trials.length, "canonical trial count differs");
  const candidateByTrial = new Map(baseline.candidate_images.map((image) => [image.trial_id, image]));
  for (let trialIndex = 0; trialIndex < trials.length; trialIndex += 1) {
    const trial = trials[trialIndex];
    const expected = corpus.canonical_trials[trialIndex];
    requireRecord(trial, `canonical trial ${expected.id}`);
    requireExactKeys(trial, ["trial_id", "predecessor_topology", "recreations"], `canonical trial ${expected.id}`);
    requireCondition(trial.trial_id === expected.id, `canonical trial ${expected.id} order differs`);
    validateTopologyBinding(
      trial.predecessor_topology,
      expected.predecessor_baseline.path,
      metricBindings,
      `canonical trial ${expected.id} predecessor topology`,
    );
    requireArray(trial.recreations, `canonical trial ${expected.id} recreations`);
    requireCondition(
      trial.recreations.length === corpus.canonical_profile.recreations,
      `canonical trial ${expected.id} recreation count differs`,
    );
    for (let index = 0; index < trial.recreations.length; index += 1) {
      const recreation = trial.recreations[index];
      const label = `canonical trial ${expected.id} recreation ${index}`;
      requireRecord(recreation, label);
      requireExactKeys(recreation, [
        "index", "adapter", "resident_points", "point_footprint", "timing", "resources", "capture_artifact_path",
        "candidate_topology", "component_bridge_check", "feature_checks", "dense_region_checks",
      ], label);
      requireCondition(recreation.index === index, `${label} index differs`);
      validateAdapter(recreation.adapter, label);
      requireJsonEqual(recreation.adapter, {
        name: environment.adapter_name,
        backend: environment.backend,
      }, `${label} adapter`);
      positiveInteger(recreation.resident_points, `${label} resident points`);
      validatePointFootprintFacts(
        recreation.point_footprint,
        "antialiased",
        "multisample4x",
        corpus.canonical_profile,
        recreation.resident_points,
        corpus,
        failures,
        label,
      );
      validateTiming(
        recreation.timing,
        corpus.timing_limits,
        expected.predecessor_timing,
        predecessorTiming,
        expected.id,
        index,
        failures,
        label,
      );
      validateResources(
        recreation.resources,
        recreation.point_footprint.selected,
        corpus.canonical_profile,
        corpus,
        failures,
        label,
      );
      const capture = requireArtifact(artifacts, recreation.capture_artifact_path, "image/png", `${label} capture`);
      requireCondition(capture.trial_id === expected.id, `${label} capture trial differs`);
      requireCondition(capture.recreation_index === index, `${label} capture recreation differs`);
      requireCondition(capture.width === corpus.canonical_profile.physical_width
        && capture.height === corpus.canonical_profile.physical_height, `${label} capture dimensions differ`);
      validateTopologyBinding(recreation.candidate_topology, capture.path, metricBindings, `${label} candidate topology`);
      validateTopologyPair(
        trial.predecessor_topology.report,
        recreation.candidate_topology.report,
        corpus.metric_limits,
        failures,
        label,
      );
      validateComponentBridgeBinding(
        recreation.component_bridge_check,
        expected.predecessor_baseline.path,
        capture.path,
        corpus.metric_limits.minimum_component_clear_separation_pixels,
        metricBindings,
        `${label} component bridges`,
      );
      gate(
        recreation.component_bridge_check.report.bridging_candidate_component_count === 0,
        failures,
        `${label} bridges separated predecessor components`,
      );
      validateFeatureChecks(recreation.feature_checks, corpus.metric_limits, failures, label);
      const focusedContract = corpus.focused_trials.find(({ id }) => id === expected.id);
      validateDenseRegionChecks(
        recreation.dense_region_checks,
        focusedContract?.dense_regions ?? [],
        expected.predecessor_baseline.path,
        capture.path,
        metricBindings,
        corpus.metric_limits,
        failures,
        label,
      );
      const candidateBaseline = candidateByTrial.get(expected.id);
      gate(capture.decoded_sha256 === candidateBaseline.decoded_sha256, failures, `${label} decoded capture differs from pinned candidate baseline`);
    }
  }
}

function validateTiming(
  timing,
  limits,
  corpusPredecessorTiming,
  predecessorTiming,
  trialId,
  recreationIndex,
  failures,
  label,
) {
  requireRecord(timing, `${label} timing`);
  requireExactKeys(timing, [
    "frame_interval_samples_milliseconds", "frame_submission_samples_milliseconds",
    "frame_interval", "frame_submission", "predecessor_frame_interval_p95_milliseconds",
    "predecessor_frame_submission_p95_milliseconds", "first_coverage_milliseconds",
    "settled_view_milliseconds",
  ], `${label} timing`);
  for (const [samplesField, summaryField] of [
    ["frame_interval_samples_milliseconds", "frame_interval"],
    ["frame_submission_samples_milliseconds", "frame_submission"],
  ]) {
    const samples = timing[samplesField];
    requireArray(samples, `${label} ${samplesField}`);
    requireCondition(samples.length === 30, `${label} ${samplesField} must contain 30 frames`);
    requireJsonEqual(timing[summaryField], summarizeFootprintTiming(samples), `${label} ${summaryField}`);
  }
  for (const field of [
    "predecessor_frame_interval_p95_milliseconds", "predecessor_frame_submission_p95_milliseconds",
    "first_coverage_milliseconds", "settled_view_milliseconds",
  ]) requireFiniteNonnegative(timing[field], `${label} ${field}`);
  requireCondition(
    timing.predecessor_frame_interval_p95_milliseconds
      === corpusPredecessorTiming.maximum_recreation_frame_interval_p95_milliseconds,
    `${label} predecessor interval p95 differs from the corpus`,
  );
  requireCondition(
    timing.predecessor_frame_submission_p95_milliseconds
      === corpusPredecessorTiming.maximum_recreation_frame_submission_p95_milliseconds,
    `${label} predecessor submission p95 differs from the corpus`,
  );
  if (predecessorTiming !== undefined) {
    const expected = lookupMap(predecessorTiming, `${trialId}:${recreationIndex}`, "predecessor timing");
    requireCondition(timing.predecessor_frame_interval_p95_milliseconds === expected.frame_interval_p95_milliseconds,
      `${label} predecessor interval p95 differs`);
    requireCondition(timing.predecessor_frame_submission_p95_milliseconds === expected.frame_submission_p95_milliseconds,
      `${label} predecessor submission p95 differs`);
  }
  gate(timing.frame_interval.p95 <= limits.frame_interval_p95_milliseconds, failures, `${label} interval p95 exceeds ceiling`);
  gate(timing.frame_submission.p95 <= limits.frame_submission_p95_milliseconds, failures, `${label} submission p95 exceeds ceiling`);
  gate(predecessorRatioPass(timing.frame_interval.p95, timing.predecessor_frame_interval_p95_milliseconds, limits.maximum_predecessor_ratio), failures,
    `${label} interval p95 exceeds predecessor ratio`);
  gate(predecessorRatioPass(timing.frame_submission.p95, timing.predecessor_frame_submission_p95_milliseconds, limits.maximum_predecessor_ratio), failures,
    `${label} submission p95 exceeds predecessor ratio`);
  gate(timing.first_coverage_milliseconds <= limits.first_coverage_milliseconds, failures, `${label} first coverage exceeds ceiling`);
  gate(timing.settled_view_milliseconds <= limits.settled_view_milliseconds, failures, `${label} settled view exceeds ceiling`);
}

function validateFocusedTrials(trials, baseline, corpus, artifacts, metricBindings, failures, environment) {
  requireArray(trials, "focused trials");
  const profiles = [corpus.canonical_profile, ...corpus.scale_profiles];
  const expectedPairs = corpus.focused_trials.flatMap((trial) => profiles.map((profile) => [trial, profile]));
  const baselines = new Map(baseline.focused_images.map((image) => [
    `${image.trial_id}:${image.profile_id}`,
    image,
  ]));
  requireCondition(trials.length === expectedPairs.length, "focused DPR trial count differs");
  for (let index = 0; index < expectedPairs.length; index += 1) {
    const [expectedTrial, profile] = expectedPairs[index];
    const trial = trials[index];
    const label = `focused trial ${expectedTrial.id} ${profile.id}`;
    requireRecord(trial, label);
    requireExactKeys(trial, [
      "trial_id", "profile_id", "adapter", "resident_points", "point_footprint", "resources", "candidate_artifact_path",
      "baseline_artifact_path", "isolated_footprints",
    ], label);
    requireCondition(trial.trial_id === expectedTrial.id && trial.profile_id === profile.id, `${label} order differs`);
    validateAdapter(trial.adapter, label);
    requireJsonEqual(trial.adapter, {
      name: environment.adapter_name,
      backend: environment.backend,
    }, `${label} adapter`);
    positiveInteger(trial.resident_points, `${label} resident points`);
    validatePointFootprintFacts(
      trial.point_footprint,
      "antialiased",
      profile.expected_status,
      profile,
      trial.resident_points,
      corpus,
      failures,
      label,
    );
    validateResources(trial.resources, trial.point_footprint.selected, profile, corpus, failures, label);
    const artifact = requireArtifact(artifacts, trial.candidate_artifact_path, "image/png", `${label} candidate capture`);
    requireCondition(artifact.trial_id === expectedTrial.id, `${label} candidate capture trial differs`);
    requireCondition(artifact.width === profile.physical_width && artifact.height === profile.physical_height,
      `${label} candidate capture dimensions differ`);
    const pinnedBaseline = baselines.get(`${expectedTrial.id}:${profile.id}`);
    requireCondition(pinnedBaseline !== undefined, `${label} baseline is absent`);
    requireCondition(trial.baseline_artifact_path === pinnedBaseline.path, `${label} baseline path differs`);
    gate(artifact.decoded_sha256 === pinnedBaseline.decoded_sha256, failures,
      `${label} candidate differs from the same-pin focused baseline`);
    validateIsolatedFootprints(trial.isolated_footprints, expectedTrial, trial, corpus, metricBindings, failures, label);
  }
}

function validateIsolatedFootprints(samples, expectedTrial, trial, corpus, metricBindings, failures, label) {
  requireArray(samples, `${label} isolated footprints`);
  requireCondition(samples.length === expectedTrial.isolated_ordinals.length, `${label} isolated footprint count differs`);
  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index];
    const ordinal = expectedTrial.isolated_ordinals[index];
    requireRecord(sample, `${label} isolated footprint ${ordinal}`);
    requireExactKeys(sample, ["ordinal", "center_foreground", "candidate"], `${label} isolated footprint ${ordinal}`);
    requireCondition(sample.ordinal === ordinal, `${label} isolated footprint ordinal differs`);
    requireCondition(sample.center_foreground === true, `${label} isolated footprint center is not foreground`);
    validateFootprintBinding(sample.candidate, trial.candidate_artifact_path, metricBindings, `${label} candidate footprint ${ordinal}`);
    const candidate = sample.candidate.report;
    gate(candidate.coverage.root_mean_square_error <= corpus.metric_limits.coverage_rmse, failures,
      `${label} footprint ${ordinal} RMSE exceeds ceiling`);
    gate(candidate.corner_leakage.exact_distance_outer.pixel_count
      <= corpus.metric_limits.maximum_outer_leakage_pixels, failures,
    `${label} footprint ${ordinal} leaks beyond radius margin`);
    gate(candidate.corner_leakage.all_quad_corners_clear === true, failures,
      `${label} footprint ${ordinal} fills a quad corner`);
    gate(candidate.centroid.error_pixels !== null
      && candidate.centroid.error_pixels <= corpus.metric_limits.maximum_centroid_distance_pixels,
    failures, `${label} footprint ${ordinal} centroid exceeds ceiling`);
  }
}

function validateLocalGpuFixture(fixture, corpus, artifacts, failures) {
  requireRecord(fixture, "local GPU fixture");
  requireExactKeys(fixture, [
    "evidence_source", "browser_observation", "environment", "local_test_evidence",
    "diameters_physical_pixels", "subpixel_center_phases", "preferred",
    "single_sample", "pick_independence", "transient_bounds",
    "resource_fallback",
  ], "local GPU fixture");
  requireCondition(fixture.evidence_source === "local_renderer_gpu_test",
    "local GPU fixture source differs");
  requireCondition(fixture.browser_observation === null,
    "local GPU fixture must not fabricate a browser observation");
  validateLocalEnvironment(fixture.environment, "local GPU fixture");
  requireExactKeys(fixture.local_test_evidence, [
    "quality", "pick_independence", "resource_accounting",
  ], "local GPU fixture test evidence");
  validateLocalTestEvidence(fixture.local_test_evidence.quality,
    "antialiased_footprint_quality_matrix", artifacts, "local GPU quality fixture");
  validateLocalTestEvidence(fixture.local_test_evidence.pick_independence,
    "four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking",
    artifacts, "local GPU pick-independence fixture");
  validateLocalTestEvidence(fixture.local_test_evidence.resource_accounting,
    "exact_high_water_accounts_for_pick_and_eye_dome_targets",
    artifacts, "local GPU resource-accounting fixture");
  requireJsonEqual(fixture.diameters_physical_pixels, [2, 3, 4, 5, 6],
    "local GPU fixture diameters");
  requireArray(fixture.subpixel_center_phases, "local GPU fixture subpixel phases");
  requireCondition(fixture.subpixel_center_phases.length >= 8,
    "local GPU fixture requires at least eight subpixel phases");
  const phases = new Set();
  for (const phase of fixture.subpixel_center_phases) {
    requireCondition(Array.isArray(phase) && phase.length === 2
      && phase.every((value) => Number.isFinite(value) && value >= 0 && value < 1),
    "local GPU fixture subpixel phase is invalid");
    phases.add(phase.join(","));
  }
  requireCondition(phases.size === fixture.subpixel_center_phases.length,
    "local GPU fixture subpixel phases are duplicated");
  requireRecord(fixture.preferred, "local GPU preferred metrics");
  requireExactKeys(fixture.preferred, [
    "maximum_coverage_rmse", "maximum_exact_distance_outer_leakage_pixels",
    "all_centers_foreground", "all_quad_corners_clear",
  ], "local GPU preferred metrics");
  requireRecord(fixture.single_sample, "local GPU single-sample metrics");
  requireExactKeys(fixture.single_sample, ["coverage_rmse_at_preferred_worst_case"],
    "local GPU single-sample metrics");
  requireFiniteNonnegative(fixture.preferred.maximum_coverage_rmse,
    "local GPU preferred RMSE");
  requireFiniteNonnegative(fixture.single_sample.coverage_rmse_at_preferred_worst_case,
    "local GPU single-sample RMSE");
  requireCondition(Number.isSafeInteger(fixture.preferred.maximum_exact_distance_outer_leakage_pixels)
    && fixture.preferred.maximum_exact_distance_outer_leakage_pixels >= 0,
  "local GPU exact-distance leakage is invalid");
  gate(fixture.preferred.maximum_coverage_rmse <= corpus.metric_limits.coverage_rmse,
    failures, "local GPU preferred RMSE exceeds ceiling");
  gate(fixture.preferred.maximum_coverage_rmse
    <= fixture.single_sample.coverage_rmse_at_preferred_worst_case
      * (1 - corpus.metric_limits.minimum_predecessor_rmse_improvement_fraction),
  failures, "local GPU preferred RMSE improvement is insufficient");
  gate(fixture.preferred.maximum_exact_distance_outer_leakage_pixels
    <= corpus.metric_limits.maximum_outer_leakage_pixels,
  failures, "local GPU preferred footprint leaks beyond radius margin");
  gate(fixture.preferred.all_centers_foreground === true,
    failures, "local GPU preferred footprint lost an ideal center");
  gate(fixture.preferred.all_quad_corners_clear === true,
    failures, "local GPU preferred footprint leaked into a quad corner");
  requireJsonEqual(fixture.pick_independence, {
    display_diameter_physical_pixels: 18,
    nominal_pick_diameter_physical_pixels: 2.4,
    visual_only_probe_offset_physical_pixels: [5, 0],
    visual_only_probe_result: "miss",
    nominal_probe_result: "expected_identity",
  }, "local GPU pick independence");
  requireJsonEqual(fixture.transient_bounds, {
    preferred_non_edl_bytes_per_pixel: 40,
    preferred_edl_bytes_per_pixel: 48,
    fallback_bytes_per_pixel: 8,
    maximum_preferred_physical_pixels: corpus.policy.maximum_multisample_physical_pixels,
    maximum_preferred_transient_bytes:
      corpus.policy.maximum_multisample_physical_pixels * 48,
    renderer_transient_byte_ceiling: corpus.policy.renderer_transient_byte_ceiling,
  }, "local GPU exact transient bounds");
  requireRecord(fixture.resource_fallback, "local GPU resource fallback");
  requireExactKeys(fixture.resource_fallback, [
    "hard_circle_mask", "nominal_pick_identity",
  ], "local GPU resource fallback");
  validateHardCircleMask(fixture.resource_fallback.hard_circle_mask,
    "local GPU resource fallback");
  validateNominalPickIdentity(fixture.resource_fallback.nominal_pick_identity,
    "local GPU resource fallback");
}

function validateFallbackTrials(reference, trials, corpus, artifacts, failures, environment) {
  validatePickReference(reference, corpus, artifacts);
  requireArray(trials, "fallback trials");
  const expected = [
    ["single_sample", "single_sample", "local_renderer_test", "single_sample", "single_sample_request_never_becomes_a_fallback"],
    ["unsupported_fallback", "antialiased", "local_renderer_test", "unsupported_fallback", "capability_fallback_precedes_the_viewport_resource_check"],
    ["resource_fallback", "antialiased", "attended_browser", "resource_fallback", null],
  ];
  requireCondition(trials.length === expected.length, "fallback trial count differs");
  for (let index = 0; index < expected.length; index += 1) {
    const [id, requested, source, selected, testCase] = expected[index];
    const trial = trials[index];
    const label = `${id} trial`;
    requireRecord(trial, label);
    requireExactKeys(trial, [
      "id", "evidence_source", "physical_width", "physical_height", "selection",
      "resources", "pick_probes", "hard_circle_mask", "nominal_pick_identity",
      "browser_observation", "local_test_evidence",
    ], label);
    requireCondition(trial.id === id && trial.evidence_source === source, `${label} identity differs`);
    positiveInteger(trial.physical_width, `${label} physical width`);
    positiveInteger(trial.physical_height, `${label} physical height`);
    const profile = id === "resource_fallback" ? corpus.fallback_profile : {
      physical_width: trial.physical_width,
      physical_height: trial.physical_height,
    };
    if (id === "resource_fallback") {
      requireCondition(trial.physical_width === profile.physical_width
        && trial.physical_height === profile.physical_height,
      `${label} viewport differs from the corpus`);
    }
    if (source === "local_renderer_test") {
      validateLocalRendererSelection(trial.selection, requested, selected, label);
      requireCondition(trial.browser_observation === null, `${label} must not fabricate a browser observation`);
      validatePickProbes(trial.pick_probes, label);
      requireJsonEqual(trial.pick_probes, reference.pick_probes, `${label} pick identities`);
      requireCondition(trial.resources === null,
        `${label} must not fabricate browser resource diagnostics`);
      validateHardCircleMask(trial.hard_circle_mask, label);
      validateNominalPickIdentity(trial.nominal_pick_identity, label);
      validateLocalTestEvidence(trial.local_test_evidence, testCase, artifacts, label);
    } else {
      validateResourceFallbackSelection(trial.selection, requested, selected, label);
      requireCondition(trial.local_test_evidence === null, `${label} local-test evidence must be null`);
      requireCondition(trial.hard_circle_mask === null
        && trial.nominal_pick_identity === null,
      `${label} browser observation must not substitute for local fallback proof`);
      requireRecord(trial.browser_observation, `${label} browser observation`);
      requireExactKeys(trial.browser_observation, [
        "profile_id", "capture_performed", "adapter", "resident_points", "point_footprint", "resources",
      ], `${label} browser observation`);
      requireCondition(trial.browser_observation.profile_id === corpus.fallback_profile.id, `${label} browser profile differs`);
      requireCondition(trial.browser_observation.capture_performed === false, `${label} must not capture the oversized viewport`);
      validateAdapter(trial.browser_observation.adapter, label);
      requireJsonEqual(trial.browser_observation.adapter, {
        name: environment.adapter_name,
        backend: environment.backend,
      }, `${label} browser adapter`);
      requireJsonEqual(trial.pick_probes, reference.pick_probes, `${label} pick identities`);
      validateResources(trial.resources, selected, profile, corpus, failures, label);
      gate(trial.resources.multisample_color_bytes === 0 && trial.resources.multisample_depth_bytes === 0,
        failures, `${label} allocated multisample targets`);
      requireCondition(trial.resources.eye_dome_active === false,
        `${label} fixture unexpectedly enabled eye-dome targets`);
      requireCondition(trial.resources.pick_targets_retained === true,
        `${label} did not retain the targets used by its pick probes`);
      positiveInteger(trial.browser_observation.resident_points, `${label} browser resident points`);
      validatePointFootprintFacts(
        trial.browser_observation.point_footprint,
        requested,
        selected,
        corpus.fallback_profile,
        trial.browser_observation.resident_points,
        corpus,
        failures,
        label,
      );
      requireJsonEqual(trial.browser_observation.resources, trial.resources, `${label} browser resources`);
    }
  }
}

function validatePickReference(reference, corpus, artifacts) {
  requireRecord(reference, "preferred pick reference");
  requireExactKeys(reference, [
    "profile_id", "resident_points", "point_footprint", "pick_probes", "pick_mask_artifact_path",
  ], "preferred pick reference");
  requireCondition(reference.profile_id === corpus.canonical_profile.id,
    "preferred pick reference profile differs");
  positiveInteger(reference.resident_points, "preferred pick reference resident points");
  const failures = [];
  validatePointFootprintFacts(
    reference.point_footprint,
    "antialiased",
    "multisample4x",
    corpus.canonical_profile,
    reference.resident_points,
    corpus,
    failures,
    "preferred pick reference",
  );
  requireCondition(failures.length === 0, failures.join("; "));
  validatePickProbes(reference.pick_probes, "preferred pick reference");
  const pickTrial = corpus.focused_trials.find(({ nominal_pick_ordinals: ordinals }) => ordinals !== undefined);
  requireCondition(pickTrial !== undefined, "preferred pick trial is absent from the corpus");
  requireJsonEqual(reference.pick_probes.map(({ ordinal }) => ordinal), pickTrial.nominal_pick_ordinals,
    "preferred pick ordinals");
  requireArtifact(artifacts, reference.pick_mask_artifact_path, "image/png", "preferred pick mask");
}

function validateLocalTestEvidence(evidence, caseId, artifacts, label) {
  requireRecord(evidence, `${label} local test evidence`);
  requireExactKeys(evidence, ["artifact_path", "case", "source_test", "result"],
    `${label} local test evidence`);
  requireCondition(artifacts.has(evidence.artifact_path), `${label} local test output artifact is absent`);
  requireCondition(artifacts.get(evidence.artifact_path).media_type === "application/json",
    `${label} local test output media type differs`);
  requireCondition(evidence.case === caseId && evidence.source_test === caseId
    && evidence.result === "passed", `${label} test result differs`);
}

function validatePointFootprintFacts(
  facts,
  requested,
  selected,
  profile,
  residentPoints,
  corpus,
  failures,
  label,
) {
  requireRecord(facts, `${label} point footprint`);
  requireExactKeys(facts, [
    "requested", "selected", "nominal_pick_size_physical_pixels", "display_size_physical_pixels",
  ], `${label} point footprint`);
  requireCondition(facts.requested === requested, `${label} requested footprint differs`);
  requireCondition(facts.selected === selected, `${label} selected footprint differs`);
  requireCondition(facts.nominal_pick_size_physical_pixels
    === corpus.policy.nominal_pick_diameter_physical_pixels, `${label} nominal pick size differs`);
  requireFiniteNonnegative(facts.display_size_physical_pixels, `${label} display size`);
  gate(
    Object.is(
      Math.fround(facts.display_size_physical_pixels),
      projectedDensityDisplayDiameter(profile, residentPoints, corpus.policy),
    ),
    failures,
    `${label} display size differs from resident-point density`,
  );
}

function validateLocalRendererSelection(selection, requested, selected, label) {
  requireRecord(selection, `${label} renderer selection`);
  requireExactKeys(selection, [
    "requested", "selected", "sample_count", "multisample_pipeline_created",
  ],
    `${label} renderer selection`);
  requireCondition(selection.requested === requested, `${label} requested footprint differs`);
  requireCondition(selection.selected === selected, `${label} selected footprint differs`);
  requireCondition(selection.sample_count === 1, `${label} renderer sample count differs`);
  requireCondition(selection.multisample_pipeline_created === false,
    `${label} unexpectedly created a multisample pipeline`);
}

function validateResourceFallbackSelection(selection, requested, selected, label) {
  requireRecord(selection, `${label} renderer selection`);
  requireExactKeys(selection, [
    "requested", "selected", "sample_count", "multisample_target_allocated",
  ], `${label} renderer selection`);
  requireCondition(selection.requested === requested, `${label} requested footprint differs`);
  requireCondition(selection.selected === selected, `${label} selected footprint differs`);
  requireCondition(selection.sample_count === 1, `${label} renderer sample count differs`);
  requireCondition(selection.multisample_target_allocated === false,
    `${label} unexpectedly allocated multisample targets`);
}

function validateResources(resources, selected, profile, corpus, failures, label) {
  requireRecord(resources, `${label} resources`);
  const expected = expectedPointFootprintResources({
    selected,
    physicalWidth: profile.physical_width,
    physicalHeight: profile.physical_height,
    eyeDomeActive: resources.eye_dome_active,
    pickTargetsRetained: resources.pick_targets_retained,
    ceilingBytes: corpus.policy.renderer_transient_byte_ceiling,
  });
  requireJsonEqual(resources, expected, `${label} exact transient resources`);
  requireCondition(resources.eye_dome_active === false,
    `${label} qualification fixture unexpectedly enabled eye-dome targets`);
  gate(resources.renderer_transient_texture_bytes <= resources.renderer_transient_byte_ceiling,
    failures, `${label} renderer transient bytes exceed ceiling`);
  if (selected === "multisample4x") {
    gate(resources.physical_pixels <= corpus.policy.maximum_multisample_physical_pixels,
      failures, `${label} preferred path exceeds pixel ceiling`);
  }
}

function validateTopologyBinding(binding, artifactPath, metricBindings, label) {
  requireRecord(binding, label);
  requireExactKeys(binding, [
    "kind", "metric_id", "artifact_path", "rectangle", "background_rgba",
    "maximum_background_channel_delta", "foreground_threshold", "report",
  ], label);
  requireCondition(binding.kind === "background_difference_topology_v1", `${label} kind differs`);
  requireCondition(binding.artifact_path === artifactPath, `${label} artifact path differs`);
  validateMetricId(binding.metric_id, metricBindings, label);
  validateRectangle(binding.rectangle, label);
  validateRgba(binding.background_rgba, `${label} background`);
  requireCondition(Number.isInteger(binding.maximum_background_channel_delta)
    && binding.maximum_background_channel_delta >= 0 && binding.maximum_background_channel_delta <= 8,
  `${label} background delta is invalid`);
  requireCondition(binding.foreground_threshold === 0.5, `${label} foreground threshold differs`);
  validateTopologyReport(binding.report, binding.rectangle, label);
  metricBindings.set(binding.metric_id, binding);
}

function validateComponentBridgeBinding(
  binding,
  predecessorArtifactPath,
  candidateArtifactPath,
  minimumClearSeparationPixels,
  metricBindings,
  label,
) {
  requireRecord(binding, label);
  requireExactKeys(binding, [
    "kind", "metric_id", "predecessor_artifact_path", "candidate_artifact_path",
    "rectangle", "background_rgba", "maximum_background_channel_delta",
    "foreground_threshold", "minimum_clear_separation_pixels", "report",
  ], label);
  requireCondition(binding.kind === "background_difference_component_bridges_v1",
    `${label} kind differs`);
  requireCondition(binding.predecessor_artifact_path === predecessorArtifactPath,
    `${label} predecessor artifact path differs`);
  requireCondition(binding.candidate_artifact_path === candidateArtifactPath,
    `${label} candidate artifact path differs`);
  validateMetricId(binding.metric_id, metricBindings, label);
  validateRectangle(binding.rectangle, label);
  validateRgba(binding.background_rgba, `${label} background`);
  requireCondition(Number.isInteger(binding.maximum_background_channel_delta)
    && binding.maximum_background_channel_delta >= 0
    && binding.maximum_background_channel_delta <= 8,
  `${label} background delta is invalid`);
  requireCondition(binding.foreground_threshold === 0.5,
    `${label} foreground threshold differs`);
  requireCondition(binding.minimum_clear_separation_pixels === minimumClearSeparationPixels,
    `${label} clear separation differs`);
  validateComponentBridgeReport(
    binding.report,
    binding.rectangle,
    minimumClearSeparationPixels,
    label,
  );
  metricBindings.set(binding.metric_id, binding);
}

function validateComponentBridgeReport(report, rectangle, minimumClearSeparationPixels, label) {
  requireRecord(report, `${label} report`);
  requireExactKeys(report, [
    "schema", "rectangle", "connectivity", "minimum_clear_separation_pixels",
    "predecessor_component_count", "candidate_component_count",
    "bridging_candidate_component_count", "first_bridge",
  ], `${label} report`);
  requireCondition(report.schema === COMPONENT_BRIDGE_METRICS_SCHEMA,
    `${label} report schema differs`);
  requireJsonEqual(report.rectangle, rectangle, `${label} report rectangle`);
  requireCondition(report.connectivity === 4, `${label} report connectivity differs`);
  requireCondition(report.minimum_clear_separation_pixels === minimumClearSeparationPixels,
    `${label} report clear separation differs`);
  for (const field of [
    "predecessor_component_count",
    "candidate_component_count",
    "bridging_candidate_component_count",
  ]) {
    nonnegativeSafeInteger(report[field], `${label} report ${field}`);
  }
  if (report.bridging_candidate_component_count === 0) {
    requireCondition(report.first_bridge === null, `${label} report fabricates a bridge witness`);
    return;
  }
  requireRecord(report.first_bridge, `${label} report first bridge`);
  requireExactKeys(report.first_bridge, [
    "candidate_component", "predecessor_components",
  ], `${label} report first bridge`);
  nonnegativeSafeInteger(
    report.first_bridge.candidate_component,
    `${label} report bridge candidate component`,
  );
  requireCondition(Array.isArray(report.first_bridge.predecessor_components)
    && report.first_bridge.predecessor_components.length === 2
    && report.first_bridge.predecessor_components.every(
      (value) => Number.isSafeInteger(value) && value >= 0,
    ), `${label} report predecessor bridge pair is invalid`);
}

function validateFootprintBinding(binding, artifactPath, metricBindings, label) {
  requireRecord(binding, label);
  requireExactKeys(binding, [
    "kind", "metric_id", "artifact_path", "rectangle", "center", "radius_pixels",
    "foreground_rgba", "background_rgba", "report",
  ], label);
  requireCondition(binding.kind === "known_endpoint_disk_v1", `${label} kind differs`);
  requireCondition(binding.artifact_path === artifactPath, `${label} artifact path differs`);
  validateMetricId(binding.metric_id, metricBindings, label);
  validateRectangle(binding.rectangle, label);
  requireCondition(Array.isArray(binding.center) && binding.center.length === 2
    && binding.center.every(Number.isFinite), `${label} center is invalid`);
  requireCondition(Number.isFinite(binding.radius_pixels) && binding.radius_pixels >= 1
    && binding.radius_pixels <= 3, `${label} radius differs from diameter 2 through 6`);
  validateRgba(binding.foreground_rgba, `${label} foreground`);
  validateRgba(binding.background_rgba, `${label} background`);
  validateFootprintReport(binding.report, binding, label);
  metricBindings.set(binding.metric_id, binding);
}

function validateTopologyReport(report, rectangle, label) {
  requireRecord(report, `${label} report`);
  requireCondition(report.schema === REGION_TOPOLOGY_METRICS_SCHEMA, `${label} report schema differs`);
  requireJsonEqual(report.rectangle, rectangle, `${label} report rectangle`);
  requireCondition(Number.isSafeInteger(report.foreground_pixels) && report.foreground_pixels >= 0, `${label} foreground pixels are invalid`);
  requireCondition(Number.isFinite(report.foreground_fraction) && report.foreground_fraction >= 0
    && report.foreground_fraction <= 1, `${label} foreground fraction is invalid`);
  requireCondition(Number.isSafeInteger(report.solid_2x2_blocks) && report.solid_2x2_blocks >= 0, `${label} solid blocks are invalid`);
  for (const name of ["foreground", "background"]) {
    requireRecord(report[name], `${label} ${name} components`);
    requireCondition(report[name].connectivity === 4, `${label} ${name} connectivity differs`);
    for (const field of ["component_count", "left_right_bridge_components", "top_bottom_bridge_components"]) {
      requireCondition(Number.isSafeInteger(report[name][field]) && report[name][field] >= 0, `${label} ${name} ${field} is invalid`);
    }
  }
}

function validateFootprintReport(report, binding, label) {
  requireRecord(report, `${label} report`);
  requireCondition(report.schema === POINT_FOOTPRINT_METRICS_SCHEMA, `${label} report schema differs`);
  requireJsonEqual(report.rectangle, binding.rectangle, `${label} report rectangle`);
  requireJsonEqual(report.center, binding.center, `${label} report center`);
  requireCondition(report.radius_pixels === binding.radius_pixels, `${label} report radius differs`);
  requireRecord(report.coverage, `${label} coverage`);
  for (const field of ["root_mean_square_error", "partial_edge_pixels"]) {
    requireCondition(Number.isFinite(report.coverage[field]) && report.coverage[field] >= 0, `${label} coverage ${field} is invalid`);
  }
  requireRecord(report.centroid, `${label} centroid`);
  requireCondition(report.centroid.error_pixels === null
    || Number.isFinite(report.centroid.error_pixels) && report.centroid.error_pixels >= 0,
  `${label} centroid error is invalid`);
  requireRecord(report.corner_leakage?.exact_distance_outer, `${label} exact-distance leakage`);
  requireCondition(report.corner_leakage.all_quad_corners_clear === true
    || report.corner_leakage.all_quad_corners_clear === false,
  `${label} quad-corner disposition is invalid`);
  requireCondition(report.corner_leakage.exact_distance_outer.margin_physical_pixels === 0.75,
    `${label} exact-distance margin differs`);
  requireCondition(Number.isSafeInteger(report.corner_leakage.exact_distance_outer.pixel_count)
    && report.corner_leakage.exact_distance_outer.pixel_count >= 0, `${label} leakage count is invalid`);
}

function validateTopologyPair(predecessor, candidate, limits, failures, label) {
  const ratio = predecessor.foreground_fraction === 0 ? Number.POSITIVE_INFINITY
    : candidate.foreground_fraction / predecessor.foreground_fraction;
  gate(ratio >= limits.foreground_fraction_predecessor_ratio.minimum
    && ratio <= limits.foreground_fraction_predecessor_ratio.maximum,
  failures, `${label} foreground fraction ratio differs`);
  gate(candidate.foreground.component_count >= predecessor.foreground.component_count,
    failures, `${label} merged predecessor foreground components`);
  gate(candidate.foreground.left_right_bridge_components <= predecessor.foreground.left_right_bridge_components,
    failures, `${label} introduced a left-right foreground bridge`);
  gate(candidate.foreground.top_bottom_bridge_components <= predecessor.foreground.top_bottom_bridge_components,
    failures, `${label} introduced a top-bottom foreground bridge`);
}

function validateFeatureChecks(features, limits, failures, label) {
  requireArray(features, `${label} feature checks`);
  requireCondition(features.length > 0, `${label} feature checks are empty`);
  const ids = new Set();
  for (const feature of features) {
    requireRecord(feature, `${label} feature`);
    requireExactKeys(feature, ["id", "predecessor_foreground_pixels", "candidate_foreground_pixels", "centroid_distance_pixels"], `${label} feature`);
    requireCondition(typeof feature.id === "string" && feature.id.length > 0 && !ids.has(feature.id), `${label} feature id is invalid or duplicated`);
    ids.add(feature.id);
    positiveInteger(feature.predecessor_foreground_pixels, `${label} feature predecessor foreground pixels`);
    requireCondition(Number.isSafeInteger(feature.candidate_foreground_pixels) && feature.candidate_foreground_pixels >= 0,
      `${label} feature candidate foreground pixels are invalid`);
    requireFiniteNonnegative(feature.centroid_distance_pixels, `${label} feature centroid distance`);
    gate(feature.candidate_foreground_pixels > 0, failures, `${label} feature ${feature.id} disappeared`);
    gate(feature.centroid_distance_pixels <= limits.maximum_feature_centroid_distance_pixels,
      failures, `${label} feature ${feature.id} centroid moved`);
  }
}

function validateDenseRegionChecks(
  regions,
  expectedRegions,
  predecessorArtifactPath,
  candidateArtifactPath,
  metricBindings,
  limits,
  failures,
  label,
) {
  requireArray(regions, `${label} dense region checks`);
  requireCondition(regions.length === expectedRegions.length,
    `${label} dense region count differs`);
  for (let index = 0; index < regions.length; index += 1) {
    const region = regions[index];
    requireRecord(region, `${label} dense region ${index}`);
    requireExactKeys(region, ["rectangle", "predecessor", "candidate"],
      `${label} dense region ${index}`);
    requireJsonEqual(region.rectangle, expectedRegions[index],
      `${label} dense region ${index} rectangle`);
    validateTopologyBinding(
      region.predecessor,
      predecessorArtifactPath,
      metricBindings,
      `${label} dense region ${index} predecessor`,
    );
    validateTopologyBinding(
      region.candidate,
      candidateArtifactPath,
      metricBindings,
      `${label} dense region ${index} candidate`,
    );
    const budget = evaluateDenseSolidBlockBudget(
      region.predecessor.report,
      region.candidate.report,
      limits,
    );
    gate(budget.passed, failures,
      `${label} dense region ${index} violates its ${budget.rule} solid-block budget`);
  }
}

function validatePickProbes(probes, label) {
  requireArray(probes, `${label} pick probes`);
  requireCondition(probes.length > 0, `${label} pick probes are empty`);
  for (const probe of probes) {
    requireRecord(probe, `${label} pick probe`);
    requireExactKeys(probe, [
      "ordinal", "generation", "source_identity", "batch_key", "batch_version", "point_ordinal",
    ], `${label} pick probe`);
    requireCondition(Number.isSafeInteger(probe.ordinal) && probe.ordinal >= 0, `${label} pick ordinal is invalid`);
    requireCondition(SOURCE_IDENTITY.test(probe.source_identity), `${label} source identity is invalid`);
    for (const field of ["generation", "batch_key", "batch_version"]) {
      requireCondition(Number.isSafeInteger(probe[field]) && probe[field] >= 0, `${label} ${field} is invalid`);
    }
    requireCondition(probe.point_ordinal === String(probe.ordinal), `${label} point ordinal differs`);
  }
}

function validateHardCircleMask(mask, label) {
  requireRecord(mask, `${label} hard-circle mask`);
  requireExactKeys(mask, [
    "width", "height", "byte_length", "reference_sha256", "observed_sha256", "equivalent",
  ], `${label} hard-circle mask`);
  positiveInteger(mask.width, `${label} hard-circle mask width`);
  positiveInteger(mask.height, `${label} hard-circle mask height`);
  requireCondition(mask.byte_length === mask.width * mask.height,
    `${label} hard-circle mask byte length differs`);
  requireCondition(SHA256.test(mask.reference_sha256) && SHA256.test(mask.observed_sha256),
    `${label} hard-circle mask SHA-256 is invalid`);
  requireCondition(mask.reference_sha256 === mask.observed_sha256
    && mask.equivalent === true, `${label} hard-circle mask differs from single sample`);
}

function validateNominalPickIdentity(identity, label) {
  requireRecord(identity, `${label} nominal pick identity`);
  requireExactKeys(identity, ["expected", "observed", "matched"],
    `${label} nominal pick identity`);
  for (const [name, value] of [["expected", identity.expected], ["observed", identity.observed]]) {
    requireRecord(value, `${label} nominal pick ${name}`);
    requireExactKeys(value, [
      "generation", "source_identity", "batch_key", "batch_version", "point_ordinal",
    ], `${label} nominal pick ${name}`);
    requireCondition(SOURCE_IDENTITY.test(value.source_identity),
      `${label} nominal pick ${name} source identity is invalid`);
    for (const field of ["generation", "batch_key", "batch_version"]) {
      requireCondition(Number.isSafeInteger(value[field]) && value[field] >= 0,
        `${label} nominal pick ${name} ${field} is invalid`);
    }
    requireCondition(Number.isSafeInteger(value.point_ordinal) && value.point_ordinal >= 0,
      `${label} nominal pick ${name} ordinal is invalid`);
  }
  requireJsonEqual(identity.observed, identity.expected, `${label} nominal pick identity`);
  requireCondition(identity.matched === true, `${label} nominal pick identity did not match`);
}

function validateRecomputedMetrics(bindings, recomputed) {
  if (recomputed === undefined) return;
  requireCondition(recomputed instanceof Map, "recomputed metrics must be a Map");
  requireCondition(recomputed.size === bindings.size, "recomputed metric count differs");
  for (const [metricId, binding] of bindings) {
    requireCondition(recomputed.has(metricId), `recomputed metrics omit ${metricId}`);
    requireJsonEqual(recomputed.get(metricId), binding.report, `recomputed metric ${metricId}`);
  }
}

function collectEvidenceArtifactPaths(evidence, paths) {
  for (const trial of evidence.canonical_trials) {
    for (const recreation of trial.recreations) paths.add(recreation.capture_artifact_path);
  }
  for (const trial of evidence.focused_trials) {
    paths.add(trial.candidate_artifact_path);
  }
  for (const provenance of Object.values(evidence.local_gpu_fixture.local_test_evidence)) {
    paths.add(provenance.artifact_path);
  }
  paths.add(evidence.pick_identity_reference.pick_mask_artifact_path);
  for (const trial of evidence.fallback_trials) {
    if (trial.local_test_evidence !== null) paths.add(trial.local_test_evidence.artifact_path);
  }
}

function validateImageArtifact(artifact, label) {
  validateDigestRecord({
    path: artifact?.path,
    byte_length: artifact?.encoded_byte_length,
    sha256: artifact?.encoded_sha256,
  }, label);
  requireExactKeys(artifact, [
    "kind", "trial_id", "recreation_index", "profile_id", "path", "mime_type", "encoding",
    "width", "height", "encoded_byte_length", "encoded_sha256", "decoded_byte_length",
    "decoded_sha256", "authority",
  ], label);
  requireCondition(typeof artifact.kind === "string" && artifact.kind.length > 0, `${label} kind is invalid`);
  requireCondition(artifact.trial_id === null || typeof artifact.trial_id === "string", `${label} trial id is invalid`);
  requireCondition(artifact.recreation_index === null
    || Number.isSafeInteger(artifact.recreation_index) && artifact.recreation_index >= 0, `${label} recreation index is invalid`);
  requireCondition(artifact.profile_id === null || typeof artifact.profile_id === "string", `${label} profile id is invalid`);
  requireCondition(artifact.mime_type === "image/png" && artifact.encoding === "png-rgba8-filter-0", `${label} encoding differs`);
  positiveInteger(artifact.width, `${label} width`);
  positiveInteger(artifact.height, `${label} height`);
  requireCondition(artifact.decoded_byte_length === artifact.width * artifact.height * 4, `${label} decoded byte length differs`);
  requireCondition(SHA256.test(artifact.decoded_sha256), `${label} decoded SHA-256 is invalid`);
  requireCondition(artifact.authority === "presentation_only", `${label} authority differs`);
}

function validateDigestRecords(records, label) {
  requireArray(records, label);
  requireCondition(records.length > 0, `${label} are empty`);
  const paths = new Set();
  for (const record of records) {
    validateDigestRecord(record, label);
    requireCondition(!paths.has(record.path), `${label} duplicate ${record.path}`);
    paths.add(record.path);
  }
}

function validateDigestRecord(record, label) {
  requireRecord(record, label);
  requireCondition(typeof record.path === "string" && isSafeRepositoryPath(record.path), `${label} path is invalid`);
  positiveInteger(record.byte_length, `${label} byte length`);
  requireCondition(SHA256.test(record.sha256), `${label} SHA-256 is invalid`);
}

function validateRectangle(rectangle, label) {
  requireRecord(rectangle, `${label} rectangle`);
  requireExactKeys(rectangle, ["x", "y", "width", "height"], `${label} rectangle`);
  for (const field of ["x", "y"]) requireCondition(Number.isSafeInteger(rectangle[field]) && rectangle[field] >= 0, `${label} rectangle ${field} is invalid`);
  positiveInteger(rectangle.width, `${label} rectangle width`);
  positiveInteger(rectangle.height, `${label} rectangle height`);
}

function validateRgba(rgba, label) {
  requireCondition(Array.isArray(rgba) && rgba.length === 4
    && rgba.every((channel) => Number.isInteger(channel) && channel >= 0 && channel <= 255), `${label} is invalid`);
}

function validateAdapter(adapter, label) {
  requireRecord(adapter, `${label} adapter`);
  requireExactKeys(adapter, ["name", "backend"], `${label} adapter`);
  for (const field of ["name", "backend"]) {
    requireCondition(typeof adapter[field] === "string" && adapter[field].length > 0,
      `${label} adapter ${field} is invalid`);
  }
}

function validateLocalEnvironment(environment, label) {
  requireRecord(environment, `${label} environment`);
  requireExactKeys(environment, ["operating_system", "adapter_name", "backend"],
    `${label} environment`);
  for (const field of ["operating_system", "adapter_name", "backend"]) {
    requireCondition(typeof environment[field] === "string" && environment[field].length > 0,
      `${label} environment ${field} is invalid`);
  }
}

function validateMetricId(metricId, bindings, label) {
  requireCondition(typeof metricId === "string" && /^[a-z0-9][a-z0-9._:/-]+$/.test(metricId), `${label} metric id is invalid`);
  requireCondition(!bindings.has(metricId), `${label} metric id is duplicated`);
}

function requireArtifact(artifacts, artifactPath, mediaType, label) {
  requireCondition(typeof artifactPath === "string" && artifacts.has(artifactPath), `${label} artifact is absent`);
  const artifact = artifacts.get(artifactPath);
  const actualType = artifact.mime_type ?? artifact.media_type;
  requireCondition(actualType === mediaType, `${label} media type differs`);
  return artifact;
}

function addArtifact(artifacts, artifact, label) {
  requireCondition(!artifacts.has(artifact.path), `${label} path ${artifact.path} is duplicated`);
  artifacts.set(artifact.path, artifact);
}

function lookupMap(map, key, label) {
  requireCondition(map instanceof Map && map.has(key), `${label} omits ${key}`);
  return map.get(key);
}

function predecessorRatioPass(candidate, predecessor, maximumRatio) {
  return predecessor === 0 ? true : candidate <= predecessor * maximumRatio;
}

function timestamp(value, label) {
  requireCondition(typeof value === "string" && /^\d{4}-\d{2}-\d{2}T/.test(value), `${label} is invalid`);
  const milliseconds = Date.parse(value);
  requireCondition(Number.isFinite(milliseconds), `${label} is invalid`);
  return milliseconds;
}

function positiveInteger(value, label) {
  requireCondition(Number.isSafeInteger(value) && value > 0, `${label} is invalid`);
  return value;
}

function nonnegativeSafeInteger(value, label) {
  requireCondition(Number.isSafeInteger(value) && value >= 0, `${label} is invalid`);
  return value;
}

function requireFiniteNonnegative(value, label) {
  requireCondition(Number.isFinite(value) && value >= 0, `${label} is invalid`);
}

function requireArray(value, label) {
  requireCondition(Array.isArray(value), `${label} must be an array`);
}

function requireExactKeys(record, keys, label) {
  requireRecord(record, label);
  requireJsonEqual(Object.keys(record).sort(), [...keys].sort(), `${label} fields`);
}

function requireJsonEqual(actual, expected, label) {
  requireCondition(jsonEqual(actual, expected), `${label} differ`);
}

function isSafeRepositoryPath(value) {
  return value.length > 0
    && !value.startsWith("/")
    && !value.includes("\\")
    && !value.split("/").includes("..")
    && !value.split("/").includes("");
}

function gate(condition, failures, message) {
  if (!condition) failures.push(message);
}
