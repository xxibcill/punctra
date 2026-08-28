import { sha256Hex } from "./visual-png.js";
import { validateRubricEvidenceShape } from "./visual-rubric.js";
import { createVisualValidator } from "./visual-validation.js";

export const VISUAL_CORPUS_SCHEMA = "punctra-browser-visual-corpus-v1";
export const TRANSFER_SCHEMA = "punctra-browser-transfer-v2";
export const TRANSFER_RECORD_BYTES = 32;
export const MAX_TRANSFER_BATCH_POINTS = 1_024;
export const MAX_TRANSFER_BATCHES = 8;
export const VISUAL_TRIAL_COUNT = 9;
export const GENERATED_VISUAL_TRIAL_COUNT = 5;
export const AUTZEN_VISUAL_TRIAL_COUNT = 4;
export const VISUAL_VIEWPORT = Object.freeze({
  css_width: 320,
  css_height: 240,
  requested_device_pixel_ratio: 2,
  physical_width: 640,
  physical_height: 480,
});
const { requireCondition, requireRecord } = createVisualValidator("Visual corpus invalid");
export const REQUIRED_GENERATED_CONDITIONS = Object.freeze([
  "sparse",
  "dense",
  "layered",
  "high_dynamic_range",
  "classification",
  "large_world",
  "mixed_lod",
]);
export const DISPLAY_MODES = Object.freeze([
  "neutral",
  "elevation",
  "rgb",
  "intensity",
  "classification",
]);
export const RAW_VIEWER_INITIAL_DISPLAY_MODE = "rgb";
export const RUBRIC_PROMPTS = Object.freeze([
  "depth",
  "shape",
  "density_transition",
  "color_meaning",
  "selection",
  "false_feature",
]);
export const RUBRIC_OUTCOMES = Object.freeze([
  "clear",
  "ambiguous",
  "false_feature",
  "not_visible",
  "not_observed",
]);

const GENERATED_CONDITION_FACT_PATHS = Object.freeze({
  sparse: Object.freeze(["condition_facts.sparse_batch"]),
  dense: Object.freeze(["condition_facts.dense_batches"]),
  layered: Object.freeze(["condition_facts.layer_pairs"]),
  high_dynamic_range: Object.freeze([
    "condition_facts.attribute_extrema.intensity",
    "condition_facts.attribute_extrema.rgb_channel",
  ]),
  classification: Object.freeze(["condition_facts.attribute_extrema.classifications"]),
  large_world: Object.freeze([
    "condition_facts.large_world_origin",
    "condition_facts.minimum_dense_axis_spacing",
  ]),
  mixed_lod: Object.freeze([
    "condition_facts.lod_relation",
    "condition_facts.stable_lod_cut",
  ]),
});
const DERIVED_MODE_FACT_PATHS = Object.freeze({
  classification: Object.freeze([
    "condition_facts.classifications",
    "condition_facts.xy_grid",
    "condition_facts.maximum_absolute_world_origin",
  ]),
  elevation: Object.freeze([
    "condition_facts.relative_bounds",
    "condition_facts.overlapping_depth",
    "condition_facts.xy_grid",
    "condition_facts.maximum_absolute_world_origin",
  ]),
  intensity: Object.freeze([
    "condition_facts.intensity",
    "condition_facts.xy_grid",
    "condition_facts.maximum_absolute_world_origin",
  ]),
  rgb: Object.freeze([
    "condition_facts.rgb_channel",
    "condition_facts.overlapping_depth",
    "condition_facts.xy_grid",
    "condition_facts.maximum_absolute_world_origin",
  ]),
});

const GENERATED_SCENE_ID = "generated-visual-composite-v1";
const GENERATED_SOURCE_IDENTITY = "21".repeat(32);
const GENERATED_WORLD_ORIGIN = Object.freeze([6_378_137.25, 4_782_951.5, 1_234.75]);
const GENERATED_SOURCE_Z_RANGE = Object.freeze([
  GENERATED_WORLD_ORIGIN[2] - 4,
  GENERATED_WORLD_ORIGIN[2] + 8,
]);
const CLASSIFICATIONS = Object.freeze([1, 2, 5, 6, 9, 17, 31]);
const RGB_LEVELS = Object.freeze([
  Object.freeze([0, 0, 0]),
  Object.freeze([65_535, 0, 1_024]),
  Object.freeze([0, 65_535, 32_768]),
  Object.freeze([4_096, 512, 65_535]),
  Object.freeze([65_535, 65_535, 65_535]),
]);
const INTENSITY_LEVELS = Object.freeze([0, 512, 16_384, 32_768, 64_000, 65_535]);

/** Loads and validates the closed visual-corpus manifest. */
export async function loadVisualCorpus(url, options = {}) {
  const fetchImplementation = options.fetchImplementation ?? globalThis.fetch;
  requireCondition(typeof fetchImplementation === "function", "Fetch is unavailable");
  const resolvedUrl = new URL(url, globalThis.location?.href ?? "http://localhost/").href;
  const response = await fetchImplementation(resolvedUrl, {
    cache: "no-store",
    credentials: "same-origin",
  });
  requireCondition(response?.ok, `visual corpus returned HTTP ${response?.status ?? "unknown"}`);
  const corpusBytes = new Uint8Array(await response.arrayBuffer());
  let corpus;
  try {
    corpus = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(corpusBytes));
  } catch (error) {
    throw new Error(`Visual corpus invalid: corpus JSON could not be decoded: ${error?.message ?? error}`);
  }
  return {
    corpus: validateVisualCorpus(corpus),
    corpus_url: resolvedUrl,
    corpus_byte_length: corpusBytes.byteLength,
    corpus_sha256: await sha256Hex(corpusBytes),
  };
}

/**
 * Validates the fixed corpus as one coherent product rather than accepting
 * caller-selected scene or camera fragments.
 */
