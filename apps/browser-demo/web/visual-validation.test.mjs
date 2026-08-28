import assert from "node:assert/strict";
import test from "node:test";

import {
  cloneJson,
  createVisualValidator,
  errorMessage,
  jsonEqual,
  parsePageUrl,
} from "./visual-validation.js";

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

test("shared visual JSON helpers clone and compare serializable values", () => {
  const source = { nested: { value: 7 } };
  const cloned = cloneJson(source);
  assert.deepEqual(cloned, source);
  assert.notEqual(cloned, source);
  assert(jsonEqual(cloned, source));
  assert(!jsonEqual(cloned, { nested: { value: 8 } }));
});

test("shared visual URL and error helpers preserve caller-specific errors", () => {
  assert.equal(parsePageUrl("http://127.0.0.1:8000/visual.html", "Visual fixture invalid").pathname, "/visual.html");
  assert.throws(
    () => parsePageUrl("not a URL", "Visual fixture invalid"),
    /^Error: Visual fixture invalid: page URL is invalid:/,
  );
  assert.equal(errorMessage(new Error("failed")), "failed");
  assert.equal(errorMessage("failed"), "failed");
});
