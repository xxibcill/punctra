import assert from "node:assert/strict";
import test from "node:test";

import {
  failureCause,
  failureState,
  isPreserveViewerFailure,
  preserveViewerFailure,
  preservesCurrentViewer,
} from "./failure-policy.js";

test("worker failure preserves the current frame while fused failures recreate", () => {
  assert.equal(preservesCurrentViewer({ code: "worker_failed" }), true);
  assert.equal(preservesCurrentViewer({ code: "surface_timeout" }), true);
  assert.equal(preservesCurrentViewer({ code: "surface_lost" }), false);
  assert.equal(preservesCurrentViewer({ code: "browser_module" }), false);
});

test("a host-marked pre-publication failure preserves its viewer", () => {
  const cause = { code: "offline" };
  const marked = preserveViewerFailure(cause);

  assert.equal(failureCause(marked), cause);
  assert.equal(isPreserveViewerFailure(marked), true);
  assert.equal(isPreserveViewerFailure(cause), false);
  assert.equal(preservesCurrentViewer(cause, marked), true);
  assert.equal(preservesCurrentViewer(cause), false);
});

test("only initialization capability failures publish unsupported state", () => {
  assert.equal(failureState({ code: "webgpu_unavailable" }), "unsupported");
  assert.equal(failureState({ code: "worker_failed" }), "failed");
});
