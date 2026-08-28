export const RUBRIC_REVIEW_PLAN_SCHEMA = "punctra-browser-visual-rubric-review-plan-v1";
export const RUBRIC_PRESENTATION_SCHEMA = "punctra-browser-visual-rubric-presentation-v1";

const ARTIFACT_IDENTITY_FIELDS = Object.freeze([
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
]);

/**
 * Binds every rubric prompt to one already-recorded final capture per trial.
 * The first recreation is fixed so the attended UI and offline verifier cannot
 * silently choose a more favorable recreation.
 */
export function createRubricReviewPlan(policy, trialResults, registryArtifacts) {
  validatePolicy(policy);
  requireCondition(Array.isArray(trialResults), "rubric trial results must be an array");
  requireCondition(Array.isArray(registryArtifacts), "rubric artifact registry must be an array");
  const trials = new Map(trialResults.map((trial) => [trial.trial_id, trial]));
  requireCondition(trials.size === trialResults.length, "rubric trial results contain duplicate identities");
  const registry = new Map(registryArtifacts.map((artifact) => [artifact.path, artifact]));
  requireCondition(registry.size === registryArtifacts.length, "rubric artifact registry contains duplicate paths");

  const prompts = {};
  for (const prompt of policy.prompts) {
    const trialIds = policy.trial_bindings[prompt];
    const artifacts = trialIds.map((trialId) => {
      const trial = trials.get(trialId);
      requireCondition(trial !== undefined, `rubric ${prompt} trial ${trialId} is absent`);
      requireCondition(trial.recreations?.length === 3, `rubric ${prompt} trial ${trialId} did not complete three recreations`);
      const artifact = trial.recreations?.[0]?.capture?.artifact;
      requireRecord(artifact, `rubric ${prompt} trial ${trialId} final capture`);
      requireCondition(artifact.trial_id === trialId, `rubric ${prompt} capture trial identity differs`);
      requireCondition(artifact.recreation_index === 0, `rubric ${prompt} capture recreation differs`);
      const registered = registry.get(artifact.path);
      requireRecord(registered, `rubric ${prompt} registry artifact ${artifact.path}`);
      const identity = artifactIdentity(artifact);
      requireCondition(deepEqual(identity, artifactIdentity(registered)), `rubric ${prompt} artifact identity differs from the registry`);
      return identity;
    });
    prompts[prompt] = {
      trial_ids: [...trialIds],
      artifact_paths: artifacts.map(({ path }) => path),
      artifact_identities: artifacts,
    };
  }
  return {
    schema: RUBRIC_REVIEW_PLAN_SCHEMA,
    prompts,
  };
}