export function validateVisualCorpus(value) {
  requireRecord(value, "visual corpus");
  requireCondition(value.schema === VISUAL_CORPUS_SCHEMA, "visual corpus schema differs");
  requireCondition(value.release === "0.21.0-alpha.1", "visual corpus release differs");
  validateViewport(value.viewport);
  validateCapturePolicy(value.capture);
  validatePresentationPolicy(value.presentation_policy);
  validateRequiredCapabilities(value.required_capabilities);
  validateResourceLimits(value.resource_limits);
  requireCondition(value.transport.maximum_evidence_json_bytes === value.resource_limits.evidence_json_bytes, "visual transport evidence ceiling differs from resources");
  requireCondition(value.transport.maximum_baseline_inputs_json_bytes === value.resource_limits.baseline_inputs_json_bytes, "visual transport baseline-input ceiling differs from resources");
  requireCondition(
    value.transport.maximum_archive_bytes
      === value.resource_limits.total_encoded_artifact_bytes + value.transport.maximum_archive_overhead_bytes,
    "visual transport archive ceiling accounting differs",
  );
  requireRecord(value.settling, "settling policy");
  requireInteger(value.settling.quiet_frames, "quiet frame count", 30, 30);
  requireInteger(value.settling.capture_poll_frame_ceiling, "capture poll frame ceiling", 1, 600);
  requireInteger(value.settling.transition_frame_count, "transition frame count", 2, 32);
  validateTimingLimits(value.timing_limits);
  validateTransportPolicy(value.transport);

  requireRecord(value.tolerance_profiles, "tolerance profiles");
  requireCondition(Object.keys(value.tolerance_profiles).length > 0, "tolerance profiles are empty");

  requireRecord(value.rubric, "rubric");
  requireCondition(value.rubric.schema === "punctra-browser-interpretation-rubric-v1", "rubric schema differs");
  requireCondition(arrayEquals(value.rubric.prompts, RUBRIC_PROMPTS), "rubric prompts differ");
  requireCondition(arrayEquals(value.rubric.outcomes, RUBRIC_OUTCOMES), "rubric outcomes differ");
  requireInteger(value.rubric.note_character_limit, "rubric note limit", 1, 280);
  requireRecord(value.rubric.trial_bindings, "rubric trial bindings");
  requireCondition(arrayEquals(Object.keys(value.rubric.trial_bindings), RUBRIC_PROMPTS), "rubric trial-binding prompts differ");
  for (const prompt of RUBRIC_PROMPTS) {
    const binding = value.rubric.trial_bindings[prompt];
    requireCondition(Array.isArray(binding) && binding.length > 0 && new Set(binding).size === binding.length, `rubric ${prompt} trial bindings differ`);
  }

  requireCondition(Array.isArray(value.sources) && value.sources.length >= 2, "visual sources are incomplete");
  const sources = new Map();
  for (const source of value.sources) {
    validateSource(source);
    requireCondition(!sources.has(source.id), `duplicate visual source ${source.id}`);
    sources.set(source.id, source);
  }
  requireCondition(
    [...sources.values()].some((source) => source.kind === "derived_pvis"),
    "the licensed derived Source is missing",
  );

  requireCondition(
    Array.isArray(value.trials) && value.trials.length === VISUAL_TRIAL_COUNT,
    `visual corpus requires exactly ${VISUAL_TRIAL_COUNT} trials`,
  );
  const trialIds = new Set();
  const modes = new Set();
  const projections = new Set();
  const generatedConditions = new Set();
  let hasSelection = false;
  let hasEmptySelection = false;
  let hasMixedLod = false;
  let generatedTrialCount = 0;
  let autzenTrialCount = 0;
  for (const trial of value.trials) {
    validateTrial(trial, sources, value.tolerance_profiles);
    requireCondition(!trialIds.has(trial.id), `duplicate visual trial ${trial.id}`);
    trialIds.add(trial.id);
    modes.add(trial.display_mode);
    projections.add(trial.camera === "source" ? "perspective" : trial.camera.projection);
    const source = sources.get(trial.source_id);
    if (source.kind === "generated") {
      generatedTrialCount += 1;
      for (const condition of trial.conditions) generatedConditions.add(condition);
    } else {
      autzenTrialCount += 1;
    }
    hasSelection ||= trial.selection.ordinals.length > 0;
    hasEmptySelection ||= trial.selection.ordinals.length === 0;
    hasMixedLod ||= trial.temporal_trace.kind === "mixed_lod_parent_child";
  }
  requireCondition(arrayEquals([...modes].sort(), [...DISPLAY_MODES].sort()), "all five display modes are required");
  requireCondition(projections.has("perspective") && projections.has("orthographic"), "both camera projections are required");
  requireCondition(hasSelection && hasEmptySelection, "selected and unselected trials are required");
  requireCondition(hasMixedLod, "a parent/child mixed-LOD trace is required");
  requireCondition(generatedTrialCount === GENERATED_VISUAL_TRIAL_COUNT, `visual corpus requires exactly ${GENERATED_VISUAL_TRIAL_COUNT} generated trials`);
  requireCondition(autzenTrialCount === AUTZEN_VISUAL_TRIAL_COUNT, `visual corpus requires exactly ${AUTZEN_VISUAL_TRIAL_COUNT} Autzen-derived trials`);
  for (const [prompt, binding] of Object.entries(value.rubric.trial_bindings)) {
    requireCondition(binding.every((trialId) => trialIds.has(trialId)), `rubric ${prompt} binds an unknown trial`);
  }
  for (const condition of REQUIRED_GENERATED_CONDITIONS) {
    requireCondition(generatedConditions.has(condition), `generated trial coverage is missing ${condition}`);
  }
  validateConditionCoverage(value.condition_coverage, sources, value.trials);
  return value;
}

/** Materializes one fixed trial, including exact transfer-v2 batches. */
export async function materializeVisualTrial(corpus, trialId, options = {}) {
  validateVisualCorpus(corpus);
  const trial = corpus.trials.find((entry) => entry.id === trialId);
  requireCondition(trial !== undefined, `unknown visual trial ${JSON.stringify(trialId)}`);
  const source = corpus.sources.find((entry) => entry.id === trial.source_id);
  const materialized = source.kind === "generated"
    ? await materializeGeneratedSource(source, options)
    : await materializeDerivedSource(source, options);
  const camera = trial.camera === "source" ? materialized.source_camera : trial.camera;
  requireCondition(camera !== undefined, `visual trial ${trial.id} has no camera`);
  if (source.kind === "derived_pvis") {
    validateMaterializedConditionFacts(corpus.condition_coverage, trial, materialized.input_facts);
  }
  return {
    trial,
    source,
    camera,
    source_identity: materialized.source_identity,
    world_origin: materialized.world_origin,
    source_z_range: materialized.source_z_range,
    point_count: materialized.point_count,
    batches: materialized.batches,
    input_facts: materialized.input_facts,
  };
}

/** Returns the deterministic generated scene and its derived condition facts. */
export function generateVisualScene(generator = GENERATED_SCENE_ID) {
  requireCondition(generator === GENERATED_SCENE_ID, `unsupported visual generator ${JSON.stringify(generator)}`);
  let ordinal = 0;
  const batches = [];

  const parent = [];
  for (let row = 0; row < 15; row += 1) {
    for (let column = 0; column < 15; column += 1) {
      parent.push(authoredPoint(
        ordinal++,
        -18 + (16 * column) / 14,
        -12 + (24 * row) / 14,
        -1 + ((column % 3) - 1) * 0.18 + ((row % 4) - 1.5) * 0.11,
      ));
    }
  }
  batches.push(sceneBatch(0, "lod_parent", parent));

  const child = [];
  for (let row = 0; row < 29; row += 1) {
    for (let column = 0; column < 29; column += 1) {
      child.push(authoredPoint(
        ordinal++,
        -18 + (16 * column) / 28,
        -12 + (24 * row) / 28,
        -1 + ((column % 5) - 2) * 0.07 + ((row % 5) - 2) * 0.05,
      ));
    }
  }
  batches.push(sceneBatch(1, "lod_child", child));

  const layered = [];
  for (let layer = 0; layer < 2; layer += 1) {
    for (let row = 0; row < 20; row += 1) {
      for (let column = 0; column < 20; column += 1) {
        layered.push(authoredPoint(
          ordinal++,
          2 + (16 * column) / 19,
          -12 + (24 * row) / 19,
          (layer === 0 ? -2.25 : 3.25) + ((column + row) % 4) * 0.08,
        ));
      }
    }
  }
  batches.push(sceneBatch(2, "depth_layers", layered));

  const sparse = [];
  for (let index = 0; index < 96; index += 1) {
    sparse.push(authoredPoint(
      ordinal++,
      -20 + (40 * index) / 95,
      14,
      5.4 + ((index % 7) - 3) * 0.12,
    ));
  }
  for (let index = 0; index < 48; index += 1) {
    sparse.push(authoredPoint(
      ordinal++,
      0,
      -14 + (28 * index) / 47,
      6.2 + ((index % 5) - 2) * 0.1,
    ));
  }
  for (let index = 0; index < 48; index += 1) {
    sparse.push(authoredPoint(
      ordinal++,
      -19.25 + (index % 12) * 3.5,
      -13 + Math.floor(index / 12) * 8,
      -3.4 + (index % 6) * 1.35,
    ));
  }
  batches.push(sceneBatch(3, "sparse_features", sparse));

  const adjacentCoarse = [];
  for (let row = 0; row < 9; row += 1) {
    for (let column = 0; column < 5; column += 1) {
      adjacentCoarse.push(authoredPoint(
        ordinal++,
        -1.5 + (3 * column) / 4,
        -12 + (24 * row) / 8,
        -1 + ((column % 3) - 1) * 0.16 + ((row % 3) - 1) * 0.1,
      ));
    }
  }
  batches.push(sceneBatch(4, "lod_adjacent_coarse", adjacentCoarse));

  const scene = {
    generator: GENERATED_SCENE_ID,
    transfer_schema: TRANSFER_SCHEMA,
    source_identity: GENERATED_SOURCE_IDENTITY,
    world_origin: [...GENERATED_WORLD_ORIGIN],
    source_z_range: [...GENERATED_SOURCE_Z_RANGE],
    batches,
    lod_relations: [{ parent_batch_index: 0, child_batch_index: 1 }],
    stable_lod_relations: [{ dense_batch_index: 1, coarse_batch_index: 4 }],
    selection_ordinals: [batches[3].points[0].ordinal, batches[3].points[47].ordinal],
  };
  scene.point_count = batches.reduce((total, batch) => total + batch.points.length, 0);
  scene.conditions = deriveGeneratedConditions(scene);
  return scene;
}

