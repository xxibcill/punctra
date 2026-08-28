import { parseRawJson } from "./visual-capture.js";
import { createVisualValidator } from "./visual-validation.js";

export const NOMINAL_PICK_EVIDENCE_SCHEMA = "punctra-browser-nominal-pick-evidence-v1";
export const NOMINAL_PICK_POLL_FRAME_CEILING = 180;
export const NOMINAL_PICK_ATTEMPT_CEILING = 1_024;

const { requireCondition, requireRecord } = createVisualValidator("Visual selection invalid");

export async function verifyNominalPickCoverage(rawViewer, trial, expectations, options = {}) {
  if (trial.selection.ordinals.length === 0) {
    requireCondition(expectations.length === 0, `trial ${trial.id} unexpectedly supplied nominal-pick expectations`);
    return null;
  }
  requireCondition(expectations.length === trial.selection.ordinals.length, `trial ${trial.id} nominal-pick expectation count differs`);
  const before = parseRawJson(rawViewer.diagnostics(), `trial ${trial.id} pre-pick diagnostics`);
  requireCondition(before.highlights.point_count === 0, `trial ${trial.id} nominal picks followed decorative highlights`);
  const rendered = parseRawJson(rawViewer.render(), `trial ${trial.id} nominal-pick frame diagnostics`);
  requireCondition(rendered.highlights.point_count === 0, `trial ${trial.id} nominal-pick frame contains decorative highlights`);

  const requestFrame = options.requestFrame ?? browserAnimationFrame;
  const pollFrameCeiling = options.pollFrameCeiling ?? NOMINAL_PICK_POLL_FRAME_CEILING;
  const checks = [];
  for (const expectation of expectations) {
    validateExpectation(expectation, trial);
    const candidatePixels = nominalPickPixels(expectation.expected_pixel, expectation.nominal_region);
    requireCondition(candidatePixels.length <= NOMINAL_PICK_ATTEMPT_CEILING, `trial ${trial.id} nominal-pick region exceeds its attempt ceiling`);
    const attempts = [];
    for (const pixel of candidatePixels) {
      const attempt = await performPick(rawViewer, trial, expectation, pixel, {
        requestFrame,
        pollFrameCeiling,
      });
      attempts.push(attempt);
      if (attempt.matched) break;
    }
    const matched = attempts.at(-1);
    requireCondition(matched?.matched === true, `trial ${trial.id} Point ${expectation.ordinal} was not pickable in its authored region`);
    checks.push({
      ordinal: expectation.ordinal,
      feature_id: expectation.feature_id,
      expected_pixel: [...expectation.expected_pixel],
      nominal_region: { ...expectation.nominal_region },
      expected: {
        generation: expectation.generation,
        batch_key: expectation.batch_key,
        batch_version: expectation.batch_version,
        source_identity: expectation.source_identity,
        point_ordinal: String(expectation.ordinal),
      },
      matched_pixel: [...matched.pixel],
      attempt_count: attempts.length,
      poll_frames_total: attempts.reduce((total, attempt) => total + attempt.poll_frames, 0),
      attempts,
      passed: true,
    });
  }

  return {
    schema: NOMINAL_PICK_EVIDENCE_SCHEMA,
    gating: true,
    execution_order: "before_presentation_only_highlights",
    point_identity_authority: trial.selection.point_identity_authority,
    nominal_pick_coverage_authority: trial.selection.nominal_pick_coverage_authority,
    pick_authority: "provisional_gpu_hint",
    highlight_authority: trial.selection.highlight_authority,
    highlight_point_count_during_checks: 0,
    poll_frame_ceiling: pollFrameCeiling,
    attempt_ceiling_per_region: NOMINAL_PICK_ATTEMPT_CEILING,
    checks,
    passed: true,
  };
}

