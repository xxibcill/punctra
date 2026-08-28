import assert from "node:assert/strict";
import test from "node:test";

import { createVisualValidator } from "./visual-validation.js";

test("visual validators preserve caller-specific errors", () => {
  const { requireCondition, requireRecord } = createVisualValidator("Visual fixture invalid");
  assert.doesNotThrow(() => requireCondition(true, "ignored"));
  assert.doesNotThrow(() => requireRecord({}, "fixture"));
  assert.throws(
    () => requireCondition(false, "digest differs"),
    /^Error: Visual fixture invalid: digest differs$/,
  );
  assert.throws(
    () => requireRecord([], "fixture"),
    /^Error: Visual fixture invalid: fixture must be an object$/,
  );
});