/** Encodes authored points into the private transfer-v2 ABI. */
export function encodeTransferV2(points) {
  requireCondition(Array.isArray(points), "transfer points must be an array");
  requireInteger(points.length, "transfer Point count", 1, MAX_TRANSFER_BATCH_POINTS);
  const bytes = new Uint8Array(points.length * TRANSFER_RECORD_BYTES);
  const view = new DataView(bytes.buffer);
  let previousOrdinal = -1;
  points.forEach((point, index) => {
    validateAuthoredPoint(point, previousOrdinal);
    previousOrdinal = point.ordinal;
    const offset = index * TRANSFER_RECORD_BYTES;
    view.setBigUint64(offset, BigInt(point.ordinal), true);
    point.relative_position.forEach((value, axis) => view.setFloat32(offset + 8 + axis * 4, value, true));
    view.setUint16(offset + 20, point.intensity, true);
    view.setUint8(offset + 22, point.classification);
    view.setUint8(offset + 23, 0);
    point.rgb.forEach((value, channel) => view.setUint16(offset + 24 + channel * 2, value, true));
    view.setUint16(offset + 30, 0, true);
  });
  return bytes;
}

/** Decodes and validates transfer-v2 bytes for fixture and test inspection. */
export function decodeTransferV2(input, previousOrdinal = -1) {
  const bytes = asUint8Array(input, "transfer payload");
  requireCondition(bytes.byteLength > 0 && bytes.byteLength % TRANSFER_RECORD_BYTES === 0, "transfer payload width differs");
  requireCondition(bytes.byteLength / TRANSFER_RECORD_BYTES <= MAX_TRANSFER_BATCH_POINTS, "transfer batch exceeds Point ceiling");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const points = [];
  let last = previousOrdinal;
  for (let offset = 0; offset < bytes.byteLength; offset += TRANSFER_RECORD_BYTES) {
    const ordinalBig = view.getBigUint64(offset, true);
    requireCondition(ordinalBig <= BigInt(Number.MAX_SAFE_INTEGER), "transfer ordinal exceeds JavaScript exact range");
    const ordinal = Number(ordinalBig);
    requireCondition(ordinal > last, "transfer ordinals are not globally increasing");
    requireCondition(view.getUint8(offset + 23) === 0 && view.getUint16(offset + 30, true) === 0, "transfer reserved bytes are nonzero");
    const relativePosition = [0, 1, 2].map((axis) => view.getFloat32(offset + 8 + axis * 4, true));
    requireCondition(relativePosition.every(Number.isFinite), "transfer position is not finite");
    points.push({
      ordinal,
      relative_position: relativePosition,
      intensity: view.getUint16(offset + 20, true),
      classification: view.getUint8(offset + 22),
      rgb: [0, 1, 2].map((channel) => view.getUint16(offset + 24 + channel * 2, true)),
    });
    last = ordinal;
  }
  return points;
}

/** Projects one authored generated Point through the exact fixed trial camera. */
export function projectAuthoredPoint(point, worldOrigin, camera, viewport = VISUAL_VIEWPORT) {
  validateViewport(viewport);
  return projectAuthoredPointInViewport(point, worldOrigin, camera, viewport);
}

/** Projects one authored Point into a bounded noncanonical physical viewport. */
export function projectAuthoredPointAtViewport(point, worldOrigin, camera, viewport) {
  validateProjectionViewport(viewport);
  return projectAuthoredPointInViewport(point, worldOrigin, camera, viewport);
}

function projectAuthoredPointInViewport(point, worldOrigin, camera, viewport) {
  validateAuthoredPoint(point, -1);
  requireFiniteTriple(worldOrigin, "projection world origin");
  validateCamera(camera, "projection camera");
  const world = point.relative_position.map((value, axis) => value + worldOrigin[axis]);
  const forward = normalize(subtract(camera.target, camera.eye));
  const right = normalize(cross(forward, camera.up));
  const correctedUp = cross(right, forward);
  const relative = subtract(world, camera.eye);
  const cameraX = dot(relative, right);
  const cameraY = dot(relative, correctedUp);
  const cameraDepth = dot(relative, forward);
  requireCondition(cameraDepth >= camera.near_distance && cameraDepth <= camera.far_distance, "authored Point is outside the camera depth interval");
  const aspect = viewport.physical_width / viewport.physical_height;
  let normalizedX;
  let normalizedY;
  if (camera.projection === "perspective") {
    const halfVertical = Math.tan(camera.vertical_field_of_view_radians / 2);
    normalizedX = cameraX / (cameraDepth * halfVertical * aspect);
    normalizedY = cameraY / (cameraDepth * halfVertical);
  } else {
    normalizedX = cameraX / (camera.vertical_world_height * aspect / 2);
    normalizedY = cameraY / (camera.vertical_world_height / 2);
  }
  const exactX = (normalizedX + 1) * viewport.physical_width / 2;
  const exactY = (1 - normalizedY) * viewport.physical_height / 2;
  return {
    x: Math.round(exactX),
    y: Math.round(exactY),
    exact_x: exactX,
    exact_y: exactY,
    camera_depth: cameraDepth,
  };
}

function validateProjectionViewport(viewport) {
  requireRecord(viewport, "projection viewport");
  for (const field of ["css_width", "css_height", "requested_device_pixel_ratio", "physical_width", "physical_height"]) {
    requireCondition(Number.isFinite(viewport[field]) && viewport[field] > 0, `projection viewport ${field} is invalid`);
  }
  requireCondition(Number.isSafeInteger(viewport.physical_width) && Number.isSafeInteger(viewport.physical_height), "projection viewport physical dimensions must be integers");
  requireCondition(viewport.physical_width <= 4_096 && viewport.physical_height <= 4_096, "projection viewport axis exceeds its ceiling");
  requireCondition(viewport.physical_width * viewport.physical_height <= 8_388_608, "projection viewport area exceeds its ceiling");
  requireCondition(Math.round(viewport.css_width * viewport.requested_device_pixel_ratio) === viewport.physical_width, "projection viewport physical width differs");
  requireCondition(Math.round(viewport.css_height * viewport.requested_device_pixel_ratio) === viewport.physical_height, "projection viewport physical height differs");
}

/** Validates a non-gating attended interpretation record. */
export function validateRubricObservation(value, policy) {
  return validateRubricEvidenceShape(value, policy);
}

async function materializeGeneratedSource(source, options) {
  const scene = generateVisualScene(source.generator);
  requireCondition(scene.source_identity === source.source_identity, "generated Source identity differs");
  requireCondition(scene.point_count === source.point_count, "generated Point count differs");
  requireCondition(scene.batches.length === source.batch_count, "generated batch count differs");
  requireCondition(arrayEquals(scene.conditions, source.conditions), "generated conditions differ");
  const batches = scene.batches.map((batch) => encodeTransferV2(batch.points));
  const payload = concatenateBytes(batches);
  requireCondition(payload.byteLength === source.transfer_byte_length, "generated transfer byte length differs");
  const digestImplementation = options.sha256Implementation ?? sha256Hex;
  const payloadDigest = await digestImplementation(payload);
  requireCondition(payloadDigest === source.payload_sha256, "generated payload SHA-256 differs");
  return {
    source_identity: scene.source_identity,
    world_origin: scene.world_origin,
    source_z_range: scene.source_z_range,
    point_count: scene.point_count,
    batches,
    input_facts: {
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
      payload_sha256: payloadDigest,
    },
  };
}

