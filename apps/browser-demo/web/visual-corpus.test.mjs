import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  AUTZEN_VISUAL_TRIAL_COUNT,
  DISPLAY_MODES,
  GENERATED_VISUAL_TRIAL_COUNT,
  RAW_VIEWER_INITIAL_DISPLAY_MODE,
  REQUIRED_GENERATED_CONDITIONS,
  VISUAL_TRIAL_COUNT,
  VISUAL_VIEWPORT,
  decodeTransferV2,
  encodeTransferV2,
  generateVisualScene,
  loadVisualCorpus,
  materializeVisualTrial,
  projectAuthoredPoint,
  validateRubricObservation,
  validateVisualCorpus,
} from "./visual-corpus.js";
import { sha256Hex } from "./visual-png.js";

const FIXTURE_DIRECTORY = new URL("./fixtures/visual-v1/", import.meta.url);
const CORPUS_URL = new URL("corpus.json", FIXTURE_DIRECTORY);

test("checked-in corpus is closed, representative, and binds all modes and projections", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));
  assert.equal(corpus.trials.length, VISUAL_TRIAL_COUNT);
  assert.equal(corpus.trials.filter((trial) => trial.source_id.startsWith("generated")).length, GENERATED_VISUAL_TRIAL_COUNT);
  assert.equal(corpus.trials.filter((trial) => trial.source_id.startsWith("autzen")).length, AUTZEN_VISUAL_TRIAL_COUNT);
  assert.deepEqual(corpus.viewport, VISUAL_VIEWPORT);
  assert.deepEqual(
    [...new Set(corpus.trials.map((trial) => trial.display_mode))].sort(),
    [...DISPLAY_MODES].sort(),
  );
  assert.deepEqual(
    [...new Set(corpus.trials
      .filter((trial) => trial.source_id.startsWith("generated"))
      .flatMap((trial) => trial.conditions))].sort(),
    [...REQUIRED_GENERATED_CONDITIONS].sort(),
  );
  assert(corpus.trials.some((trial) => trial.camera === "source" || trial.camera.projection === "perspective"));
  assert(corpus.trials.some((trial) => trial.camera?.projection === "orthographic"));
  assert(corpus.trials.some((trial) => trial.selection.ordinals.length > 0));
  assert(corpus.trials.some((trial) => trial.selection.ordinals.length === 0));
  assert.equal(corpus.presentation_policy.canonical_clear_rgba8.join(","), "19,20,19,255");
  assert.equal(corpus.resource_limits.peak_live_canonical_images, 2);
  assert.equal(corpus.resource_limits.total_encoded_artifact_bytes, 1_207_959_552);
  assert.equal(corpus.transport.maximum_entries, 896);
  assert.equal(corpus.transport.maximum_baseline_inputs_json_bytes, 1_048_576);
  assert.equal(corpus.transport.maximum_archive_overhead_bytes, 35_651_584);
  assert.equal(corpus.transport.maximum_archive_bytes, 1_243_611_136);
  assert.equal(corpus.resource_limits.evidence_json_bytes, 33_554_432);
  assert.equal(corpus.resource_limits.baseline_inputs_json_bytes, 1_048_576);
  assert.equal(corpus.timing_limits.first_coverage_milliseconds, 10_000);
  assert.equal(corpus.timing_limits.settled_view_milliseconds, 15_000);
  assert.equal(corpus.timing_limits.representative_frame_interval_p95_milliseconds, 50);
  assert.equal(corpus.timing_limits.representative_frame_submission_p95_milliseconds, 16.7);
  for (const trial of corpus.trials) {
    const expectedVersion = trial.display_mode === RAW_VIEWER_INITIAL_DISPLAY_MODE ? 1 : 2;
    assert.equal(trial.expected_presentation_version, expectedVersion);
    assert(trial.expected_settled_batch_versions.every((version) => version === expectedVersion));
  }
  assert.deepEqual(
    corpus.trials
      .filter((trial) => trial.source_id === "autzen-classified-derived-sample-v1")
      .map((trial) => trial.display_mode)
      .sort(),
    ["classification", "elevation", "intensity", "rgb"],
  );
});

