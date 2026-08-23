import assert from "node:assert/strict";
import test from "node:test";

import {
  WORKER_OUTPUT_TYPES,
  createWorkerMessage,
} from "./streaming-protocol.js";

test("worker output is limited to the documented four message types", () => {
  assert.deepEqual(WORKER_OUTPUT_TYPES, ["state", "batch", "complete", "failure"]);

  const deployment = { deployment_id: "fixture-v1" };
  assert.deepEqual(
    createWorkerMessage("operation-1", "state", {
      phase: "deployment",
      deployment,
    }),
    {
      schema: "punctra-browser-worker-v1",
      type: "state",
      operation_id: "operation-1",
      phase: "deployment",
      deployment,
    },
  );

  assert.throws(
    () => createWorkerMessage("operation-1", "deployment", { deployment }),
    (error) => error.code === "manifest_invalid",
  );
});