/** Builds the immutable attended observation after every bound image loaded. */
export function buildRubricObservation(options) {
  const {
    policy,
    plan,
    captureCompletedAt,
    submittedAt,
    sessionLabel,
    answers,
  } = options;
  validatePolicy(policy);
  validateReviewPlan(plan, policy);
  requireIsoTimestamp(captureCompletedAt, "rubric capture-completed timestamp");
  requireIsoTimestamp(submittedAt, "rubric submitted timestamp");
  requireCondition(compareTimestamps(captureCompletedAt, submittedAt) <= 0, "rubric submission predates capture completion");
  requireCondition(typeof sessionLabel === "string" && sessionLabel.length >= 1 && sessionLabel.length <= 64, "rubric session label is invalid");
  requireRecord(answers, "rubric attended answers");
  requireCondition(sameMembers(Object.keys(answers), policy.prompts), "rubric attended answers are incomplete");

  const observation = {
    session_label: sessionLabel,
    capture_completed_at: captureCompletedAt,
    submitted_at: submittedAt,
    answers: {},
  };
  const presentationOrders = new Set();
  const loadOrders = new Set();
  const selectionOrders = new Set();
  for (const prompt of policy.prompts) {
    const input = answers[prompt];
    requireRecord(input, `rubric attended answer ${prompt}`);
    const planned = plan.prompts[prompt];
    requireCondition(policy.outcomes.includes(input.outcome), `rubric outcome ${prompt} is invalid`);
    requireCondition(typeof input.note === "string" && input.note.length <= policy.note_character_limit, `rubric note ${prompt} is too long`);
    requireIsoTimestamp(input.selected_at, `rubric ${prompt} selected timestamp`);
    requirePositiveInteger(input.selection_order, `rubric ${prompt} selection order`);
    requireUnique(selectionOrders, input.selection_order, `rubric ${prompt} selection order`);
    validatePresentation(input.presentation, planned, {
      captureCompletedAt,
      submittedAt,
      presentationOrders,
      loadOrders,
    });
    requireCondition(compareTimestamps(input.presentation.presented_at, input.selected_at) <= 0, `rubric ${prompt} selection predates presentation`);
    requireCondition(compareTimestamps(input.selected_at, submittedAt) <= 0, `rubric ${prompt} selection follows submission`);
    observation.answers[prompt] = {
      outcome: input.outcome,
      note: input.note,
      shown: true,
      trial_ids: [...planned.trial_ids],
      artifact_paths: [...planned.artifact_paths],
      artifact_identities: cloneJson(planned.artifact_identities),
      presentation: cloneJson(input.presentation),
      selected_at: input.selected_at,
      selection_order: input.selection_order,
    };
  }
  return validateRubricEvidenceShape(observation, policy);
}

/** Produces an explicit nonclaim for a template or a run that never reached review. */
export function createUnobservedRubricObservation(policy, options = {}) {
  validatePolicy(policy);
  const sessionLabel = options.sessionLabel ?? "unavailable";
  const note = options.note ?? "Run ended before a valid attended observation was recorded.";
  requireCondition(typeof sessionLabel === "string" && sessionLabel.length >= 1 && sessionLabel.length <= 64, "rubric session label is invalid");
  requireCondition(typeof note === "string" && note.length <= policy.note_character_limit, "rubric fallback note is too long");
  return {
    session_label: sessionLabel,
    answers: Object.fromEntries(policy.prompts.map((prompt) => [prompt, {
      outcome: "not_observed",
      note,
      shown: false,
      trial_ids: [...policy.trial_bindings[prompt]],
    }])),
  };
}