test("corpus validation rejects missing, extra, or rebalanced trials", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));

  const missing = structuredClone(corpus);
  missing.trials.pop();
  assert.throws(() => validateVisualCorpus(missing), /requires exactly 9 trials/);

  const extra = structuredClone(corpus);
  const extraTrial = structuredClone(extra.trials.find(({ source_id: sourceId }) => sourceId.startsWith("generated")));
  extraTrial.id = "generated-extra-trial";
  extra.trials.push(extraTrial);
  assert.throws(() => validateVisualCorpus(extra), /requires exactly 9 trials/);

  const rebalanced = structuredClone(corpus);
  const generated = structuredClone(rebalanced.trials.find(({ source_id: sourceId }) => sourceId.startsWith("generated")));
  generated.id = "generated-rebalanced-trial";
  const autzenIndex = rebalanced.trials.findIndex(({ source_id: sourceId }) => sourceId.startsWith("autzen"));
  rebalanced.trials[autzenIndex] = generated;
  assert.throws(() => validateVisualCorpus(rebalanced), /requires exactly 5 generated trials/);
});

test("generated scene, batch roles, transfer bytes, and digest are deterministic", async () => {
  const first = generateVisualScene();
  const second = generateVisualScene();
  assert.deepEqual(second, first);
  assert.equal(first.point_count, 2_103);
  assert.deepEqual(first.batches.map((batch) => [batch.role, batch.points.length]), [
    ["lod_parent", 225],
    ["lod_child", 841],
    ["depth_layers", 800],
    ["sparse_features", 192],
    ["lod_adjacent_coarse", 45],
  ]);
  assert.deepEqual(first.stable_lod_relations, [{ dense_batch_index: 1, coarse_batch_index: 4 }]);
  assert.deepEqual(first.conditions, REQUIRED_GENERATED_CONDITIONS);

  const payloads = first.batches.map((batch) => encodeTransferV2(batch.points));
  const combined = concatenate(payloads);
  assert.equal(combined.byteLength, 67_296);
  assert.equal(await sha256Hex(combined), "dec11f50be83a59f8567440207d17eaea5650063ce88b73d56721f5614f3fd99");
  let previousOrdinal = -1;
  for (const payload of payloads) {
    const decoded = decodeTransferV2(payload, previousOrdinal);
    assert.equal(decoded.length, payload.byteLength / 32);
    previousOrdinal = decoded.at(-1).ordinal;
  }
  assert.equal(previousOrdinal, 2_102);
});

test("stable mixed-LOD cut is authored, adjacent, and distinct from the replacement trace", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));
  const source = corpus.sources.find((entry) => entry.kind === "generated");
  assert.deepEqual(source.condition_facts.stable_lod_cut, {
    dense_batch_index: 1,
    dense_points: 841,
    dense_xy_bounds: { min: [-18, -12], max: [-2, 12] },
    dense_points_per_xy_area: 2.1901041666666665,
    coarse_batch_index: 4,
    coarse_points: 45,
    coarse_xy_bounds: { min: [-1.5, -12], max: [1.5, 12] },
    coarse_points_per_xy_area: 0.625,
    adjacent_x_gap: 0.5,
    density_ratio: 3.5041666666666664,
    settled_dense_weight_u8: 255,
    settled_coarse_weight_u8: 255,
    distinct_from_parent_child_replacement: true,
  });

  const materialized = await materializeVisualTrial(corpus, "generated-neutral-mixed-lod-perspective");
  assert.deepEqual(materialized.input_facts.stable_lod_relations, [
    { dense_batch_index: 1, coarse_batch_index: 4 },
  ]);
  const seam = materialized.trial.features.find((feature) => feature.id === "lod-transition-and-stable-cut-seam");
  assert.deepEqual(seam.binding.authored_point_ordinals, [112, 645, 659, 2078]);

  const tampered = structuredClone(corpus);
  tampered.sources[0].condition_facts.stable_lod_cut.density_ratio = 1;
  assert.throws(() => validateVisualCorpus(tampered), /stable LOD-cut facts differ/);
});

