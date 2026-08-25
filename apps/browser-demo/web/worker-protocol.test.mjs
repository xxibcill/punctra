import assert from "node:assert/strict";
import test from "node:test";

import { loadStreamingProtocol } from "./module-loader.js";
import {
  INVALID_WORKER_OPERATION_ID,
  MAX_WORKER_FAILURE_MESSAGE_CHARACTERS,
  WORKER_SCHEMA,
  WORKER_OUTPUT_TYPES,
  boundedWorkerFailureMessage,
  isWorkerOperationId,
  workerFailure,
  workerOperationId,
} from "./worker-protocol.js";

const { createWorkerMessage } = await loadStreamingProtocol("worker-protocol-test");

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

test("worker identities and failure details are bounded before publication", () => {
  assert.equal(isWorkerOperationId("operation-1"), true);
  assert.equal(isWorkerOperationId("x".repeat(129)), false);
  assert.equal(workerOperationId("operation-1"), "operation-1");
  assert.equal(workerOperationId("x".repeat(129)), INVALID_WORKER_OPERATION_ID);
  assert.equal(workerOperationId(undefined), INVALID_WORKER_OPERATION_ID);

  const oversized = "external".repeat(100);
  const bounded = boundedWorkerFailureMessage(oversized);
  assert.equal(bounded.length, MAX_WORKER_FAILURE_MESSAGE_CHARACTERS);
  assert.match(bounded, /…$/);
  assert.equal(workerFailure(oversized).message, bounded);
});
