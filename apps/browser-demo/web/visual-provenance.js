import { createVisualValidator, parsePageUrl } from "./visual-validation.js";

export const VISUAL_VERIFIER_PATH = "scripts/verify-browser-visual-baseline.mjs";
export const VISUAL_ATTENDED_LANE = Object.freeze({
  id: "codex-iab-chromium-151-macos-26-apple-m5-pro",
  execution: "visible_user_gesture",
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
const { requireCondition } = createVisualValidator("Visual provenance invalid");

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
    attended_lane: { ...VISUAL_ATTENDED_LANE },
  };
}

function exactQueryValue(url, field) {
  const values = url.searchParams.getAll(field);
  requireCondition(values.length <= 1, `${field} query must not be repeated`);
  return values[0] ?? null;
}
