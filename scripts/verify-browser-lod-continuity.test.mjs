import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { canonicalPath, verifyDigest } from "./verify-browser-lod-continuity.mjs";

test("independent artifact reads reject escape paths and changed exact bytes", () => {
  for (const value of ["../outside.png", "/tmp/capture.png", "docs//capture.png", "docs/./capture.png", "docs/a/../../capture.png"]) {
    assert.throws(() => canonicalPath(value), /path/);
  }
  assert.equal(canonicalPath("docs/releases/v0.23-browser-lod-artifacts/verify/candidate.png"), "docs/releases/v0.23-browser-lod-artifacts/verify/candidate.png");
  const bytes = Buffer.from("actual captured bytes");
  const record = { byte_length: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
  verifyDigest(bytes, record, "capture");
  const changed = Buffer.from(bytes);
  changed[0] ^= 1;
  assert.throws(() => verifyDigest(changed, record, "capture"), /SHA-256/);
  assert.throws(() => verifyDigest(bytes.subarray(1), record, "capture"), /byte length/);
});
