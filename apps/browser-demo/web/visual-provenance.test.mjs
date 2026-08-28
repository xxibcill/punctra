import assert from "node:assert/strict";
import test from "node:test";

import {
  VISUAL_TRUSTED_CONTROL_SCHEMA,
  VISUAL_VERIFIER_PATH,
  VisualTrustedControlGate,
  visualVerifyProvenanceFromUrl,
} from "./visual-provenance.js";

const COMMIT = "1".repeat(40);
const SHA256 = "a".repeat(64);

test("final verify provenance is absent until all pins are explicitly supplied", () => {
  assert.equal(
    visualVerifyProvenanceFromUrl("http://127.0.0.1:8000/visual.html?mode=verify"),
    null,
  );
  assert.throws(
    () => visualVerifyProvenanceFromUrl(
      `http://127.0.0.1:8000/visual.html?implementation_commit=${COMMIT}`,
    ),
    /query is incomplete/,
  );
});

test("final verify provenance binds only the qualified implementation and verifier pins", () => {
  const url = new URL("http://127.0.0.1:8000/visual.html?mode=verify");
  url.searchParams.set("implementation_commit", COMMIT);
  url.searchParams.set("verifier_byte_length", "140956");
  url.searchParams.set("verifier_sha256", SHA256);
  assert.deepEqual(visualVerifyProvenanceFromUrl(url.href), {
    implementation_commit: COMMIT,
    verifier: {
      path: VISUAL_VERIFIER_PATH,
      byte_length: 140956,
      sha256: SHA256,
    },
  });
});

test("trusted control activations are visible, browser-issued, and single use", () => {
  const gate = new VisualTrustedControlGate();
  const control = {};
  const event = {
    type: "click",
    isTrusted: true,
    currentTarget: control,
  };
  const activation = gate.issue(event, {
    control,
    controlId: "run-corpus",
    visibilityState: "visible",
    recordedAt: "2026-08-28T08:00:00.000Z",
  });
  assert.deepEqual(gate.consume(activation, "run-corpus"), {
    schema: VISUAL_TRUSTED_CONTROL_SCHEMA,
    control_id: "run-corpus",
    event_type: "click",
    trust_source: "event_is_trusted",
    event_is_trusted: true,
    transient_user_activation: false,
    document_visibility_state: "visible",
    recorded_at: "2026-08-28T08:00:00.000Z",
  });
  assert.throws(() => gate.consume(activation, "run-corpus"), /fresh trusted control activation/);
});

test("transient user activation supplies browser trust when the control event is synthetic", () => {
  const gate = new VisualTrustedControlGate();
  const control = {};
  const activation = gate.issue({ type: "click", isTrusted: false, currentTarget: control }, {
    control,
    controlId: "run-corpus",
    visibilityState: "visible",
    userActivationIsActive: true,
    recordedAt: "2026-08-28T08:00:00.000Z",
  });
  assert.deepEqual(gate.consume(activation, "run-corpus"), {
    schema: VISUAL_TRUSTED_CONTROL_SCHEMA,
    control_id: "run-corpus",
    event_type: "click",
    trust_source: "transient_user_activation",
    event_is_trusted: false,
    transient_user_activation: true,
    document_visibility_state: "visible",
    recorded_at: "2026-08-28T08:00:00.000Z",
  });
});

test("synthetic inactive and hidden-document control activations are rejected", () => {
  const gate = new VisualTrustedControlGate();
  const control = {};
  assert.throws(() => gate.issue({ type: "click", isTrusted: false, currentTarget: control }, {
    control,
    controlId: "run-corpus",
    visibilityState: "visible",
  }), /browser-trusted event or active transient user activation/);
  assert.throws(() => gate.issue({ type: "click", isTrusted: true, currentTarget: control }, {
    control,
    controlId: "run-corpus",
    visibilityState: "hidden",
  }), /visible document/);
});

test("final verify provenance rejects malformed, unsafe, and repeated pins", () => {
  const valid = `implementation_commit=${COMMIT}&verifier_byte_length=140956&verifier_sha256=${SHA256}`;
  assert.throws(
    () => visualVerifyProvenanceFromUrl(`http://127.0.0.1:8000/visual.html?${valid}&verifier_sha256=${SHA256}`),
    /must not be repeated/,
  );
  assert.throws(
    () => visualVerifyProvenanceFromUrl(`http://127.0.0.1:8000/visual.html?${valid.replace(COMMIT, "ABC")}`),
    /implementation commit query is invalid/,
  );
  assert.throws(
    () => visualVerifyProvenanceFromUrl(`http://127.0.0.1:8000/visual.html?${valid.replace("140956", "01")}`),
    /verifier byte length query is invalid/,
  );
  assert.throws(
    () => visualVerifyProvenanceFromUrl(`http://127.0.0.1:8000/visual.html?${valid.replace(SHA256, "f")}`),
    /verifier SHA-256 query is invalid/,
  );
});