async function materializeDerivedSource(source, options) {
  const fetchImplementation = options.fetchImplementation ?? globalThis.fetch;
  const corpusUrl = options.corpusUrl ?? globalThis.location?.href;
  requireCondition(typeof fetchImplementation === "function", "Fetch is unavailable");
  requireCondition(corpusUrl !== undefined, "corpus URL is required for a derived Source");
  const manifestUrl = new URL(source.manifest_path, corpusUrl).href;
  const manifestResponse = await fetchImplementation(manifestUrl, { cache: "no-store", credentials: "same-origin" });
  requireCondition(manifestResponse?.ok, `derived manifest returned HTTP ${manifestResponse?.status ?? "unknown"}`);
  const manifest = await manifestResponse.json();
  validateDerivedManifest(manifest, source);
  const payloadUrl = new URL(manifest.sample.path, manifestUrl).href;
  const payloadResponse = await fetchImplementation(payloadUrl, { cache: "no-store", credentials: "same-origin" });
  requireCondition(payloadResponse?.ok, `derived payload returned HTTP ${payloadResponse?.status ?? "unknown"}`);
  const payload = new Uint8Array(await payloadResponse.arrayBuffer());
  requireCondition(payload.byteLength === manifest.sample.byte_length, "derived payload byte length differs");
  const digestImplementation = options.sha256Implementation ?? sha256Hex;
  requireCondition(await digestImplementation(payload) === manifest.sample.sha256, "derived payload SHA-256 differs");
  const batchBytes = MAX_TRANSFER_BATCH_POINTS * TRANSFER_RECORD_BYTES;
  const batches = [];
  let previousOrdinal = -1;
  for (let offset = 0; offset < payload.byteLength; offset += batchBytes) {
    const batch = payload.slice(offset, Math.min(payload.byteLength, offset + batchBytes));
    const decoded = decodeTransferV2(batch, previousOrdinal);
    previousOrdinal = decoded.at(-1).ordinal;
    batches.push(batch);
  }
  requireCondition(batches.length <= MAX_TRANSFER_BATCHES, "derived payload exceeds batch ceiling");
  return {
    source_identity: manifest.source.source_identity,
    world_origin: [...manifest.source.world_origin],
    source_z_range: [manifest.source.bounds.min[2], manifest.source.bounds.max[2]],
    point_count: manifest.sample.point_count,
    batches,
    source_camera: manifest.camera,
    input_facts: {
      kind: "derived_pvis",
      fixture_id: manifest.fixture_id,
      manifest_url: manifestUrl,
      payload_url: payloadUrl,
      payload_bytes: payload.byteLength,
      payload_sha256: manifest.sample.sha256,
      upstream_source_sha256: manifest.source.sha256,
      permission: manifest.permission,
      conditions: manifest.conditions,
      condition_facts: manifest.condition_facts,
    },
  };
}

function authoredPoint(ordinal, x, y, z) {
  const attributeIndex = ordinal % 210;
  return {
    ordinal,
    relative_position: [Math.fround(x), Math.fround(y), Math.fround(z)],
    intensity: INTENSITY_LEVELS[attributeIndex % INTENSITY_LEVELS.length],
    classification: CLASSIFICATIONS[attributeIndex % CLASSIFICATIONS.length],
    rgb: [...RGB_LEVELS[attributeIndex % RGB_LEVELS.length]],
  };
}

function sceneBatch(index, role, points) {
  requireInteger(index, "batch index", 0, MAX_TRANSFER_BATCHES - 1);
  requireCondition(points.length > 0 && points.length <= MAX_TRANSFER_BATCH_POINTS, `batch ${index} size differs`);
  return { index, role, points };
}

function deriveGeneratedConditions(scene) {
  const sparse = scene.batches.some((batch) => batch.role === "sparse_features" && batch.points.length <= 256);
  const dense = scene.batches.some((batch) => batch.points.length >= 768);
  const depthLayers = scene.batches.find((batch) => batch.role === "depth_layers")?.points ?? [];
  const lowerCoordinates = new Set(depthLayers.slice(0, depthLayers.length / 2).map(pointCoordinateKey));
  const layered = depthLayers.slice(depthLayers.length / 2).every((point) => lowerCoordinates.has(pointCoordinateKey(point)));
  const points = scene.batches.flatMap((batch) => batch.points);
  const intensities = points.map((point) => point.intensity);
  const rgbChannels = points.flatMap((point) => point.rgb);
  const highDynamicRange = Math.min(...intensities) === 0
    && Math.max(...intensities) === 65_535
    && Math.min(...rgbChannels) === 0
    && Math.max(...rgbChannels) === 65_535;
  const classification = new Set(points.map((point) => point.classification)).size >= CLASSIFICATIONS.length;
  const largeWorld = scene.world_origin.some((coordinate) => Math.abs(coordinate) >= 1_000_000)
    && minimumPositiveAxisSpacing(scene.batches[1].points, 0) < 1;
  const replacementTrace = scene.lod_relations.some(({ parent_batch_index: parent, child_batch_index: child }) => {
    const parentBatch = scene.batches[parent];
    const childBatch = scene.batches[child];
    return parentBatch?.role === "lod_parent"
      && childBatch?.role === "lod_child"
      && parentBatch.points.length < childBatch.points.length
      && boundsOverlap(pointBounds(parentBatch.points), pointBounds(childBatch.points));
  });
  const stableCut = scene.stable_lod_relations.some(({ dense_batch_index: dense, coarse_batch_index: coarse }) => {
    const denseBatch = scene.batches[dense];
    const coarseBatch = scene.batches[coarse];
    if (denseBatch?.role !== "lod_child" || coarseBatch?.role !== "lod_adjacent_coarse") return false;
    const denseBounds = pointBounds(denseBatch.points);
    const coarseBounds = pointBounds(coarseBatch.points);
    const xGap = coarseBounds.minimum[0] - denseBounds.maximum[0];
    return denseBatch.points.length > coarseBatch.points.length
      && xGap > 0
      && xGap <= minimumPositiveAxisSpacing(denseBatch.points, 0)
      && intervalsOverlap(denseBounds.minimum[1], denseBounds.maximum[1], coarseBounds.minimum[1], coarseBounds.maximum[1]);
  });
  const mixedLod = replacementTrace && stableCut;
  const facts = { sparse, dense, layered, high_dynamic_range: highDynamicRange, classification, large_world: largeWorld, mixed_lod: mixedLod };
  for (const [condition, satisfied] of Object.entries(facts)) {
    requireCondition(satisfied, `generated scene does not derive ${condition}`);
  }
  return REQUIRED_GENERATED_CONDITIONS.filter((condition) => facts[condition]);
}

function validateSource(source) {
  requireRecord(source, "visual source");
  requireIdentifier(source.id, "visual source id");
  requireCondition(source.kind === "generated" || source.kind === "derived_pvis", `visual source ${source.id} kind differs`);
  requireCondition(Array.isArray(source.conditions) && source.conditions.length > 0, `visual source ${source.id} conditions are empty`);
  requireCondition(source.conditions.every((condition) => REQUIRED_GENERATED_CONDITIONS.includes(condition) || condition === "permitted_real_source"), `visual source ${source.id} condition differs`);
  if (source.kind === "generated") {
    requireCondition(source.generator === GENERATED_SCENE_ID, "generated source algorithm differs");
    requireCondition(source.source_identity === GENERATED_SOURCE_IDENTITY, "generated Source identity differs");
    requireInteger(source.point_count, "generated Point count", 1, MAX_TRANSFER_BATCH_POINTS * MAX_TRANSFER_BATCHES);
    requireInteger(source.batch_count, "generated batch count", 1, MAX_TRANSFER_BATCHES);
    requireInteger(source.transfer_byte_length, "generated transfer byte length", TRANSFER_RECORD_BYTES, MAX_TRANSFER_BATCH_POINTS * MAX_TRANSFER_BATCHES * TRANSFER_RECORD_BYTES);
    requireCondition(/^[0-9a-f]{64}$/.test(source.payload_sha256), "generated payload SHA-256 differs");
    requireCondition(arrayEquals(source.conditions, REQUIRED_GENERATED_CONDITIONS), "generated source conditions differ");
    validateGeneratedSourceFacts(source);
  } else {
    requireCondition(typeof source.manifest_path === "string" && source.manifest_path.endsWith(".json"), "derived source manifest path differs");
    validateExpectedView(source.expected_view, {
      points: 4_096,
      batches: 4,
      bytes: 131_072,
      removed: [],
      resident: 4_096,
      drawCalls: 4,
    });
    requireRecord(source.condition_derivation, "derived condition derivation");
    requireCondition(source.condition_derivation.kind === "licensed-fixed-ordinal-block-sample", "derived condition rule differs");
    requireCondition(source.condition_derivation.fixture_manifest === source.manifest_path, "derived condition manifest differs");
    requireCondition(source.condition_derivation.payload === "./autzen-classified-sample.pvis", "derived condition payload differs");
    requireCondition(Array.isArray(source.condition_derivation.measured_from) && source.condition_derivation.measured_from.length >= 3, "derived measured condition facts are incomplete");
    requireCondition(Array.isArray(source.condition_derivation.not_claimed) && source.condition_derivation.not_claimed.includes("complete_source_coverage"), "derived nonclaims are incomplete");
  }
}

