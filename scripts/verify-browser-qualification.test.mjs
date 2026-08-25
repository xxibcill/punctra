import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { QUALIFICATION_LIMITS } from "../apps/browser-demo/web/qualification.js";
import { verifyBrowserQualificationMatrix } from "./verify-browser-qualification.mjs";

const matrixUrl = new URL("../docs/releases/v0.19-browser-matrix.json", import.meta.url);
const matrix = JSON.parse(await readFile(matrixUrl, "utf8"));

test("the checked-in browser qualification matrix derives a passing result", () => {
  assert.equal(verifyBrowserQualificationMatrix(matrix), true);
});

test("an over-limit observation fails even when the recorded pass flag is true", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.cold.first_coverage_milliseconds =
    QUALIFICATION_LIMITS.firstCoverageMilliseconds + 1;

  assert.throws(
    () => verifyBrowserQualificationMatrix(tampered),
    /cold first sampled Coverage exceeded 10000 ms/,
  );
});

test("the recorded pass flag must agree with the derived result", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.passed = false;
  assert.throws(() => verifyBrowserQualificationMatrix(tampered));
});
