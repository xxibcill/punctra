import assert from "node:assert/strict";
import test from "node:test";

import {
  VISUAL_TRUSTED_CONTROL_SCHEMA,
  VISUAL_VERIFIER_PATH,
  VisualTrustedControlGate,
  loadVisualVerifyProvenance,
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

test("final verify provenance must match the running checkout and verifier bytes", async () => {
  const pageUrl = new URL("http://127.0.0.1:8000/visual.html?mode=verify");
  pageUrl.searchParams.set("implementation_commit", COMMIT);
  pageUrl.searchParams.set("verifier_byte_length", "140956");
  pageUrl.searchParams.set("verifier_sha256", SHA256);
  const expected = {
    schema: "punctra-browser-visual-verify-pins-v1",
    accepted: {
      implementation_commit: COMMIT,
      verifier: {
        path: VISUAL_VERIFIER_PATH,
        byte_length: 140956,
        sha256: SHA256,
      },
    },
    running: {
      implementation_commit: COMMIT,
      verifier: {
        path: VISUAL_VERIFIER_PATH,
        byte_length: 140956,
        sha256: SHA256,
      },
    },
  };
  const requestedVerifier = {
      path: VISUAL_VERIFIER_PATH,
      byte_length: 140956,
      sha256: SHA256,
  };
  const requests = [];
  const fetchPins = async (url, init) => {
    requests.push({ url: String(url), init });
    return new Response(JSON.stringify(expected), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  };

  assert.deepEqual(await loadVisualVerifyProvenance(pageUrl.href, fetchPins), {
    implementation_commit: COMMIT,
    verifier: requestedVerifier,
  });
  assert.deepEqual(requests, [{
    url: "http://127.0.0.1:8000/qualification-visual-pins.json",
    init: { cache: "no-store", credentials: "same-origin" },
  }]);

  const wrongCommit = structuredClone(expected);
  wrongCommit.running.implementation_commit = "2".repeat(40);
  await assert.rejects(
    () => loadVisualVerifyProvenance(pageUrl.href, async () => new Response(JSON.stringify(wrongCommit))),
    /implementation commit differs from the running checkout/,
  );

  const staleBaseline = structuredClone(expected);
  staleBaseline.accepted.verifier.sha256 = "b".repeat(64);
  await assert.rejects(
    () => loadVisualVerifyProvenance(pageUrl.href, async () => new Response(JSON.stringify(staleBaseline))),
    /verifier SHA-256 differs from the checked-in baseline/,
  );
});

test("trusted control activations require transient activation and remain single use", () => {
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
    userActivationIsActive: true,
    recordedAt: "2026-08-28T08:00:00.000Z",
  });
  assert.deepEqual(gate.consume(activation, "run-corpus"), {
    schema: VISUAL_TRUSTED_CONTROL_SCHEMA,
    control_id: "run-corpus",
    event_type: "click",
    trust_source: "transient_user_activation",
    event_is_trusted: true,
    transient_user_activation: true,
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

test("inactive and hidden-document control activations are rejected", () => {
  const gate = new VisualTrustedControlGate();
  const control = {};
  assert.throws(() => gate.issue({ type: "click", isTrusted: false, currentTarget: control }, {
    control,
    controlId: "run-corpus",
    visibilityState: "visible",
  }), /active transient user activation/);
  assert.throws(() => gate.issue({ type: "click", isTrusted: true, currentTarget: control }, {
    control,
    controlId: "run-corpus",
    visibilityState: "visible",
  }), /active transient user activation/);
  assert.throws(() => gate.issue({ type: "click", isTrusted: true, currentTarget: control }, {
    control,
    controlId: "run-corpus",
    visibilityState: "hidden",
    userActivationIsActive: true,
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
