import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";

import { captureChildExit } from "./child-process.mjs";

test("captured child exit remains awaitable after the process already settled", async () => {
  const child = new EventEmitter();
  const exit = captureChildExit(child);

  child.emit("exit", 0, null);

  assert.deepEqual(await exit, { code: 0, signal: null });
});
