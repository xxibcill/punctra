import assert from "node:assert/strict";
import test from "node:test";

import {
  VISUAL_ATTENDED_LANE,
  VISUAL_VERIFIER_PATH,
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

test("final verify provenance binds the qualified verifier and attended lane", () => {
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
    attended_lane: { ...VISUAL_ATTENDED_LANE },
  });
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