function validateExpectation(expectation, trial) {
  requireRecord(expectation, `trial ${trial.id} nominal-pick expectation`);
  requireCondition(trial.selection.ordinals.includes(expectation.ordinal), `trial ${trial.id} nominal-pick ordinal differs`);
  requireCondition(Array.isArray(expectation.expected_pixel) && expectation.expected_pixel.length === 2, `trial ${trial.id} nominal-pick pixel differs`);
  requireCondition(expectation.expected_pixel.every((value) => Number.isInteger(value) && value >= 0), `trial ${trial.id} nominal-pick pixel is invalid`);
  requireRecord(expectation.nominal_region, `trial ${trial.id} nominal-pick region`);
  const [x, y] = expectation.expected_pixel;
  requireCondition(
    x >= expectation.nominal_region.x
      && y >= expectation.nominal_region.y
      && x < expectation.nominal_region.x + expectation.nominal_region.width
      && y < expectation.nominal_region.y + expectation.nominal_region.height,
    `trial ${trial.id} nominal-pick pixel lies outside its authored region`,
  );
}

async function performPick(rawViewer, trial, expectation, pixel, options) {
  const [x, y] = pixel;
  const pending = parseRawJson(
    rawViewer.beginPick(x, y),
    `trial ${trial.id} Point ${expectation.ordinal} begin-pick diagnostics`,
  );
  requireCondition(pending.pick.status === "pending", `trial ${trial.id} Point ${expectation.ordinal} pick did not become pending`);
  let observed;
  let pollFrames = 0;
  for (; pollFrames < options.pollFrameCeiling; pollFrames += 1) {
    await options.requestFrame();
    const diagnostics = parseRawJson(
      rawViewer.pollPick(),
      `trial ${trial.id} Point ${expectation.ordinal} poll-pick diagnostics`,
    );
    requireCondition(diagnostics.highlights.point_count === 0, `trial ${trial.id} Point ${expectation.ordinal} pick overlapped decorative highlights`);
    if (diagnostics.pick.status === "pending") continue;
    requireCondition(diagnostics.pick.status === "hit" || diagnostics.pick.status === "miss", `trial ${trial.id} Point ${expectation.ordinal} pick status differs`);
    observed = diagnostics.pick;
    pollFrames += 1;
    break;
  }
  const cancelled = parseRawJson(
    rawViewer.cancelPick(),
    `trial ${trial.id} Point ${expectation.ordinal} pick cleanup diagnostics`,
  );
  requireCondition(cancelled.pick.status === "not_requested", `trial ${trial.id} Point ${expectation.ordinal} pick cleanup differs`);
  requireCondition(observed !== undefined, `trial ${trial.id} Point ${expectation.ordinal} pick exceeded its poll ceiling`);
  return {
    pixel: [...pixel],
    observed: { ...observed },
    poll_frames: pollFrames,
    matched: observedPickMatches(observed, expectation),
  };
}

function observedPickMatches(observed, expectation) {
  return observed.status === "hit"
    && observed.authority === "provisional_gpu_hint"
    && observed.generation === expectation.generation
    && observed.batch_key === expectation.batch_key
    && observed.batch_version === expectation.batch_version
    && observed.source_identity === expectation.source_identity
    && observed.point_ordinal === String(expectation.ordinal);
}

export function nominalPickPixels(expectedPixel, region) {
  const pixels = [];
  for (let y = region.y; y < region.y + region.height; y += 1) {
    for (let x = region.x; x < region.x + region.width; x += 1) pixels.push([x, y]);
  }
  return pixels.sort((left, right) => {
    const leftDistance = squaredDistance(left, expectedPixel);
    const rightDistance = squaredDistance(right, expectedPixel);
    return leftDistance - rightDistance || left[1] - right[1] || left[0] - right[0];
  });
}

function squaredDistance(left, right) {
  return (left[0] - right[0]) ** 2 + (left[1] - right[1]) ** 2;
}

function browserAnimationFrame() {
  return new Promise((resolve) => globalThis.requestAnimationFrame(resolve));
}