/** Validates both the checked-in empty template and post-capture attended evidence. */
export function validateRubricEvidenceShape(value, policy) {
  validatePolicy(policy);
  requireRecord(value, "rubric observation");
  requireCondition(typeof value.session_label === "string" && value.session_label.length >= 1 && value.session_label.length <= 64, "rubric session label is invalid");
  requireRecord(value.answers, "rubric answers");
  requireCondition(sameMembers(Object.keys(value.answers), policy.prompts), "rubric answers are incomplete");
  const shownAnswers = policy.prompts.filter((prompt) => value.answers[prompt]?.shown === true);
  const attended = shownAnswers.length > 0;
  requireCondition(!attended || shownAnswers.length === policy.prompts.length, "rubric attended observation is only partially shown");
  if (attended) {
    requireIsoTimestamp(value.capture_completed_at, "rubric capture-completed timestamp");
    requireIsoTimestamp(value.submitted_at, "rubric submitted timestamp");
    requireCondition(compareTimestamps(value.capture_completed_at, value.submitted_at) <= 0, "rubric submission predates capture completion");
  }
  const presentationOrders = new Set();
  const loadOrders = new Set();
  const selectionOrders = new Set();
  for (const prompt of policy.prompts) {
    const answer = value.answers[prompt];
    requireRecord(answer, `rubric answer ${prompt}`);
    requireCondition(policy.outcomes.includes(answer.outcome), `rubric outcome ${prompt} is invalid`);
    requireCondition(typeof answer.note === "string" && answer.note.length <= policy.note_character_limit, `rubric note ${prompt} is too long`);
    requireCondition(deepEqual(answer.trial_ids, policy.trial_bindings[prompt]), `rubric trial binding ${prompt} differs`);
    if (!answer.shown) {
      requireCondition(answer.outcome === "not_observed", `rubric hidden outcome ${prompt} must be not observed`);
      continue;
    }
    const plan = {
      trial_ids: answer.trial_ids,
      artifact_paths: answer.artifact_paths,
      artifact_identities: answer.artifact_identities,
    };
    validatePlannedPrompt(plan, prompt);
    requireCondition(plan.artifact_identities.every((artifact, index) => artifact.trial_id === plan.trial_ids[index]), `rubric ${prompt} artifact-to-trial binding differs`);
    validatePresentation(answer.presentation, plan, {
      captureCompletedAt: value.capture_completed_at,
      submittedAt: value.submitted_at,
      presentationOrders,
      loadOrders,
    });
    requireIsoTimestamp(answer.selected_at, `rubric ${prompt} selected timestamp`);
    requirePositiveInteger(answer.selection_order, `rubric ${prompt} selection order`);
    requireUnique(selectionOrders, answer.selection_order, `rubric ${prompt} selection order`);
    requireCondition(compareTimestamps(answer.presentation.presented_at, answer.selected_at) <= 0, `rubric ${prompt} selection predates presentation`);
    requireCondition(compareTimestamps(answer.selected_at, value.submitted_at) <= 0, `rubric ${prompt} selection follows submission`);
  }
  return value;
}

export function artifactIdentity(artifact) {
  requireRecord(artifact, "rubric artifact");
  const identity = {};
  for (const field of ARTIFACT_IDENTITY_FIELDS) {
    requireCondition(Object.hasOwn(artifact, field), `rubric artifact ${field} is absent`);
    identity[field] = artifact[field];
  }
  requireCondition(typeof identity.path === "string" && identity.path.length > 0, "rubric artifact path is invalid");
  requireCondition(/^[0-9a-f]{64}$/.test(identity.encoded_sha256), "rubric artifact encoded SHA-256 is invalid");
  requireCondition(/^[0-9a-f]{64}$/.test(identity.decoded_sha256), "rubric artifact decoded SHA-256 is invalid");
  for (const field of ["width", "height", "encoded_byte_length", "decoded_byte_length"]) {
    requireCondition(Number.isSafeInteger(identity[field]) && identity[field] > 0, `rubric artifact ${field} is invalid`);
  }
  return identity;
}

function validateReviewPlan(plan, policy) {
  requireRecord(plan, "rubric review plan");
  requireCondition(plan.schema === RUBRIC_REVIEW_PLAN_SCHEMA, "rubric review-plan schema differs");
  requireRecord(plan.prompts, "rubric review-plan prompts");
  requireCondition(sameMembers(Object.keys(plan.prompts), policy.prompts), "rubric review-plan prompts differ");
  for (const prompt of policy.prompts) {
    const planned = plan.prompts[prompt];
    validatePlannedPrompt(planned, prompt);
    requireCondition(deepEqual(planned.trial_ids, policy.trial_bindings[prompt]), `rubric review-plan ${prompt} trial bindings differ`);
  }
}

function validatePlannedPrompt(planned, prompt) {
  requireRecord(planned, `rubric review plan ${prompt}`);
  requireCondition(Array.isArray(planned.trial_ids) && planned.trial_ids.length > 0, `rubric review-plan ${prompt} trials are absent`);
  requireCondition(Array.isArray(planned.artifact_paths) && planned.artifact_paths.length === planned.trial_ids.length, `rubric review-plan ${prompt} paths differ`);
  requireCondition(Array.isArray(planned.artifact_identities) && planned.artifact_identities.length === planned.trial_ids.length, `rubric review-plan ${prompt} identities differ`);
  const identities = planned.artifact_identities.map(artifactIdentity);
  requireCondition(deepEqual(planned.artifact_paths, identities.map(({ path }) => path)), `rubric review-plan ${prompt} path identities differ`);
}

