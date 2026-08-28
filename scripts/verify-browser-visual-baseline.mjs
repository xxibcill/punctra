import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  DISPLAY_MODES,
  REQUIRED_GENERATED_CONDITIONS,
  RUBRIC_OUTCOMES,
  RUBRIC_PROMPTS,
  decodeTransferV2,
  encodeTransferV2,
  generateVisualScene,
  validateRubricObservation,
  validateVisualCorpus,
} from "../apps/browser-demo/web/visual-corpus.js";
import {
  compareCanonicalImages as deriveCanonicalComparison,
  createDifferenceImage,
  summarizeTemporalPairs,
  validateToleranceProfile as validateSharedToleranceProfile,
} from "../apps/browser-demo/web/visual-comparison.js";
import { decodeRgba8Png } from "../apps/browser-demo/web/visual-png.js";
import {
  VISUAL_ATTENDED_LANE,
  VISUAL_TRUSTED_CONTROL_SCHEMA,
} from "../apps/browser-demo/web/visual-provenance.js";
import {
  QUALIFICATION_LANE,
  QUALIFICATION_RUNTIME_LANE,
} from "../apps/browser-demo/web/qualification-lane.js";

export { compareCanonicalImages } from "../apps/browser-demo/web/visual-comparison.js";

export const VISUAL_BASELINE_SCHEMA = "punctra-browser-visual-baseline-v1";
export const VISUAL_EVIDENCE_SCHEMA = "punctra-browser-visual-evidence-v1";
export const VISUAL_RELEASE = "0.21.0-alpha.1";
export const VISUAL_VERIFIER_PATH = "scripts/verify-browser-visual-baseline.mjs";
export const MAX_PINNED_FILE_BYTES = 80 * 1024 * 1024;

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const defaultBaselinePath = "docs/releases/v0.21-browser-visual-baseline.json";
const MAX_PINNED_SIZE_OUTPUT_BYTES = 32;
const canonicalViewport = Object.freeze({
  css_width: 320,
  css_height: 240,
  requested_device_pixel_ratio: 2,
  physical_width: 640,
  physical_height: 480,
});
const toleranceCaps = Object.freeze({
  channel_threshold: 2,
  maximum_channel_delta: 4,
  unstable_pixel_fraction: 0.001,
  feature_centroid_distance_pixels: 1,
});
const expectedAuthorities = Object.freeze({
  canonical_image: "presentation_only",
  feature_report: "presentation_only",
  provisional_pick: "provisional_gpu_hint",
  exact_confirmation: "exact_source_record",
  source_coverage: "source_or_authored_facts_only",
  query_completion: "not_inferred_from_visual_evidence",
});
const expectedExternalEvidence = Object.freeze({
  cross_browser: false,
  cross_operating_system: false,
  cross_adapter: false,
  cross_device: false,
  physical_display_presentation: false,
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
const expectedUnavailableMeasurements = Object.freeze([
  "driver_gpu_memory_bytes",
  "energy",
  "gpu_completion_time",
  "physical_cache_allocation_bytes",
  "physical_display_panel_presentation",
  "process_resident_memory_bytes",
  "thermal_state",
]);
const expectedUnavailableEvidence = Object.freeze(Object.fromEntries(
  expectedUnavailableMeasurements.map((name) => [name, null]),
));
const expectedColorCapabilities = Object.freeze({
  gamut_srgb: true,
  gamut_p3: true,
  gamut_rec2020: false,
  dynamic_range_high: true,
  video_dynamic_range_high: false,
  configured_surface_color_space: "srgb",
  display_icc_profile: null,
  physical_panel_hdr_state: null,
});
const expectedProtectedAppearancePaths = Object.freeze([
  "apps/browser-demo/src/display.rs",
  "apps/browser-demo/src/scene.rs",
  "crates/render-wgpu/src/eye_dome.wgsl",
  "crates/render-wgpu/src/frame.rs",
  "crates/render-wgpu/src/pipeline.rs",
  "crates/render-wgpu/src/point.wgsl",
  "crates/render-wgpu/src/renderer.rs",
]);

export async function verifyBrowserVisualBaseline(baseline, options = {}) {
  requireRecord(baseline, "visual baseline");
  assert.equal(baseline.schema, VISUAL_BASELINE_SCHEMA);
  assert.equal(baseline.release, VISUAL_RELEASE);
  const context = createVerificationContext(options);

  await verifyPins(baseline.pins, context);
  const predecessor = await verifyPredecessor(baseline.predecessor, context);
  await verifyPackageRuntime(baseline.package_runtime, context);
  await verifyPointAppearance(baseline.point_appearance, predecessor, context);
  const { corpus, autzenManifest } = await verifyCorpus(baseline.corpus, context);
  const baselineInputs = await verifyBaselineInputsPolicy(
    baseline.baseline_inputs,
    baseline.package_runtime,
    corpus,
    context,
  );
  verifyTrialContract(baseline.trial_contract, corpus, autzenManifest);
  await verifyCanonicalLane(baseline.canonical_lane, corpus, context);
  verifyTolerancePolicy(baseline.tolerance_policy, corpus);
  verifyResourcePolicy(baseline.resources, corpus, predecessor);
  verifyAuthorityPolicy(baseline.authority);
  await verifyRubricPolicy(baseline.rubric, corpus, context);
  verifyEvidencePolicy(baseline.evidence);
  assert.deepEqual(baseline.external_evidence, expectedExternalEvidence);
  assert.deepEqual([...baseline.unavailable_measurements].sort(), [...expectedUnavailableMeasurements]);

  return { baseline, corpus, predecessor, autzenManifest, baselineInputs };
}

export async function verifyBrowserVisualEvidence(evidence, verifiedBaseline, options = {}) {
  requireRecord(evidence, "visual evidence");
  const { baseline, corpus, autzenManifest, baselineInputs } = normalizeVerifiedBaseline(verifiedBaseline);
  verifyEvidenceBytes(options.evidenceBytes, evidence, corpus.resource_limits.evidence_json_bytes);
  const context = createVerificationContext(options);
  assert.deepEqual(Object.keys(evidence).sort(), [
    "artifact_resources",
    "artifacts",
    "baseline_inputs",
    "capture_completed_at",
    "capture_policy",
    "completed_at",
    "corpus",
    "environment",
    "external_evidence",
    "fatal_error",
    "mode",
    "presentation_policy",
    "provenance",
    "release",
    "rubric",
    "schema",
    "started_at",
    "summary",
    "tolerance_profiles",
    "trials",
  ]);
  assert.equal(evidence.schema, VISUAL_EVIDENCE_SCHEMA);
  assert.equal(evidence.release, baseline.release);
  assert.equal(evidence.mode, "verify");
  assert.match(evidence.started_at, /^\d{4}-\d{2}-\d{2}T/);
  assert.match(evidence.capture_completed_at, /^\d{4}-\d{2}-\d{2}T/);
  assert.match(evidence.completed_at, /^\d{4}-\d{2}-\d{2}T/);
  const startedMilliseconds = Date.parse(evidence.started_at);
  const captureCompletedMilliseconds = Date.parse(evidence.capture_completed_at);
  const completedMilliseconds = Date.parse(evidence.completed_at);
  assert(Number.isFinite(startedMilliseconds));
  assert(Number.isFinite(captureCompletedMilliseconds));
  assert(Number.isFinite(completedMilliseconds));
  assert(captureCompletedMilliseconds >= startedMilliseconds, "visual capture completion precedes its start");
  assert(completedMilliseconds >= startedMilliseconds, "visual evidence completion precedes its start");
  assert(completedMilliseconds >= captureCompletedMilliseconds, "visual evidence submission precedes capture completion");
  assert.equal(evidence.fatal_error, null);
  assert.equal(evidence.corpus.path, baseline.corpus.artifact.path);
  assert.equal(evidence.corpus.schema, corpus.schema);
  assert.equal(evidence.corpus.release, corpus.release);
  assert.equal(evidence.corpus.byte_length, baseline.corpus.artifact.byte_length);
  assert.equal(evidence.corpus.sha256, baseline.corpus.artifact.sha256);
  assert.deepEqual(evidence.baseline_inputs, {
    path: baseline.baseline_inputs.artifact.path,
    schema: baselineInputs.schema,
    byte_length: baseline.baseline_inputs.artifact.byte_length,
    sha256: baseline.baseline_inputs.artifact.sha256,
  });
  await verifyEvidenceProvenance(evidence.provenance, baseline, context, evidence.started_at);
  assert.equal(evidence.provenance.observation_date, evidence.started_at.slice(0, 10));
  verifyEvidenceEnvironment(evidence.environment, baseline.canonical_lane, corpus);
  assert.deepEqual(evidence.capture_policy, corpus.capture);
  assert.deepEqual(evidence.presentation_policy, corpus.presentation_policy);
  assert.deepEqual(evidence.tolerance_profiles, corpus.tolerance_profiles);
  verifyEvidenceExternalBoundary(evidence.external_evidence);
  const artifactRegistry = verifyArtifactRegistry(evidence.artifacts, corpus.timing_limits);

  const expectedTrials = new Map(corpus.trials.map((trial) => [trial.id, trial]));
  const expectedBaselines = new Map(baselineInputs.canonical_baselines.map((record) => [record.trial_id, record]));
  requireArray(evidence.trials, "visual evidence trials");
  assert.equal(evidence.trials.length, expectedTrials.size, "evidence must contain every fixed trial exactly once");
  const observedTrialIds = new Set();
  const derivedResults = [];
  for (const result of evidence.trials) {
    const trial = expectedTrials.get(result.trial_id);
    assert(trial, `evidence contains unknown trial ${JSON.stringify(result.trial_id)}`);
    assert(!observedTrialIds.has(result.trial_id), `evidence duplicates trial ${result.trial_id}`);
    observedTrialIds.add(result.trial_id);
    const expectedCamera = trial.camera === "source" ? autzenManifest.camera : trial.camera;
    derivedResults.push(await verifyTrialEvidence(
      result,
      trial,
      expectedCamera,
      autzenManifest,
      expectedBaselines.get(trial.id),
      corpus,
      artifactRegistry,
      context,
    ));
  }
  assert.deepEqual([...observedTrialIds].sort(), [...expectedTrials.keys()].sort());
  verifyRubricEvidence(evidence.rubric, baseline.rubric, evidence, artifactRegistry);
  verifyArtifactResourceEvidence(evidence.artifact_resources, artifactRegistry, corpus.resource_limits);
  verifyEvidenceSummary(evidence.summary, derivedResults, artifactRegistry);
  assert.deepEqual(
    [...artifactRegistry.values()].filter(({ used }) => !used).map(({ record }) => record.path),
    [],
    "every published visual artifact must be bound to a derived trial check",
  );
  return { evidence, trialResults: derivedResults };
}

export function verifyEvidenceBytes(bytes, evidence, ceiling) {
  assert(bytes instanceof Uint8Array, "visual evidence verification requires its exact JSON bytes");
  assert(Number.isSafeInteger(ceiling) && ceiling > 0, "visual evidence JSON ceiling is invalid");
  assert(bytes.byteLength <= ceiling, "visual evidence JSON exceeded its independent byte ceiling");
  assertJsonEqual(parseJson(bytes, "visual evidence bytes"), evidence, "visual evidence object");
}

function assertJsonEqual(expected, actual, label) {
  if (Object.is(expected, actual)) return;
  assert.equal(typeof actual, typeof expected, `${label} differs from its supplied bytes`);
  if (expected === null || actual === null || typeof expected !== "object") {
    assert.fail(`${label} differs from its supplied bytes`);
  }
  const expectedArray = Array.isArray(expected);
  assert.equal(Array.isArray(actual), expectedArray, `${label} differs from its supplied bytes`);
  if (expectedArray) {
    assert.equal(actual.length, expected.length, `${label} differs from its supplied bytes`);
    for (let index = 0; index < expected.length; index += 1) {
      assertJsonEqual(expected[index], actual[index], `${label}[${index}]`);
    }
    return;
  }
  const expectedKeys = Object.keys(expected);
  const actualKeys = Object.keys(actual);
  assert.equal(actualKeys.length, expectedKeys.length, `${label} differs from its supplied bytes`);
  for (const key of expectedKeys) {
    assert(Object.hasOwn(actual, key), `${label}.${key} is absent from its supplied bytes`);
    assertJsonEqual(expected[key], actual[key], `${label}.${key}`);
  }
}

function assertBytesEqual(actual, expected, label) {
  assert(actual instanceof Uint8Array && expected instanceof Uint8Array, `${label}: values are not byte arrays`);
  assert.equal(actual.byteLength, expected.byteLength, `${label}: byte lengths differ`);
  for (let index = 0; index < expected.byteLength; index += 1) {
    if (actual[index] !== expected[index]) assert.fail(`${label}: byte ${index} differs`);
  }
}

export async function verifyCanonicalImageRecord(record, viewport, contextOptions = {}) {
  const context = createVerificationContext(contextOptions);
  requireRecord(record, "canonical image record");
  validateImageArtifactMetadata(record);
  const bytes = await context.readRepositoryFile(record.path);
  assert.equal(bytes.byteLength, record.encoded_byte_length, `${record.path} byte length drifted`);
  assert.equal(sha256(bytes), record.encoded_sha256, `${record.path} SHA-256 drifted`);
  let image = context.decodedImageCache.get(record.encoded_sha256);
  if (image === undefined) {
    image = await decodeRgba8Png(bytes);
    context.decodedImageCache.set(record.encoded_sha256, image);
  }
  assert.equal(image.width, viewport.physical_width, `${record.path} width differs`);
  assert.equal(image.height, viewport.physical_height, `${record.path} height differs`);
  assert.equal(record.width, image.width);
  assert.equal(record.height, image.height);
  assert.equal(record.decoded_byte_length, image.data.byteLength);
  assert.equal(record.decoded_sha256, sha256(image.data));
  context.imageDigestByObject.set(image, record.decoded_sha256);
  return image;
}

function validateImageArtifactMetadata(record) {
  validateRepositoryPath(record.path);
  assert(Number.isSafeInteger(record.encoded_byte_length) && record.encoded_byte_length > 0);
  assert.match(record.encoded_sha256, /^[0-9a-f]{64}$/);
  assert(Number.isSafeInteger(record.decoded_byte_length) && record.decoded_byte_length > 0);
  assert.match(record.decoded_sha256, /^[0-9a-f]{64}$/);
  assert(Number.isSafeInteger(record.width) && record.width > 0);
  assert(Number.isSafeInteger(record.height) && record.height > 0);
  if (record.mime_type !== undefined) assert.equal(record.mime_type, "image/png");
  if (record.encoding !== undefined) assert.equal(record.encoding, "png-rgba8-filter-0");
  if (record.authority !== undefined) assert.equal(record.authority, "presentation_only");
}

async function verifyPins(pins, context) {
  requireRecord(pins, "visual baseline pins");
  verifyFullCommit(pins.implementation_commit, "visual implementation commit");
  if (context.expectedImplementationCommit !== undefined) {
    assert.equal(pins.implementation_commit, context.expectedImplementationCommit);
  }
  await context.requireCommit(pins.implementation_commit);
  requireArray(pins.qualified_paths, "qualified implementation paths");
  assert.equal(new Set(pins.qualified_paths).size, pins.qualified_paths.length, "qualified paths must be unique");
  for (const required of requiredQualifiedPaths()) {
    assert(pins.qualified_paths.includes(required), `qualified paths omit ${required}`);
  }
  for (const qualifiedPath of pins.qualified_paths) {
    validateRepositoryPath(qualifiedPath);
    const [current, pinned] = await Promise.all([
      context.readRepositoryFile(qualifiedPath),
      context.readPinnedFile(pins.implementation_commit, qualifiedPath),
    ]);
    assert.deepEqual(current, pinned, `${qualifiedPath} differs from the implementation pin`);
  }
  assert.equal(pins.verifier.path, VISUAL_VERIFIER_PATH);
  await verifyDigestRecord(pins.verifier, context);
}

async function verifyPredecessor(record, context) {
  assert.equal(record.path, "docs/releases/v0.20-browser-baseline.json");
  const bytes = await verifyDigestRecord(record, context);
  const predecessor = parseJson(bytes, record.path);
  assert.equal(predecessor.schema, "punctra-browser-integration-baseline-v1");
  assert.equal(predecessor.release, "0.20.0-alpha.1");
  verifyFullCommit(predecessor.qualification.implementation_commit, "v0.20 implementation commit");
  return predecessor;
}

async function verifyPackageRuntime(policy, context) {
  requireRecord(policy, "package/runtime policy");
  const [viewerBytes, reactBytes] = await Promise.all([
    verifyDigestRecord(policy.viewer_manifest, context),
    verifyDigestRecord(policy.react_manifest, context),
  ]);
  const viewer = parseJson(viewerBytes, policy.viewer_manifest.path);
  const react = parseJson(reactBytes, policy.react_manifest.path);
  assert.equal(viewer.name, "@punctra/viewer");
  assert.equal(viewer.version, VISUAL_RELEASE);
  assert.equal(react.name, "@punctra/react");
  assert.equal(react.version, VISUAL_RELEASE);
  assert.equal(react.peerDependencies["@punctra/viewer"], VISUAL_RELEASE);
  assert.deepEqual(Object.keys(viewer.exports).sort(), [".", "./exact-query", "./input", "./package.json"]);
  const runtimePaths = [
    "apps/browser-demo/web/package.json",
    "apps/browser-demo/web/pkg/browser_demo.js",
    "apps/browser-demo/web/pkg/browser_demo_bg.wasm",
  ];
  assert.deepEqual(policy.built_runtime_artifacts.map(({ path: artifactPath }) => artifactPath), runtimePaths);
  for (const artifact of policy.built_runtime_artifacts) await verifyDigestRecord(artifact, context);
  assert.equal(policy.capture_interface, "private_browser_demo_only");
  assert.equal(policy.public_capture_export, false);
  for (const source of [
    await context.readRepositoryFile("apps/browser-demo/web/sdk.js", "utf8"),
    await context.readRepositoryFile("apps/browser-demo/web/sdk.d.ts", "utf8"),
    await context.readRepositoryFile("packages/react/index.d.ts", "utf8"),
  ]) {
    assert.doesNotMatch(source, /beginFrameCapture|pollFrameCapture|visual trial|readback/i);
  }
}

async function verifyBaselineInputsPolicy(policy, packageRuntime, corpus, context) {
  requireRecord(policy, "pre-pin visual baseline inputs policy");
  assert.deepEqual(Object.keys(policy), ["artifact"]);
  assert.equal(
    policy.artifact.path,
    "apps/browser-demo/web/fixtures/visual-v1/baseline-inputs.json",
  );
  const bytes = await verifyDigestRecord(policy.artifact, context);
  assert(bytes.byteLength <= corpus.resource_limits.baseline_inputs_json_bytes,
    "baseline-input manifest exceeded its independent byte ceiling");
  const manifest = parseJson(bytes, policy.artifact.path);
  assert.equal(bytes.toString("utf8"), `${JSON.stringify(manifest, null, 2)}\n`,
    "baseline-input manifest is not canonically encoded");
  assert.deepEqual(Object.keys(manifest).sort(), [
    "canonical_baselines", "package_artifact", "release", "schema",
  ]);
  assert.equal(manifest.schema, "punctra-browser-visual-baseline-inputs-v1");
  assert.equal(manifest.release, VISUAL_RELEASE);
  assert.equal(Object.hasOwn(manifest, "implementation_commit"), false,
    "baseline-input manifest must not self-pin its implementation commit");
  assert.deepEqual(manifest.package_artifact, {
    package_name: "@punctra/viewer",
    package_version: VISUAL_RELEASE,
    runtime_artifacts: packageRuntime.built_runtime_artifacts,
  });
  assert.equal(manifest.canonical_baselines.length, corpus.trials.length);
  for (let index = 0; index < corpus.trials.length; index += 1) {
    const trial = corpus.trials[index];
    const record = manifest.canonical_baselines[index];
    requireRecord(record, `canonical baseline input ${trial.id}`);
    assert.deepEqual(Object.keys(record).sort(), [
      "decoded_byte_length", "decoded_sha256", "encoded_byte_length", "encoded_sha256",
      "height", "path", "trial_id", "width",
    ]);
    assert.equal(record.trial_id, trial.id);
    assert.equal(
      record.path,
      `apps/browser-demo/web/fixtures/visual-v1/baselines/${path.posix.basename(trial.baseline_path)}`,
    );
    assert.equal(record.width, corpus.viewport.physical_width);
    assert.equal(record.height, corpus.viewport.physical_height);
    await verifyCanonicalImageRecord(record, corpus.viewport, context);
  }
  return manifest;
}

async function verifyPointAppearance(policy, predecessor, context) {
  requireRecord(policy, "Point appearance policy");
  assert.equal(policy.change, "unchanged_from_v0.20");
  assert.equal(policy.predecessor_implementation_commit, predecessor.qualification.implementation_commit);
  assert.deepEqual(policy.presentation_policy, predecessor.presentation_policy);
  assert.deepEqual(policy.protected_files.map(({ path: filePath }) => filePath), expectedProtectedAppearancePaths);
  for (const record of policy.protected_files) {
    const current = await verifyDigestRecord(record, context);
    const predecessorBytes = await context.readPinnedFile(policy.predecessor_implementation_commit, record.path);
    assert.deepEqual(current, predecessorBytes, `${record.path} changed Point appearance since v0.20`);
  }
}

async function verifyCorpus(policy, context) {
  requireRecord(policy, "visual corpus policy");
  assert.equal(policy.artifact.path, "apps/browser-demo/web/fixtures/visual-v1/corpus.json");
  const corpusBytes = await verifyDigestRecord(policy.artifact, context);
  const corpus = validateVisualCorpus(parseJson(corpusBytes, policy.artifact.path));
  assert.equal(corpus.release, VISUAL_RELEASE);
  verifyTransportPolicy(corpus.transport, corpus.resource_limits);
  await verifyGeneratedCorpusSource(policy.generated, corpus);
  const autzenManifest = await verifyAutzenCorpusSource(policy.autzen, corpus, context);
  if (context.runFixtureGenerator) await verifyIsolatedAutzenRegeneration(policy.autzen, context);
  return { corpus, autzenManifest };
}

function verifyTransportPolicy(transport, limits) {
  assert.deepEqual(transport, {
    format: "ustar-uncompressed",
    archive_filename: "v0.21-browser-visual-evidence.tar",
    evidence_repository_path: "docs/releases/v0.21-browser-visual-evidence.json",
    maximum_entries: 896,
    maximum_evidence_json_bytes: limits.evidence_json_bytes,
    maximum_baseline_inputs_json_bytes: limits.baseline_inputs_json_bytes,
    maximum_archive_structure_bytes: 1_048_576,
    maximum_archive_overhead_bytes: limits.evidence_json_bytes + limits.baseline_inputs_json_bytes + 1_048_576,
    maximum_archive_bytes:
      limits.total_encoded_artifact_bytes + limits.evidence_json_bytes + limits.baseline_inputs_json_bytes + 1_048_576,
  });
}

async function verifyIsolatedAutzenRegeneration(expected, context) {
  const outputDirectory = await mkdtemp(path.join(tmpdir(), "punctra-visual-v0.21-"));
  try {
    context.runCommand("cargo", [
      "run", "--quiet", "-p", "browser-demo", "--bin", "generate_visual_source_fixture",
      "--", "--output-dir", outputDirectory,
    ]);
    for (const record of [expected.manifest, expected.payload]) {
      const regenerated = await readFile(path.join(outputDirectory, path.basename(record.path)));
      const committed = await context.readRepositoryFile(record.path);
      assert.deepEqual(regenerated, committed, `${record.path} did not regenerate byte-identically`);
    }
  } finally {
    await rm(outputDirectory, { recursive: true, force: true });
  }
}

async function verifyGeneratedCorpusSource(expected, corpus) {
  const source = corpus.sources.find(({ kind }) => kind === "generated");
  assert(source, "generated visual source is missing");
  const scene = generateVisualScene(source.generator);
  verifyMixedLodScene(scene, source);
  assert.deepEqual(
    source.condition_facts,
    deriveGeneratedConditionFacts(scene),
    "generated condition facts drifted from authored Point bytes",
  );
  const payload = concatenateBytes(scene.batches.map(({ points }) => encodeTransferV2(points)));
  const derived = {
    id: scene.generator,
    source_identity: scene.source_identity,
    point_count: scene.point_count,
    batch_roles: scene.batches.map(({ index, role, points }) => ({ index, role, point_count: points.length })),
    lod_relations: scene.lod_relations,
    stable_lod_relations: scene.stable_lod_relations,
    world_origin: scene.world_origin,
    source_z_range: scene.source_z_range,
    conditions: scene.conditions,
    transfer_byte_length: payload.byteLength,
    payload_sha256: sha256(payload),
  };
  assert.deepEqual(expected, derived, "generated visual facts drifted from the executable generator");
  assert.equal(source.source_identity, derived.source_identity);
  assert.equal(source.point_count, derived.point_count);
  assert.equal(source.batch_count, derived.batch_roles.length);
  assert.equal(source.transfer_byte_length, derived.transfer_byte_length);
  assert.equal(source.payload_sha256, derived.payload_sha256);
  assert.deepEqual(source.conditions, derived.conditions);
}

function deriveGeneratedConditionFacts(scene) {
  const batchByRole = new Map(scene.batches.map((batch) => [batch.role, batch]));
  const sparse = batchByRole.get("sparse_features");
  const child = batchByRole.get("lod_child");
  const layered = batchByRole.get("depth_layers");
  const parent = batchByRole.get("lod_parent");
  const coarse = batchByRole.get("lod_adjacent_coarse");
  assert(sparse && child && layered && parent && coarse);
  const allPoints = scene.batches.flatMap(({ points }) => points);
  const intensity = allPoints.map(({ intensity: value }) => value);
  const rgb = allPoints.flatMap(({ rgb: value }) => value);
  const classifications = [...new Set(allPoints.map(({ classification }) => classification))]
    .sort((left, right) => left - right);
  const pairCount = layered.points.length / 2;
  const layerSeparations = Array.from({ length: pairCount }, (_, index) => (
    layered.points[index + pairCount].relative_position[2] - layered.points[index].relative_position[2]
  ));
  const denseBounds = pointBounds(child.points);
  const coarseBounds = pointBounds(coarse.points);
  const denseArea = (denseBounds.maximum[0] - denseBounds.minimum[0])
    * (denseBounds.maximum[1] - denseBounds.minimum[1]);
  const coarseArea = (coarseBounds.maximum[0] - coarseBounds.minimum[0])
    * (coarseBounds.maximum[1] - coarseBounds.minimum[1]);
  const denseDensity = child.points.length / denseArea;
  const coarseDensity = coarse.points.length / coarseArea;
  return {
    sparse_batch: {
      batch_index: sparse.index,
      point_count: sparse.points.length,
      maximum_points: 256,
    },
    dense_batches: [child, layered].map(({ index, points }) => ({
      batch_index: index,
      point_count: points.length,
    })),
    layer_pairs: {
      batch_index: layered.index,
      paired_xy_count: pairCount,
      minimum_z_separation: Math.min(...layerSeparations),
    },
    attribute_extrema: {
      intensity: [Math.min(...intensity), Math.max(...intensity)],
      rgb_channel: [Math.min(...rgb), Math.max(...rgb)],
      classifications,
    },
    large_world_origin: scene.world_origin,
    minimum_dense_axis_spacing: minimumPositiveXySpacing(child.points),
    lod_relation: {
      parent_batch_index: parent.index,
      parent_points: parent.points.length,
      child_batch_index: child.index,
      child_points: child.points.length,
    },
    stable_lod_cut: {
      dense_batch_index: child.index,
      dense_points: child.points.length,
      dense_xy_bounds: {
        min: denseBounds.minimum.slice(0, 2),
        max: denseBounds.maximum.slice(0, 2),
      },
      dense_points_per_xy_area: denseDensity,
      coarse_batch_index: coarse.index,
      coarse_points: coarse.points.length,
      coarse_xy_bounds: {
        min: coarseBounds.minimum.slice(0, 2),
        max: coarseBounds.maximum.slice(0, 2),
      },
      coarse_points_per_xy_area: coarseDensity,
      adjacent_x_gap: coarseBounds.minimum[0] - denseBounds.maximum[0],
      density_ratio: denseDensity / coarseDensity,
      settled_dense_weight_u8: 255,
      settled_coarse_weight_u8: 255,
      distinct_from_parent_child_replacement: true,
    },
  };
}

function minimumPositiveXySpacing(points) {
  let minimum = Number.POSITIVE_INFINITY;
  for (const axis of [0, 1]) {
    const values = [...new Set(points.map(({ relative_position: position }) => position[axis]))]
      .sort((left, right) => left - right);
    for (let index = 1; index < values.length; index += 1) {
      const spacing = values[index] - values[index - 1];
      if (spacing > 0) minimum = Math.min(minimum, spacing);
    }
  }
  assert(Number.isFinite(minimum));
  return minimum;
}

function verifyMixedLodScene(scene, source) {
  const batchByRole = new Map(scene.batches.map((batch) => [batch.role, batch]));
  const parent = batchByRole.get("lod_parent");
  const replacement = batchByRole.get("lod_child");
  const adjacentCoarse = batchByRole.get("lod_adjacent_coarse");
  assert(parent && replacement && adjacentCoarse, "mixed LOD requires parent, replacement, and adjacent coarse node roles");
  assert(scene.lod_relations.some((relation) => relation.parent_batch_index === parent.index
    && relation.child_batch_index === replacement.index), "mixed LOD replacement trace is absent");
  assert(scene.stable_lod_relations.some((relation) => relation.dense_batch_index === replacement.index
    && relation.coarse_batch_index === adjacentCoarse.index), "stable adjacent mixed-LOD relation is absent");
  assert(parent.points.length < replacement.points.length, "replacement density must exceed its parent density");
  assert(replacement.points.length > adjacentCoarse.points.length, "stable dense node must exceed adjacent coarse density");
  assert.equal(boundedRegionsAreAdjacent(replacement.points, adjacentCoarse.points), true, "stable mixed-LOD regions are not adjacent");

  const expected = source.expected_view;
  assert(expected.settled_removed_batch_indices.includes(parent.index), "mixed-LOD parent is not retired at settlement");
  assert.equal(expected.settled_presentation_weights_u8[replacement.index], 255);
  assert.equal(expected.settled_presentation_weights_u8[adjacentCoarse.index], 255);
  const stableFact = source.condition_facts.stable_lod_cut;
  assert.equal(stableFact.dense_batch_index, replacement.index);
  assert.equal(stableFact.coarse_batch_index, adjacentCoarse.index);
}

function boundedRegionsAreAdjacent(leftPoints, rightPoints) {
  const left = pointBounds(leftPoints);
  const right = pointBounds(rightPoints);
  for (let axis = 0; axis < 2; axis += 1) {
    const gap = intervalGap(left.minimum[axis], left.maximum[axis], right.minimum[axis], right.maximum[axis]);
    const separatedOrTouching = left.maximum[axis] <= right.minimum[axis]
      || right.maximum[axis] <= left.minimum[axis];
    const otherAxis = 1 - axis;
    const otherOverlap = intervalGap(
      left.minimum[otherAxis], left.maximum[otherAxis],
      right.minimum[otherAxis], right.maximum[otherAxis],
    ) === 0;
    if (separatedOrTouching && gap <= 2 && otherOverlap) return true;
  }
  return false;
}

function pointBounds(points) {
  const minimum = [Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY];
  const maximum = [Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY];
  for (const point of points) {
    point.relative_position.forEach((coordinate, axis) => {
      minimum[axis] = Math.min(minimum[axis], coordinate);
      maximum[axis] = Math.max(maximum[axis], coordinate);
    });
  }
  return { minimum, maximum };
}

function intervalGap(leftMinimum, leftMaximum, rightMinimum, rightMaximum) {
  if (leftMaximum < rightMinimum) return rightMinimum - leftMaximum;
  if (rightMaximum < leftMinimum) return leftMinimum - rightMaximum;
  return 0;
}

async function verifyAutzenCorpusSource(expected, corpus, context) {
  const source = corpus.sources.find(({ kind }) => kind === "derived_pvis");
  assert(source, "derived Autzen source is missing");
  const [manifestBytes, payloadBytes, upstreamBytes] = await Promise.all([
    verifyDigestRecord(expected.manifest, context),
    verifyDigestRecord(expected.payload, context),
    verifyDigestRecord(expected.upstream_source, context),
  ]);
  const manifest = parseJson(manifestBytes, expected.manifest.path);
  assert.equal(source.fixture_id, manifest.fixture_id);
  assert.equal(source.manifest_path, `./${path.posix.basename(expected.manifest.path)}`);
  assert.equal(source.condition_derivation.fixture_manifest, source.manifest_path);
  assert.equal(source.condition_derivation.payload, manifest.sample.path);
  assert.equal(manifest.schema, "punctra-browser-visual-source-v1");
  assert.equal(manifest.sample.path, `./${path.posix.basename(expected.payload.path)}`);
  assert.equal(manifest.sample.byte_length, payloadBytes.byteLength);
  assert.equal(manifest.sample.sha256, sha256(payloadBytes));
  assert.equal(manifest.source.byte_length, upstreamBytes.byteLength);
  assert.equal(manifest.source.sha256, sha256(upstreamBytes));
  assert.equal(manifest.source.repository_path, expected.upstream_source.path);
  assert.equal(manifest.source.source_identity, "4461a593f1be469e8171aa1eff3aed4615c7f80a02f12b2c608941ae2ae10c24");
  assert.equal(manifest.source.point_count, 10_653_336);
  assert.equal(manifest.source.format, "LAZ 1.4 point format 7");
  assert.equal(manifest.source.coordinate_units, "US survey feet");
  assert.equal(manifest.sample.point_count, 4_096);
  assert.equal(manifest.sample.record_bytes, 32);
  const conditionFacts = deriveAutzenConditionFacts(payloadBytes, manifest.source.world_origin);
  assert.deepEqual(manifest.condition_facts, conditionFacts, "Autzen condition facts were not derived from pvis bytes");
  assert.deepEqual(manifest.conditions, ["permitted_real_source", ...conditionFacts.derived_conditions]);
  verifyAutzenRecipe(manifest.derivation);
  assert.equal(manifest.derivation.output_source_identity, manifest.source.source_identity);
  assert.deepEqual(
    manifest.derivation.selected_spans,
    Array.from({ length: 64 }, (_, block) => ({
      first_ordinal: Math.floor((manifest.source.point_count - 64) * block / 63),
      point_count: 64,
    })),
  );
  verifyAutzenPermission(manifest.permission);
  assert.deepEqual(manifest.conditions, source.conditions);
  assert.equal(expected.generator.path, "apps/browser-demo/src/bin/generate_visual_source_fixture.rs");
  await verifyDigestRecord(expected.generator, context);
  assert.equal(
    expected.verification_command,
    "cargo run --quiet -p browser-demo --bin generate_visual_source_fixture -- --output-dir <isolated-directory>",
  );
  return manifest;
}

function deriveAutzenConditionFacts(payload, worldOrigin) {
  const records = decodeTransferRecords(payload);
  const relativeBounds = pointBounds(records);
  const xyCounts = new Uint32Array(32 * 32);
  const xyMinimumZ = new Float64Array(32 * 32).fill(Number.POSITIVE_INFINITY);
  const xyMaximumZ = new Float64Array(32 * 32).fill(Number.NEGATIVE_INFINITY);
  const depthCounts = new Uint32Array(32);
  const classificationCounts = new Uint32Array(256);
  let intensityMinimum = 65_535;
  let intensityMaximum = 0;
  let rgbMinimum = 65_535;
  let rgbMaximum = 0;

  for (const record of records) {
    const [x, y, z] = record.relative_position;
    const xBin = conditionBin(x, relativeBounds.minimum[0], relativeBounds.maximum[0]);
    const yBin = conditionBin(y, relativeBounds.minimum[1], relativeBounds.maximum[1]);
    const cell = yBin * 32 + xBin;
    xyCounts[cell] += 1;
    xyMinimumZ[cell] = Math.min(xyMinimumZ[cell], z);
    xyMaximumZ[cell] = Math.max(xyMaximumZ[cell], z);
    depthCounts[conditionBin(z, relativeBounds.minimum[2], relativeBounds.maximum[2])] += 1;
    intensityMinimum = Math.min(intensityMinimum, record.intensity);
    intensityMaximum = Math.max(intensityMaximum, record.intensity);
    record.rgb.forEach((channel) => {
      rgbMinimum = Math.min(rgbMinimum, channel);
      rgbMaximum = Math.max(rgbMaximum, channel);
    });
    classificationCounts[record.classification] += 1;
  }

  const occupiedCells = [...xyCounts].filter((count) => count > 0).length;
  const singletonCells = [...xyCounts].filter((count) => count === 1).length;
  const denseCells = [...xyCounts].filter((count) => count >= 8).length;
  const layeredSpans = [...xyCounts].flatMap((count, index) => (
    count > 1 ? [xyMaximumZ[index] - xyMinimumZ[index]] : []
  ));
  const layeredCells = layeredSpans.filter((span) => span >= 10).length;
  const occupiedDepthBins = [...depthCounts].filter((count) => count > 0).length;
  const minimumSpacing = minimumPositiveAxisSpacing(records);
  const maximumAbsoluteWorldOrigin = Math.max(...worldOrigin.map(Math.abs));
  const classifications = [...classificationCounts].flatMap((count, value) => (
    count > 0 ? [{ value, count }] : []
  ));
  const derivedConditions = [];
  if (singletonCells > 0) derivedConditions.push("sparse");
  if (denseCells > 0) derivedConditions.push("dense");
  if (layeredCells > 0 && occupiedDepthBins > 1) derivedConditions.push("layered");
  if (intensityMaximum - intensityMinimum >= 32_768 && rgbMaximum - rgbMinimum >= 32_768) {
    derivedConditions.push("high_dynamic_range");
  }
  if (classifications.length > 1) derivedConditions.push("classification");
  if (maximumAbsoluteWorldOrigin >= 500_000 && minimumSpacing < 1) derivedConditions.push("large_world");

  return {
    schema: "punctra-browser-visual-sample-conditions-v1",
    relative_bounds: { min: relativeBounds.minimum, max: relativeBounds.maximum },
    minimum_positive_axis_spacing: minimumSpacing,
    xy_grid: {
      columns: 32,
      rows: 32,
      occupied_cells: occupiedCells,
      empty_cells: xyCounts.length - occupiedCells,
      singleton_cells: singletonCells,
      cells_with_at_least_8_points: denseCells,
      maximum_points_per_cell: Math.max(...xyCounts),
    },
    overlapping_depth: {
      xy_grid_columns: 32,
      xy_grid_rows: 32,
      cells_with_at_least_10_unit_z_span: layeredCells,
      maximum_cell_z_span: Math.max(...layeredSpans),
    },
    depth_bins: { count: 32, occupied_bins: occupiedDepthBins },
    intensity: { minimum: intensityMinimum, maximum: intensityMaximum },
    rgb_channel: { minimum: rgbMinimum, maximum: rgbMaximum },
    classifications,
    maximum_absolute_world_origin: maximumAbsoluteWorldOrigin,
    thresholds: {
      dense_cell_minimum_points: 8,
      layered_cell_minimum_z_span: 10,
      high_dynamic_range_minimum_span: 32_768,
      large_world_minimum_absolute_origin: 500_000,
      large_world_maximum_spacing_exclusive: 1,
    },
    derived_conditions: derivedConditions,
  };
}

function decodeTransferRecords(payload) {
  const records = [];
  let previousOrdinal = -1;
  for (let offset = 0; offset < payload.byteLength; offset += 32 * 1_024) {
    const decoded = decodeTransferV2(payload.subarray(offset, offset + 32 * 1_024), previousOrdinal);
    previousOrdinal = decoded.at(-1).ordinal;
    records.push(...decoded);
  }
  return records;
}

function conditionBin(value, minimum, maximum) {
  if (value >= maximum || minimum === maximum) return 31;
  const normalized = Math.max(0, Math.min(1, (value - minimum) / (maximum - minimum)));
  return Math.min(31, Math.floor(normalized * 32));
}

function minimumPositiveAxisSpacing(records) {
  let minimum = Number.POSITIVE_INFINITY;
  for (let axis = 0; axis < 3; axis += 1) {
    const values = [...new Set(records.map((record) => record.relative_position[axis]))]
      .sort((left, right) => left - right);
    for (let index = 1; index < values.length; index += 1) {
      const spacing = values[index] - values[index - 1];
      if (spacing > 0) minimum = Math.min(minimum, spacing);
    }
  }
  assert(Number.isFinite(minimum), "Autzen sample has no positive axis spacing");
  return minimum;
}

function verifyAutzenRecipe(recipe) {
  assert.equal(recipe.schema, "punctra-visual-ordinal-block-sample-v1");
  assert.equal(recipe.block_count, 64);
  assert.equal(recipe.points_per_block, 64);
  assert.equal(recipe.record_schema, "punctra-browser-transfer-v2");
  assert.equal(recipe.output_order, "ascending canonical Source ordinal");
  assert.equal(recipe.attribute_mapping, "copy raw intensity/red/green/blue u16 and classification u8 without rescaling");
  assert.match(recipe.position_mapping, /world_origin/);
  assert.match(recipe.position_mapping, /f64/);
  assert.match(recipe.position_mapping, /f32/);
  assert.deepEqual(recipe.attributes, ["intensity", "classification", "red", "green", "blue"]);
  assert.equal(recipe.selected_spans.length, 64);
  assert(recipe.selected_spans.every(({ point_count }) => point_count === 64));
  assert(recipe.selected_spans.every((span, index, spans) => index === 0 || span.first_ordinal > spans[index - 1].first_ordinal));
  assert.deepEqual(recipe.discarded_fields, [
    "return_number", "number_of_returns", "synthetic", "key_point", "withheld", "overlap",
    "scanner_channel", "scan_direction_flag", "edge_of_flight_line", "user_data", "scan_angle",
    "point_source_id", "gps_time",
  ]);
}

function verifyAutzenPermission(permission) {
  assert.equal(permission.creator, "PDAL/data contributors");
  assert.equal(permission.license, "CC BY 4.0");
  assert.equal(permission.license_url, "https://creativecommons.org/licenses/by/4.0/");
  assert.equal(permission.upstream_revision, "360327d2ae791b9d52c57b610a5a6b5c1b08c878");
  assert.match(permission.upstream_url, /PDAL\/data\/blob\/360327d2ae791b9d52c57b610a5a6b5c1b08c878/);
  assert.equal(permission.derived_sample_and_image_publication, true);
  assert.equal(permission.derivative_redistribution, true);
  assert.match(permission.redistribution, /permits redistribution/i);
  assert.match(permission.modification_notice, /selected 64 fixed Source-ordinal blocks/i);
  assert(permission.attribution.length > 0);
}

function verifyTrialContract(contract, corpus, autzenManifest) {
  requireRecord(contract, "trial contract");
  assert.equal(contract.trial_count, 9);
  assert.equal(contract.generated_trial_count, 5);
  assert.equal(contract.autzen_trial_count, 4);
  assert.deepEqual(contract.required_generated_conditions, REQUIRED_GENERATED_CONDITIONS);
  assert.deepEqual(contract.display_modes, DISPLAY_MODES);
  assert.deepEqual(contract.projections, ["orthographic", "perspective"]);
  assert.deepEqual(contract.coverage_labels, ["authored", "complete", "sampled"]);
  assert.equal(contract.selection_states, "selected_and_unselected_required");
  assert.equal(contract.mixed_lod_trace, "parent_replacement_to_settled_cut_required");

  const sourceById = new Map(corpus.sources.map((source) => [source.id, source]));
  const generatedConditions = new Set();
  const modes = new Set();
  const projections = new Set();
  let selected = false;
  let unselected = false;
  let mixedLod = false;
  const autzenModes = new Set();
  let generatedTrialCount = 0;
  let autzenTrialCount = 0;
  for (const trial of corpus.trials) {
    const source = sourceById.get(trial.source_id);
    modes.add(trial.display_mode);
    projections.add(trial.camera === "source" ? "perspective" : trial.camera.projection);
    selected ||= trial.selection.ordinals.length > 0;
    unselected ||= trial.selection.ordinals.length === 0;
    mixedLod ||= trial.temporal_trace.kind === "mixed_lod_parent_child";
    assert.equal(trial.coverage, source.kind === "generated" ? "authored" : "sampled");
    if (source.kind === "derived_pvis") {
      autzenTrialCount += 1;
      autzenModes.add(trial.display_mode);
    } else {
      generatedTrialCount += 1;
    }
    if (source.kind === "derived_pvis") verifyAutzenTrialFeatureBindings(trial, source, autzenManifest);
    if (trial.conditions.includes("mixed_lod")) {
      assert.equal(trial.temporal_trace.kind, "mixed_lod_parent_child");
    }
    if (trial.temporal_trace.kind === "mixed_lod_parent_child") {
      assert(trial.conditions.includes("mixed_lod"));
    }
    if (source.kind === "generated") trial.conditions.forEach((condition) => generatedConditions.add(condition));
    verifyTrialEvidenceFacts(trial, source, corpus);
  }
  assert.equal(corpus.trials.length, 9);
  assert.equal(generatedTrialCount, 5);
  assert.equal(autzenTrialCount, 4);
  assert.deepEqual([...modes].sort(), [...DISPLAY_MODES].sort());
  assert.deepEqual([...projections].sort(), ["orthographic", "perspective"]);
  assert.equal(selected && unselected, true);
  assert.equal(mixedLod, true);
  assert.deepEqual([...generatedConditions].sort(), [...REQUIRED_GENERATED_CONDITIONS].sort());
  assert.deepEqual([...autzenModes].sort(), ["classification", "elevation", "intensity", "rgb"]);
  const conditionCoverage = deriveConditionCoverage(corpus, autzenManifest);
  assert.deepEqual(contract.condition_coverage, conditionCoverage);
  assert.deepEqual(contract.condition_fact_snapshot, deriveConditionFactSnapshot(corpus, autzenManifest));
}

function verifyAutzenTrialFeatureBindings(trial, source, autzenManifest) {
  assert(trial.features.length > 0, `Autzen trial ${trial.id} has no measurable feature region`);
  for (const feature of trial.features) {
    assert.deepEqual(feature.binding, {
      kind: "derived_sample_region",
      fixture_id: source.fixture_id,
      sample_sha256: autzenManifest.sample.sha256,
    });
  }
}

export function deriveConditionCoverage(corpus, autzenManifest) {
  const generated = corpus.sources.find(({ kind }) => kind === "generated");
  const autzen = corpus.sources.find(({ kind }) => kind === "derived_pvis");
  assert(generated && autzen, "condition coverage requires both generated and Autzen sources");
  const generatedFactPaths = new Map([
    ["sparse", ["condition_facts.sparse_batch"]],
    ["dense", ["condition_facts.dense_batches"]],
    ["layered", ["condition_facts.layer_pairs"]],
    ["high_dynamic_range", [
      "condition_facts.attribute_extrema.intensity",
      "condition_facts.attribute_extrema.rgb_channel",
    ]],
    ["classification", ["condition_facts.attribute_extrema.classifications"]],
    ["large_world", [
      "condition_facts.large_world_origin",
      "condition_facts.minimum_dense_axis_spacing",
    ]],
    ["mixed_lod", ["condition_facts.lod_relation", "condition_facts.stable_lod_cut"]],
  ]);
  const derivedModeFactPaths = new Map([
    ["classification", [
      "condition_facts.classifications",
      "condition_facts.xy_grid",
      "condition_facts.maximum_absolute_world_origin",
    ]],
    ["elevation", [
      "condition_facts.relative_bounds",
      "condition_facts.overlapping_depth",
      "condition_facts.xy_grid",
      "condition_facts.maximum_absolute_world_origin",
    ]],
    ["intensity", [
      "condition_facts.intensity",
      "condition_facts.xy_grid",
      "condition_facts.maximum_absolute_world_origin",
    ]],
    ["rgb", [
      "condition_facts.rgb_channel",
      "condition_facts.overlapping_depth",
      "condition_facts.xy_grid",
      "condition_facts.maximum_absolute_world_origin",
    ]],
  ]);
  const table = {
    schema: "punctra-browser-visual-condition-coverage-v1",
    generated: REQUIRED_GENERATED_CONDITIONS.map((condition) => {
      const entry = {
        condition,
        source_id: generated.id,
        trial_ids: corpus.trials
          .filter((trial) => trial.source_id === generated.id && trial.conditions.includes(condition))
          .map(({ id }) => id),
        fact_paths: generatedFactPaths.get(condition),
      };
      if (condition === "mixed_lod") entry.required_temporal_trace = "mixed_lod_parent_child";
      return entry;
    }),
    derived_modes: [...derivedModeFactPaths].map(([displayMode, factPaths]) => {
      const trials = corpus.trials.filter((trial) => trial.source_id === autzen.id && trial.display_mode === displayMode);
      assert.equal(trials.length, 1, `Autzen ${displayMode} requires exactly one fixed trial`);
      return {
        source_id: autzen.id,
        trial_id: trials[0].id,
        display_mode: displayMode,
        fact_paths: factPaths,
      };
    }),
  };
  for (const entry of table.generated) {
    assert(entry.trial_ids.length > 0, `generated condition ${entry.condition} has no fact-bound trial`);
    for (const factPath of entry.fact_paths) assert.notEqual(resolveFactPath(generated, factPath), undefined);
    if (entry.required_temporal_trace !== undefined) {
      assert(entry.trial_ids.every((trialId) => corpus.trials.find(({ id }) => id === trialId)
        .temporal_trace.kind === entry.required_temporal_trace));
    }
  }
  for (const entry of table.derived_modes) {
    for (const factPath of entry.fact_paths) assert.notEqual(resolveFactPath(autzenManifest, factPath), undefined);
  }
  return table;
}

function resolveFactPath(value, factPath) {
  return factPath.split(".").reduce((current, segment) => current?.[segment], value);
}

function deriveConditionFactSnapshot(corpus, manifest) {
  const generated = corpus.sources.find(({ kind }) => kind === "generated");
  const autzen = corpus.sources.find(({ kind }) => kind === "derived_pvis");
  const idsFor = (sourceId, predicate) => corpus.trials
    .filter((trial) => trial.source_id === sourceId && predicate(trial))
    .map(({ id }) => id);
  const generatedEntry = (condition, fact) => ({
    trial_ids: idsFor(generated.id, (trial) => trial.conditions.includes(condition)),
    fact,
  });
  const autzenModeEntry = (mode, fact) => ({
    trial_ids: idsFor(autzen.id, (trial) => trial.display_mode === mode),
    fact,
  });
  const facts = generated.condition_facts;
  return {
    schema: "punctra-browser-visual-condition-coverage-v1",
    generated: {
      sparse: generatedEntry("sparse", { sparse_batch: facts.sparse_batch }),
      dense: generatedEntry("dense", { dense_batches: facts.dense_batches }),
      layered: generatedEntry("layered", { layer_pairs: facts.layer_pairs }),
      high_dynamic_range: generatedEntry("high_dynamic_range", {
        attribute_extrema: facts.attribute_extrema,
      }),
      classification: generatedEntry("classification", {
        classifications: facts.attribute_extrema.classifications,
      }),
      large_world: generatedEntry("large_world", {
        world_origin: facts.large_world_origin,
        minimum_dense_axis_spacing: facts.minimum_dense_axis_spacing,
      }),
      mixed_lod: generatedEntry("mixed_lod", {
        replacement: facts.lod_relation,
        stable_lod_cut: facts.stable_lod_cut,
      }),
    },
    autzen: {
      dense_real_world_structure: {
        trial_ids: idsFor(autzen.id, (trial) => trial.conditions.includes("dense")),
        fact: { xy_grid: manifest.condition_facts.xy_grid },
      },
      classification: autzenModeEntry("classification", {
        classifications: manifest.condition_facts.classifications,
      }),
      intensity: autzenModeEntry("intensity", { intensity: manifest.condition_facts.intensity }),
      rgb: autzenModeEntry("rgb", { rgb_channel: manifest.condition_facts.rgb_channel }),
      elevation: autzenModeEntry("elevation", {
        relative_z_bounds: [
          manifest.condition_facts.relative_bounds.min[2],
          manifest.condition_facts.relative_bounds.max[2],
        ],
        overlapping_depth: manifest.condition_facts.overlapping_depth,
      }),
      large_world_coordinates: {
        trial_ids: idsFor(autzen.id, (trial) => trial.conditions.includes("large_world")),
        fact: {
          maximum_absolute_world_origin: manifest.condition_facts.maximum_absolute_world_origin,
          minimum_positive_axis_spacing: manifest.condition_facts.minimum_positive_axis_spacing,
        },
      },
    },
  };
}

function verifyTrialEvidenceFacts(trial, source, corpus) {
  requireRecord(corpus.presentation_policy, "visual presentation policy");
  assert.equal(corpus.presentation_policy.canonical_clear_rgba8.length, 4);
  assert(Number.isFinite(corpus.presentation_policy.default_point_size_physical_pixels));
  requireRecord(corpus.required_capabilities, "visual capability policy");
  assert.equal(corpus.required_capabilities.webgpu, true);
  assert.equal(corpus.required_capabilities.fallback_allowed, false);
  assert.equal(corpus.required_capabilities.fallback_state, "none");
  if (source.expected_view !== undefined) {
    requireRecord(source.expected_view, `source ${source.id} expected View facts`);
    assert.equal(source.expected_view.generation, 1);
    assert(Number.isSafeInteger(source.expected_view.published_points) && source.expected_view.published_points > 0);
    assert(Number.isSafeInteger(source.expected_view.settled_resident_points) && source.expected_view.settled_resident_points > 0);
    assert(Array.isArray(source.expected_view.batch_keys) && source.expected_view.batch_keys.length > 0);
    assert(Array.isArray(source.expected_view.settled_presentation_weights_u8));
    assert.equal(source.expected_view.batch_keys.length, source.expected_view.settled_presentation_weights_u8.length);
  }
}

async function verifyCanonicalLane(lane, corpus, context) {
  requireRecord(lane, "canonical attended lane");
  assert.equal(lane.id, VISUAL_ATTENDED_LANE.id);
  assert.equal(lane.id, QUALIFICATION_LANE.id);
  assert.equal(lane.qualification_status, QUALIFICATION_LANE.status);
  assert.equal(lane.qualification_lane_record.path, "apps/browser-demo/web/qualification-lane.js");
  await verifyDigestRecord(lane.qualification_lane_record, context);
  assert.deepEqual(lane.viewport, canonicalViewport);
  assert.deepEqual(corpus.viewport, canonicalViewport);
  assert.equal(lane.physical_limits.maximum_axis_pixels, 4_096);
  assert.equal(lane.physical_limits.maximum_area_pixels, 8_388_608);
  assert.equal(canonicalViewport.physical_width * canonicalViewport.physical_height, 307_200);
  assert.equal(corpus.settling.quiet_frames, 30);
  assert.equal(lane.settling.quiet_frames, 30);
  assert.equal(lane.calibration_repetitions, 3);
  assert.equal(lane.fallback, "none");
  assert.deepEqual(lane.capture, corpus.capture);
  assert.equal(lane.capture.canonical_format, "rgba8");
  assert.equal(lane.capture.origin, "top_left");
  assert.equal(lane.capture.lossless_artifact, "png-rgba8-filter-0");
  assert.equal(lane.capture.presentation_claim, "offscreen_not_presented");
}

function verifyTolerancePolicy(policy, corpus) {
  requireRecord(policy, "tolerance policy");
  assert.deepEqual(policy.caps, toleranceCaps);
  assert.equal(policy.calibration_repetitions, 3);
  assert.equal(policy.generated_settled_temporal_unstable_pixels, 0);
  assert.equal(policy.aggregate_override, false);
  assert.deepEqual(policy.profiles, corpus.tolerance_profiles);
  for (const [name, profile] of Object.entries(policy.profiles)) {
    verifyToleranceProfile(profile, `tolerance profile ${name}`);
  }
  const exact = policy.profiles["settled-generated-exact-v1"];
  assert(exact, "settled generated exact profile is missing");
  assert(Object.values(exact).every((value) => value === 0), "settled generated temporal profile must be exact");
}

function verifyToleranceProfile(profile, label) {
  requireRecord(profile, label);
  validateSharedToleranceProfile(profile);
  assert(Number.isInteger(profile.channel_threshold) && profile.channel_threshold >= 0);
  assert(profile.channel_threshold <= toleranceCaps.channel_threshold);
  assert(Number.isFinite(profile.maximum_channel_delta) && profile.maximum_channel_delta >= 0);
  assert(profile.maximum_channel_delta <= toleranceCaps.maximum_channel_delta);
  assert(Number.isFinite(profile.unstable_pixel_fraction) && profile.unstable_pixel_fraction >= 0);
  assert(profile.unstable_pixel_fraction <= toleranceCaps.unstable_pixel_fraction);
  assert(Number.isFinite(profile.feature_centroid_distance_pixels) && profile.feature_centroid_distance_pixels >= 0);
  assert(profile.feature_centroid_distance_pixels <= toleranceCaps.feature_centroid_distance_pixels);
  for (const name of ["mean_channel_delta", "rms_channel_delta", "p95_channel_delta", "coverage_fraction_delta", "feature_occupancy_fraction_delta"]) {
    assert(Number.isFinite(profile[name]) && profile[name] >= 0, `${label} ${name} is invalid`);
  }
}

function verifyResourcePolicy(resources, corpus, predecessor) {
  requireRecord(resources, "resource policy");
  assert.deepEqual(resources.inherited_viewer_limits, predecessor.qualification.limits);
  assert.deepEqual(resources.capture_limits, corpus.resource_limits);
  assert.deepEqual(resources.timing_limits, corpus.timing_limits);
  assert.deepEqual(corpus.timing_limits, {
    schema: "punctra-browser-visual-timing-limits-v1",
    first_coverage_milliseconds: predecessor.qualification.limits.firstCoverageMilliseconds,
    settled_view_milliseconds: predecessor.qualification.limits.settledViewMilliseconds,
    representative_frame_interval_p95_milliseconds: predecessor.qualification.limits.frameIntervalP95Milliseconds,
    representative_frame_submission_p95_milliseconds: predecessor.qualification.limits.frameSubmissionP95Milliseconds,
    capture_begin_submission_milliseconds_per_frame: 100,
    capture_poll_wait_milliseconds_per_frame: 5_000,
    capture_poll_call_milliseconds_per_frame: 100,
    capture_canonical_copy_milliseconds_per_frame: 100,
    capture_submitted_work_done_callback_milliseconds_per_frame: 5_000,
    capture_readback_mapping_callback_milliseconds_per_frame: 5_000,
    png_encode_milliseconds_per_artifact: 5_000,
    artifact_encoding_milliseconds_per_artifact: 7_500,
    comparison_milliseconds_per_pair: 5_000,
    settled_capture_total_milliseconds_per_recreation: 150_000,
    transition_capture_total_milliseconds_per_recreation: 45_000,
    settled_comparison_total_milliseconds_per_recreation: 150_000,
    transition_comparison_total_milliseconds_per_recreation: 45_000,
    artifact_encoding_total_milliseconds_per_recreation: 300_000,
  });
  assert.equal(resources.maximum_retained_canonical_images, 2);
  assert.equal(resources.representative_frame_cost_excludes_capture, true);
  assert.equal(resources.capture_overhead_separately_bounded, true);
  for (const name of [
    "canonical_pixel_bytes", "capture_texture_bytes", "row_aligned_readback_bytes",
    "encoder_working_bytes", "encoded_png_bytes", "total_encoded_artifact_bytes", "evidence_json_bytes",
    "baseline_inputs_json_bytes",
  ]) {
    assert(Number.isSafeInteger(resources.capture_limits[name]) && resources.capture_limits[name] > 0, `capture resource ${name} is absent`);
  }
  assert.equal(resources.capture_limits.canonical_pixel_bytes, 640 * 480 * 4);
  assert.equal(resources.capture_limits.capture_texture_bytes, 640 * 480 * 4);
  assert.equal(resources.capture_limits.row_aligned_readback_bytes, 640 * 480 * 4);
  assert.equal(resources.capture_limits.peak_live_canonical_images, 2);
}

function verifyAuthorityPolicy(authority) {
  assert.deepEqual(authority, expectedAuthorities);
}

async function verifyRubricPolicy(policy, corpus, context) {
  requireRecord(policy, "interpretation rubric policy");
  const bytes = await verifyDigestRecord(policy.template, context);
  const template = parseJson(bytes, policy.template.path);
  assert.equal(template.schema, "punctra-browser-interpretation-rubric-v1");
  assert.equal(template.release, VISUAL_RELEASE);
  assert.equal(template.template, true);
  assert.equal(template.session_label, "not_observed");
  validateRubricObservation(template, corpus.rubric);
  assert.deepEqual(Object.keys(template).sort(), [
    "answers", "authority", "privacy", "release", "schema", "session_label", "template",
  ]);
  assert.deepEqual(Object.keys(template.answers), RUBRIC_PROMPTS);
  for (const prompt of RUBRIC_PROMPTS) {
    const answer = template.answers[prompt];
    assert.deepEqual(Object.keys(answer).sort(), ["note", "outcome", "shown", "trial_ids"]);
    assert.equal(answer.outcome, "not_observed");
    assert.equal(answer.note, "");
    assert.equal(answer.shown, false);
    assert.deepEqual(answer.trial_ids, corpus.rubric.trial_bindings[prompt]);
  }
  assert.deepEqual(template.privacy, {
    anonymous_session_label_only: true,
    stores_name: false,
    stores_contact_information: false,
    stores_employer_or_credentials: false,
    stores_private_source_path: false,
    stores_unrelated_browser_data: false,
  });
  assert.deepEqual(template.authority, {
    gating: false,
    independent_human_evidence: false,
    professional_usability_evidence: false,
  });
  assert.deepEqual(policy.prompts, RUBRIC_PROMPTS);
  assert.deepEqual(policy.outcomes, RUBRIC_OUTCOMES);
  assert.deepEqual(policy.trial_bindings, corpus.rubric.trial_bindings);
  assert.equal(policy.note_character_limit, 280);
  assert.equal(policy.gating, false);
  assert.equal(policy.independent_human_evidence, false);
}

function verifyEvidencePolicy(policy) {
  requireRecord(policy, "visual evidence policy");
  assert.equal(policy.schema, VISUAL_EVIDENCE_SCHEMA);
  assert.equal(policy.path, "docs/releases/v0.21-browser-visual-evidence.json");
  assert.equal(policy.required_recreations_per_trial, 3);
  assert.equal(policy.derive_pass_from_observations, true);
  assert.equal(policy.recorded_pass_flag_authoritative, false);
}

async function verifyEvidenceProvenance(provenance, baseline, context, startedAt) {
  requireRecord(provenance, "visual evidence provenance");
  assert.deepEqual(Object.keys(provenance).sort(), [
    "attended_lane",
    "final_pin_required",
    "implementation_commit",
    "observation_date",
    "package_artifact",
    "run_initiation",
    "verifier",
  ]);
  assert.equal(provenance.implementation_commit, baseline.pins.implementation_commit);
  assert.deepEqual(provenance.verifier, baseline.pins.verifier);
  assert.match(provenance.observation_date, /^\d{4}-\d{2}-\d{2}$/);
  assert.deepEqual(provenance.attended_lane, VISUAL_ATTENDED_LANE);
  assert.equal(provenance.attended_lane.id, baseline.canonical_lane.id);
  const activationMilliseconds = verifyTrustedControlActivation(
    provenance.run_initiation,
    "run-corpus",
    "click",
  );
  const startMilliseconds = Date.parse(startedAt);
  assert(activationMilliseconds <= startMilliseconds, "run-corpus activation follows the run start");
  assert(startMilliseconds - activationMilliseconds <= 5_000, "run-corpus activation is stale");
  assert.equal(provenance.final_pin_required, false);
  requireRecord(provenance.package_artifact, "visual evidence package artifact");
  assert.equal(provenance.package_artifact.package_version, VISUAL_RELEASE);
  assert.equal(provenance.package_artifact.package_name, "@punctra/viewer");
  const runtimePaths = baseline.package_runtime.built_runtime_artifacts
    .map(({ path: artifactPath }) => artifactPath);
  assert.deepEqual(provenance.package_artifact.runtime_artifacts.map(({ path: artifactPath }) => artifactPath), runtimePaths);
  for (let index = 0; index < provenance.package_artifact.runtime_artifacts.length; index += 1) {
    const artifact = provenance.package_artifact.runtime_artifacts[index];
    const expected = baseline.package_runtime.built_runtime_artifacts[index];
    assert.deepEqual(artifact, expected);
    await verifyDigestRecord(expected, context);
  }
}

function verifyTrustedControlActivation(activation, controlId, eventType) {
  requireRecord(activation, `${controlId} trusted control activation`);
  assert.deepEqual(Object.keys(activation).sort(), [
    "control_id",
    "document_visibility_state",
    "event_is_trusted",
    "event_type",
    "recorded_at",
    "schema",
    "transient_user_activation",
    "trust_source",
  ]);
  assert.equal(activation.schema, VISUAL_TRUSTED_CONTROL_SCHEMA);
  assert.equal(activation.control_id, controlId);
  assert.equal(activation.event_type, eventType);
  assert.equal(typeof activation.event_is_trusted, "boolean");
  assert.equal(typeof activation.transient_user_activation, "boolean");
  assert(activation.event_is_trusted || activation.transient_user_activation, `${controlId} lacks browser activation proof`);
  assert.equal(
    activation.trust_source,
    activation.event_is_trusted ? "event_is_trusted" : "transient_user_activation",
  );
  assert.equal(activation.document_visibility_state, "visible");
  assert.match(activation.recorded_at, /^\d{4}-\d{2}-\d{2}T/);
  const activationMilliseconds = Date.parse(activation.recorded_at);
  assert(Number.isFinite(activationMilliseconds), `${controlId} activation timestamp is invalid`);
  return activationMilliseconds;
}

function verifyEvidenceEnvironment(environment, lane, corpus) {
  requireRecord(environment, "attended environment");
  for (const name of ["browser", "document", "screen", "viewport", "color_capabilities", "webgpu", "fallback", "host", "capture"]) {
    requireRecord(environment[name], `attended ${name}`);
  }
  assert.equal(environment.schema, "punctra-browser-visual-environment-v1");
  assert.deepEqual(environment.attended_lane, VISUAL_ATTENDED_LANE);
  assert.equal(environment.attended_lane.id, lane.id);
  assert.deepEqual(environment.host, canonicalQualificationHost(QUALIFICATION_RUNTIME_LANE.host));
  assert.equal(environment.browser.user_agent, QUALIFICATION_RUNTIME_LANE.browser.userAgent);
  assert.equal(environment.browser.platform, QUALIFICATION_RUNTIME_LANE.browser.platform);
  assert.equal(environment.browser.language, QUALIFICATION_RUNTIME_LANE.browser.language);
  assert.equal(environment.browser.logical_processors, QUALIFICATION_RUNTIME_LANE.browser.logicalProcessors);
  assert.equal(environment.document.secure_context, true);
  assert.equal(environment.document.visibility_state, "visible");
  assert.equal(environment.document.cross_origin_isolated, false);
  assert.deepEqual(environment.screen, {
    width_css_pixels: QUALIFICATION_RUNTIME_LANE.screen.width,
    height_css_pixels: QUALIFICATION_RUNTIME_LANE.screen.height,
    color_depth_bits: QUALIFICATION_RUNTIME_LANE.screen.colorDepth,
    pixel_depth_bits: QUALIFICATION_RUNTIME_LANE.screen.pixelDepth,
  });
  assert.equal(environment.viewport.requested_css_width, 320);
  assert.equal(environment.viewport.requested_css_height, 240);
  assert.equal(environment.viewport.requested_device_pixel_ratio, 2);
  assert.equal(environment.viewport.observed_window_device_pixel_ratio, 2);
  assert.equal(environment.viewport.observed_css_width, 320);
  assert.equal(environment.viewport.observed_css_height, 240);
  assert.equal(environment.viewport.canvas_bitmap_width, 640);
  assert.equal(environment.viewport.canvas_bitmap_height, 480);
  assert.equal(environment.viewport.visual_viewport_scale, 1);
  assertFinitePositive(environment.viewport.visual_viewport_width, "visual viewport width");
  assertFinitePositive(environment.viewport.visual_viewport_height, "visual viewport height");
  assert(
    environment.viewport.visual_viewport_width <= environment.screen.width_css_pixels,
    "visual viewport width exceeded the observed screen",
  );
  assert(
    environment.viewport.visual_viewport_height <= environment.screen.height_css_pixels,
    "visual viewport height exceeded the observed screen",
  );
  assert.equal(environment.fallback.allowed, false);
  assert.equal(environment.fallback.requested, false);
  assert.equal(environment.fallback.used, false);
  assert.deepEqual(environment.canonical_requirements, corpus.required_capabilities);
  assert.deepEqual(environment.webgpu, QUALIFICATION_RUNTIME_LANE.capabilities);
  assert.deepEqual(environment.capture, {
    source_format: corpus.required_capabilities.capture_source_format,
    source_channel_order: corpus.required_capabilities.capture_source_channel_order,
    source_encoding: "linear",
    canonical_format: corpus.capture.canonical_format,
    canonical_channel_order: corpus.capture.canonical_channel_order,
    canonical_encoding: corpus.capture.canonical_encoding,
    configured_surface_color_space: "srgb",
    origin: corpus.capture.origin,
    normalization: corpus.required_capabilities.capture_canonicalization,
  });
  assert.deepEqual(environment.color_capabilities, expectedColorCapabilities);
  assert.deepEqual(environment.unavailable_measurements, expectedUnavailableEvidence);
}

export function canonicalQualificationHost(host) {
  requireRecord(host, "qualification runtime host");
  requireRecord(host.operatingSystem, "qualification runtime operating system");
  requireRecord(host.device, "qualification runtime device");
  requireRecord(host.package, "qualification runtime package");
  assert.deepEqual(Object.keys(host).sort(), ["device", "displayPath", "operatingSystem", "package", "schema"]);
  assert.deepEqual(Object.keys(host.operatingSystem).sort(), ["architecture", "build", "name", "version"]);
  assert.deepEqual(Object.keys(host.device).sort(), ["class", "gpu", "gpuClass", "gpuCores", "metalSupport"]);
  assert.deepEqual(Object.keys(host.package).sort(), ["name", "version"]);
  assert.equal(host.schema, "punctra-qualification-host-v1");
  return {
    schema: host.schema,
    operating_system: {
      name: host.operatingSystem.name,
      version: host.operatingSystem.version,
      build: host.operatingSystem.build,
      architecture: host.operatingSystem.architecture,
    },
    device: {
      class: host.device.class,
      gpu: host.device.gpu,
      gpu_cores: host.device.gpuCores,
      gpu_class: host.device.gpuClass,
      metal_support: host.device.metalSupport,
    },
    display_path: host.displayPath,
    package: {
      name: host.package.name,
      version: host.package.version,
    },
  };
}

function verifyEvidenceExternalBoundary(externalEvidence) {
  assert.deepEqual(externalEvidence, expectedExternalEvidence);
}

function verifyArtifactRegistry(artifacts, timingLimits) {
  requireArray(artifacts, "visual artifact registry");
  assert(artifacts.length > 0, "visual artifact registry is empty");
  const registry = new Map();
  for (const record of artifacts) {
    requireRecord(record, "visual artifact");
    validateImageArtifactMetadata(record);
    const commonKeys = [
      "authority", "decoded_byte_length", "decoded_sha256", "encoded_byte_length", "encoded_sha256",
      "encoding", "filename", "frame_index", "height", "kind", "mime_type", "path",
      "recreation_index", "trial_id", "width",
    ];
    if (record.kind === "baseline_png") {
      assert.deepEqual(Object.keys(record).sort(), commonKeys.sort(), `baseline artifact ${record.path} has unexpected fields`);
    } else {
      assert.deepEqual(Object.keys(record).sort(), [
        ...commonKeys, "artifact_encoding_milliseconds", "encode_milliseconds", "png_encode_milliseconds",
      ].sort(), `captured artifact ${record.path} has unexpected fields`);
      assert.equal(record.encode_milliseconds, record.png_encode_milliseconds);
      assertFiniteNonnegative(record.png_encode_milliseconds, `artifact ${record.path} PNG encoding`);
      assertFiniteNonnegative(record.artifact_encoding_milliseconds, `artifact ${record.path} encoding`);
      assert(record.png_encode_milliseconds <= timingLimits.png_encode_milliseconds_per_artifact,
        `artifact ${record.path} PNG encoding exceeded its independent ceiling`);
      assert(record.artifact_encoding_milliseconds <= timingLimits.artifact_encoding_milliseconds_per_artifact,
        `artifact ${record.path} encoding exceeded its independent ceiling`);
      assert(record.artifact_encoding_milliseconds >= record.png_encode_milliseconds,
        `artifact ${record.path} total encoding is less than PNG encoding`);
    }
    assert(!registry.has(record.path), `visual artifact path is duplicated: ${record.path}`);
    assert(
      record.path.startsWith("docs/releases/v0.21-browser-visual-artifacts/")
        || record.path.startsWith("apps/browser-demo/web/fixtures/visual-v1/baselines/"),
      `visual artifact path is outside the accepted roots: ${record.path}`,
    );
    registry.set(record.path, { record, used: false });
  }
  return registry;
}

async function readBoundArtifact(reference, registry, viewport, context) {
  const pathValue = typeof reference === "string" ? reference : reference?.path;
  assertNonemptyString(pathValue, "bound artifact path");
  const entry = registry.get(pathValue);
  assert(entry, `artifact ${pathValue} is absent from the evidence registry`);
  if (typeof reference !== "string") assert.deepEqual(reference, entry.record);
  entry.used = true;
  return verifyCanonicalImageRecord(entry.record, viewport, context);
}

async function verifyTrialEvidence(
  result,
  trial,
  expectedCamera,
  autzenManifest,
  expectedBaseline,
  corpus,
  registry,
  context,
) {
  requireRecord(result, `trial evidence ${trial.id}`);
  const source = corpus.sources.find(({ id }) => id === trial.source_id);
  assert(source, `trial ${trial.id} source is absent`);
  const runtimeSource = expectedRuntimeSourceFacts(source, autzenManifest);
  assert.equal(result.source_id, trial.source_id);
  assert.equal(result.display_mode, trial.display_mode);
  assert.equal(result.projection, expectedCamera.projection);
  assert.deepEqual(result.conditions, trial.conditions);
  assert.deepEqual(result.camera, expectedCamera);
  assert.deepEqual(result.selection, trial.selection);
  assert.deepEqual(result.features, trial.features);
  assert.equal(result.tolerance_profile, trial.tolerance_profile);
  assert.equal(result.temporal_tolerance_profile, trial.temporal_tolerance_profile);
  assert.deepEqual(result.expected_view, source.expected_view);
  verifyInputFacts(result.input_facts, source, autzenManifest);
  verifyCoverage(result.coverage, trial, source, false);
  verifyBatchFacts(result.batch_facts, trial, source.expected_view);

  verifyArtifactDescriptor(result.baseline, {
    kind: "baseline_png",
    trialId: trial.id,
    recreationIndex: null,
    frameIndex: null,
  });
  assert.equal(
    result.baseline.path,
    `apps/browser-demo/web/fixtures/visual-v1/baselines/${path.posix.basename(trial.baseline_path)}`,
  );
  assert.deepEqual(rubricArtifactIdentity(result.baseline), {
    kind: "baseline_png",
    trial_id: expectedBaseline.trial_id,
    recreation_index: null,
    frame_index: null,
    path: expectedBaseline.path,
    width: expectedBaseline.width,
    height: expectedBaseline.height,
    encoded_byte_length: expectedBaseline.encoded_byte_length,
    encoded_sha256: expectedBaseline.encoded_sha256,
    decoded_byte_length: expectedBaseline.decoded_byte_length,
    decoded_sha256: expectedBaseline.decoded_sha256,
  }, `trial ${trial.id} baseline differs from the pre-pin input manifest`);
  const baselineImage = await readBoundArtifact(result.baseline, registry, corpus.viewport, context);

  requireArray(result.recreations, `trial ${trial.id} recreations`);
  assert.equal(result.recreations.length, 3, `trial ${trial.id} needs one initial and two recreated observations`);
  const comparisons = [];
  for (let index = 0; index < result.recreations.length; index += 1) {
    const recreation = result.recreations[index];
    requireRecord(recreation, `trial ${trial.id} recreation ${index}`);
    assert.equal(recreation.index, index);
    assert.equal(recreation.environment_match, true);
    verifySettlement(recreation.settlement, trial, source, runtimeSource, expectedCamera, corpus, {
      expectedCaptureCount: 0,
      representative: true,
      observedBatches: recreation.capture?.facts?.batches,
    });
    verifyBatchFacts(recreation.batch_facts, trial, source.expected_view);
    verifyCoverage(recreation.coverage, trial, source, true);
    const nominalPickPassed = verifyNominalPickEvidence(
      recreation.nominal_pick,
      trial,
      source,
      runtimeSource,
    );
    verifyCoreDiagnostics(recreation.diagnostics, trial, source, runtimeSource, expectedCamera, corpus);

    const candidate = await verifyCaptureArtifact(
      recreation.capture,
      trial,
      source,
      runtimeSource,
      index,
      corpus,
      registry,
      context,
    );
    const derived = deriveEvidenceComparison(baselineImage, candidate, comparisonOptions(
      corpus,
      trial,
      trial.tolerance_profile,
    ), context);
    assert.deepEqual(
      recreation.comparison,
      derived,
      `trial ${trial.id} recreation ${index} comparison was not derived from decoded pixels`,
    );

    const temporal = await verifyTemporalEvidence(
      recreation.temporal,
      recreation.capture,
      trial,
      source,
      runtimeSource,
      expectedCamera,
      index,
      corpus,
      registry,
      context,
    );
    verifyResourceEvidence(
      recreation.resources,
      recreation.capture,
      temporal.timing,
      temporal.artifactPrefix,
      recreation.settlement,
      trial,
      source,
      runtimeSource,
      recreation.diagnostics,
      corpus,
    );
    verifyCleanup(recreation.cleanup, recreation.resources);

    const derivedPass = recreation.environment_match
      && nominalPickPassed
      && derived.passed
      && temporal.passed
      && temporal.transitionComplete
      && recreation.resources.cleanup.after_final_capture.pending_tickets === 0
      && recreation.resources.cleanup.after_final_capture.owned_textures === 0
      && recreation.resources.cleanup.after_final_capture.owned_readback_buffers === 0
      && recreation.resources.cleanup.after_shutdown.pending_tickets === 0
      && recreation.resources.cleanup.after_shutdown.owned_textures === 0
      && recreation.resources.cleanup.after_shutdown.owned_readback_buffers === 0
      && recreation.cleanup.shutdown_phase === "shutdown"
      && recreation.cleanup.raw_viewer_freed;
    assert.equal(derivedPass, true, `trial ${trial.id} recreation ${index} did not satisfy every independent gate`);
    assert.equal(recreation.passed, derivedPass, `trial ${trial.id} recreation ${index} recorded pass differs`);
    assert.deepEqual(recreation.failures, []);
    comparisons.push(derived);
  }
  const passed = comparisons.length === 3 && comparisons.every(({ passed: comparisonPassed }) => comparisonPassed);
  assert.equal(passed, true);
  assert.equal(result.passed, passed, `trial ${trial.id} recorded pass differs from derived recreations`);
  assert.deepEqual(result.failures, []);
  return { trial_id: trial.id, passed, comparisons };
}

function verifyNominalPickEvidence(evidence, trial, source, runtimeSource) {
  if (trial.selection.ordinals.length === 0) {
    assert.equal(evidence, null, `trial ${trial.id} issued an undeclared nominal pick`);
    return true;
  }
  requireRecord(evidence, `trial ${trial.id} nominal-pick evidence`);
  assert.equal(evidence.schema, "punctra-browser-nominal-pick-evidence-v1");
  assert.equal(evidence.gating, true);
  assert.equal(evidence.execution_order, "before_presentation_only_highlights");
  assert.equal(evidence.point_identity_authority, trial.selection.point_identity_authority);
  assert.equal(evidence.nominal_pick_coverage_authority, trial.selection.nominal_pick_coverage_authority);
  assert.equal(evidence.pick_authority, "provisional_gpu_hint");
  assert.equal(evidence.highlight_authority, trial.selection.highlight_authority);
  assert.equal(evidence.highlight_point_count_during_checks, 0);
  assert.equal(evidence.poll_frame_ceiling, 180);
  assert.equal(evidence.attempt_ceiling_per_region, 9);
  requireArray(evidence.checks, `trial ${trial.id} nominal-pick checks`);
  assert.equal(evidence.checks.length, trial.selection.nominal_pick_regions.length);
  assert.equal(source.kind, "generated", `trial ${trial.id} nominal picks require authored generated Points`);
  for (let index = 0; index < evidence.checks.length; index += 1) {
    const check = evidence.checks[index];
    const region = trial.selection.nominal_pick_regions[index];
    const feature = trial.features.find(({ id }) => id === region.feature_id);
    assert(feature, `trial ${trial.id} nominal-pick feature is absent`);
    const ordinalIndex = feature.binding.authored_point_ordinals.indexOf(region.ordinal);
    assert(ordinalIndex >= 0, `trial ${trial.id} nominal-pick Point binding is absent`);
    const batchIndex = batchIndexForOrdinal(runtimeSource.batchPointCounts, region.ordinal);
    const expectedIdentity = {
      generation: source.expected_view.generation,
      batch_key: source.expected_view.batch_keys[batchIndex],
      batch_version: trial.expected_settled_batch_versions[batchIndex],
      source_identity: runtimeSource.sourceIdentity,
      point_ordinal: String(region.ordinal),
    };
    assert.deepEqual(check, {
      ordinal: region.ordinal,
      feature_id: region.feature_id,
      expected_pixel: feature.binding.expected_pixels[ordinalIndex],
      tolerance_pixels: feature.binding.tolerance_pixels,
      nominal_region: feature.rectangle,
      expected: expectedIdentity,
      matched_pixel: check.matched_pixel,
      attempt_count: check.attempt_count,
      poll_frames_total: check.poll_frames_total,
      attempts: check.attempts,
      passed: true,
    });
    const candidatePixels = verifierNominalPickPixels(
      check.expected_pixel,
      check.nominal_region,
      check.tolerance_pixels,
    );
    assertPositiveInteger(check.attempt_count, `trial ${trial.id} nominal-pick attempt count`);
    assert(check.attempt_count <= evidence.attempt_ceiling_per_region, `trial ${trial.id} nominal pick exceeded its attempt ceiling`);
    assert.equal(check.attempts.length, check.attempt_count);
    assert.deepEqual(check.attempts.map(({ pixel }) => pixel), candidatePixels.slice(0, check.attempt_count));
    let pollFramesTotal = 0;
    for (let attemptIndex = 0; attemptIndex < check.attempts.length; attemptIndex += 1) {
      const attempt = check.attempts[attemptIndex];
      requireRecord(attempt.observed, `trial ${trial.id} nominal-pick observation`);
      assert(attempt.observed.status === "hit" || attempt.observed.status === "miss");
      assert.equal(attempt.observed.authority, "provisional_gpu_hint");
      if (attempt.observed.status === "hit") {
        assert.equal(attempt.observed.generation, expectedIdentity.generation);
        assert.equal(attempt.observed.source_identity, expectedIdentity.source_identity);
        assertPositiveInteger(attempt.observed.batch_key, `trial ${trial.id} nominal-pick batch key`);
        assertPositiveInteger(attempt.observed.batch_version, `trial ${trial.id} nominal-pick batch version`);
        assert.match(attempt.observed.point_ordinal, /^(?:0|[1-9][0-9]*)$/);
      } else {
        for (const field of ["generation", "batch_key", "batch_version", "source_identity", "point_ordinal"]) {
          assert.equal(attempt.observed[field], null);
        }
      }
      const matched = attempt.observed.status === "hit"
        && Object.entries(expectedIdentity).every(([field, value]) => attempt.observed[field] === value);
      assert.equal(attempt.matched, matched);
      assert.equal(matched, attemptIndex === check.attempts.length - 1);
      assertPositiveInteger(attempt.poll_frames, `trial ${trial.id} nominal-pick poll frames`);
      assert(attempt.poll_frames <= evidence.poll_frame_ceiling, `trial ${trial.id} nominal pick exceeded its poll ceiling`);
      pollFramesTotal += attempt.poll_frames;
    }
    assert.equal(check.poll_frames_total, pollFramesTotal);
    assert.deepEqual(check.matched_pixel, check.attempts.at(-1).pixel);
  }
  assert.equal(evidence.passed, true);
  return true;
}

function verifierNominalPickPixels(expectedPixel, region, tolerancePixels) {
  const candidates = [];
  for (let y = expectedPixel[1] - tolerancePixels; y <= expectedPixel[1] + tolerancePixels; y += 1) {
    for (let x = expectedPixel[0] - tolerancePixels; x <= expectedPixel[0] + tolerancePixels; x += 1) {
      const insideRegion = x >= region.x
        && y >= region.y
        && x < region.x + region.width
        && y < region.y + region.height;
      if (insideRegion) candidates.push([x, y]);
    }
  }
  return candidates.sort((left, right) => {
    const leftDistance = (left[0] - expectedPixel[0]) ** 2 + (left[1] - expectedPixel[1]) ** 2;
    const rightDistance = (right[0] - expectedPixel[0]) ** 2 + (right[1] - expectedPixel[1]) ** 2;
    return leftDistance - rightDistance || left[1] - right[1] || left[0] - right[0];
  });
}

function batchIndexForOrdinal(batchPointCounts, ordinal) {
  let firstOrdinal = 0;
  for (let batchIndex = 0; batchIndex < batchPointCounts.length; batchIndex += 1) {
    const afterLastOrdinal = firstOrdinal + batchPointCounts[batchIndex];
    if (ordinal >= firstOrdinal && ordinal < afterLastOrdinal) return batchIndex;
    firstOrdinal = afterLastOrdinal;
  }
  assert.fail(`selected Point ${ordinal} is absent from generated batches`);
}

function expectedRuntimeSourceFacts(source, autzenManifest) {
  if (source.kind === "generated") {
    const scene = generateVisualScene(source.generator);
    const maximumBatchPoints = Math.max(...scene.batches.map(({ points }) => points.length));
    return {
      sourceIdentity: scene.source_identity,
      worldOrigin: scene.world_origin,
      sourceZRange: scene.source_z_range,
      maximumBatchPoints,
      maximumBatchBytes: maximumBatchPoints * 32,
      batchPointCounts: scene.batches.map(({ points }) => points.length),
    };
  }
  const maximumBatchPoints = Math.min(1_024, autzenManifest.sample.point_count);
  return {
    sourceIdentity: autzenManifest.source.source_identity,
    worldOrigin: autzenManifest.source.world_origin,
    sourceZRange: [autzenManifest.source.bounds.min[2], autzenManifest.source.bounds.max[2]],
    maximumBatchPoints,
    maximumBatchBytes: maximumBatchPoints * 32,
    batchPointCounts: Array.from(
      { length: source.expected_view.published_batches },
      (_, index) => Math.min(1_024, autzenManifest.sample.point_count - index * 1_024),
    ),
  };
}

function verifyInputFacts(facts, source, autzenManifest) {
  requireRecord(facts, `source ${source.id} input facts`);
  if (source.kind === "generated") {
    const scene = generateVisualScene(source.generator);
    const payload = concatenateBytes(scene.batches.map(({ points }) => encodeTransferV2(points)));
    assert.deepEqual(facts, {
      kind: "generated",
      generator: scene.generator,
      conditions: scene.conditions,
      batch_roles: scene.batches.map(({ index, role, points }) => ({
        batch_index: index,
        role,
        point_count: points.length,
      })),
      lod_relations: scene.lod_relations,
      stable_lod_relations: scene.stable_lod_relations,
      transfer_bytes: payload.byteLength,
      payload_sha256: sha256(payload),
    });
    return;
  }
  assert.equal(facts.kind, "derived_pvis");
  assert.equal(facts.fixture_id, autzenManifest.fixture_id);
  assertUrlPathSuffix(facts.manifest_url, source.manifest_path.replace(/^\.\//, "/"));
  assertUrlPathSuffix(facts.payload_url, autzenManifest.sample.path.replace(/^\.\//, "/"));
  assert.equal(facts.payload_bytes, autzenManifest.sample.byte_length);
  assert.equal(facts.payload_sha256, autzenManifest.sample.sha256);
  assert.equal(facts.upstream_source_sha256, autzenManifest.source.sha256);
  assert.deepEqual(facts.permission, autzenManifest.permission);
  assert.deepEqual(facts.conditions, autzenManifest.conditions);
}

function assertUrlPathSuffix(value, suffix) {
  assertNonemptyString(value, "evidence URL");
  const parsed = new URL(value);
  assert(parsed.pathname.endsWith(suffix), `${value} does not bind ${suffix}`);
  assert.equal(parsed.hash, "");
}

function verifyCoverage(coverage, trial, source, recreation) {
  requireRecord(coverage, `trial ${trial.id} Coverage`);
  const expected = recreation ? {
    declared: trial.coverage,
    expected_points: source.expected_view.published_points,
    published_points: source.expected_view.published_points,
    settled_drawn_points: source.expected_view.settled_drawn_points,
    settled_resident_points: source.expected_view.settled_resident_points,
    declared_authority: "source_or_authored_facts_only",
    settled_draw_authority: "presentation_only",
    query_completion: "not_inferred_from_visual_evidence",
  } : {
    declared: trial.coverage,
    raw_stream: source.expected_view.stream_coverage,
    expected_points: source.expected_view.published_points,
    settled_drawn_points: source.expected_view.settled_drawn_points,
    declared_authority: "source_or_authored_facts_only",
    settled_draw_authority: "presentation_only",
    query_completion: "not_inferred_from_visual_evidence",
  };
  assert.deepEqual(coverage, expected);
}

function verifyBatchFacts(batchFacts, trial, expectedView) {
  assert.deepEqual(batchFacts, {
    schema: "punctra-browser-visual-batch-facts-v1",
    presentation_version: trial.expected_presentation_version,
    entries: expectedView.batch_keys.map((batchKey, batchIndex) => ({
      batch_index: batchIndex,
      batch_key: batchKey,
      initial_version: expectedView.initial_batch_versions[batchIndex],
      settled_version: trial.expected_settled_batch_versions[batchIndex],
      presentation_weight_u8: expectedView.settled_presentation_weights_u8[batchIndex],
      state: expectedView.settled_removed_batch_indices.includes(batchIndex) ? "removed" : "resident",
    })),
  });
}

function verifySettlement(settlement, trial, source, runtimeSource, expectedCamera, corpus, options) {
  requireRecord(settlement, `trial ${trial.id} settlement`);
  assert.equal(settlement.schema, "punctra-browser-quiet-window-v1");
  assert.equal(settlement.complete, true);
  assert.equal(settlement.quiet_frames, 30);
  assert.equal(settlement.required_frames, 30);
  assert.equal(settlement.observed_frames, 30);
  assert.equal(settlement.observed_frame_captures, options.expectedCaptureCount);
  assert(Number.isSafeInteger(settlement.first_settled_frame) && settlement.first_settled_frame >= 1);
  assert.equal(settlement.first_rendered_frame, settlement.first_settled_frame);
  assert.equal(settlement.quiet_window_complete_frame, settlement.last_rendered_frame);
  assert.equal(settlement.last_rendered_frame - settlement.first_rendered_frame, 29);
  assert.equal(settlement.generation, source.expected_view.generation);
  assert.equal(settlement.coverage, source.expected_view.stream_coverage);
  assert.deepEqual(settlement.animation_frame_scheduler, {
    authority: "runner_owned_request_animation_frame_tracker",
    scheduled: 30,
    resolved: 30,
    pending: 0,
  });
  verifyTimingSamples(
    settlement.frame_interval_samples_milliseconds,
    settlement.frame_interval_milliseconds,
    30,
    "quiet-frame intervals",
  );
  verifyTimingSamples(
    settlement.frame_submission_samples_milliseconds,
    settlement.frame_submission_milliseconds,
    30,
    "quiet-frame submissions",
  );
  if (options.representative) {
    assert(settlement.frame_interval_milliseconds.p95
      <= corpus.timing_limits.representative_frame_interval_p95_milliseconds,
    "representative frame interval p95 exceeded its independent ceiling");
    assert(settlement.frame_submission_milliseconds.p95
      <= corpus.timing_limits.representative_frame_submission_p95_milliseconds,
    "representative frame submission p95 exceeded its independent ceiling");
  }
  assert.deepEqual(Object.keys(settlement.stable_facts).sort(), [
    "camera",
    "capabilities",
    "capture_resources",
    "display_authority",
    "display_mode",
    "frame",
    "highlights",
    "phase",
    "streaming",
    "viewport",
  ], `trial ${trial.id} quiet-window stable-facts projection differs`);
  verifyCoreDiagnostics(
    settlement.stable_facts,
    trial,
    source,
    runtimeSource,
    expectedCamera,
    corpus,
    { requirePickFacts: false },
  );
  verifyPendingWork(settlement.pending_work, settlement, trial, source, runtimeSource, options.observedBatches);
}

function verifyTimingSamples(samples, summary, count, label) {
  requireArray(samples, `${label} samples`);
  assert.equal(samples.length, count);
  for (const sample of samples) assertFiniteNonnegative(sample, `${label} sample`);
  assert.deepEqual(summary, summarizeTimingSamples(samples), `${label} summary was not derived from raw samples`);
}

function summarizeTimingSamples(samples) {
  const ordered = [...samples].sort((left, right) => left - right);
  const percentile = (value) => ordered[Math.max(0, Math.ceil(ordered.length * value / 100) - 1)];
  return {
    count: ordered.length,
    p50: percentile(50),
    p95: percentile(95),
    maximum: ordered.at(-1),
  };
}

function expectedSettledCaptureBatches(trial, source, runtimeSource) {
  const removed = new Set(source.expected_view.settled_removed_batch_indices);
  return runtimeSource.batchPointCounts.map((pointCount, batchIndex) => ({
    batch_index: batchIndex,
    key: source.expected_view.batch_keys[batchIndex],
    version: trial.expected_settled_batch_versions[batchIndex],
    point_count: pointCount,
    state: "resident",
    presentation_weight_u8: source.expected_view.settled_presentation_weights_u8[batchIndex],
  })).filter(({ batch_index: batchIndex }) => !removed.has(batchIndex));
}

function verifyPendingWork(pending, settlement, trial, source, runtimeSource, observedBatches) {
  requireRecord(pending, `trial ${trial.id} pending-work evidence`);
  const expectedBatches = expectedSettledCaptureBatches(trial, source, runtimeSource);
  assert.deepEqual(observedBatches, expectedBatches, `trial ${trial.id} final capture batches differ from settled facts`);
  const stable = settlement.stable_facts;
  const expectedIndices = new Set(expectedBatches.map(({ batch_index: batchIndex }) => batchIndex));
  const observedIndices = new Set(observedBatches.map(({ batch_index: batchIndex }) => batchIndex));
  const categories = {
    load: stable.phase === "ready" ? 0 : 1,
    request: 0,
    publication: stable.streaming.phase === "complete"
      && stable.streaming.expected_points === stable.streaming.published_points ? 0 : 1,
    replacement: [...expectedIndices].filter((batchIndex) => !observedIndices.has(batchIndex)).length,
    retirement: [...observedIndices].filter((batchIndex) => !expectedIndices.has(batchIndex)).length,
    recolor: stable.display_mode === trial.display_mode
      && observedBatches.every((batch) => expectedBatches.some((candidate) => (
        candidate.batch_index === batch.batch_index
          && candidate.version === batch.version
          && candidate.presentation_weight_u8 === batch.presentation_weight_u8
      ))) ? 0 : 1,
    highlight: stable.highlights.point_count === trial.selection.ordinals.length ? 0 : 1,
    scheduled_render: settlement.animation_frame_scheduler.pending,
  };
  assert.deepEqual(pending, {
    schema: "punctra-browser-visual-pending-work-v1",
    categories,
    total: Object.values(categories).reduce((total, value) => total + value, 0),
    sources: {
      load: { viewer_phase: stable.phase },
      request: { transfer_path: "private_direct_transfer_v2" },
      publication: {
        stream_phase: stable.streaming.phase,
        expected_points: stable.streaming.expected_points,
        published_points: stable.streaming.published_points,
      },
      replacement_and_retirement: {
        authority: "renderer_accepted_capture_batch_snapshot",
        expected_batches: expectedBatches,
        observed_batches: observedBatches,
      },
      recolor: {
        expected_display_mode: trial.display_mode,
        observed_display_mode: stable.display_mode,
        expected_batches: expectedBatches,
        observed_batches: observedBatches,
      },
      highlight: {
        expected_points: trial.selection.ordinals.length,
        observed_points: stable.highlights.point_count,
      },
      scheduled_render: settlement.animation_frame_scheduler,
    },
  });
  assert.equal(pending.total, 0, `trial ${trial.id} retained pending work`);
}

function verifyCoreDiagnostics(
  diagnostics,
  trial,
  source,
  runtimeSource,
  expectedCamera,
  corpus,
  options = { requirePickFacts: true },
) {
  requireRecord(diagnostics, `trial ${trial.id} diagnostics`);
  assert.equal(diagnostics.phase, "ready");
  assert.deepEqual(diagnostics.capabilities, QUALIFICATION_RUNTIME_LANE.capabilities);
  assert.equal(diagnostics.viewport.css_width, corpus.viewport.css_width);
  assert.equal(diagnostics.viewport.css_height, corpus.viewport.css_height);
  assert.equal(diagnostics.viewport.device_pixel_ratio, corpus.viewport.requested_device_pixel_ratio);
  assert.equal(diagnostics.viewport.physical_width, corpus.viewport.physical_width);
  assert.equal(diagnostics.viewport.physical_height, corpus.viewport.physical_height);
  assert.equal(diagnostics.viewport.surface_bytes, corpus.viewport.physical_width * corpus.viewport.physical_height * 4);
  assert.equal(diagnostics.display_mode, trial.display_mode);
  assert.equal(diagnostics.display_authority, corpus.presentation_policy.display_authority);
  verifyObservedCamera(diagnostics.camera, expectedCamera);

  const streaming = diagnostics.streaming;
  requireRecord(streaming, `trial ${trial.id} streaming diagnostics`);
  assert.equal(streaming.phase, "complete");
  assert.equal(streaming.source_identity, runtimeSource.sourceIdentity);
  assert.equal(streaming.expected_points, source.expected_view.published_points);
  assert.equal(streaming.published_points, source.expected_view.published_points);
  assert.equal(streaming.published_batches, source.expected_view.published_batches);
  assert.equal(streaming.transferred_bytes, source.expected_view.transferred_bytes);
  assert.equal(streaming.coverage, source.expected_view.stream_coverage);
  assert.equal(streaming.view_id, source.expected_view.view_id);
  assert.equal(streaming.generation, source.expected_view.generation);
  assert.equal(streaming.presentation_version, trial.expected_presentation_version);
  assert.equal(streaming.retained_record_bytes, source.expected_view.transferred_bytes);
  assert.equal(streaming.main_thread_batch_points_high_water, runtimeSource.maximumBatchPoints);
  assert.equal(streaming.main_thread_batch_bytes_high_water, runtimeSource.maximumBatchBytes);
  assert.deepEqual(streaming.world_origin, runtimeSource.worldOrigin);
  assert.deepEqual(streaming.source_z_range, runtimeSource.sourceZRange);
  assert.equal(streaming.display_mode, trial.display_mode);
  assert(streaming.retained_record_bytes <= corpus.resource_limits.retained_record_bytes);
  assert(streaming.main_thread_batch_bytes_high_water <= corpus.resource_limits.queued_range_bytes);

  const frame = diagnostics.frame;
  requireRecord(frame, `trial ${trial.id} frame diagnostics`);
  assert.equal(frame.view_generation, source.expected_view.generation);
  assert.equal(frame.drawn_points, source.expected_view.settled_drawn_points);
  assert.equal(frame.draw_calls, source.expected_view.settled_draw_calls);
  assert.equal(frame.resident_bytes, source.expected_view.settled_resident_points * 24);
  assert(Number.isSafeInteger(frame.transient_texture_bytes) && frame.transient_texture_bytes >= 0);
  assert(frame.transient_texture_bytes <= corpus.resource_limits.renderer_transient_texture_bytes);
  assert.equal(frame.surface_suboptimal, false);
  verifyCaptureCleanupFacts(diagnostics.capture_resources, `trial ${trial.id} diagnostics capture resources`);
  if (options.requirePickFacts) {
    requireRecord(diagnostics.pick, `trial ${trial.id} pick diagnostics`);
    assert.equal(diagnostics.pick.status, "not_requested");
    assert.equal(diagnostics.pick.authority, "provisional_gpu_hint");
    for (const name of ["generation", "batch_key", "batch_version", "source_identity", "point_ordinal"]) {
      assert.equal(diagnostics.pick[name], null);
    }
  } else {
    assert.equal("pick" in diagnostics, false, `trial ${trial.id} quiet-window facts retained pick state`);
  }

  assert.equal(diagnostics.highlights.point_count, trial.selection.ordinals.length);
  assert.equal(diagnostics.highlights.authority, "presentation_only");
  if (trial.selection.ordinals.length === 0) {
    assert.equal(diagnostics.highlights.source_identity, null);
    assert.equal(diagnostics.highlights.generation, null);
  } else {
    assert.equal(diagnostics.highlights.generation, source.expected_view.generation);
    assert.equal(diagnostics.highlights.source_identity, runtimeSource.sourceIdentity);
  }
}

function verifyObservedCamera(observed, expected) {
  requireRecord(observed, "observed camera");
  assert.deepEqual(observed.eye, expected.eye);
  assert.deepEqual(observed.target, expected.target);
  assert.deepEqual(observed.up, expected.up);
  assert.equal(observed.projection, expected.projection);
  assertObservedF32(observed.near_distance, expected.near_distance, "observed camera near distance");
  assertObservedF32(observed.far_distance, expected.far_distance, "observed camera far distance");
  if (expected.projection === "perspective") {
    assertObservedF32(
      observed.vertical_field_of_view_radians,
      expected.vertical_field_of_view_radians,
      "observed camera field of view",
    );
    assert.equal(observed.vertical_world_height, null);
  } else {
    assert.equal(observed.vertical_field_of_view_radians, null);
    assert.equal(observed.vertical_world_height, expected.vertical_world_height);
  }
}

function assertObservedF32(observed, expected, label) {
  assert(Number.isFinite(observed), `${label} must be finite`);
  assert.equal(Math.fround(observed), Math.fround(expected), `${label} differs`);
}

async function verifyCaptureArtifact(capture, trial, source, runtimeSource, recreationIndex, corpus, registry, context) {
  verifyCaptureRecord(capture, trial, source, runtimeSource, corpus);
  verifyArtifactDescriptor(capture.artifact, {
    kind: "recreation_png",
    trialId: trial.id,
    recreationIndex,
    frameIndex: 29,
  });
  return readBoundArtifact(capture.artifact, registry, corpus.viewport, context);
}

function verifyCaptureRecord(capture, trial, source, runtimeSource, corpus, expectedBatches) {
  requireRecord(capture, `trial ${trial.id} frame capture`);
  assert.equal(capture.schema, "punctra-browser-canonical-capture-v1");
  const facts = capture.facts;
  requireRecord(facts, `trial ${trial.id} capture facts`);
  const required = corpus.required_capabilities;
  const expectedFacts = {
    schema: corpus.capture.schema,
    status: "ready",
    completion: "map_callback_completed_and_copied",
    presentation: corpus.capture.presentation_claim,
    width: corpus.viewport.physical_width,
    height: corpus.viewport.physical_height,
    configured_surface_color_space: "srgb",
    canonical_format: corpus.capture.canonical_format,
    canonical_channel_order: corpus.capture.canonical_channel_order,
    origin: corpus.capture.origin,
    bytes_per_pixel: 4,
    tight_bytes_per_row: corpus.viewport.physical_width * 4,
    output_bytes: corpus.resource_limits.canonical_pixel_bytes,
    color_texture_bytes: corpus.resource_limits.capture_texture_bytes,
    source_format: required.capture_source_format,
    source_channel_order: required.capture_source_channel_order,
    canonical_encoding: corpus.capture.canonical_encoding,
    normalization: required.capture_canonicalization,
    canonical_pixel_bytes: corpus.resource_limits.canonical_pixel_bytes,
    physical_presentation_observed: false,
  };
  for (const [name, value] of Object.entries(expectedFacts)) assert.equal(facts[name], value, `capture ${name} differs`);
  assert(["linear", "srgb"].includes(facts.source_encoding));
  assert.equal(facts.canonical_encoding, facts.source_encoding);
  assert.equal(facts.row_alignment_bytes, 256);
  assert.equal(facts.padded_bytes_per_row, corpus.viewport.physical_width * 4);
  assert.equal(facts.staging_buffer_bytes, corpus.resource_limits.staging_buffer_bytes);
  assert.equal(facts.view_generation, source.expected_view.generation);
  const batches = expectedBatches ?? expectedSettledCaptureBatches(trial, source, runtimeSource);
  assert.equal(facts.batch_state_authority, "renderer_accepted_updates");
  assert.deepEqual(facts.batches, batches, `trial ${trial.id} capture-bound renderer batches differ`);
  assert.equal(facts.drawn_points, batches.reduce((total, batch) => total + batch.point_count, 0));
  assert.equal(facts.draw_calls, batches.length);
  assert.equal(facts.resident_bytes, batches.reduce((total, batch) => total + batch.point_count * 24, 0));
  assert(Number.isSafeInteger(facts.renderer_transient_texture_bytes) && facts.renderer_transient_texture_bytes >= 0);
  assert(facts.renderer_transient_texture_bytes <= corpus.resource_limits.renderer_transient_texture_bytes);

  const completion = facts.completion_callbacks;
  requireRecord(completion, `trial ${trial.id} capture callback facts`);
  assert.deepEqual(Object.keys(completion).sort(), [
    "origin", "readback_mapping_callback_milliseconds", "schema", "submitted_work_done_callback_milliseconds",
  ]);
  assert.equal(completion.schema, "punctra-browser-frame-capture-completion-v1");
  assert.equal(completion.origin, "begin_frame_capture_monotonic_clock");

  const timing = capture.timing;
  requireRecord(timing, `trial ${trial.id} capture timing`);
  assert.deepEqual(Object.keys(timing).sort(), [
    "animation_frames",
    "begin_submission_milliseconds",
    "callback_elapsed_origin",
    "callback_ordering",
    "canonical_copy_milliseconds",
    "physical_gpu_timing",
    "poll_call_milliseconds",
    "poll_count",
    "poll_wait_milliseconds",
    "readback_mapping_callback_milliseconds",
    "submitted_work_done_callback_milliseconds",
    "total_milliseconds",
  ]);
  const limits = corpus.timing_limits;
  const bounded = [
    ["begin_submission_milliseconds", "capture_begin_submission_milliseconds_per_frame"],
    ["poll_wait_milliseconds", "capture_poll_wait_milliseconds_per_frame"],
    ["poll_call_milliseconds", "capture_poll_call_milliseconds_per_frame"],
    ["canonical_copy_milliseconds", "capture_canonical_copy_milliseconds_per_frame"],
    ["submitted_work_done_callback_milliseconds", "capture_submitted_work_done_callback_milliseconds_per_frame"],
    ["readback_mapping_callback_milliseconds", "capture_readback_mapping_callback_milliseconds_per_frame"],
  ];
  for (const [field, ceiling] of bounded) {
    assertFiniteNonnegative(timing[field], `capture timing ${field}`);
    assert(timing[field] <= limits[ceiling], `capture timing ${field} exceeded its independent ceiling`);
  }
  assert.equal(timing.submitted_work_done_callback_milliseconds, completion.submitted_work_done_callback_milliseconds);
  assert.equal(timing.readback_mapping_callback_milliseconds, completion.readback_mapping_callback_milliseconds);
  assert.equal(timing.callback_elapsed_origin, completion.origin);
  assert.equal(timing.callback_ordering, "not_inferred");
  assert.equal(timing.physical_gpu_timing, "not_observed");
  assertFiniteNonnegative(timing.total_milliseconds, "capture timing total_milliseconds");
  assert(Number.isSafeInteger(timing.poll_count) && timing.poll_count >= 1);
  assert(timing.poll_count <= corpus.settling.capture_poll_frame_ceiling);
  assert.equal(timing.animation_frames, timing.poll_count);
  for (const name of ["begin_submission_milliseconds", "poll_wait_milliseconds", "poll_call_milliseconds", "canonical_copy_milliseconds"]) {
    assert(timing.total_milliseconds >= timing[name], `capture total timing is less than ${name}`);
  }
  assert.deepEqual(capture.resource_facts, {
    capture_texture_bytes: facts.color_texture_bytes,
    row_aligned_readback_bytes: facts.staging_buffer_bytes,
    canonical_pixel_bytes: facts.canonical_pixel_bytes,
    peak_live_canonical_images_during_capture: 1,
  });
}

async function verifyTemporalEvidence(
  temporal,
  finalCapture,
  trial,
  source,
  runtimeSource,
  expectedCamera,
  recreationIndex,
  corpus,
  registry,
  context,
) {
  requireRecord(temporal, `trial ${trial.id} temporal evidence`);
  assert.equal(temporal.kind, trial.temporal_trace.kind);
  assert.deepEqual(temporal.trace, trial.temporal_trace);
  assert.equal(temporal.quiet_frame_count, 30);
  const settled = temporal.settled_window;
  requireRecord(settled, `trial ${trial.id} settled temporal window`);
  assert.equal(settled.schema, "punctra-settled-quiet-window-evidence-v1");
  assert.equal(settled.gating, true);
  assert.equal(settled.frame_count, 30);
  assert.equal(settled.pair_count, 29);
  verifySettlement(settled.capture_window, trial, source, runtimeSource, expectedCamera, corpus, {
    expectedCaptureCount: 30,
    representative: false,
    observedBatches: finalCapture.facts.batches,
  });
  requireArray(settled.frames, `trial ${trial.id} settled frames`);
  requireArray(settled.pairs, `trial ${trial.id} settled pairs`);
  assert.equal(settled.frames.length, 30);
  assert.equal(settled.pairs.length, 29);

  const derivedPairs = [];
  let previousImage;
  let previousPath;
  let settledCaptureMilliseconds = 0;
  for (let index = 0; index < settled.frames.length; index += 1) {
    const frame = settled.frames[index];
    requireRecord(frame, `trial ${trial.id} settled frame ${index}`);
    assert.equal(frame.index, index);
    verifyCaptureRecord(frame.capture, trial, source, runtimeSource, corpus);
    settledCaptureMilliseconds += frame.capture.timing.total_milliseconds;
    verifyArtifactDescriptor(frame.artifact, {
      kind: index === 29 ? "recreation_png" : "settled_quiet_frame_png",
      trialId: trial.id,
      recreationIndex,
      frameIndex: index,
    });
    const image = await readBoundArtifact(frame.artifact, registry, corpus.viewport, context);
    if (index > 0) {
      const comparison = deriveEvidenceComparison(previousImage, image, comparisonOptions(
        corpus,
        trial,
        trial.temporal_tolerance_profile,
      ), context);
      const pair = {
        from_index: index - 1,
        to_index: index,
        from_id: previousPath,
        to_id: frame.artifact.path,
        from_path: previousPath,
        to_path: frame.artifact.path,
        comparison,
        comparison_milliseconds: settled.pairs[index - 1].comparison_milliseconds,
      };
      verifyComparisonMilliseconds(pair.comparison_milliseconds, corpus.timing_limits, `trial ${trial.id} settled pair ${index - 1}`);
      assert.deepEqual(settled.pairs[index - 1], pair, `trial ${trial.id} settled pair ${index - 1} was not pixel-derived`);
      derivedPairs.push(pair);
    }
    previousImage = image;
    previousPath = frame.artifact.path;
  }
  const finalFrame = settled.frames.at(-1);
  assert.deepEqual(finalCapture.artifact, finalFrame.artifact);
  const finalCaptureWithoutArtifact = { ...finalCapture };
  delete finalCaptureWithoutArtifact.artifact;
  assert.deepEqual(finalCaptureWithoutArtifact, finalFrame.capture);

  const summary = summarizeTemporalPairs(settled.frames.length, derivedPairs);
  assert.deepEqual(settled.summary, summary, `trial ${trial.id} settled temporal summary was not derived`);
  const expectedWorst = {
    pair_index: summary.worst_pair_index,
    ...summary.worst_pair,
    difference_policy: "maximum-absolute-rgba-channel-delta-as-opaque-grayscale-v1",
  };
  const differenceArtifact = settled.worst_pair.difference_artifact;
  const observedWorstWithoutArtifact = { ...settled.worst_pair };
  delete observedWorstWithoutArtifact.difference_artifact;
  assert.deepEqual(observedWorstWithoutArtifact, expectedWorst);
  verifyArtifactDescriptor(differenceArtifact, {
    kind: "settled_quiet_worst_difference_png",
    trialId: trial.id,
    recreationIndex,
    frameIndex: summary.worst_pair_index,
  });
  const [worstFrom, worstTo, differenceImage] = await Promise.all([
    readBoundArtifact(summary.worst_pair.from_path, registry, corpus.viewport, context),
    readBoundArtifact(summary.worst_pair.to_path, registry, corpus.viewport, context),
    readBoundArtifact(differenceArtifact, registry, corpus.viewport, context),
  ]);
  assertBytesEqual(
    differenceImage.data,
    createDifferenceImage(worstFrom, worstTo).data,
    `trial ${trial.id} worst-pair difference PNG was not pixel-derived`,
  );
  if (source.kind === "generated") {
    assert(derivedPairs.every(({ comparison }) => comparison.pixels.unstable === 0), "generated settled temporal pixels must be exact");
  }
  assert.equal(summary.passed, true, `trial ${trial.id} settled temporal window failed`);
  const transitionFacts = await verifyTransitionEvidence(
    temporal.transition,
    trial,
    source,
    runtimeSource,
    recreationIndex,
    corpus,
    registry,
    context,
  );
  return {
    passed: summary.passed,
    transitionComplete: transitionFacts.complete,
    timing: {
      settledCaptureSamples: settled.frames.map(({ capture }) => capture.timing),
      settledCaptureMilliseconds,
      settledComparisonSamples: derivedPairs.map(({ comparison_milliseconds: value }) => value),
      settledComparisonMilliseconds: derivedPairs.reduce((total, { comparison_milliseconds: value }) => total + value, 0),
      transitionCaptureSamples: transitionFacts.captureSamples,
      transitionCaptureMilliseconds: transitionFacts.captureMilliseconds,
      transitionComparisonSamples: transitionFacts.comparisonSamples,
      transitionComparisonMilliseconds: transitionFacts.comparisonMilliseconds,
    },
    artifactPrefix: artifactPrefixFacts(registry, differenceArtifact.path),
  };
}

async function verifyTransitionEvidence(
  transition,
  trial,
  source,
  runtimeSource,
  recreationIndex,
  corpus,
  registry,
  context,
) {
  if (trial.temporal_trace.kind === "static") {
    assert.equal(transition, null);
    return {
      complete: true,
      captureSamples: [],
      captureMilliseconds: 0,
      comparisonSamples: [],
      comparisonMilliseconds: 0,
    };
  }
  requireRecord(transition, `trial ${trial.id} mixed-LOD transition`);
  const trace = trial.temporal_trace;
  assert.equal(transition.schema, "punctra-mixed-lod-transition-evidence-v1");
  assert.equal(transition.gating, false);
  assert.equal(transition.parent_batch_index, trace.parent_batch_index);
  assert.equal(transition.child_batch_index, trace.child_batch_index);
  assert.equal(transition.parent_removed_after_transition, true);
  assert.equal(transition.interpretation, "recorded_dynamic_transition_not_a_static_tolerance_gate");
  const stableRelation = source.kind === "generated"
    ? generateVisualScene(source.generator).stable_lod_relations.find(
      ({ dense_batch_index: denseBatchIndex }) => denseBatchIndex === trace.child_batch_index,
    )
    : undefined;
  assert.deepEqual(transition.stable_lod_cut, {
    ...stableRelation,
    dense_weight_u8: 255,
    coarse_weight_u8: 255,
    resident_through_transition: true,
  });
  requireArray(transition.frames, `trial ${trial.id} transition frames`);
  assert.equal(transition.frames.length, corpus.settling.transition_frame_count);
  const derivedPairs = [];
  let previousImage;
  let previousPath;
  const captureSamples = [];
  for (let index = 0; index < transition.frames.length; index += 1) {
    const frame = transition.frames[index];
    assert.equal(frame.index, index);
    assert.equal(frame.child_weight_u8, trace.child_weights_u8[index]);
    assert.equal(frame.parent_weight_u8, 255 - trace.child_weights_u8[index]);
    const weights = [...source.expected_view.settled_presentation_weights_u8];
    weights[trace.parent_batch_index] = frame.parent_weight_u8;
    weights[trace.child_batch_index] = frame.child_weight_u8;
    const transitionBatches = runtimeSource.batchPointCounts.map((pointCount, batchIndex) => ({
      batch_index: batchIndex,
      key: source.expected_view.batch_keys[batchIndex],
      version: trial.expected_settled_batch_versions[batchIndex],
      point_count: pointCount,
      state: "resident",
      presentation_weight_u8: weights[batchIndex],
    }));
    verifyCaptureRecord(frame.capture, trial, source, runtimeSource, corpus, transitionBatches);
    captureSamples.push(frame.capture.timing);
    verifyArtifactDescriptor(frame.artifact, {
      kind: "mixed_lod_transition_png",
      trialId: trial.id,
      recreationIndex,
      frameIndex: index,
    });
    const image = await readBoundArtifact(frame.artifact, registry, corpus.viewport, context);
    if (index > 0) {
      const comparisonMilliseconds = transition.comparisons.pairs[index - 1].comparison_milliseconds;
      verifyComparisonMilliseconds(comparisonMilliseconds, corpus.timing_limits, `trial ${trial.id} transition pair ${index - 1}`);
      derivedPairs.push({
        from_index: index - 1,
        to_index: index,
        from_id: previousPath,
        to_id: frame.artifact.path,
        comparison: deriveEvidenceComparison(previousImage, image, comparisonOptions(
          corpus,
          trial,
          trial.tolerance_profile,
        ), context),
        comparison_milliseconds: comparisonMilliseconds,
      });
    }
    previousImage = image;
    previousPath = frame.artifact.path;
  }
  const comparisons = summarizeTemporalPairs(transition.frames.length, derivedPairs);
  assert.deepEqual(transition.comparisons, comparisons, `trial ${trial.id} transition comparisons were not pixel-derived`);
  const changedPairCount = derivedPairs.filter(({ comparison }) => comparison.pixels?.unstable > 0).length;
  assert.equal(transition.changed_pair_count, changedPairCount);
  const complete = transition.frames.length === corpus.settling.transition_frame_count
    && transition.frames[0].child_weight_u8 === 0
    && transition.frames.at(-1).child_weight_u8 === 255
    && changedPairCount > 0
    && stableRelation !== undefined;
  assert.equal(complete, true, `trial ${trial.id} mixed-LOD transition was incomplete`);
  assert.equal(transition.complete, complete);
  requireRecord(transition.timing, `trial ${trial.id} transition timing`);
  assert.equal(transition.timing.schema, "punctra-browser-visual-transition-timing-v1");
  assert.deepEqual(transition.timing.capture_samples, captureSamples);
  const captureMilliseconds = captureSamples.reduce((total, sample) => total + sample.total_milliseconds, 0);
  assert.equal(transition.timing.capture_total_milliseconds, captureMilliseconds);
  const comparisonSamples = derivedPairs.map(({ comparison_milliseconds: value }) => value);
  const comparisonMilliseconds = comparisonSamples.reduce((total, value) => total + value, 0);
  assert.deepEqual(transition.timing.comparison_samples_milliseconds, comparisonSamples);
  assert.equal(transition.timing.comparison_total_milliseconds, comparisonMilliseconds);
  assert(captureMilliseconds <= corpus.timing_limits.transition_capture_total_milliseconds_per_recreation,
    `trial ${trial.id} transition capture exceeded its independent ceiling`);
  assert(comparisonMilliseconds <= corpus.timing_limits.transition_comparison_total_milliseconds_per_recreation,
    `trial ${trial.id} transition comparison exceeded its independent ceiling`);
  return { complete, captureSamples, captureMilliseconds, comparisonSamples, comparisonMilliseconds };
}

function verifyComparisonMilliseconds(value, limits, label) {
  assertFiniteNonnegative(value, `${label} comparison milliseconds`);
  assert(value <= limits.comparison_milliseconds_per_pair, `${label} comparison exceeded its independent ceiling`);
}

function comparisonOptions(corpus, trial, profileName) {
  return {
    toleranceProfile: corpus.tolerance_profiles[profileName],
    features: trial.features,
    backgroundRgba: corpus.presentation_policy.canonical_clear_rgba8,
  };
}

function deriveEvidenceComparison(reference, candidate, options, context) {
  const referenceDigest = context.imageDigestByObject.get(reference);
  const candidateDigest = context.imageDigestByObject.get(candidate);
  const key = referenceDigest === undefined || candidateDigest === undefined
    ? undefined
    : `${referenceDigest}:${candidateDigest}:${JSON.stringify(options)}`;
  if (key !== undefined && context.comparisonCache.has(key)) return context.comparisonCache.get(key);
  const report = deriveCanonicalComparison(reference, candidate, options);
  if (key !== undefined) context.comparisonCache.set(key, report);
  return report;
}

function verifyResourceEvidence(
  resources,
  capture,
  temporalTiming,
  artifactPrefix,
  representativeSettlement,
  trial,
  source,
  runtimeSource,
  diagnostics,
  corpus,
) {
  const limits = corpus.resource_limits;
  requireRecord(resources, `trial ${trial.id} resource evidence`);
  assert.equal(resources.schema, "punctra-browser-visual-resource-evidence-v1");
  for (const name of ["renderer", "transfer", "capture", "cleanup", "timing", "unavailable"]) {
    requireRecord(resources[name], `trial ${trial.id} ${name} resources`);
  }
  assert.equal(resources.renderer.resident_points, source.expected_view.settled_resident_points);
  assert.equal(resources.renderer.resident_bytes, source.expected_view.settled_resident_points * 24);
  assert.equal(resources.renderer.batches, source.expected_view.settled_draw_calls);
  assert.equal(resources.renderer.highlight_points, trial.selection.ordinals.length);
  assert.equal(resources.renderer.drawn_points, source.expected_view.settled_drawn_points);
  assert.equal(resources.renderer.draw_calls, source.expected_view.settled_draw_calls);
  assert.equal(resources.renderer.transient_texture_bytes, diagnostics.frame.transient_texture_bytes);
  assert.equal(resources.renderer.canvas_surface_bytes, diagnostics.viewport.surface_bytes);
  assertAtMost(resources.renderer.resident_points, limits.renderer_resident_points, "renderer resident Points");
  assertAtMost(resources.renderer.resident_bytes, limits.renderer_resident_bytes, "renderer resident bytes");
  assertAtMost(resources.renderer.transient_texture_bytes, limits.renderer_transient_texture_bytes, "renderer transient texture bytes");
  assertAtMost(resources.renderer.canvas_surface_bytes, limits.canvas_surface_bytes, "canvas surface bytes");
  assertAtMost(resources.renderer.batches, limits.renderer_batches, "renderer batches");
  assertAtMost(resources.renderer.highlight_points, limits.highlight_points, "highlight Points");

  assert.equal(resources.transfer.retained_record_bytes, diagnostics.streaming.retained_record_bytes);
  assert.equal(resources.transfer.main_thread_batch_bytes_high_water, diagnostics.streaming.main_thread_batch_bytes_high_water);
  assert.equal(resources.transfer.retained_record_bytes, source.expected_view.transferred_bytes);
  assert.equal(resources.transfer.main_thread_batch_bytes_high_water, runtimeSource.maximumBatchBytes);
  assert.equal(resources.transfer.worker_staging_bytes, 0);
  assert.equal(resources.transfer.queued_range_bytes, 0);
  assert.equal(resources.transfer.concurrent_response_bytes, source.kind === "derived_pvis" ? source.expected_view.transferred_bytes : 0);
  assert.equal(resources.transfer.memory_cache_bytes, 0);
  assert.equal(resources.transfer.persistent_cache_bytes, 0);
  assert.equal(resources.transfer.path, "private_direct_transfer_v2");
  assertAtMost(resources.transfer.retained_record_bytes, limits.retained_record_bytes, "retained record bytes");
  assertAtMost(resources.transfer.main_thread_batch_bytes_high_water, limits.queued_range_bytes, "main-thread batch bytes");
  assertAtMost(resources.transfer.worker_staging_bytes, limits.worker_staging_bytes, "Worker staging bytes");
  assertAtMost(resources.transfer.queued_range_bytes, limits.queued_range_bytes, "queued range bytes");
  assertAtMost(resources.transfer.concurrent_response_bytes, limits.concurrent_response_bytes, "concurrent response bytes");
  assertAtMost(resources.transfer.memory_cache_bytes, limits.memory_cache_bytes, "memory cache bytes");
  assertAtMost(resources.transfer.persistent_cache_bytes, limits.persistent_cache_bytes, "persistent cache bytes");

  assert.equal(resources.capture.capture_texture_bytes, capture.facts.color_texture_bytes);
  assert.equal(resources.capture.staging_buffer_bytes, capture.facts.staging_buffer_bytes);
  assert.equal(resources.capture.row_aligned_readback_bytes, capture.facts.staging_buffer_bytes);
  assert.equal(resources.capture.canonical_pixel_bytes, capture.facts.canonical_pixel_bytes);
  assert.equal(resources.capture.encoded_png_bytes, artifactPrefix.maximumEncodedBytes);
  assert.equal(resources.capture.total_encoded_artifact_bytes, artifactPrefix.totalEncodedBytes);
  assert.equal(resources.capture.png_scanline_bytes, limits.png_scanline_bytes);
  assert.equal(
    resources.capture.encoder_working_bytes,
    limits.canonical_pixel_bytes + limits.png_scanline_bytes + limits.comparison_workspace_bytes,
  );
  assert.equal(resources.capture.baseline_decoded_bytes, limits.canonical_pixel_bytes);
  assert.equal(resources.capture.comparison_workspace_bytes, 1_024);
  assert.equal(resources.capture.peak_live_canonical_images, 2);
  assertAtMost(resources.capture.capture_texture_bytes, limits.capture_texture_bytes, "capture texture bytes");
  assertAtMost(resources.capture.staging_buffer_bytes, limits.staging_buffer_bytes, "staging buffer bytes");
  assertAtMost(resources.capture.row_aligned_readback_bytes, limits.row_aligned_readback_bytes, "readback bytes");
  assertAtMost(resources.capture.canonical_pixel_bytes, limits.canonical_pixel_bytes, "canonical pixel bytes");
  assertAtMost(resources.capture.encoded_png_bytes, limits.encoded_png_bytes, "encoded PNG bytes");
  assertAtMost(resources.capture.total_encoded_artifact_bytes, limits.total_encoded_artifact_bytes, "total encoded artifact bytes");
  assertAtMost(resources.capture.encoder_working_bytes, limits.encoder_working_bytes, "encoder working bytes");
  assertAtMost(resources.capture.comparison_workspace_bytes, limits.comparison_workspace_bytes, "comparison workspace bytes");
  assertAtMost(resources.capture.peak_live_canonical_images, limits.peak_live_canonical_images, "live canonical images");

  assert.deepEqual(resources.cleanup.after_final_capture, diagnostics.capture_resources);
  verifyCaptureCleanupFacts(resources.cleanup.after_final_capture, "after final capture");
  verifyCaptureCleanupFacts(resources.cleanup.after_shutdown, "after shutdown");
  verifyRecreationTiming(
    resources.timing,
    temporalTiming,
    artifactPrefix.records.filter(({ record }) => (
      record.trial_id === trial.id
        && record.recreation_index === capture.artifact.recreation_index
    )).map(({ record }) => record),
    representativeSettlement,
    corpus.timing_limits,
  );
  assert.deepEqual(resources.unavailable, {
    gpu_or_driver_allocation_bytes: null,
    process_resident_bytes: null,
    physical_cache_allocation_bytes: null,
  });
}

function verifyRecreationTiming(timing, temporal, artifacts, representativeSettlement, limits) {
  requireRecord(timing, "recreation timing evidence");
  assert.equal(timing.schema, "punctra-browser-visual-timing-evidence-v1");
  requireRecord(timing.lifecycle, "recreation lifecycle timing");
  assert.deepEqual(Object.keys(timing.lifecycle).sort(), [
    "first_coverage", "first_coverage_milliseconds", "schema", "settled_view", "settled_view_milliseconds", "start",
  ]);
  assert.equal(timing.lifecycle.schema, "punctra-browser-visual-lifecycle-timing-v1");
  assert.equal(timing.lifecycle.start, "fresh_private_viewer_creation");
  assert.equal(timing.lifecycle.first_coverage, "first_renderer_accepted_batch_and_sampled_frame_submission");
  assert.equal(timing.lifecycle.settled_view, "complete_stream_camera_display_mode_and_frame_submission");
  assertFiniteNonnegative(timing.lifecycle.first_coverage_milliseconds, "first Coverage milliseconds");
  assertFiniteNonnegative(timing.lifecycle.settled_view_milliseconds, "settled View milliseconds");
  assert(timing.lifecycle.first_coverage_milliseconds <= limits.first_coverage_milliseconds,
    "first Coverage exceeded its independent ceiling");
  assert(timing.lifecycle.settled_view_milliseconds <= limits.settled_view_milliseconds,
    "settled View exceeded its independent ceiling");
  assert(timing.lifecycle.settled_view_milliseconds >= timing.lifecycle.first_coverage_milliseconds,
    "settled View preceded first Coverage");

  assert.deepEqual(timing.representative_frames, {
    capture_free: true,
    frame_count: representativeSettlement.quiet_frames,
    frame_interval_samples_milliseconds: representativeSettlement.frame_interval_samples_milliseconds,
    frame_submission_samples_milliseconds: representativeSettlement.frame_submission_samples_milliseconds,
    frame_interval_milliseconds: representativeSettlement.frame_interval_milliseconds,
    frame_submission_milliseconds: representativeSettlement.frame_submission_milliseconds,
  });

  const expectedSettledCapture = captureTimingWindow(temporal.settledCaptureSamples);
  const expectedTransitionCapture = captureTimingWindow(temporal.transitionCaptureSamples);
  assert.deepEqual(timing.capture, {
    settled: expectedSettledCapture,
    transition: expectedTransitionCapture,
    all_windows_total_milliseconds:
      expectedSettledCapture.totals.total_milliseconds + expectedTransitionCapture.totals.total_milliseconds,
  });
  assert(expectedSettledCapture.totals.total_milliseconds
    <= limits.settled_capture_total_milliseconds_per_recreation,
  "settled capture total exceeded its independent ceiling");
  assert(expectedTransitionCapture.totals.total_milliseconds
    <= limits.transition_capture_total_milliseconds_per_recreation,
  "transition capture total exceeded its independent ceiling");

  const comparison = timing.comparison;
  requireRecord(comparison, "recreation comparison timing");
  verifyComparisonMilliseconds(comparison.baseline_milliseconds, limits, "baseline");
  verifyComparisonMilliseconds(comparison.worst_pair_difference_derivation_milliseconds, limits, "worst-pair difference");
  assert.deepEqual(comparison.settled_pair_samples_milliseconds, temporal.settledComparisonSamples);
  assert.equal(comparison.settled_pair_total_milliseconds, temporal.settledComparisonMilliseconds);
  assert.deepEqual(comparison.transition_pair_samples_milliseconds, temporal.transitionComparisonSamples);
  assert.equal(comparison.transition_total_milliseconds, temporal.transitionComparisonMilliseconds);
  const settledTotal = comparison.baseline_milliseconds
    + temporal.settledComparisonMilliseconds
    + comparison.worst_pair_difference_derivation_milliseconds;
  assert.equal(comparison.settled_total_milliseconds, settledTotal);
  assert.equal(comparison.all_comparisons_total_milliseconds,
    settledTotal + temporal.transitionComparisonMilliseconds);
  assert(settledTotal <= limits.settled_comparison_total_milliseconds_per_recreation,
    "settled comparison total exceeded its independent ceiling");
  assert(temporal.transitionComparisonMilliseconds
    <= limits.transition_comparison_total_milliseconds_per_recreation,
  "transition comparison total exceeded its independent ceiling");

  const artifactTiming = artifacts.map((artifact) => ({
    path: artifact.path,
    png_encode_milliseconds: artifact.png_encode_milliseconds,
    artifact_encoding_milliseconds: artifact.artifact_encoding_milliseconds,
  }));
  const pngTotal = artifactTiming.reduce((total, sample) => total + sample.png_encode_milliseconds, 0);
  const artifactTotal = artifactTiming.reduce((total, sample) => total + sample.artifact_encoding_milliseconds, 0);
  assert.deepEqual(timing.encoding, {
    artifacts: artifactTiming,
    artifact_count: artifactTiming.length,
    png_encode_total_milliseconds: pngTotal,
    artifact_encoding_total_milliseconds: artifactTotal,
  });
  assert(artifactTotal <= limits.artifact_encoding_total_milliseconds_per_recreation,
    "artifact encoding total exceeded its independent ceiling");
}

function captureTimingWindow(samples) {
  const fields = [
    "begin_submission_milliseconds",
    "poll_wait_milliseconds",
    "poll_call_milliseconds",
    "canonical_copy_milliseconds",
    "submitted_work_done_callback_milliseconds",
    "readback_mapping_callback_milliseconds",
    "total_milliseconds",
    "poll_count",
    "animation_frames",
  ];
  return {
    sample_count: samples.length,
    samples,
    totals: Object.fromEntries(fields.map((field) => [
      field,
      samples.reduce((total, sample) => total + sample[field], 0),
    ])),
  };
}

function verifyCleanup(cleanup, resources) {
  requireRecord(cleanup, "visual recreation cleanup");
  assert.equal(cleanup.shutdown_phase, "shutdown");
  assert.equal(cleanup.raw_viewer_freed, true);
  assert.deepEqual(cleanup.capture_resources, resources.cleanup);
  verifyCaptureCleanupFacts(resources.cleanup.after_final_capture, "after final capture");
  verifyCaptureCleanupFacts(resources.cleanup.after_shutdown, "after shutdown");
}

function verifyCaptureCleanupFacts(facts, label) {
  requireRecord(facts, label);
  assert.deepEqual(facts, {
    pending_tickets: 0,
    owned_textures: 0,
    owned_readback_buffers: 0,
  });
}

function artifactPrefixFacts(registry, lastPath) {
  let totalEncodedBytes = 0;
  let maximumEncodedBytes = 0;
  let totalEncodeMilliseconds = 0;
  let found = false;
  const records = [];
  for (const { record } of registry.values()) {
    records.push({ record });
    totalEncodedBytes += record.encoded_byte_length;
    maximumEncodedBytes = Math.max(maximumEncodedBytes, record.encoded_byte_length);
    totalEncodeMilliseconds += record.encode_milliseconds ?? 0;
    if (record.path === lastPath) {
      found = true;
      break;
    }
  }
  assert(found, `artifact prefix terminator ${lastPath} is absent`);
  return { totalEncodedBytes, maximumEncodedBytes, totalEncodeMilliseconds, records };
}

function assertAtMost(value, maximum, label) {
  assert(Number.isSafeInteger(value) && value >= 0, `${label} is invalid`);
  assert(value <= maximum, `${label} exceeded its independent ceiling`);
}

function assertFiniteNonnegative(value, label) {
  assert(Number.isFinite(value) && value >= 0, `${label} must be finite and nonnegative`);
}

function assertFinitePositive(value, label) {
  assert(Number.isFinite(value) && value > 0, `${label} must be finite and positive`);
}

function verifyArtifactDescriptor(record, expected) {
  requireRecord(record, "visual artifact descriptor");
  assert.equal(record.kind, expected.kind);
  assert.equal(record.trial_id, expected.trialId);
  assert.equal(record.recreation_index, expected.recreationIndex);
  assert.equal(record.frame_index, expected.frameIndex);
  assert.equal(record.filename, path.posix.basename(record.path));
  assert.equal(record.mime_type, "image/png");
  assert.equal(record.encoding, "png-rgba8-filter-0");
  assert.equal(record.authority, "presentation_only");
  if (record.encode_milliseconds !== undefined) assertFiniteNonnegative(record.encode_milliseconds, "PNG encode milliseconds");
}

function verifyRubricEvidence(rubric, policy, evidence, registry) {
  requireRecord(rubric, "attended rubric evidence");
  assert.deepEqual(Object.keys(rubric).sort(), ["gating", "observation", "review_status", "schema"]);
  assert.equal(rubric.schema, "punctra-browser-interpretation-rubric-v1");
  assert.equal(rubric.gating, false);
  assert.equal(rubric.review_status, "submitted");
  const observation = rubric.observation;
  requireRecord(observation, "attended rubric observation");
  assert.deepEqual(Object.keys(observation).sort(), [
    "answers", "capture_completed_at", "session_label", "submission", "submitted_at",
  ]);
  assertNonemptyString(observation.session_label, "attended rubric session label");
  assert.notEqual(observation.session_label, "not_observed");
  assert.notEqual(observation.session_label, "unavailable");
  assert.equal(observation.capture_completed_at, evidence.capture_completed_at);
  assert.equal(observation.submitted_at, evidence.completed_at);
  const submissionMilliseconds = verifyTrustedControlActivation(
    observation.submission,
    "submit-rubric",
    "click",
  );
  const submittedMilliseconds = Date.parse(observation.submitted_at);
  assert(submissionMilliseconds <= submittedMilliseconds, "rubric trusted submit event follows submission");
  assert(submittedMilliseconds - submissionMilliseconds <= 5_000, "rubric trusted submit event is stale");
  assert.deepEqual(Object.keys(observation.answers).sort(), [...RUBRIC_PROMPTS].sort());

  const results = new Map(evidence.trials.map((trial) => [trial.trial_id, trial]));
  const presentationOrders = [];
  const loadOrders = [];
  const selectionOrders = [];
  for (const prompt of RUBRIC_PROMPTS) {
    const answer = observation.answers[prompt];
    requireRecord(answer, `attended rubric answer ${prompt}`);
    assert.deepEqual(Object.keys(answer).sort(), [
      "artifact_identities",
      "artifact_paths",
      "note",
      "outcome",
      "presentation",
      "selected_at",
      "selection_activation",
      "selection_order",
      "shown",
      "trial_ids",
    ]);
    assert(policy.outcomes.includes(answer.outcome), `rubric outcome ${prompt} is invalid`);
    assert.equal(typeof answer.note, "string");
    assert(answer.note.length <= policy.note_character_limit, `rubric note ${prompt} is too long`);
    assert.deepEqual(answer.trial_ids, policy.trial_bindings[prompt]);

    const expectedArtifacts = answer.trial_ids.map((trialId) => {
      const result = results.get(trialId);
      assert(result, `rubric ${prompt} binds missing trial ${trialId}`);
      const artifact = result.recreations?.[0]?.capture?.artifact;
      requireRecord(artifact, `rubric ${prompt} trial ${trialId} final artifact`);
      assert.equal(artifact.kind, "recreation_png");
      assert.equal(artifact.trial_id, trialId);
      assert.equal(artifact.recreation_index, 0);
      assert.equal(artifact.frame_index, 29);
      const registered = registry.get(artifact.path);
      assert(registered, `rubric ${prompt} artifact ${artifact.path} is not registered`);
      assert.deepEqual(registered.record, artifact);
      return rubricArtifactIdentity(artifact);
    });
    assert.deepEqual(answer.artifact_paths, expectedArtifacts.map(({ path: artifactPath }) => artifactPath));
    assert.deepEqual(answer.artifact_identities, expectedArtifacts);

    const presentation = answer.presentation;
    requireRecord(presentation, `rubric ${prompt} presentation`);
    assert.deepEqual(Object.keys(presentation).sort(), [
      "artifacts", "document_visibility_state", "presentation_order", "presented_at", "schema",
    ]);
    assert.equal(presentation.schema, "punctra-browser-visual-rubric-presentation-v1");
    assert.equal(presentation.document_visibility_state, "visible");
    assertIsoTimestamp(presentation.presented_at, `rubric ${prompt} presentation`);
    assertPositiveInteger(presentation.presentation_order, `rubric ${prompt} presentation order`);
    presentationOrders.push(presentation.presentation_order);
    assert.equal(presentation.artifacts.length, expectedArtifacts.length);
    for (let index = 0; index < expectedArtifacts.length; index += 1) {
      const loaded = presentation.artifacts[index];
      const expected = expectedArtifacts[index];
      requireRecord(loaded, `rubric ${prompt} loaded artifact ${index}`);
      assert.deepEqual(Object.keys(loaded).sort(), [
        "complete", "load_order", "loaded_at", "natural_height", "natural_width", "path", "trial_id",
      ]);
      assert.equal(loaded.trial_id, expected.trial_id);
      assert.equal(loaded.path, expected.path);
      assert.equal(loaded.natural_width, expected.width);
      assert.equal(loaded.natural_height, expected.height);
      assert.equal(loaded.complete, true);
      assertIsoTimestamp(loaded.loaded_at, `rubric ${prompt} artifact ${index} load`);
      assertPositiveInteger(loaded.load_order, `rubric ${prompt} artifact ${index} load order`);
      loadOrders.push(loaded.load_order);
      assertTimestampNotBefore(loaded.loaded_at, observation.capture_completed_at, `rubric ${prompt} artifact loaded before capture completion`);
      assertTimestampNotBefore(presentation.presented_at, loaded.loaded_at, `rubric ${prompt} presentation predates artifact load`);
    }
    assert.equal(answer.shown, true, `rubric ${prompt} was not shown post-capture`);
    assertIsoTimestamp(answer.selected_at, `rubric ${prompt} selection`);
    assertPositiveInteger(answer.selection_order, `rubric ${prompt} selection order`);
    selectionOrders.push(answer.selection_order);
    const selectionMilliseconds = verifyTrustedControlActivation(
      answer.selection_activation,
      `rubric-${prompt}`,
      "change",
    );
    assert.equal(answer.selection_activation.recorded_at, answer.selected_at);
    assert(selectionMilliseconds <= submissionMilliseconds, `rubric ${prompt} selection follows the trusted submit event`);
    assertTimestampNotBefore(answer.selected_at, presentation.presented_at, `rubric ${prompt} selection predates presentation`);
    assertTimestampNotBefore(observation.submitted_at, answer.selected_at, `rubric ${prompt} selection follows submission`);
  }
  assertConsecutiveOrders(presentationOrders, RUBRIC_PROMPTS.length, "rubric presentation");
  assertUniquePositiveOrders(selectionOrders, RUBRIC_PROMPTS.length, "rubric selection");
  assertConsecutiveOrders(loadOrders, loadOrders.length, "rubric artifact load");
}

function rubricArtifactIdentity(artifact) {
  return Object.fromEntries([
    "kind",
    "trial_id",
    "recreation_index",
    "frame_index",
    "path",
    "width",
    "height",
    "encoded_byte_length",
    "encoded_sha256",
    "decoded_byte_length",
    "decoded_sha256",
  ].map((name) => [name, artifact[name]]));
}

function assertIsoTimestamp(value, label) {
  assert(typeof value === "string" && Number.isFinite(Date.parse(value)), `${label} timestamp is invalid`);
}

function assertPositiveInteger(value, label) {
  assert(Number.isSafeInteger(value) && value >= 1, `${label} is invalid`);
}

function assertTimestampNotBefore(value, lowerBound, message) {
  assert(Date.parse(value) >= Date.parse(lowerBound), message);
}

function assertConsecutiveOrders(values, expectedLength, label) {
  assert.equal(values.length, expectedLength, `${label} order count differs`);
  assert.deepEqual([...values].sort((left, right) => left - right),
    Array.from({ length: expectedLength }, (_, index) => index + 1),
    `${label} orders are not an exact unique sequence`);
}

function assertUniquePositiveOrders(values, expectedLength, label) {
  assert.equal(values.length, expectedLength, `${label} order count differs`);
  assert(values.every((value) => Number.isSafeInteger(value) && value >= 1), `${label} order is invalid`);
  assert.equal(new Set(values).size, expectedLength, `${label} orders are duplicated`);
}

function verifyArtifactResourceEvidence(resources, registry, limits) {
  requireRecord(resources, "visual artifact resource evidence");
  const records = [...registry.values()].map(({ record }) => record);
  const totalEncodedBytes = records.reduce((total, record) => total + record.encoded_byte_length, 0);
  assert.deepEqual(resources, {
    schema: "punctra-browser-visual-artifact-resources-v1",
    artifact_count: records.length,
    total_encoded_artifact_bytes: totalEncodedBytes,
    total_encoded_artifact_bytes_ceiling: limits.total_encoded_artifact_bytes,
    passed: totalEncodedBytes <= limits.total_encoded_artifact_bytes,
  });
  assert.equal(resources.passed, true, "encoded evidence artifacts exceeded the corpus ceiling");
}

function verifyEvidenceSummary(summary, trialResults, registry) {
  requireRecord(summary, "visual evidence summary");
  const failed = trialResults.filter(({ passed }) => !passed).map(({ trial_id: trialId }) => trialId);
  const passed = trialResults.length - failed.length;
  const records = [...registry.values()].map(({ record }) => record);
  const totalEncodedBytes = records.reduce((total, record) => total + record.encoded_byte_length, 0);
  assert.equal(summary.trial_count, trialResults.length);
  assert.equal(summary.completed_trials, trialResults.length);
  assert.equal(summary.passed_trials, passed);
  assert.deepEqual(summary.failed_trials, failed);
  assert.equal(summary.recreations_per_trial, 3);
  assert.equal(summary.non_gating_rubric_complete, true);
  assert.equal(summary.artifact_count, records.length);
  assert.equal(summary.total_encoded_artifact_bytes, totalEncodedBytes);
  assert.deepEqual(summary.failures, []);
  assert.equal(summary.passed, failed.length === 0);
}

async function verifyDigestRecord(record, context, keys = ["path", "byte_length", "sha256"], lengthKey = "byte_length", digestKey = "sha256") {
  requireRecord(record, "digest record");
  assert.deepEqual(Object.keys(record).sort(), [...keys].sort(), `digest record ${record.path} has unexpected fields`);
  validateRepositoryPath(record.path);
  assert(Number.isSafeInteger(record[lengthKey]) && record[lengthKey] >= 0, `${record.path} byte length is invalid`);
  assert.match(record[digestKey], /^[0-9a-f]{64}$/, `${record.path} SHA-256 is invalid`);
  const bytes = await context.readRepositoryFile(record.path);
  assert.equal(bytes.byteLength, record[lengthKey], `${record.path} byte length drifted`);
  assert.equal(sha256(bytes), record[digestKey], `${record.path} SHA-256 drifted`);
  return bytes;
}

function createVerificationContext(options) {
  return {
    expectedImplementationCommit: options.expectedImplementationCommit,
    runFixtureGenerator: options.runFixtureGenerator ?? true,
    readRepositoryFile: options.readRepositoryFile ?? readRepositoryFile,
    readPinnedFile: options.readPinnedFile ?? readPinnedFile,
    requireCommit: options.requireCommit ?? requireCommit,
    runCommand: options.runCommand ?? runCommand,
    decodedImageCache: options.decodedImageCache ?? new Map(),
    imageDigestByObject: options.imageDigestByObject ?? new WeakMap(),
    comparisonCache: options.comparisonCache ?? new Map(),
  };
}

function normalizeVerifiedBaseline(value) {
  if (value?.baseline && value?.corpus) return value;
  throw new TypeError("visual evidence verification requires a verified baseline result");
}

function requiredQualifiedPaths() {
  return [
    "Cargo.lock",
    "Cargo.toml",
    "apps/browser-demo/Cargo.toml",
    "apps/browser-demo/src/bin/generate_stream_fixture.rs",
    "apps/browser-demo/src/bin/generate_visual_source_fixture.rs",
    "apps/browser-demo/src/bin/scene_facts.rs",
    "apps/browser-demo/src/browser.rs",
    "apps/browser-demo/src/capture.rs",
    "apps/browser-demo/src/diagnostics.rs",
    "apps/browser-demo/src/display.rs",
    "apps/browser-demo/src/host.rs",
    "apps/browser-demo/src/lib.rs",
    "apps/browser-demo/src/scene.rs",
    "apps/browser-demo/src/streaming.rs",
    "apps/browser-demo/web/camera-policy.js",
    "apps/browser-demo/web/exact-query-error.js",
    "apps/browser-demo/web/exact-query.d.ts",
    "apps/browser-demo/web/exact-query.js",
    "apps/browser-demo/web/exact-query.test.mjs",
    "apps/browser-demo/web/las-exact-decoder.js",
    "apps/browser-demo/web/module-loader.js",
    "apps/browser-demo/web/module-loader.test.mjs",
    "apps/browser-demo/web/package.json",
    "apps/browser-demo/web/range-response.js",
    "apps/browser-demo/web/range-response.test.mjs",
    "apps/browser-demo/web/range-server.test.mjs",
    "apps/browser-demo/web/sdk.d.ts",
    "apps/browser-demo/web/sdk.js",
    "apps/browser-demo/web/sdk.test.mjs",
    "apps/browser-demo/web/stream-ordinals.js",
    "apps/browser-demo/web/stream-ordinals.test.mjs",
    "apps/browser-demo/web/stream-worker.js",
    "apps/browser-demo/web/streaming-protocol.js",
    "apps/browser-demo/web/streaming-protocol.test.mjs",
    "apps/browser-demo/web/viewer-api.d.ts",
    "apps/browser-demo/web/viewer-api.js",
    "apps/browser-demo/web/viewer-api.test.mjs",
    "apps/browser-demo/web/viewer-input.d.ts",
    "apps/browser-demo/web/viewer-input.js",
    "apps/browser-demo/web/viewer-input.test.mjs",
    "apps/browser-demo/web/wasm-loader.js",
    "apps/browser-demo/web/wasm-loader.test.mjs",
    "apps/browser-demo/web/worker-operation.js",
    "apps/browser-demo/web/worker-operation.test.mjs",
    "apps/browser-demo/web/worker-protocol.js",
    "apps/browser-demo/web/worker-protocol.test.mjs",
    "apps/browser-demo/web/fixtures/visual-v1/autzen-classified-sample.json",
    "apps/browser-demo/web/fixtures/visual-v1/autzen-classified-sample.pvis",
    "apps/browser-demo/web/fixtures/visual-v1/baseline-inputs.json",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/generated-neutral-mixed-lod-perspective.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/generated-elevation-layered-orthographic.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/generated-rgb-hdr-perspective.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/generated-intensity-sparse-orthographic.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/generated-classification-selection-perspective.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/autzen-rgb-perspective.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/autzen-classification-perspective.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/autzen-intensity-perspective.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/autzen-elevation-perspective.png",
    "apps/browser-demo/web/fixtures/visual-v1/corpus.json",
    "apps/browser-demo/web/qualification-lane.js",
    "apps/browser-demo/web/visual-capture.js",
    "apps/browser-demo/web/visual-capture.test.mjs",
    "apps/browser-demo/web/visual-comparison.js",
    "apps/browser-demo/web/visual-comparison.test.mjs",
    "apps/browser-demo/web/visual-corpus.js",
    "apps/browser-demo/web/visual-corpus.test.mjs",
    "apps/browser-demo/web/visual-export.js",
    "apps/browser-demo/web/visual-export.test.mjs",
    "apps/browser-demo/web/visual-archive.js",
    "apps/browser-demo/web/visual-archive.test.mjs",
    "apps/browser-demo/web/visual-baseline-inputs.js",
    "apps/browser-demo/web/visual-baseline-inputs.test.mjs",
    "apps/browser-demo/web/visual.css",
    "apps/browser-demo/web/visual-main.js",
    "apps/browser-demo/web/visual-png.js",
    "apps/browser-demo/web/visual-png.test.mjs",
    "apps/browser-demo/web/visual-provenance.js",
    "apps/browser-demo/web/visual-provenance.test.mjs",
    "apps/browser-demo/web/visual-rubric.js",
    "apps/browser-demo/web/visual-rubric.test.mjs",
    "apps/browser-demo/web/visual-run-session.js",
    "apps/browser-demo/web/visual-run-session.test.mjs",
    "apps/browser-demo/web/visual-selection.js",
    "apps/browser-demo/web/visual-selection.test.mjs",
    "apps/browser-demo/web/visual-validation.js",
    "apps/browser-demo/web/visual-validation.test.mjs",
    "apps/browser-demo/web/visual.html",
    "crates/foundation-runtime/Cargo.toml",
    "crates/foundation-runtime/src/lib.rs",
    "crates/point-contracts/Cargo.toml",
    "crates/point-contracts/src/lib.rs",
    "crates/point-index/Cargo.toml",
    "crates/point-index/src/error.rs",
    "crates/point-index/src/lib.rs",
    "crates/point-index/src/limits.rs",
    "crates/point-index/src/model.rs",
    "crates/point-index/src/persistence.rs",
    "crates/point-index/src/prepare.rs",
    "crates/point-index/src/read.rs",
    "crates/point-index/src/tree.rs",
    "crates/point-source/Cargo.toml",
    "crates/point-source/src/adapter.rs",
    "crates/point-source/src/error.rs",
    "crates/point-source/src/lib.rs",
    "crates/point-source/src/stream.rs",
    "crates/point-view/Cargo.toml",
    "crates/point-view/src/lib.rs",
    "crates/point-view/src/planning.rs",
    "crates/render-protocol/Cargo.toml",
    "crates/render-protocol/src/camera.rs",
    "crates/render-protocol/src/lib.rs",
    "crates/render-protocol/src/viewport.rs",
    "crates/render-wgpu/Cargo.toml",
    "crates/render-wgpu/src/eye_dome.wgsl",
    "crates/render-wgpu/src/frame.rs",
    "crates/render-wgpu/src/gpu.rs",
    "crates/render-wgpu/src/lib.rs",
    "crates/render-wgpu/src/pick.rs",
    "crates/render-wgpu/src/pipeline.rs",
    "crates/render-wgpu/src/point.wgsl",
    "crates/render-wgpu/src/renderer.rs",
    "crates/render-wgpu/src/targets.rs",
    "crates/source-las/Cargo.toml",
    "crates/source-las/src/decode.rs",
    "crates/source-las/src/format.rs",
    "crates/source-las/src/lib.rs",
    "docs/releases/v0.21-browser-visual-rubric-template.json",
    "examples/data/autzen-classified.laz",
    "packages/react/hook.js",
    "packages/react/index.d.ts",
    "packages/react/index.js",
    "packages/react/lifecycle.js",
    "packages/react/lifecycle.test.mjs",
    "packages/react/package.json",
    "rust-toolchain.toml",
    "scripts/build-browser-demo.sh",
    "scripts/build-browser-sdk.sh",
    "scripts/generate-browser-sdk-reference.mjs",
    "scripts/serve-browser-demo.py",
    VISUAL_VERIFIER_PATH,
  ];
}

async function readRepositoryFile(relativePath, encoding) {
  const bytes = await readFile(resolveRepositoryPath(relativePath));
  return encoding === "utf8" ? bytes.toString("utf8") : bytes;
}

export function readPinnedFile(commit, relativePath, options = {}) {
  verifyFullCommit(commit, "pinned file commit");
  validateRepositoryPath(relativePath);
  const spawn = options.spawnSync ?? spawnSync;
  const workingDirectory = options.repositoryRoot ?? repositoryRoot;
  const objectName = `${commit}:${relativePath}`;
  const sizeResult = spawn("git", ["cat-file", "-s", objectName], {
    cwd: workingDirectory,
    encoding: "utf8",
    maxBuffer: MAX_PINNED_SIZE_OUTPUT_BYTES,
    stdio: ["ignore", "pipe", "pipe"],
  });
  assertPinnedGitSucceeded(sizeResult, `cannot size ${relativePath} at ${commit}`);
  assert.equal(typeof sizeResult.stdout, "string", `pinned size output for ${relativePath} is not text`);
  assert.match(sizeResult.stdout, /^[1-9][0-9]*\n$/, `pinned size output for ${relativePath} is not a canonical positive decimal`);
  const expectedBytes = Number(sizeResult.stdout.slice(0, -1));
  assert(Number.isSafeInteger(expectedBytes), `pinned size for ${relativePath} exceeds the safe integer range`);
  assert(
    expectedBytes <= MAX_PINNED_FILE_BYTES,
    `pinned file ${relativePath} exceeds the ${MAX_PINNED_FILE_BYTES}-byte verification ceiling`,
  );

  const readResult = spawn("git", ["show", objectName], {
    cwd: workingDirectory,
    encoding: null,
    maxBuffer: MAX_PINNED_FILE_BYTES,
    stdio: ["ignore", "pipe", "pipe"],
  });
  assertPinnedGitSucceeded(readResult, `cannot read ${relativePath} at ${commit}`);
  assert(readResult.stdout instanceof Uint8Array, `pinned file ${relativePath} did not return bytes`);
  assert.equal(readResult.stdout.byteLength, expectedBytes, `pinned file ${relativePath} length differs from its preflight size`);
  return readResult.stdout;
}

function assertPinnedGitSucceeded(result, label) {
  requireRecord(result, `${label} result`);
  if (result.error !== undefined) {
    const code = typeof result.error?.code === "string" ? ` (${result.error.code})` : "";
    assert.fail(`${label}: git could not complete${code}: ${result.error?.message ?? String(result.error)}`);
  }
  if (result.status !== 0) {
    const signal = typeof result.signal === "string" ? `; signal ${result.signal}` : "";
    const stderr = result.stderr === undefined || result.stderr === null
      ? ""
      : String(result.stderr).trim();
    assert.fail(`${label}: git exited with status ${String(result.status)}${signal}${stderr.length === 0 ? "" : `: ${stderr}`}`);
  }
}

function requireCommit(commit) {
  const result = spawnSync("git", ["cat-file", "-e", `${commit}^{commit}`], {
    cwd: repositoryRoot,
    encoding: "utf8",
    stdio: ["ignore", "ignore", "pipe"],
  });
  assert.equal(result.status, 0, `implementation commit ${commit} is unavailable: ${result.stderr}`);
}

function runCommand(command, arguments_) {
  const result = spawnSync(command, arguments_, {
    cwd: repositoryRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  assert.equal(result.status, 0, `${command} ${arguments_.join(" ")} failed: ${result.stderr}`);
  return result.stdout;
}

function resolveRepositoryPath(relativePath) {
  validateRepositoryPath(relativePath);
  return path.join(repositoryRoot, relativePath);
}

function validateRepositoryPath(relativePath) {
  assert.equal(typeof relativePath, "string");
  assert(relativePath.length > 0 && !path.isAbsolute(relativePath), "artifact path must be repository-relative");
  const normalized = path.posix.normalize(relativePath);
  assert.equal(normalized, relativePath, `artifact path is not canonical: ${relativePath}`);
  assert(!normalized.startsWith("../") && normalized !== "..", `artifact path escapes repository: ${relativePath}`);
}

function parseJson(bytes, label) {
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new Error(`${label} is not valid JSON`, { cause: error });
  }
}

function verifyFullCommit(value, label) {
  assert.match(value, /^[0-9a-f]{40}$/, `${label} must be a full lowercase Git commit`);
}

function requireRecord(value, label) {
  assert(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
}

function requireArray(value, label) {
  assert(Array.isArray(value), `${label} must be an array`);
}

function assertNonemptyString(value, label) {
  assert(typeof value === "string" && value.length > 0, `${label} must be nonempty`);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function concatenateBytes(parts) {
  const output = new Uint8Array(parts.reduce((total, part) => total + part.byteLength, 0));
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.byteLength;
  }
  return output;
}

function parseCommandLine(arguments_) {
  if (arguments_.length === 0) return { evidencePath: undefined };
  assert.deepEqual(arguments_.slice(0, 1), ["--evidence"]);
  assert.equal(arguments_.length, 2, "usage: verify-browser-visual-baseline.mjs [--evidence PATH]");
  validateRepositoryPath(arguments_[1]);
  return { evidencePath: arguments_[1] };
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const { evidencePath } = parseCommandLine(process.argv.slice(2));
  const baseline = parseJson(await readRepositoryFile(defaultBaselinePath), defaultBaselinePath);
  const verified = await verifyBrowserVisualBaseline(baseline);
  if (evidencePath !== undefined) {
    const evidenceBytes = await readRepositoryFile(evidencePath);
    const evidence = parseJson(evidenceBytes, evidencePath);
    await verifyBrowserVisualEvidence(evidence, verified, { evidenceBytes });
    console.log("browser visual baseline and attended evidence passed");
  } else {
    console.log("browser visual baseline passed");
  }
}
