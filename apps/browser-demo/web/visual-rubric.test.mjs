import assert from "node:assert/strict";
import test from "node:test";

import {
  RUBRIC_PRESENTATION_SCHEMA,
  buildRubricObservation,
  createRubricReviewPlan,
  createUnobservedRubricObservation,
  validateRubricEvidenceShape,
} from "./visual-rubric.js";

const policy = Object.freeze({
  prompts: ["depth", "shape"],
  outcomes: ["clear", "ambiguous", "not_observed"],
  note_character_limit: 280,
  trial_bindings: {
    depth: ["trial-a"],
    shape: ["trial-b", "trial-a"],
  },
});

test("review plan fixes each prompt to recreation-zero registry identities", () => {
  const { trials, artifacts } = evidenceInputs();
  const plan = createRubricReviewPlan(policy, trials, artifacts);
  assert.deepEqual(plan.prompts.depth.artifact_paths, ["artifacts/trial-a.png"]);
  assert.deepEqual(plan.prompts.shape.artifact_paths, [
    "artifacts/trial-b.png",
    "artifacts/trial-a.png",
  ]);
  const moved = structuredClone(trials);
  moved[0].recreations[0].capture.artifact.path = "artifacts/moved.png";
  assert.throws(
    () => createRubricReviewPlan(policy, moved, artifacts),
    /registry artifact artifacts\/moved\.png must be an object/,
  );
  const wrongIdentity = structuredClone(artifacts);
  wrongIdentity[0].encoded_sha256 = "f".repeat(64);
  assert.throws(
    () => createRubricReviewPlan(policy, trials, wrongIdentity),
    /artifact identity differs from the registry/,
  );
});

test("attended observation binds loaded images and selections after capture", () => {
  const { trials, artifacts } = evidenceInputs();
  const plan = createRubricReviewPlan(policy, trials, artifacts);
  const observation = buildRubricObservation({
    policy,
    plan,
    captureCompletedAt: "2026-08-28T01:00:00.000Z",
    submittedAt: "2026-08-28T01:00:04.000Z",
    submission: trustedActivation("submit-rubric", "click", "2026-08-28T01:00:03.750Z"),
    sessionLabel: "maintainer-attended-1",
    answers: attendedAnswers(plan),
    requireTrustedControls: true,
  });
  assert.equal(observation.answers.depth.shown, true);
  assert.equal(observation.answers.depth.outcome, "not_observed");
  assert.deepEqual(
    observation.answers.shape.artifact_identities,
    plan.prompts.shape.artifact_identities,
  );
  assert.equal(validateRubricEvidenceShape(observation, policy), observation);

  const preCapture = attendedAnswers(plan);
  preCapture.depth.presentation.artifacts[0].loaded_at = "2026-08-28T00:59:59.999Z";
  assert.throws(
    () => buildRubricObservation({
      policy,
      plan,
      captureCompletedAt: "2026-08-28T01:00:00.000Z",
      submittedAt: "2026-08-28T01:00:04.000Z",
      submission: trustedActivation("submit-rubric", "click", "2026-08-28T01:00:03.750Z"),
      sessionLabel: "attended",
      answers: preCapture,
      requireTrustedControls: true,
    }),
    /loaded before capture completion/,
  );
});

test("final attended observations reject missing trusted selection and submit events", () => {
  const { trials, artifacts } = evidenceInputs();
  const plan = createRubricReviewPlan(policy, trials, artifacts);
  const options = {
    policy,
    plan,
    captureCompletedAt: "2026-08-28T01:00:00.000Z",
    submittedAt: "2026-08-28T01:00:04.000Z",
    submission: trustedActivation("submit-rubric", "click", "2026-08-28T01:00:03.750Z"),
    sessionLabel: "attended",
    answers: attendedAnswers(plan),
    requireTrustedControls: true,
  };
  const missingSelection = structuredClone(options);
  missingSelection.answers.depth.selection_activation = null;
  assert.throws(() => buildRubricObservation(missingSelection), /trusted selection event/);
  assert.throws(
    () => buildRubricObservation({ ...options, submission: null }),
    /trusted control activation evidence must be an object/,
  );
});

test("shown cannot be flipped without receipts and hidden answers are nonclaims", () => {
  const fallback = createUnobservedRubricObservation(policy);
  assert.equal(validateRubricEvidenceShape(fallback, policy), fallback);
  const favorableHidden = structuredClone(fallback);
  favorableHidden.answers.depth.outcome = "clear";
  assert.throws(() => validateRubricEvidenceShape(favorableHidden, policy), /must be not observed/);

  const shownFlip = structuredClone(fallback);
  shownFlip.answers.depth.shown = true;
  assert.throws(() => validateRubricEvidenceShape(shownFlip, policy), /only partially shown/);
});

function attendedAnswers(plan) {
  let loadOrder = 0;
  return Object.fromEntries(policy.prompts.map((prompt, promptIndex) => {
    const planned = plan.prompts[prompt];
    const presentation = {
      schema: RUBRIC_PRESENTATION_SCHEMA,
      presented_at: `2026-08-28T01:00:0${promptIndex + 2}.000Z`,
      presentation_order: promptIndex + 1,
      document_visibility_state: "visible",
      artifacts: planned.artifact_identities.map((artifact, artifactIndex) => ({
        trial_id: planned.trial_ids[artifactIndex],
        path: artifact.path,
        loaded_at: "2026-08-28T01:00:01.000Z",
        load_order: ++loadOrder,
        natural_width: artifact.width,
        natural_height: artifact.height,
        complete: true,
      })),
    };
    return [prompt, {
      outcome: prompt === "depth" ? "not_observed" : "clear",
      note: "",
      presentation,
      selected_at: `2026-08-28T01:00:0${promptIndex + 2}.500Z`,
      selection_order: promptIndex + 1,
      selection_activation: trustedActivation(
        `rubric-${prompt}`,
        "change",
        `2026-08-28T01:00:0${promptIndex + 2}.500Z`,
      ),
    }];
  }));
}

function trustedActivation(controlId, eventType, recordedAt) {
  return {
    schema: "punctra-browser-trusted-control-activation-v1",
    control_id: controlId,
    event_type: eventType,
    trust_source: "transient_user_activation",
    event_is_trusted: true,
    transient_user_activation: true,
    document_visibility_state: "visible",
    recorded_at: recordedAt,
  };
}

function evidenceInputs() {
  const artifacts = [artifact("trial-a", "a"), artifact("trial-b", "b")];
  return {
    artifacts,
    trials: artifacts.map((entry) => ({
      trial_id: entry.trial_id,
      recreations: Array.from({ length: 3 }, () => ({ capture: { artifact: structuredClone(entry) } })),
    })),
  };
}

function artifact(trialId, digestCharacter) {
  return {
    kind: "recreation_png",
    trial_id: trialId,
    recreation_index: 0,
    frame_index: 29,
    path: `artifacts/${trialId}.png`,
    width: 640,
    height: 480,
    encoded_byte_length: 1_000,
    encoded_sha256: digestCharacter.repeat(64),
    decoded_byte_length: 1_228_800,
    decoded_sha256: digestCharacter.repeat(64),
  };
}