test("transfer-v2 encoding preserves exact attributes and rejects reserved or ordinal corruption", () => {
  const point = {
    ordinal: 9,
    relative_position: [1.25, -2.5, 3.75],
    intensity: 65_535,
    classification: 31,
    rgb: [0, 32_768, 65_535],
  };
  const bytes = encodeTransferV2([point]);
  assert.deepEqual(decodeTransferV2(bytes), [point]);
  assert.equal(bytes[23], 0);
  assert.equal(bytes[30], 0);
  assert.equal(bytes[31], 0);

  const reserved = bytes.slice();
  reserved[23] = 1;
  assert.throws(() => decodeTransferV2(reserved), /reserved bytes/);
  assert.throws(() => decodeTransferV2(bytes, 9), /globally increasing/);
  assert.throws(() => encodeTransferV2([point, point]), /ordinals are not increasing/);
});

test("authored feature projections bind Point identity to the fixed camera and rectangle", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));
  const scene = generateVisualScene();
  const points = new Map(scene.batches.flatMap((batch) => batch.points).map((point) => [point.ordinal, point]));
  const trial = corpus.trials.find((entry) => entry.id === "generated-neutral-mixed-lod-perspective");
  const binding = trial.features[0].binding;
  const projected = binding.authored_point_ordinals.map((ordinal) => {
    const pixel = projectAuthoredPoint(points.get(ordinal), scene.world_origin, trial.camera);
    return [pixel.x, pixel.y];
  });
  assert.deepEqual(projected, binding.expected_pixels);

  const tampered = structuredClone(corpus);
  tampered.trials[0].features[0].binding.expected_pixels[0][0] += 2;
  assert.throws(() => validateVisualCorpus(tampered), /feature projection differs/);
});

test("selected ordinals use independent bounded nominal-pick regions", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));
  const trial = corpus.trials.find((entry) => entry.id === "generated-classification-selection-perspective");
  assert.deepEqual(trial.selection.nominal_pick_regions, [
    { ordinal: 1866, feature_id: "selected-point-1866-nominal-pick" },
    { ordinal: 1913, feature_id: "selected-point-1913-nominal-pick" },
  ]);
  assert.equal(trial.selection.point_identity_authority, "authored_source_fact");
  assert.equal(trial.selection.highlight_authority, "presentation_only");
  assert.deepEqual(
    trial.features.map(({ rectangle }) => rectangle.width * rectangle.height),
    [576, 576],
  );

  const tampered = structuredClone(corpus);
  tampered.trials[4].features[0].rectangle = { x: 0, y: 0, width: 640, height: 480 };
  assert.throws(() => validateVisualCorpus(tampered), /nominal pick region is too broad/);
});

test("condition coverage binds generated claims and every Autzen mode to executable facts", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));
  assert.deepEqual(
    corpus.condition_coverage.generated.map(({ condition }) => condition),
    REQUIRED_GENERATED_CONDITIONS,
  );
  assert.deepEqual(
    corpus.condition_coverage.derived_modes.map(({ display_mode: mode }) => mode),
    ["classification", "elevation", "intensity", "rgb"],
  );
  const mixedLod = corpus.condition_coverage.generated.find(({ condition }) => condition === "mixed_lod");
  assert.deepEqual(mixedLod.fact_paths, [
    "condition_facts.lod_relation",
    "condition_facts.stable_lod_cut",
  ]);
  assert.equal(mixedLod.required_temporal_trace, "mixed_lod_parent_child");

  const swapped = structuredClone(corpus);
  [swapped.condition_coverage.generated[0].fact_paths, swapped.condition_coverage.generated[1].fact_paths]
    = [swapped.condition_coverage.generated[1].fact_paths, swapped.condition_coverage.generated[0].fact_paths];
  assert.throws(() => validateVisualCorpus(swapped), /sparse fact mapping differs/);

  const omitted = structuredClone(corpus);
  omitted.condition_coverage.derived_modes.find(({ display_mode: mode }) => mode === "rgb").fact_paths.pop();
  assert.throws(() => validateVisualCorpus(omitted), /rgb fact mapping differs/);
});

