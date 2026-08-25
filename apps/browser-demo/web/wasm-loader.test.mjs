import assert from "node:assert/strict";
import test from "node:test";

import { createWasmModuleLoader } from "./wasm-loader.js";

test("concurrent callers share one Wasm initialization", async () => {
  const initialization = deferred();
  let initializationCalls = 0;
  const createRawViewer = () => {};
  const loadBindings = createLoader({
    createRawViewer,
    initializeWasm() {
      initializationCalls += 1;
      return initialization.promise;
    },
  });
  const wasmUrl = new URL("https://assets.test/first.wasm");

  const first = loadBindings(wasmUrl);
  const second = loadBindings(wasmUrl);
  await Promise.resolve();
  assert.equal(initializationCalls, 1);

  initialization.resolve();
  assert.deepEqual(await first, { createViewer: createRawViewer });
  assert.deepEqual(await second, { createViewer: createRawViewer });
});

test("failed Wasm initialization clears state for a corrected retry", async () => {
  let initializationCalls = 0;
  const loadBindings = createLoader({
    createRawViewer() {},
    initializeWasm() {
      initializationCalls += 1;
      if (initializationCalls === 1) throw new Error("bad Wasm response");
      return Promise.resolve();
    },
  });
  const wasmUrl = new URL("https://assets.test/retry.wasm");

  await assert.rejects(
    loadBindings(wasmUrl),
    (error) => error instanceof TestViewerError && error.code === "internal",
  );
  await loadBindings(wasmUrl);

  assert.equal(initializationCalls, 2);
});

test("successful Wasm initialization rejects a different URL", async () => {
  const loadBindings = createLoader({
    createRawViewer() {},
    initializeWasm: async () => {},
  });

  await loadBindings(new URL("https://assets.test/first.wasm"));
  await assert.rejects(
    loadBindings(new URL("https://assets.test/second.wasm")),
    (error) => error instanceof TestViewerError && error.code === "invalid_argument",
  );
});

function createLoader({ createRawViewer, initializeWasm }) {
  return createWasmModuleLoader({
    createRawViewer,
    initializeWasm,
    ViewerError: TestViewerError,
  });
}

function deferred() {
  let resolve;
  const promise = new Promise((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

class TestViewerError extends Error {
  constructor(code, message, details = {}) {
    super(message);
    this.code = code;
    this.safeAction = details.safeAction;
  }
}
