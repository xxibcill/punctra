import assert from "node:assert/strict";
import test from "node:test";

import {
  loadExactQueryModules,
  loadStreamingProtocol,
  loadViewerModules,
  loadWorkerModules,
} from "./module-loader.js";

test("central module loaders return every dependency graph", async () => {
  const [exactQuery, streamingProtocol, viewer, worker] = await Promise.all([
    loadExactQueryModules("module-loader-test"),
    loadStreamingProtocol("module-loader-test"),
    loadViewerModules("module-loader-test"),
    loadWorkerModules("module-loader-test"),
  ]);

  assert.equal(typeof exactQuery[0].validateManifest, "function");
  assert.equal(typeof exactQuery[1].validateBoundRangeResponse, "function");
  assert.equal(typeof streamingProtocol.workerFailure, "function");
  assert.equal(typeof viewer[0].appendTransferredOrdinals, "function");
  assert.equal(typeof viewer[1].runWorkerOperation, "function");
  assert.equal(typeof viewer[2].workerFailure, "function");
  assert.equal(typeof viewer[3].CAMERA_PROJECTION_POLICIES, "object");
  assert.equal(typeof worker[0].runStreamingOperation, "function");
  assert.equal(typeof worker[1].workerOperationId, "function");
});