test("materialization verifies generated digest and the licensed Autzen pvis", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));
  const generated = await materializeVisualTrial(corpus, "generated-rgb-hdr-perspective");
  assert.equal(generated.point_count, 2_103);
  assert.equal(generated.input_facts.payload_sha256, "dec11f50be83a59f8567440207d17eaea5650063ce88b73d56721f5614f3fd99");

  const manifest = await readFile(new URL("autzen-classified-sample.json", FIXTURE_DIRECTORY));
  const pvis = await readFile(new URL("autzen-classified-sample.pvis", FIXTURE_DIRECTORY));
  const fetchImplementation = async (url) => {
    if (String(url).endsWith("autzen-classified-sample.json")) {
      return new Response(manifest, { status: 200, headers: { "content-type": "application/json" } });
    }
    if (String(url).endsWith("autzen-classified-sample.pvis")) return new Response(pvis, { status: 200 });
    return new Response("missing", { status: 404 });
  };
  const derived = await materializeVisualTrial(corpus, "autzen-rgb-perspective", {
    corpusUrl: "https://fixtures.test/visual-v1/corpus.json",
    fetchImplementation,
  });
  assert.equal(derived.point_count, 4_096);
  assert.equal(derived.batches.length, 4);
  assert.equal(derived.batches.every((batch) => batch.byteLength === 32_768), true);
  assert.equal(derived.input_facts.permission.derivative_redistribution, true);
  assert.match(derived.input_facts.permission.modification_notice, /fixed Source-ordinal blocks/);
  assert.deepEqual(derived.input_facts.condition_facts.rgb_channel, { minimum: 7424, maximum: 61184 });

  const tampered = structuredClone(corpus);
  tampered.sources[0].payload_sha256 = "00".repeat(32);
  await assert.rejects(
    materializeVisualTrial(tampered, "generated-rgb-hdr-perspective"),
    /payload SHA-256 differs/,
  );
});

test("corpus loader uses one immutable URL and validates fetched JSON", async () => {
  const bytes = await readFile(CORPUS_URL);
  const calls = [];
  const loaded = await loadVisualCorpus("https://fixtures.test/visual-v1/corpus.json", {
    fetchImplementation: async (url, options) => {
      calls.push({ url, options });
      return new Response(bytes, { status: 200, headers: { "content-type": "application/json" } });
    },
  });
  assert.equal(loaded.corpus.schema, "punctra-browser-visual-corpus-v1");
  assert.equal(loaded.corpus_url, "https://fixtures.test/visual-v1/corpus.json");
  assert.deepEqual(calls, [{
    url: "https://fixtures.test/visual-v1/corpus.json",
    options: { cache: "no-store", credentials: "same-origin" },
  }]);
});

test("rubric template validation is complete, bound, and an explicit nonclaim", async () => {
  const corpus = validateVisualCorpus(await readJson(CORPUS_URL));
  const observation = {
    session_label: "maintainer-attended-1",
    answers: Object.fromEntries(corpus.rubric.prompts.map((prompt) => [prompt, {
      outcome: "not_observed",
      note: "",
      shown: false,
      trial_ids: [...corpus.rubric.trial_bindings[prompt]],
    }])),
  };
  assert.equal(validateRubricObservation(observation, corpus.rubric), observation);
  const incomplete = structuredClone(observation);
  delete incomplete.answers.shape;
  assert.throws(() => validateRubricObservation(incomplete, corpus.rubric), /incomplete/);
  const verbose = structuredClone(observation);
  verbose.answers.depth.note = "x".repeat(281);
  assert.throws(() => validateRubricObservation(verbose, corpus.rubric), /too long/);
  const unbound = structuredClone(observation);
  unbound.answers.depth.trial_ids = ["autzen-rgb-perspective"];
  assert.throws(() => validateRubricObservation(unbound, corpus.rubric), /trial binding depth differs/);
  const hiddenClaim = structuredClone(observation);
  hiddenClaim.answers.depth.outcome = "false_feature";
  assert.throws(() => validateRubricObservation(hiddenClaim, corpus.rubric), /must be not observed/);
});

async function readJson(url) {
  return JSON.parse(await readFile(url, "utf8"));
}

function concatenate(parts) {
  const result = new Uint8Array(parts.reduce((total, part) => total + part.byteLength, 0));
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.byteLength;
  }
  return result;
}