function validateTrial(trial, sources, toleranceProfiles) {
  requireRecord(trial, "visual trial");
  requireIdentifier(trial.id, "visual trial id");
  requireCondition(sources.has(trial.source_id), `trial ${trial.id} source is unknown`);
  requireCondition(DISPLAY_MODES.includes(trial.display_mode), `trial ${trial.id} display mode differs`);
  if (trial.camera !== "source") validateCamera(trial.camera, `trial ${trial.id} camera`);
  requireCondition(trial.coverage === "authored" || trial.coverage === "sampled" || trial.coverage === "complete", `trial ${trial.id} Coverage differs`);
  requireCondition(Array.isArray(trial.conditions) && trial.conditions.length > 0, `trial ${trial.id} conditions are empty`);
  const source = sources.get(trial.source_id);
  requireCondition(
    trial.coverage === (source.kind === "generated" ? "authored" : "sampled"),
    `trial ${trial.id} Coverage does not follow its Source facts`,
  );
  requireCondition(trial.conditions.every((condition) => source.conditions.includes(condition)), `trial ${trial.id} claims an unsupported condition`);
  requireCondition(Object.hasOwn(toleranceProfiles, trial.tolerance_profile), `trial ${trial.id} tolerance profile is unknown`);
  requireCondition(Object.hasOwn(toleranceProfiles, trial.temporal_tolerance_profile), `trial ${trial.id} temporal tolerance profile is unknown`);
  requireInteger(trial.expected_presentation_version, `trial ${trial.id} presentation version`, 1, 2);
  const expectedPresentationVersion = trial.display_mode === RAW_VIEWER_INITIAL_DISPLAY_MODE ? 1 : 2;
  requireCondition(trial.expected_presentation_version === expectedPresentationVersion, `trial ${trial.id} presentation version does not follow a fresh viewer`);
  requireCondition(
    Array.isArray(trial.expected_settled_batch_versions)
      && trial.expected_settled_batch_versions.length === source.expected_view.published_batches
      && trial.expected_settled_batch_versions.every((version) => version === expectedPresentationVersion),
    `trial ${trial.id} settled batch versions differ`,
  );
  requireRecord(trial.selection, `trial ${trial.id} selection`);
  requireCondition(Array.isArray(trial.selection.ordinals) && trial.selection.ordinals.length <= 32, `trial ${trial.id} selection differs`);
  requireCondition(trial.selection.ordinals.every((ordinal) => Number.isSafeInteger(ordinal) && ordinal >= 0), `trial ${trial.id} selection ordinal differs`);
  requireCondition(Array.isArray(trial.features) && trial.features.length > 0, `trial ${trial.id} features are empty`);
  const featureIds = new Set();
  for (const feature of trial.features) {
    requireRecord(feature, `trial ${trial.id} feature`);
    requireIdentifier(feature.id, `trial ${trial.id} feature id`);
    requireCondition(!featureIds.has(feature.id), `trial ${trial.id} feature is duplicated`);
    featureIds.add(feature.id);
    validateRectangle(feature.rectangle, `trial ${trial.id} feature ${feature.id}`);
    requireInteger(feature.minimum_foreground_pixels, `trial ${trial.id} feature occupancy`, 1, VISUAL_VIEWPORT.physical_width * VISUAL_VIEWPORT.physical_height);
    validateFeatureBinding(feature.binding, trial, sources.get(trial.source_id), feature.rectangle);
  }
  if (trial.selection.ordinals.length > 0) {
    requireCondition(trial.selection.point_identity_authority === "authored_source_fact", `trial ${trial.id} selection Point authority differs`);
    requireCondition(trial.selection.highlight_authority === "presentation_only", `trial ${trial.id} highlight authority differs`);
    requireCondition(trial.selection.nominal_pick_coverage_authority === "projected_authored_point_fact", `trial ${trial.id} nominal pick authority differs`);
    const boundOrdinals = new Set(trial.features.flatMap((feature) => feature.binding.authored_point_ordinals ?? []));
    requireCondition(trial.selection.ordinals.every((ordinal) => boundOrdinals.has(ordinal)), `trial ${trial.id} selection is not bound to a projected feature`);
    requireCondition(Array.isArray(trial.selection.nominal_pick_regions), `trial ${trial.id} nominal pick regions are missing`);
    const featureById = new Map(trial.features.map((feature) => [feature.id, feature]));
    requireCondition(trial.selection.nominal_pick_regions.length === trial.selection.ordinals.length, `trial ${trial.id} nominal pick region count differs`);
    for (const region of trial.selection.nominal_pick_regions) {
      requireRecord(region, `trial ${trial.id} nominal pick region`);
      const feature = featureById.get(region.feature_id);
      requireCondition(feature !== undefined, `trial ${trial.id} nominal pick feature is absent`);
      requireCondition(trial.selection.ordinals.includes(region.ordinal), `trial ${trial.id} nominal pick ordinal is not selected`);
      requireCondition(feature.binding.authored_point_ordinals.length === 1 && feature.binding.authored_point_ordinals[0] === region.ordinal, `trial ${trial.id} nominal pick binding differs`);
      requireCondition(feature.rectangle.width <= 32 && feature.rectangle.height <= 32, `trial ${trial.id} nominal pick region is too broad`);
    }
  }
  requireRecord(trial.temporal_trace, `trial ${trial.id} temporal trace`);
  requireCondition(trial.temporal_trace.kind === "static" || trial.temporal_trace.kind === "mixed_lod_parent_child", `trial ${trial.id} temporal trace differs`);
  if (trial.temporal_trace.kind === "mixed_lod_parent_child") {
    requireInteger(trial.temporal_trace.parent_batch_index, "parent batch index", 0, MAX_TRANSFER_BATCHES - 1);
    requireInteger(trial.temporal_trace.child_batch_index, "child batch index", 0, MAX_TRANSFER_BATCHES - 1);
    requireCondition(trial.temporal_trace.parent_batch_index !== trial.temporal_trace.child_batch_index, "parent and child batch indices match");
    requireCondition(Array.isArray(trial.temporal_trace.child_weights_u8), "mixed-LOD weights are missing");
    requireCondition(trial.temporal_trace.child_weights_u8.length >= 2 && trial.temporal_trace.child_weights_u8.length <= 32, "mixed-LOD weight count differs");
    requireCondition(trial.temporal_trace.child_weights_u8[0] === 0 && trial.temporal_trace.child_weights_u8.at(-1) === 255, "mixed-LOD endpoints differ");
    requireCondition(trial.temporal_trace.child_weights_u8.every((weight, index, weights) => Number.isInteger(weight) && weight >= 0 && weight <= 255 && (index === 0 || weight > weights[index - 1])), "mixed-LOD weights are not strictly increasing");
  }
  requireCondition(typeof trial.baseline_path === "string" && trial.baseline_path.endsWith(".png"), `trial ${trial.id} baseline path differs`);
}

function validateDerivedManifest(manifest, source) {
  requireRecord(manifest, "derived Source manifest");
  requireCondition(manifest.schema === "punctra-browser-visual-source-v1", "derived Source schema differs");
  requireCondition(manifest.fixture_id === source.fixture_id, "derived fixture identity differs");
  requireRecord(manifest.sample, "derived sample");
  requireInteger(manifest.sample.point_count, "derived sample Point count", 1, MAX_TRANSFER_BATCH_POINTS * MAX_TRANSFER_BATCHES);
  requireCondition(manifest.sample.record_bytes === TRANSFER_RECORD_BYTES, "derived transfer record width differs");
  requireCondition(manifest.sample.byte_length === manifest.sample.point_count * TRANSFER_RECORD_BYTES, "derived transfer byte length differs");
  requireCondition(/^[0-9a-f]{64}$/.test(manifest.sample.sha256), "derived sample SHA-256 differs");
  requireRecord(manifest.source, "derived upstream Source");
  requireCondition(/^[0-9a-f]{64}$/.test(manifest.source.source_identity), "derived Source identity differs");
  requireFiniteTriple(manifest.source.world_origin, "derived world origin");
  requireRecord(manifest.source.bounds, "derived Source bounds");
  requireFiniteTriple(manifest.source.bounds.min, "derived minimum bounds");
  requireFiniteTriple(manifest.source.bounds.max, "derived maximum bounds");
  requireCondition(manifest.source.bounds.min.every((entry, axis) => entry <= manifest.source.bounds.max[axis]), "derived Source bounds are inverted");
  requireRecord(manifest.permission, "derived permission");
  requireCondition(manifest.permission.derived_sample_and_image_publication === true, "derived image publication is not permitted");
  requireCondition(manifest.permission.derivative_redistribution === true, "derived redistribution is not permitted");
  requireCondition(typeof manifest.permission.modification_notice === "string" && manifest.permission.modification_notice.length >= 32, "derived modification notice is incomplete");
  requireRecord(manifest.condition_facts, "derived condition facts");
  requireCondition(manifest.condition_facts.schema === "punctra-browser-visual-sample-conditions-v1", "derived condition-fact schema differs");
  requireCondition(
    arrayEquals(manifest.condition_facts.derived_conditions, source.conditions.filter((condition) => condition !== "permitted_real_source")),
    "derived condition facts do not bind the declared Source conditions",
  );
  validateCamera(manifest.camera, "derived camera");
}

