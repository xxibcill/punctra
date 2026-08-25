import assert from "node:assert/strict";
import test from "node:test";

import { runWorkerOperation } from "./worker-operation.js";

test("worker operation owns creation, settlement, and termination", async () => {
  const worker = new FakeWorker();
  const pending = runWorkerOperation({
    WorkerConstructor: class {
      constructor(url, options) {
        assert.equal(url, "worker.js");
        assert.deepEqual(options, { type: "module", name: "operation-1" });
        return worker;
      }
    },
    workerUrl: "worker.js",
    workerName: "operation-1",
    timeoutMilliseconds: 100,
    timeoutFailure: { code: "timeout" },
    errorFailure: (event) => ({ code: "error", message: event.message }),
    messageErrorFailure: { code: "message_error" },
    initialMessage: { type: "start" },
    onMessage(message, controls) {
      if (message.type === "complete") controls.resolve(message.value);
    },
  });

  assert.deepEqual(worker.messages, [{ type: "start" }]);
  worker.emit("message", { data: { type: "complete", value: 42 } });
  worker.emit("message", { data: { type: "complete", value: 99 } });

  assert.equal(await pending, 42);
  assert.equal(worker.terminations, 1);
});

test("worker operation delegates construction to the bundler-aware factory", async () => {
  const worker = new FakeWorker();
  let construction;
  const pending = runWorkerOperation({
    WorkerConstructor: undefined,
    workerFactory(url, options) {
      construction = { url, options };
      return worker;
    },
    workerUrl: new URL("https://assets.test/stream-worker.js"),
    workerName: "operation-factory",
    timeoutMilliseconds: 100,
    timeoutFailure: { code: "timeout" },
    errorFailure: (event) => ({ code: "error", message: event.message }),
    messageErrorFailure: { code: "message_error" },
    initialMessage: { type: "start" },
    onMessage(message, controls) {
      if (message.type === "complete") controls.resolve(message.value);
    },
  });

  assert.deepEqual(construction, {
    url: new URL("https://assets.test/stream-worker.js"),
    options: { type: "module", name: "operation-factory" },
  });
  worker.emit("message", { data: { type: "complete", value: 7 } });
  assert.equal(await pending, 7);
  assert.equal(worker.terminations, 1);
});

test("worker operation maps errors and terminates", async () => {
  const worker = new FakeWorker();
  const pending = runWorkerOperation({
    WorkerConstructor: class { constructor() { return worker; } },
    workerUrl: "worker.js",
    workerName: "operation-2",
    timeoutMilliseconds: 100,
    timeoutFailure: { code: "timeout" },
    errorFailure: (event) => ({ code: "error", message: event.message }),
    messageErrorFailure: { code: "message_error" },
    initialMessage: { type: "start" },
    onMessage() {},
  });

  worker.emit("error", { message: "worker crashed" });

  await assert.rejects(pending, (error) => error.code === "error");
  assert.equal(worker.terminations, 1);
});

test("worker operation forwards cancellation and still owns terminal settlement", async () => {
  const worker = new FakeWorker();
  const controller = new AbortController();
  const cancellationMessage = { type: "cancel" };
  const pending = runWorkerOperation({
    WorkerConstructor: class { constructor() { return worker; } },
    workerUrl: "worker.js",
    workerName: "operation-3",
    timeoutMilliseconds: 100,
    timeoutFailure: { code: "timeout" },
    errorFailure: (event) => ({ code: "error", message: event.message }),
    messageErrorFailure: { code: "message_error" },
    initialMessage: { type: "start" },
    signal: controller.signal,
    cancellationMessage,
    onMessage(message, controls) {
      if (message.type === "cancelled") controls.reject({ code: "cancelled" });
    },
  });

  controller.abort();
  assert.deepEqual(worker.messages, [{ type: "start" }, cancellationMessage]);
  worker.emit("message", { data: { type: "cancelled" } });
  await assert.rejects(pending, (error) => error.code === "cancelled");
  assert.equal(worker.terminations, 1);
});

class FakeWorker {
  constructor() {
    this.listeners = new Map();
    this.messages = [];
    this.terminations = 0;
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  postMessage(message) {
    this.messages.push(message);
  }

  terminate() {
    this.terminations += 1;
  }

  emit(type, event) {
    this.listeners.get(type)?.(event);
  }
}
