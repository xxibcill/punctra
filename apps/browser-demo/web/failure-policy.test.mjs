import assert from "node:assert/strict";
import test from "node:test";

import {
  failureState,
  preservesCurrentViewer,
} from "./failure-policy.js";

test("worker failure preserves the current frame while fused failures recreate", () => {
  assert.equal(preservesCurrentViewer({ code: "worker_failed" }), true);
  assert.equal(preservesCurrentViewer({ code: "surface_timeout" }), true);
  assert.equal(preservesCurrentViewer({ code: "surface_lost" }), false);
  assert.equal(preservesCurrentViewer({ code: "browser_module" }), false);
});

test("only initialization capability failures publish unsupported state", () => {
  assert.equal(failureState({ code: "webgpu_unavailable" }), "unsupported");
  assert.equal(failureState({ code: "worker_failed" }), "failed");
});