function validateConditionCoverage(contract, sources, trials) {
  requireRecord(contract, "condition coverage");
  requireCondition(contract.schema === "punctra-browser-visual-condition-coverage-v1", "condition-coverage schema differs");
  requireCondition(Array.isArray(contract.generated), "generated condition coverage is missing");
  requireCondition(
    arrayEquals(contract.generated.map(({ condition }) => condition), REQUIRED_GENERATED_CONDITIONS),
    "generated condition-coverage order differs",
  );
  for (const entry of contract.generated) {
    requireRecord(entry, `generated condition coverage ${entry.condition}`);
    const source = sources.get(entry.source_id);
    requireCondition(source?.kind === "generated", `generated condition coverage ${entry.condition} Source differs`);
    const expectedTrials = trials
      .filter((trial) => trial.source_id === entry.source_id && trial.conditions.includes(entry.condition))
      .map(({ id }) => id);
    requireCondition(arrayEquals(entry.trial_ids, expectedTrials), `generated condition coverage ${entry.condition} trials differ`);
    requireCondition(
      arrayEquals(entry.fact_paths, GENERATED_CONDITION_FACT_PATHS[entry.condition]),
      `generated condition coverage ${entry.condition} fact mapping differs`,
    );
    validateFactPaths(entry.fact_paths, source, `generated condition coverage ${entry.condition}`);
    if (entry.condition === "mixed_lod") {
      requireCondition(entry.required_temporal_trace === "mixed_lod_parent_child", "mixed-LOD condition trace differs");
      requireCondition(expectedTrials.every((trialId) => trials.find(({ id }) => id === trialId).temporal_trace.kind === entry.required_temporal_trace), "mixed-LOD condition trial is static");
    }
  }

  requireCondition(Array.isArray(contract.derived_modes), "derived mode coverage is missing");
  requireCondition(
    arrayEquals(contract.derived_modes.map(({ display_mode: mode }) => mode), Object.keys(DERIVED_MODE_FACT_PATHS)),
    "derived display-mode coverage differs",
  );
  const derivedTrialIds = trials.filter((trial) => sources.get(trial.source_id)?.kind === "derived_pvis").map(({ id }) => id).sort();
  requireCondition(
    arrayEquals(contract.derived_modes.map(({ trial_id: trialId }) => trialId).sort(), derivedTrialIds),
    "derived mode coverage does not bind every derived trial",
  );
  for (const entry of contract.derived_modes) {
    requireRecord(entry, `derived mode coverage ${entry.display_mode}`);
    const source = sources.get(entry.source_id);
    const trial = trials.find(({ id }) => id === entry.trial_id);
    requireCondition(source?.kind === "derived_pvis", `derived mode coverage ${entry.display_mode} Source differs`);
    requireCondition(trial?.source_id === entry.source_id && trial.display_mode === entry.display_mode, `derived mode coverage ${entry.display_mode} trial differs`);
    requireCondition(
      arrayEquals(entry.fact_paths, DERIVED_MODE_FACT_PATHS[entry.display_mode]),
      `derived mode coverage ${entry.display_mode} fact mapping differs`,
    );
    requireCondition(entry.fact_paths.every((path) => path.startsWith("condition_facts.")), `derived mode coverage ${entry.display_mode} fact path differs`);
  }
}

function validateMaterializedConditionFacts(contract, trial, inputFacts) {
  const entry = contract.derived_modes.find(({ trial_id: trialId }) => trialId === trial.id);
  requireCondition(entry !== undefined, `derived trial ${trial.id} has no condition-fact coverage`);
  for (const path of entry.fact_paths) {
    requireCondition(resolveFactPath(inputFacts, path) !== undefined, `derived trial ${trial.id} condition fact ${path} is absent`);
  }
}

function validateFactPaths(paths, source, label) {
  requireCondition(Array.isArray(paths) && paths.length > 0, `${label} fact paths are empty`);
  for (const path of paths) {
    requireCondition(resolveFactPath(source, path) !== undefined, `${label} fact ${path} is absent`);
  }
}

function resolveFactPath(root, path) {
  if (typeof path !== "string" || !/^[a-z0-9_]+(?:\.[a-z0-9_]+)*$/.test(path)) return undefined;
  return path.split(".").reduce(
    (value, field) => value !== null && typeof value === "object" ? value[field] : undefined,
    root,
  );
}

function validateCapturePolicy(policy) {
  requireRecord(policy, "capture policy");
  const expected = {
    schema: "punctra-browser-frame-capture-v1",
    kind: "private_offscreen_gpu_readback",
    canonical_format: "rgba8",
    canonical_channel_order: "rgba",
    canonical_encoding: "linear",
    origin: "top_left",
    lossless_artifact: "png-rgba8-filter-0",
    presentation_claim: "offscreen_not_presented",
  };
  for (const [field, value] of Object.entries(expected)) requireCondition(policy[field] === value, `capture policy ${field} differs`);
}

function validateTransportPolicy(policy) {
  requireRecord(policy, "visual transport policy");
  const expected = {
    format: "ustar-uncompressed",
    archive_filename: "v0.21-browser-visual-evidence.tar",
    evidence_repository_path: "docs/releases/v0.21-browser-visual-evidence.json",
    maximum_entries: 896,
    maximum_evidence_json_bytes: 33_554_432,
    maximum_baseline_inputs_json_bytes: 1_048_576,
    maximum_archive_structure_bytes: 1_048_576,
    maximum_archive_overhead_bytes: 35_651_584,
    maximum_archive_bytes: 1_243_611_136,
  };
  for (const [field, value] of Object.entries(expected)) {
    requireCondition(policy[field] === value, `visual transport ${field} differs`);
  }
  requireCondition(
    policy.maximum_archive_overhead_bytes
      === policy.maximum_evidence_json_bytes
        + policy.maximum_baseline_inputs_json_bytes
        + policy.maximum_archive_structure_bytes,
    "visual transport overhead accounting differs",
  );
}

function validateTimingLimits(limits) {
  requireRecord(limits, "visual timing limits");
  const expected = {
    schema: "punctra-browser-visual-timing-limits-v1",
    first_coverage_milliseconds: 10_000,
    settled_view_milliseconds: 15_000,
    representative_frame_interval_p95_milliseconds: 50,
    representative_frame_submission_p95_milliseconds: 16.7,
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
  };
  requireCondition(
    Object.keys(limits).length === Object.keys(expected).length,
    "visual timing-limit fields differ",
  );
  for (const [field, value] of Object.entries(expected)) {
    requireCondition(limits[field] === value, `visual timing limit ${field} differs`);
  }
}

function validatePresentationPolicy(policy) {
  requireRecord(policy, "presentation policy");
  const expected = {
    default_point_size_physical_pixels: 7,
    display_point_size_physical_pixels: 7,
    depth_compare: "less_equal",
    depth_write_enabled: true,
    color_blend: "alpha_blending",
    primitive_topology: "triangle_list",
    presentation_latency_frames: 2,
    display_authority: "progressive_gpu_non_authoritative",
  };
  for (const [field, value] of Object.entries(expected)) requireCondition(policy[field] === value, `presentation policy ${field} differs`);
  requireCondition(arrayEquals(policy.highlight_linear_rgb, [0.78, 0.66, 0.2]), "presentation highlight color differs");
  requireCondition(arrayEquals(policy.clear_linear_rgba, [0.075, 0.078, 0.075, 1]), "presentation clear color differs");
  requireCondition(arrayEquals(policy.canonical_clear_rgba8, [19, 20, 19, 255]), "presentation canonical clear color differs");
}

