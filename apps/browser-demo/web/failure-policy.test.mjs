import assert from "node:assert/strict";
import test from "node:test";

import {
  UNSUPPORTED_INITIALIZATION_CODES,
  failureCause,
  failureLabel,
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
  for (const code of UNSUPPORTED_INITIALIZATION_CODES) {
    assert.equal(failureState({ code }), "unsupported");
  }
  assert.equal(failureState({ code: "worker_failed" }), "failed");
});

test("failure labels publish the closed browser harness tokens", () => {
  assert.equal(failureLabel("unsupported"), "UNSUPPORTED");
  assert.equal(failureLabel("failed"), "FAIL");
});
