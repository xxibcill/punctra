import assert from "node:assert/strict";
import test from "node:test";

import {
  WORKER_SCHEMA,
  WORKER_OUTPUT_TYPES,
  workerFailure,
} from "./worker-protocol.js";
import { createWorkerMessage } from "./streaming-protocol.js";

test("worker output is limited to the documented four message types", () => {
  assert.equal(WORKER_SCHEMA, "punctra-browser-worker-v1");
  assert.deepEqual(WORKER_OUTPUT_TYPES, ["state", "batch", "complete", "failure"]);

  assert.deepEqual(workerFailure("worker crashed"), {
    schema: WORKER_SCHEMA,
    type: "failure",
    code: "worker_failed",
    message: "worker crashed",
    safe_action: "Terminate the worker, keep the current frame, and create a new worker before retrying.",
  });

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