function validateRequiredCapabilities(capabilities) {
  requireRecord(capabilities, "required capabilities");
  const expected = {
    secure_context: true,
    webgpu: true,
    fallback_allowed: false,
    fallback_state: "none",
    surface_format: "Bgra8Unorm",
    surface_color_space: "srgb",
    composite_alpha_mode: "Opaque",
    present_mode: "fifo",
    render_attachment: true,
    blendable: true,
    capture_source_format: "bgra8_unorm",
    capture_source_channel_order: "bgra",
    capture_canonicalization: "bgra_to_rgba",
  };
  for (const [field, value] of Object.entries(expected)) requireCondition(capabilities[field] === value, `required capability ${field} differs`);
}

function validateResourceLimits(limits) {
  requireRecord(limits, "resource limits");
  const expected = {
    renderer_resident_points: 8_192,
    renderer_resident_bytes: 196_608,
    renderer_batches: 8,
    highlight_points: 32,
    canvas_surface_bytes: 33_554_432,
    renderer_transient_texture_bytes: 67_108_864,
    retained_record_bytes: 262_144,
    worker_staging_bytes: 327_680,
    queued_range_bytes: 524_288,
    concurrent_response_bytes: 262_144,
    memory_cache_bytes: 524_288,
    persistent_cache_bytes: 4_194_304,
    canonical_pixel_bytes: 1_228_800,
    capture_texture_bytes: 1_228_800,
    staging_buffer_bytes: 1_228_800,
    row_aligned_readback_bytes: 1_228_800,
    png_scanline_bytes: 1_229_280,
    encoder_working_bytes: 2_524_096,
    encoded_png_bytes: 1_310_720,
    total_encoded_artifact_bytes: 1_207_959_552,
    evidence_json_bytes: 33_554_432,
    baseline_inputs_json_bytes: 1_048_576,
    comparison_workspace_bytes: 65_536,
    peak_live_canonical_images: 2,
  };
  for (const [field, value] of Object.entries(expected)) requireCondition(limits[field] === value, `resource limit ${field} differs`);
}

function validateGeneratedSourceFacts(source) {
  const scene = generateVisualScene(source.generator);
  validateExpectedView(source.expected_view, {
    points: scene.point_count,
    batches: scene.batches.length,
    bytes: source.transfer_byte_length,
    removed: [0],
    resident: scene.point_count - scene.batches[0].points.length,
    drawCalls: scene.batches.length - 1,
  });
  const facts = source.condition_facts;
  requireRecord(facts, "generated condition facts");
  requireRecord(facts.sparse_batch, "generated sparse facts");
  requireRecord(facts.layer_pairs, "generated layer facts");
  requireRecord(facts.attribute_extrema, "generated attribute-extrema facts");
  requireRecord(facts.lod_relation, "generated LOD-replacement facts");
  requireRecord(facts.stable_lod_cut, "generated stable LOD-cut facts");
  requireCondition(facts.sparse_batch.batch_index === 3 && facts.sparse_batch.point_count === scene.batches[3].points.length && facts.sparse_batch.maximum_points === 256, "generated sparse facts differ");
  requireCondition(arrayEquals(facts.dense_batches.map((entry) => entry.batch_index), [1, 2]), "generated dense batch identities differ");
  requireCondition(arrayEquals(facts.dense_batches.map((entry) => entry.point_count), [841, 800]), "generated dense Point counts differ");
  requireCondition(facts.layer_pairs.batch_index === 2 && facts.layer_pairs.paired_xy_count === 400 && facts.layer_pairs.minimum_z_separation === 5.5, "generated layer facts differ");
  requireCondition(arrayEquals(facts.attribute_extrema.intensity, [0, 65_535]), "generated intensity extrema differ");
  requireCondition(arrayEquals(facts.attribute_extrema.rgb_channel, [0, 65_535]), "generated RGB extrema differ");
  requireCondition(arrayEquals(facts.attribute_extrema.classifications, CLASSIFICATIONS), "generated classification facts differ");
  requireCondition(arrayEquals(facts.large_world_origin, scene.world_origin), "generated large-world facts differ");
  requireCondition(facts.minimum_dense_axis_spacing === minimumPositiveAxisSpacing(scene.batches[1].points, 0), "generated spacing fact differs");
  requireCondition(
    facts.lod_relation.parent_batch_index === 0
      && facts.lod_relation.parent_points === scene.batches[0].points.length
      && facts.lod_relation.child_batch_index === 1
      && facts.lod_relation.child_points === scene.batches[1].points.length,
    "generated LOD relation differs",
  );
  const denseBounds = pointBounds(scene.batches[1].points);
  const coarseBounds = pointBounds(scene.batches[4].points);
  const denseDensity = pointsPerXyArea(scene.batches[1].points, denseBounds);
  const coarseDensity = pointsPerXyArea(scene.batches[4].points, coarseBounds);
  requireRecord(facts.stable_lod_cut.dense_xy_bounds, "generated dense LOD-cut bounds");
  requireRecord(facts.stable_lod_cut.coarse_xy_bounds, "generated coarse LOD-cut bounds");
  requireCondition(
    facts.stable_lod_cut.dense_batch_index === 1
      && facts.stable_lod_cut.dense_points === scene.batches[1].points.length
      && arrayEquals(facts.stable_lod_cut.dense_xy_bounds.min, denseBounds.minimum.slice(0, 2))
      && arrayEquals(facts.stable_lod_cut.dense_xy_bounds.max, denseBounds.maximum.slice(0, 2))
      && facts.stable_lod_cut.dense_points_per_xy_area === denseDensity
      && facts.stable_lod_cut.coarse_batch_index === 4
      && facts.stable_lod_cut.coarse_points === scene.batches[4].points.length
      && arrayEquals(facts.stable_lod_cut.coarse_xy_bounds.min, coarseBounds.minimum.slice(0, 2))
      && arrayEquals(facts.stable_lod_cut.coarse_xy_bounds.max, coarseBounds.maximum.slice(0, 2))
      && facts.stable_lod_cut.coarse_points_per_xy_area === coarseDensity
      && facts.stable_lod_cut.adjacent_x_gap === coarseBounds.minimum[0] - denseBounds.maximum[0]
      && facts.stable_lod_cut.density_ratio === denseDensity / coarseDensity
      && facts.stable_lod_cut.settled_dense_weight_u8 === 255
      && facts.stable_lod_cut.settled_coarse_weight_u8 === 255
      && facts.stable_lod_cut.distinct_from_parent_child_replacement === true,
    "generated stable LOD-cut facts differ",
  );
}

function validateExpectedView(view, expected) {
  requireRecord(view, "expected View facts");
  requireCondition(view.view_id === 16 && view.generation === 1, "expected View identity differs");
  requireCondition(view.published_points === expected.points && view.published_batches === expected.batches && view.transferred_bytes === expected.bytes, "expected View publication differs");
  requireCondition(view.stream_coverage === "sampled", "expected View Coverage differs");
  requireCondition(arrayEquals(view.batch_keys, Array.from({ length: expected.batches }, (_, index) => index + 1)), "expected batch keys differ");
  requireCondition(view.initial_batch_version === 1, "expected initial batch version differs");
  requireCondition(arrayEquals(view.initial_batch_versions, Array(expected.batches).fill(1)), "expected initial batch versions differ");
  requireCondition(arrayEquals(view.settled_removed_batch_indices, expected.removed), "expected removed batches differ");
  requireCondition(view.settled_resident_points === expected.resident && view.settled_drawn_points === expected.resident && view.settled_draw_calls === expected.drawCalls, "expected settled draw facts differ");
  const expectedWeights = Array.from(
    { length: expected.batches },
    (_, batchIndex) => expected.removed.includes(batchIndex) ? 0 : 255,
  );
  requireCondition(arrayEquals(view.settled_presentation_weights_u8, expectedWeights), "expected presentation weights differ");
}

