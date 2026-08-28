import { createVisualValidator, parsePageUrl } from "./visual-validation.js";

export const VISUAL_VERIFIER_PATH = "scripts/verify-browser-visual-baseline.mjs";
export const VISUAL_TRUSTED_CONTROL_SCHEMA = "punctra-browser-trusted-control-activation-v1";
export const VISUAL_ATTENDED_LANE = Object.freeze({
  id: "codex-iab-chromium-151-macos-26-apple-m5-pro",
  execution: "browser_trusted_activation",
  qualification: "exact_observed_lane_only",
});

const IMPLEMENTATION_COMMIT = /^[0-9a-f]{40}$/;
const SHA256_HEX = /^[0-9a-f]{64}$/;
const POSITIVE_DECIMAL = /^[1-9][0-9]*$/;
const QUERY_FIELDS = Object.freeze([
  "implementation_commit",
  "verifier_byte_length",
  "verifier_sha256",
]);
const { requireCondition, requireRecord } = createVisualValidator("Visual provenance invalid");

export class VisualTrustedControlGate {
  #issued = new WeakSet();

  issue(event, {
    control,
    controlId,
    eventType = "click",
    visibilityState,
    userActivationIsActive = false,
    recordedAt = new Date().toISOString(),
  }) {
    requireRecord(event, "control event");
    requireCondition(event.type === eventType, `attended control must receive a ${eventType} event`);
    const eventIsTrusted = event.isTrusted === true;
    const transientUserActivation = userActivationIsActive === true;
    requireCondition(
      eventIsTrusted || transientUserActivation,
      "attended control requires a browser-trusted event or active transient user activation",
    );
    requireCondition(event.currentTarget === control, "attended control event target is invalid");
    requireCondition(visibilityState === "visible", "attended control requires a visible document");
    requireCondition(typeof controlId === "string" && controlId.length > 0, "attended control identity is invalid");
    requireCondition(!Number.isNaN(Date.parse(recordedAt)), "attended control timestamp is invalid");
    const activation = {
      evidence: Object.freeze({
        schema: VISUAL_TRUSTED_CONTROL_SCHEMA,
        control_id: controlId,
        event_type: eventType,
        trust_source: eventIsTrusted ? "event_is_trusted" : "transient_user_activation",
        event_is_trusted: eventIsTrusted,
        transient_user_activation: transientUserActivation,
        document_visibility_state: visibilityState,
        recorded_at: recordedAt,
      }),
    };
    this.#issued.add(activation);
    return activation;
  }

  consume(activation, controlId) {
    requireRecord(activation, "trusted control activation");
    requireCondition(this.#issued.delete(activation), "verify run requires a fresh trusted control activation");
    requireCondition(activation.evidence.control_id === controlId, "trusted control activation belongs to a different control");
    return { ...activation.evidence };
  }
}

export function validateTrustedControlActivationEvidence(value, { controlId, eventType }) {
  requireRecord(value, `${controlId} trusted control activation evidence`);
  requireCondition(value.schema === VISUAL_TRUSTED_CONTROL_SCHEMA, `${controlId} trusted control activation schema differs`);
  requireCondition(value.control_id === controlId, `${controlId} trusted control activation identity differs`);
  requireCondition(value.event_type === eventType, `${controlId} trusted control event type differs`);
  requireCondition(
    value.trust_source === "event_is_trusted" || value.trust_source === "transient_user_activation",
    `${controlId} trusted control activation source differs`,
  );
  requireCondition(typeof value.event_is_trusted === "boolean", `${controlId} event trust fact is invalid`);
  requireCondition(typeof value.transient_user_activation === "boolean", `${controlId} transient user activation fact is invalid`);
  requireCondition(
    value.event_is_trusted || value.transient_user_activation,
    `${controlId} control lacked browser trust and transient user activation`,
  );
  requireCondition(
    value.trust_source === (value.event_is_trusted ? "event_is_trusted" : "transient_user_activation"),
    `${controlId} trusted control activation source is inconsistent`,
  );
  requireCondition(value.document_visibility_state === "visible", `${controlId} control event was not visible`);
  requireCondition(typeof value.recorded_at === "string" && Number.isFinite(Date.parse(value.recorded_at)), `${controlId} control timestamp is invalid`);
  return value;
}

/** Reads the final implementation and verifier pins used by the visible verify button. */
export function visualVerifyProvenanceFromUrl(pageUrl) {
  const url = parsePageUrl(pageUrl, "Visual provenance invalid");
  const values = Object.fromEntries(QUERY_FIELDS.map((field) => [field, exactQueryValue(url, field)]));
  const presentCount = Object.values(values).filter((value) => value !== null).length;
  if (presentCount === 0) return null;
  requireCondition(presentCount === QUERY_FIELDS.length, "final verify provenance query is incomplete");
  requireCondition(IMPLEMENTATION_COMMIT.test(values.implementation_commit), "implementation commit query is invalid");
  requireCondition(POSITIVE_DECIMAL.test(values.verifier_byte_length), "verifier byte length query is invalid");
  const byteLength = Number(values.verifier_byte_length);
  requireCondition(Number.isSafeInteger(byteLength), "verifier byte length query exceeds the exact integer range");
  requireCondition(SHA256_HEX.test(values.verifier_sha256), "verifier SHA-256 query is invalid");
  return {
    implementation_commit: values.implementation_commit,
    verifier: {
      path: VISUAL_VERIFIER_PATH,
      byte_length: byteLength,
      sha256: values.verifier_sha256,
    },
  };
}

function exactQueryValue(url, field) {
  const values = url.searchParams.getAll(field);
  requireCondition(values.length <= 1, `${field} query must not be repeated`);
  return values[0] ?? null;
}
