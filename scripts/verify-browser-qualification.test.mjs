import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { QUALIFICATION_LIMITS } from "../apps/browser-demo/web/qualification.js";
import {
  releaseImplementationCommit,
  verifyBrowserQualificationMatrix,
} from "./verify-browser-qualification.mjs";

const matrixUrl = new URL("../docs/releases/v0.19-browser-matrix.json", import.meta.url);
const releaseRecordUrl = new URL("../docs/releases/v0.19.0.md", import.meta.url);
const [matrixSource, releaseRecord] = await Promise.all([
  readFile(matrixUrl, "utf8"),
  readFile(releaseRecordUrl, "utf8"),
]);
const matrix = JSON.parse(matrixSource);
const implementationCommit = releaseImplementationCommit(releaseRecord);

test("the checked-in browser qualification matrix derives a passing result", () => {
  assert.equal(verifyBrowserQualificationMatrix(matrix, implementationCommit), true);
});

test("an over-limit observation fails even when the recorded pass flag is true", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.cold.first_coverage_milliseconds =
    QUALIFICATION_LIMITS.firstCoverageMilliseconds + 1;

  assert.throws(
    () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
    /cold first sampled Coverage exceeded 10000 ms/,
  );
});

test("the recorded pass flag must agree with the derived result", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.passed = false;
  assert.throws(() => verifyBrowserQualificationMatrix(tampered, implementationCommit));
});

test("cold network delivery is not mislabeled as a cache hit", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.cold.verified_cache_bytes = 172_696;
  assert.throws(() => verifyBrowserQualificationMatrix(tampered, implementationCommit));
});

test("the JSON matrix and Markdown record pin the same implementation", () => {
  assert.equal(matrix.implementation_commit, implementationCommit);
  assert.throws(
    () => verifyBrowserQualificationMatrix(matrix, "0000000000000000000000000000000000000000"),
  );
});