function validateCamera(camera, label) {
  requireRecord(camera, label);
  requireCondition(camera.projection === "perspective" || camera.projection === "orthographic", `${label} projection differs`);
  requireFiniteTriple(camera.eye, `${label} eye`);
  requireFiniteTriple(camera.target, `${label} target`);
  requireFiniteTriple(camera.up, `${label} up`);
  requireCondition(Number.isFinite(camera.near_distance) && camera.near_distance > 0, `${label} near distance differs`);
  requireCondition(Number.isFinite(camera.far_distance) && camera.far_distance > camera.near_distance, `${label} far distance differs`);
  if (camera.projection === "perspective") {
    requireCondition(Number.isFinite(camera.vertical_field_of_view_radians) && camera.vertical_field_of_view_radians > 0 && camera.vertical_field_of_view_radians < Math.PI, `${label} field of view differs`);
  } else {
    requireCondition(Number.isFinite(camera.vertical_world_height) && camera.vertical_world_height > 0, `${label} world height differs`);
  }
}

function validateViewport(viewport) {
  requireRecord(viewport, "visual viewport");
  for (const [key, expected] of Object.entries(VISUAL_VIEWPORT)) {
    requireCondition(viewport[key] === expected, `visual viewport ${key} differs`);
  }
  requireCondition(viewport.css_width * viewport.requested_device_pixel_ratio === viewport.physical_width, "visual viewport width/DPR binding differs");
  requireCondition(viewport.css_height * viewport.requested_device_pixel_ratio === viewport.physical_height, "visual viewport height/DPR binding differs");
}

function validateRectangle(rectangle, label) {
  requireRecord(rectangle, `${label} rectangle`);
  requireInteger(rectangle.x, `${label} x`, 0, VISUAL_VIEWPORT.physical_width - 1);
  requireInteger(rectangle.y, `${label} y`, 0, VISUAL_VIEWPORT.physical_height - 1);
  requireInteger(rectangle.width, `${label} width`, 1, VISUAL_VIEWPORT.physical_width);
  requireInteger(rectangle.height, `${label} height`, 1, VISUAL_VIEWPORT.physical_height);
  requireCondition(rectangle.x + rectangle.width <= VISUAL_VIEWPORT.physical_width, `${label} exceeds viewport width`);
  requireCondition(rectangle.y + rectangle.height <= VISUAL_VIEWPORT.physical_height, `${label} exceeds viewport height`);
}

function validateFeatureBinding(binding, trial, source, rectangle) {
  requireRecord(binding, `trial ${trial.id} feature binding`);
  if (source.kind === "generated") {
    requireCondition(binding.kind === "authored_point_projection", `trial ${trial.id} generated feature binding differs`);
    requireCondition(binding.source_identity === source.source_identity, `trial ${trial.id} feature Source identity differs`);
    requireCondition(binding.projection_rule === "punctra-authored-camera-projection-v1", `trial ${trial.id} feature projection rule differs`);
    requireInteger(binding.tolerance_pixels, `trial ${trial.id} feature projection tolerance`, 0, 1);
    requireCondition(Array.isArray(binding.authored_point_ordinals) && binding.authored_point_ordinals.length > 0 && binding.authored_point_ordinals.length <= 16, `trial ${trial.id} feature Point binding differs`);
    requireCondition(Array.isArray(binding.expected_pixels) && binding.expected_pixels.length === binding.authored_point_ordinals.length, `trial ${trial.id} expected feature pixels differ`);
    const scene = generateVisualScene(source.generator);
    const points = new Map(scene.batches.flatMap((batch) => batch.points).map((point) => [point.ordinal, point]));
    binding.authored_point_ordinals.forEach((ordinal, index) => {
      const point = points.get(ordinal);
      requireCondition(point !== undefined, `trial ${trial.id} feature Point ${ordinal} is absent`);
      const projected = projectAuthoredPoint(point, scene.world_origin, trial.camera);
      const expected = binding.expected_pixels[index];
      requireCondition(Array.isArray(expected) && expected.length === 2 && expected.every(Number.isInteger), `trial ${trial.id} expected feature pixel differs`);
      requireCondition(Math.abs(projected.x - expected[0]) <= binding.tolerance_pixels && Math.abs(projected.y - expected[1]) <= binding.tolerance_pixels, `trial ${trial.id} feature projection differs for Point ${ordinal}`);
      requireCondition(pixelInsideRectangle(projected, rectangle), `trial ${trial.id} projected feature lies outside its rectangle`);
    });
  } else {
    requireCondition(binding.kind === "derived_sample_region", `trial ${trial.id} derived feature binding differs`);
    requireCondition(binding.fixture_id === source.fixture_id, `trial ${trial.id} derived feature fixture differs`);
    requireCondition(typeof binding.sample_sha256 === "string" && /^[0-9a-f]{64}$/.test(binding.sample_sha256), `trial ${trial.id} derived feature sample SHA-256 differs`);
  }
}

function validateAuthoredPoint(point, previousOrdinal) {
  requireRecord(point, "authored Point");
  requireCondition(Number.isSafeInteger(point.ordinal) && point.ordinal >= 0 && point.ordinal > previousOrdinal, "authored Point ordinals are not increasing");
  requireFiniteTriple(point.relative_position, "authored relative position");
  requireInteger(point.intensity, "authored intensity", 0, 65_535);
  requireInteger(point.classification, "authored classification", 0, 255);
  requireCondition(Array.isArray(point.rgb) && point.rgb.length === 3, "authored RGB differs");
  point.rgb.forEach((channel) => requireInteger(channel, "authored RGB channel", 0, 65_535));
}

function pointCoordinateKey(point) {
  return `${point.relative_position[0]},${point.relative_position[1]}`;
}

function minimumPositiveAxisSpacing(points, axis) {
  const values = [...new Set(points.map((point) => point.relative_position[axis]))].sort((left, right) => left - right);
  let minimum = Number.POSITIVE_INFINITY;
  for (let index = 1; index < values.length; index += 1) {
    const delta = values[index] - values[index - 1];
    if (delta > 0) minimum = Math.min(minimum, delta);
  }
  return minimum;
}

function pointBounds(points) {
  const minimum = [Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY, Number.POSITIVE_INFINITY];
  const maximum = [Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY, Number.NEGATIVE_INFINITY];
  for (const point of points) {
    point.relative_position.forEach((value, axis) => {
      minimum[axis] = Math.min(minimum[axis], value);
      maximum[axis] = Math.max(maximum[axis], value);
    });
  }
  return { minimum, maximum };
}

function pointsPerXyArea(points, bounds = pointBounds(points)) {
  const width = bounds.maximum[0] - bounds.minimum[0];
  const height = bounds.maximum[1] - bounds.minimum[1];
  requireCondition(width > 0 && height > 0, "generated LOD-cut area is degenerate");
  return points.length / (width * height);
}

function boundsOverlap(left, right) {
  return left.minimum.every((minimum, axis) => minimum <= right.maximum[axis] && right.minimum[axis] <= left.maximum[axis]);
}

function intervalsOverlap(leftMinimum, leftMaximum, rightMinimum, rightMaximum) {
  return leftMinimum <= rightMaximum && rightMinimum <= leftMaximum;
}

function pixelInsideRectangle(pixel, rectangle) {
  return pixel.x >= rectangle.x
    && pixel.x < rectangle.x + rectangle.width
    && pixel.y >= rectangle.y
    && pixel.y < rectangle.y + rectangle.height;
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

function subtract(left, right) {
  return left.map((value, axis) => value - right[axis]);
}

function dot(left, right) {
  return left.reduce((total, value, axis) => total + value * right[axis], 0);
}

function cross(left, right) {
  return [
    left[1] * right[2] - left[2] * right[1],
    left[2] * right[0] - left[0] * right[2],
    left[0] * right[1] - left[1] * right[0],
  ];
}

function normalize(vector) {
  const length = Math.sqrt(dot(vector, vector));
  requireCondition(Number.isFinite(length) && length > 0, "projection basis is degenerate");
  return vector.map((value) => value / length);
}

function asUint8Array(value, label) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  throw new TypeError(`${label} must be a Uint8Array or ArrayBuffer`);
}

function requireFiniteTriple(value, label) {
  requireCondition(Array.isArray(value) && value.length === 3 && value.every(Number.isFinite), `${label} must contain three finite numbers`);
}

function requireIdentifier(value, label) {
  requireCondition(typeof value === "string" && /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value), `${label} is invalid`);
}

function requireInteger(value, label, minimum, maximum) {
  requireCondition(Number.isInteger(value) && value >= minimum && value <= maximum, `${label} must be an integer from ${minimum} through ${maximum}`);
}

function arrayEquals(left, right) {
  return Array.isArray(left) && Array.isArray(right) && left.length === right.length
    && left.every((value, index) => value === right[index]);
}
