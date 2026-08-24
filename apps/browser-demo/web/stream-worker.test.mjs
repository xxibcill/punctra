import assert from "node:assert/strict";
import test from "node:test";

import {
  INVALID_WORKER_OPERATION_ID,
  WORKER_SCHEMA,
} from "./worker-protocol.js";

const listeners = new Map();
const published = [];
globalThis.self = {
  location: { href: "https://fixtures.test/stream-worker.js" },
  addEventListener(type, listener) {
    listeners.set(type, listener);
  },
  postMessage(message) {
    published.push(message);
  },
};
await import(`./stream-worker.js?test=${Date.now()}`);

test("worker rejects unbounded inbound diagnostics with bounded output", () => {
  const unbounded = "external".repeat(1_000);
  listeners.get("message")({
    data: {
      schema: WORKER_SCHEMA,
      type: unbounded,
      operation_id: "operation-1",
    },
  });
  listeners.get("message")({
    data: {
      schema: WORKER_SCHEMA,
      type: "start",
      operation_id: unbounded,
    },
  });

  assert.deepEqual(published.pop(), {
    schema: WORKER_SCHEMA,
    type: "failure",
    operation_id: INVALID_WORKER_OPERATION_ID,
    code: "manifest_invalid",
    message: "worker operation identity is invalid",
    safe_action: "Repair or select a compatible deployment manifest before retrying.",
  });
  assert.deepEqual(published.pop(), {
    schema: WORKER_SCHEMA,
    type: "failure",
    operation_id: "operation-1",
    code: "manifest_invalid",
    message: "unsupported worker message type",
    safe_action: "Repair or select a compatible deployment manifest before retrying.",
  });
});