function validatePresentation(presentation, planned, options) {
  requireRecord(presentation, "rubric presentation");
  requireCondition(presentation.schema === RUBRIC_PRESENTATION_SCHEMA, "rubric presentation schema differs");
  requireIsoTimestamp(presentation.presented_at, "rubric presented timestamp");
  requirePositiveInteger(presentation.presentation_order, "rubric presentation order");
  requireUnique(options.presentationOrders, presentation.presentation_order, "rubric presentation order");
  requireCondition(presentation.document_visibility_state === "visible", "rubric presentation document was not visible");
  requireCondition(Array.isArray(presentation.artifacts) && presentation.artifacts.length === planned.trial_ids.length, "rubric presentation artifact count differs");
  for (let index = 0; index < presentation.artifacts.length; index += 1) {
    const loaded = presentation.artifacts[index];
    requireRecord(loaded, "rubric loaded artifact");
    requireCondition(loaded.trial_id === planned.trial_ids[index], "rubric loaded artifact trial differs");
    requireCondition(loaded.path === planned.artifact_paths[index], "rubric loaded artifact path differs");
    requireIsoTimestamp(loaded.loaded_at, "rubric artifact loaded timestamp");
    requirePositiveInteger(loaded.load_order, "rubric artifact load order");
    requireUnique(options.loadOrders, loaded.load_order, "rubric artifact load order");
    requireCondition(loaded.natural_width === planned.artifact_identities[index].width, "rubric loaded artifact width differs");
    requireCondition(loaded.natural_height === planned.artifact_identities[index].height, "rubric loaded artifact height differs");
    requireCondition(loaded.complete === true, "rubric artifact load did not complete");
    requireCondition(compareTimestamps(options.captureCompletedAt, loaded.loaded_at) <= 0, "rubric artifact loaded before capture completion");
    requireCondition(compareTimestamps(loaded.loaded_at, presentation.presented_at) <= 0, "rubric presentation predates artifact load");
  }
  requireCondition(compareTimestamps(presentation.presented_at, options.submittedAt) <= 0, "rubric presentation follows submission");
}

function validatePolicy(policy) {
  requireRecord(policy, "rubric policy");
  requireCondition(Array.isArray(policy.prompts) && policy.prompts.length > 0, "rubric policy prompts are absent");
  requireCondition(Array.isArray(policy.outcomes) && policy.outcomes.includes("not_observed"), "rubric policy outcomes differ");
  requireCondition(Number.isSafeInteger(policy.note_character_limit) && policy.note_character_limit > 0, "rubric policy note limit differs");
  requireRecord(policy.trial_bindings, "rubric policy trial bindings");
}

function requireIsoTimestamp(value, label) {
  requireCondition(typeof value === "string" && Number.isFinite(Date.parse(value)), `${label} is invalid`);
}

function compareTimestamps(left, right) {
  return Date.parse(left) - Date.parse(right);
}

function requirePositiveInteger(value, label) {
  requireCondition(Number.isSafeInteger(value) && value >= 1, `${label} is invalid`);
}

function requireUnique(values, value, label) {
  if (values === undefined) return;
  requireCondition(!values.has(value), `${label} is duplicated`);
  values.add(value);
}

function sameMembers(left, right) {
  return Array.isArray(left) && Array.isArray(right)
    && left.length === right.length
    && [...left].sort().every((value, index) => value === [...right].sort()[index]);
}

function cloneJson(value) {
  return JSON.parse(JSON.stringify(value));
}

function deepEqual(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function requireRecord(value, label) {
  requireCondition(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
}

function requireCondition(condition, message) {
  if (!condition) throw new Error(`Visual rubric invalid: ${message}`);
}
