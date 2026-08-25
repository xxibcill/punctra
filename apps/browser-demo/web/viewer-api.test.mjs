import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  DISPLAY_MODES,
  VIEWER_ERROR_CODES,
  ViewerError,
  createBrowserViewer,
} from "./viewer-api.js";
import { WORKER_SCHEMA } from "./worker-protocol.js";

const SOURCE = "ab".repeat(32);
const GENERATED_SOURCE = "15".repeat(32);

test("runtime error codes and display modes agree with TypeScript declarations", async () => {
  const runtime = await import("./viewer-api.js");
  const declaration = await readFile(new URL("viewer-api.d.ts", import.meta.url), "utf8");
  const errorBlock = declaration.match(/export type ViewerErrorCode =([\s\S]*?);\n\nexport const/)[1];
  const declaredErrors = [...errorBlock.matchAll(/"([a-z0-9_]+)"/g)].map((match) => match[1]);
  const displayBlock = declaration.match(/export type DisplayMode =([\s\S]*?);/)[1];
  const declaredModes = [...displayBlock.matchAll(/"([a-z]+)"/g)].map((match) => match[1]);

  assert.deepEqual(declaredErrors, VIEWER_ERROR_CODES);
  assert.deepEqual(declaredModes, DISPLAY_MODES);
  assert.equal(new Set(VIEWER_ERROR_CODES).size, VIEWER_ERROR_CODES.length);
  assert.equal("BrowserViewer" in runtime, false);
  assert.match(declaration, /export interface BrowserViewer \{/);
  assert.match(declaration, /pause\(\): ViewerState;/);
  assert.match(declaration, /resume\(\): ViewerState;/);
  assert.match(declaration, /dispose\(\): void;/);
  assert.match(declaration, /readonly firstCoverageMilliseconds: number;/);
  assert.match(declaration, /readonly settledViewMilliseconds: number;/);
  assert.match(declaration, /readonly mainThreadBatchMillisecondsHighWater: number;/);
});

test("failed facade construction shuts down the raw viewer", async () => {
  let shutdowns = 0;
  const raw = {
    diagnostics: () => "invalid diagnostics",
    shutdown() {
      shutdowns += 1;
      return "invalid diagnostics";
    },
  };

  await assert.rejects(
    createBrowserViewer({
      bindings: { createViewer: async () => raw },
      canvas: {},
      viewport: viewport(),
    }),
    (error) => error.code === "diagnostic_serialization",
  );
  assert.equal(shutdowns, 1);
});

test("a throwing initial subscriber still receives an unsubscribe boundary", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
  });
  let deliveries = 0;

  const unsubscribe = viewer.subscribe(() => {
    deliveries += 1;
    throw new Error("fixture subscriber failure");
  });

  assert.equal(deliveries, 1);
  assert.equal(unsubscribe(), true);
  viewer.setVisible(true);
  assert.equal(deliveries, 1);
});

test("viewer exposes typed lifecycle, camera, display, state subscription, and coalesced rendering", async () => {
  const raw = new FakeRawViewer();
  const frames = [];
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    requestAnimationFrame: (callback) => {
      frames.push(callback);
      return frames.length;
    },
    cancelAnimationFrame: () => {},
  });
  const observed = [];
  const unsubscribe = viewer.subscribe((state) => observed.push(state));

  assert.equal(viewer.state().schema, "punctra-viewer-state-v1");
  assert.equal(Object.isFrozen(viewer.state()), true);
  assert.equal(viewer.state().source.identity, GENERATED_SOURCE);
  assert.equal(viewer.pause().lifecycle, "hidden");
  assert.equal(viewer.resume().lifecycle, "ready");
  viewer.setHighlights([]);
  assert.equal(viewer.state().highlights.pointCount, 0);
  viewer.setCamera({
    projection: "perspective",
    eye: [500_000, 4_599_969, 122],
    target: [500_000, 4_600_000, 100],
    up: [0, 0, 1],
    verticalFieldOfViewRadians: Math.PI / 3,
    nearDistance: 0.1,
    farDistance: 250,
  });
  viewer.setCamera({
    projection: "orthographic",
    eye: [500_000, 4_599_969, 122],
    target: [500_000, 4_600_000, 100],
    up: [0, 0, 1],
    verticalWorldHeight: 30,
    nearDistance: 0.1,
    farDistance: 250,
  });
  viewer.setDisplayMode("classification");
  const first = viewer.requestRender();
  const second = viewer.requestRender();

  assert.equal(first, second);
  assert.equal(frames.length, 1);
  assert.equal(viewer.state().render.scheduled, true);
  frames.shift()(1);
  const rendered = await first;
  assert.equal(rendered.render.renderedFrames, 1);
  assert.equal(rendered.camera.projection, "orthographic");
  assert.equal(rendered.displayMode, "classification");
  assert.equal(rendered.source.identity, GENERATED_SOURCE);
  assert.ok(observed.length >= 4);
  assert.equal(unsubscribe(), true);

  viewer.dispose();
  viewer.dispose();
  assert.equal(viewer.state().lifecycle, "destroyed");
  assert.throws(
    () => viewer.render(),
    (error) => error instanceof ViewerError && error.code === "viewer_destroyed",
  );
  assert.equal(viewer.state().failure.code, "viewer_destroyed");
});

test("viewer cancels stale scheduled rendering on hide, Source replacement, and fused failure", async () => {
  const raw = new FakeRawViewer();
  const frames = [];
  const cancelledFrames = [];
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: FixtureWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
    requestAnimationFrame: (callback) => {
      frames.push(callback);
      return frames.length;
    },
    cancelAnimationFrame: (id) => cancelledFrames.push(id),
  });

  const hiddenRender = viewer.requestRender();
  const hiddenRejection = assert.rejects(hiddenRender, (error) => error.code === "render_cancelled");
  viewer.setVisible(false);
  await hiddenRejection;
  assert.equal(viewer.state().render.scheduled, false);
  assert.deepEqual(cancelledFrames, [1]);
  const renderedBeforeStaleFrame = raw.data.rendered_frames;
  frames.shift()(1);
  assert.equal(raw.data.rendered_frames, renderedBeforeStaleFrame);

  viewer.setVisible(true);
  const generationRender = viewer.requestRender();
  const generationRejection = assert.rejects(
    generationRender,
    (error) => error.code === "render_cancelled",
  );
  await viewer.loadSource({ manifestUrl: "https://fixtures.test/deployment.json" });
  await generationRejection;
  assert.deepEqual(cancelledFrames, [1, 1]);
  const renderedAfterLoad = raw.data.rendered_frames;
  frames.shift()(2);
  assert.equal(raw.data.rendered_frames, renderedAfterLoad);

  const fusedRaw = new FusedRawViewer();
  const fusedFrames = [];
  const fusedCancellations = [];
  const fusedViewer = await createBrowserViewer({
    bindings: { createViewer: async () => fusedRaw },
    canvas: {},
    viewport: viewport(),
    requestAnimationFrame: (callback) => {
      fusedFrames.push(callback);
      return 7;
    },
    cancelAnimationFrame: (id) => fusedCancellations.push(id),
  });
  const fusedRender = fusedViewer.requestRender();
  const fusedRejection = assert.rejects(fusedRender, (error) => error.code === "render_cancelled");
  assert.throws(() => fusedViewer.setDisplayMode("rgb"), (error) => error.code === "device_lost");
  await fusedRejection;
  assert.deepEqual(fusedCancellations, [7]);
  assert.equal(fusedViewer.state().lifecycle, "destroyed");
  assert.equal(fusedViewer.state().failure.code, "device_lost");
  fusedFrames.shift()(3);
  assert.equal(fusedRaw.data.rendered_frames, 0);
});

test("normal and fused destruction cancel every owned operation", async () => {
  const pendingFrames = [];
  const cancelledFrames = [];
  const pendingPickViewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
    requestAnimationFrame: (callback) => {
      pendingFrames.push(callback);
      return 7;
    },
    cancelAnimationFrame: (id) => cancelledFrames.push(id),
  });
  pendingPickViewer.render();
  const pickFailure = pendingPickViewer.pick({ x: 10, y: 20 }).then(
    () => undefined,
    (error) => error,
  );
  await Promise.resolve();

  pendingPickViewer.destroy();
  assert.equal((await pickFailure)?.code, "viewer_destroyed");
  assert.equal(pendingFrames.length, 1);
  assert.deepEqual(cancelledFrames, [7]);

  let exactSignal;
  let resolveExact;
  const fusedViewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FusedRawViewer() },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: OwnedWorkWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
    exactQueryBridge: {
      confirm(request) {
        exactSignal = request.signal;
        return new Promise((resolve) => { resolveExact = resolve; });
      },
    },
  });
  const observed = [];
  const unsubscribe = fusedViewer.subscribe((state) => observed.push(state));
  const point = { sourceIdentity: GENERATED_SOURCE, pointOrdinal: 0, generation: 1 };
  const exactOperation = fusedViewer.confirmPoint(point);
  const loadOperation = fusedViewer.loadSource({
    manifestUrl: "https://fixtures.test/deployment.json",
  });
  await Promise.resolve();

  assert.throws(() => fusedViewer.setDisplayMode("rgb"), (error) => error.code === "device_lost");
  const factsAfterFuse = {
    exactAborted: exactSignal.aborted,
    loadActive: fusedViewer.state().load.active,
    workerMessages: OwnedWorkWorker.current.messages.map((message) => message.type),
    unsubscribed: unsubscribe(),
    observedStates: observed.length,
  };
  OwnedWorkWorker.current.fail();
  resolveExact(exactResult(point));
  await Promise.allSettled([exactOperation, loadOperation]);

  assert.equal(factsAfterFuse.exactAborted, true);
  assert.equal(factsAfterFuse.loadActive, false);
  assert.deepEqual(factsAfterFuse.workerMessages, ["start", "cancel"]);
  assert.equal(factsAfterFuse.unsubscribed, false);
  assert.equal(observed.length, factsAfterFuse.observedStates);
});

test("an in-flight Source load reports viewer destruction after a worker error", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: OwnedWorkWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
  });
  const load = viewer.loadSource({
    manifestUrl: "https://fixtures.test/deployment.json",
  });
  await Promise.resolve();

  viewer.destroy();
  OwnedWorkWorker.current.crash();

  await assert.rejects(load, (error) => error.code === "viewer_destroyed");
  assert.equal(viewer.state().failure.code, "viewer_destroyed");
});

test("surface_outdated preserves the viewer for bounded recovery", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new OutdatedRawViewer() },
    canvas: {},
    viewport: viewport(),
  });

  assert.throws(
    () => viewer.setDisplayMode("rgb"),
    (error) => error.code === "surface_outdated" && error.recoverable === true,
  );
  assert.equal(viewer.state().lifecycle, "ready");
  assert.equal(viewer.state().failure.code, "surface_outdated");
});

test("an over-limit raw resize preserves the previous viewport and accepts a valid retry", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new BoundedResizeRawViewer() },
    canvas: {},
    viewport: viewport(),
  });
  const before = viewer.state().viewport;

  assert.throws(
    () => viewer.resize({ cssWidth: 4_096, cssHeight: 4_096, devicePixelRatio: 4 }),
    (error) => error.code === "resize_viewport" && error.recoverable === true,
  );
  assert.deepEqual(viewer.state().viewport, before);

  const recovered = viewer.resize({ cssWidth: 640, cssHeight: 480, devicePixelRatio: 1.5 });
  assert.equal(recovered.lifecycle, "ready");
  assert.equal(recovered.viewport.physicalWidth, 960);
  assert.equal(recovered.viewport.physicalHeight, 720);
});

test("a worker crash before publication retains the viewer for a successful retry", async () => {
  RecoveringWorker.reset();
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: RecoveringWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
  });

  await assert.rejects(
    viewer.loadSource({ manifestUrl: "https://fixtures.test/deployment.json" }),
    (error) => error.code === "worker_failed" && error.recoverable === true,
  );
  assert.equal(viewer.state().lifecycle, "ready");
  assert.equal(viewer.state().generation, 1);

  const recovered = await viewer.loadSource({
    manifestUrl: "https://fixtures.test/deployment.json",
  });
  assert.equal(recovered.state.source.identity, SOURCE);
  assert.equal(recovered.state.lifecycle, "ready");
});

test("worker completion without first Coverage fails before changing the viewer", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: CompleteWithoutBatchWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
  });

  await assert.rejects(
    viewer.loadSource({ manifestUrl: "https://fixtures.test/deployment.json" }),
    (error) => error.code === "stream_validation" && error.recoverable === true,
  );
  assert.equal(viewer.state().lifecycle, "ready");
  assert.equal(viewer.state().source.identity, GENERATED_SOURCE);
});

test("every recreation-required renderer failure fuses the viewer", async () => {
  for (const code of [
    "pick_recording",
    "pick_readback",
    "pick_invariant",
    "transient_texture_limit",
  ]) {
    const cancelledFrames = [];
    const viewer = await createBrowserViewer({
      bindings: { createViewer: async () => new FusedRawViewer(code) },
      canvas: {},
      viewport: viewport(),
      requestAnimationFrame: () => 7,
      cancelAnimationFrame: (id) => cancelledFrames.push(id),
    });
    const scheduled = viewer.requestRender();
    const scheduledRejection = assert.rejects(
      scheduled,
      (error) => error.code === "render_cancelled",
    );

    assert.throws(() => viewer.setDisplayMode("rgb"), (error) => error.code === code);
    await scheduledRejection;
    assert.deepEqual(cancelledFrames, [7]);
    assert.equal(viewer.state().lifecycle, "destroyed");
    assert.equal(viewer.state().failure.code, code);
  }
});

test("viewer owns worker publication, streamed picking, highlights, and exact handoff", async () => {
  const raw = new FakeRawViewer();
  const exactRequests = [];
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: FixtureWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
    requestAnimationFrame: (callback) => {
      queueMicrotask(() => callback(1));
      return 1;
    },
    cancelAnimationFrame: () => {},
    exactQueryBridge: {
      async confirm(request) {
        exactRequests.push(request);
        return {
          authority: "exact_source_record",
          sourceIdentity: request.sourceIdentity,
          pointOrdinal: String(request.pointOrdinal),
          generation: request.generation,
          ticks: [1, 2, 3],
          position: [1, 2, 3],
          intensity: 4,
          classification: 2,
          rgb: [5, 6, 7],
        };
      },
    },
  });

  const loaded = await viewer.loadSource({
    manifestUrl: "https://fixtures.test/deployment.json",
    cacheMode: "memory",
    credentials: "omit",
  });
  assert.deepEqual(loaded.pointOrdinals, [7]);
  assert.equal(loaded.state.source.identity, SOURCE);
  assert.equal(loaded.state.source.coverage, "sampled");
  assert.equal(loaded.state.source.retainedRecordBytes, 32);
  assert.ok(loaded.timings.firstCoverageMilliseconds >= 0);
  assert.ok(loaded.timings.settledViewMilliseconds >= loaded.timings.firstCoverageMilliseconds);
  assert.equal(
    loaded.mainThreadMillisecondsHighWater,
    loaded.timings.mainThreadBatchMillisecondsHighWater,
  );

  const pick = await viewer.pick({ x: 10, y: 20 });
  assert.equal(pick.sourceIdentity, SOURCE);
  assert.equal(pick.pointOrdinal, "7");
  viewer.setHighlights([pick], pick.generation);
  assert.equal(viewer.state().highlights.pointCount, 1);
  assert.throws(
    () => viewer.setHighlights([{
      sourceIdentity: SOURCE,
      pointOrdinal: (1n << 64n) + 1n,
    }], pick.generation),
    (error) => error.code === "invalid_argument",
  );
  const exact = await viewer.confirmPoint(pick);
  assert.equal(exact.authority, "exact_source_record");
  assert.equal(exact.pointOrdinal, "7");
  assert.equal(exactRequests.length, 1);
  viewer.clearHighlights();
  assert.equal(viewer.state().highlights.pointCount, 0);

  raw.advanceGeneration();
  viewer.render();
  await assert.rejects(
    viewer.confirmPoint(pick),
    (error) => error.code === "stale_generation",
  );
});

test("Source timing uses one monotonic load origin and preserves the compatibility alias", async () => {
  const samples = [100, 125, 128, 180];
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: FixtureWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
    monotonicNow: () => samples.shift(),
  });

  const loaded = await viewer.loadSource({
    manifestUrl: "https://fixtures.test/deployment.json",
  });

  assert.deepEqual(loaded.timings, {
    firstCoverageMilliseconds: 28,
    settledViewMilliseconds: 80,
    mainThreadBatchMillisecondsHighWater: 3,
  });
  assert.equal(loaded.mainThreadMillisecondsHighWater, 3);
  assert.deepEqual(samples, []);
});

test("relative Source manifests keep the caller document base across Worker paths", async () => {
  const originalDocument = globalThis.document;
  globalThis.document = { baseURI: "https://caller.test/application/index.html" };
  try {
    const viewer = await createBrowserViewer({
      bindings: { createViewer: async () => new FakeRawViewer() },
      canvas: {},
      viewport: viewport(),
      WorkerConstructor: ManifestCaptureWorker,
      workerUrl: "https://caller.test/assets/stream-worker-hashed.js",
    });

    await viewer.loadSource({ manifestUrl: "./deployment.json" });

    const start = ManifestCaptureWorker.current.messages.find(
      (message) => message.type === "start",
    );
    assert.equal(
      start.manifest_url,
      "https://caller.test/application/deployment.json",
    );
  } finally {
    if (originalDocument === undefined) delete globalThis.document;
    else globalThis.document = originalDocument;
  }
});

test("viewer rejects oversized highlights before inspecting Point identities", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
  });
  let inspectedIdentities = 0;
  const points = Array.from({ length: 33 }, () => ({
    get sourceIdentity() {
      inspectedIdentities += 1;
      return GENERATED_SOURCE;
    },
    pointOrdinal: 0,
  }));

  assert.throws(
    () => viewer.setHighlights(points, 1),
    (error) => error.code === "invalid_argument" && error.message.includes("32-Point ceiling"),
  );
  assert.equal(inspectedIdentities, 0);
});

test("viewer rejects unsafe numeric Point ordinals before changing identity", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
  });

  assert.throws(
    () => viewer.setHighlights([{
      sourceIdentity: GENERATED_SOURCE,
      pointOrdinal: Number.MAX_SAFE_INTEGER + 1,
    }], 1),
    (error) => error.code === "invalid_argument"
      && error.message.includes("safe integer"),
  );
});

test("viewer rejects pick coordinates before the Wasm u32 boundary", async () => {
  const raw = new FakeRawViewer();
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    requestAnimationFrame: (callback) => {
      queueMicrotask(() => callback(1));
      return 1;
    },
    cancelAnimationFrame: () => {},
  });
  viewer.render();

  await assert.rejects(
    viewer.pick({ x: 2 ** 32, y: 0 }),
    (error) => error.code === "pick_outside_viewport",
  );
  assert.equal(raw.data.pick.status, "not_requested");
});

test("viewer disposes a cancelled raw pick before accepting another", async () => {
  const raw = new PendingRawViewer();
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    requestAnimationFrame: () => 7,
    cancelAnimationFrame: () => {},
  });
  const firstController = new AbortController();
  const first = viewer.pick({ x: 10, y: 20, signal: firstController.signal });
  await Promise.resolve();
  firstController.abort();
  await assert.rejects(first, (error) => error.code === "cancelled");

  const secondController = new AbortController();
  const second = viewer.pick({ x: 10, y: 20, signal: secondController.signal });
  await Promise.resolve();
  assert.equal(raw.pickBegins, 2);
  secondController.abort();
  await assert.rejects(second, (error) => error.code === "cancelled");
  assert.equal(raw.pickCancellations, 2);
});

test("a Source failure after partial publication fuses the viewer", async () => {
  const raw = new FakeRawViewer();
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: PartialFailureWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
  });

  await assert.rejects(
    viewer.loadSource({ manifestUrl: "https://fixtures.test/deployment.json" }),
    (error) => error.code === "cancelled" && error.recoverable === false,
  );
  assert.equal(viewer.state().lifecycle, "destroyed");
  assert.throws(
    () => viewer.render(),
    (error) => error.code === "viewer_destroyed",
  );
});

test("a Worker crash after partial publication fuses the viewer", async () => {
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => new FakeRawViewer() },
    canvas: {},
    viewport: viewport(),
    WorkerConstructor: PartialCrashWorker,
    workerUrl: "https://fixtures.test/stream-worker.js",
  });

  await assert.rejects(
    viewer.loadSource({ manifestUrl: "https://fixtures.test/deployment.json" }),
    (error) => error.code === "worker_failed" && error.recoverable === false,
  );
  assert.equal(viewer.state().lifecycle, "destroyed");
  assert.throws(
    () => viewer.render(),
    (error) => error.code === "viewer_destroyed",
  );
});

test("viewer normalizes cancellation and bounds external failures", async () => {
  const raw = new FakeRawViewer();
  raw.beginStreamBatch(SOURCE, 1, 0, 0, 0, -1, 1, 0, new Uint8Array(32));
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    exactQueryBridge: { confirm: async () => { throw new Error("x".repeat(1_000)); } },
  });
  const controller = new AbortController();
  controller.abort();

  await assert.rejects(
    viewer.confirmPoint({ sourceIdentity: SOURCE, pointOrdinal: 0, generation: 1 }, {
      signal: controller.signal,
    }),
    (error) => error.code === "exact_query_cancelled",
  );
  assert.equal(viewer.state().failure.code, "exact_query_cancelled");
  viewer.setVisible(true);
  assert.equal(viewer.state().failure.code, "exact_query_cancelled");

  assert.throws(
    () => viewer.setCamera({ projection: "invalid" }),
    (error) => error.code === "invalid_argument",
  );
  assert.equal(viewer.state().failure.code, "invalid_argument");
  const bounded = new ViewerError("internal", "x".repeat(1_000), { cause: new Error("private") });
  assert.equal(bounded.message.length, 512);
  assert.equal("cause" in bounded, false);
});

test("viewer keeps malformed exact results inside the structured error boundary", async () => {
  const raw = new FakeRawViewer();
  raw.beginStreamBatch(SOURCE, 1, 0, 0, 0, -1, 1, 0, new Uint8Array(32));
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    exactQueryBridge: {
      confirm: async () => ({
        authority: "exact_source_record",
        sourceIdentity: SOURCE,
        generation: 1,
      }),
    },
  });

  await assert.rejects(
    viewer.confirmPoint({ sourceIdentity: SOURCE, pointOrdinal: 0, generation: 1 }),
    (error) => error instanceof ViewerError
      && error.code === "exact_query_source_mismatch"
      && !("cause" in error),
  );
});

test("viewer owns cancellation for exact handoffs already in flight", async () => {
  const raw = new FakeRawViewer();
  raw.beginStreamBatch(SOURCE, 1, 0, 0, 0, -1, 1, 0, new Uint8Array(32));
  let resolveExact;
  let exactSignal;
  const viewer = await createBrowserViewer({
    bindings: { createViewer: async () => raw },
    canvas: {},
    viewport: viewport(),
    exactQueryBridge: {
      confirm(request) {
        exactSignal = request.signal;
        return new Promise((resolve) => { resolveExact = resolve; });
      },
    },
  });
  const point = { sourceIdentity: SOURCE, pointOrdinal: 0, generation: 1 };
  const externalController = new AbortController();
  const cancelled = viewer.confirmPoint(point, { signal: externalController.signal });
  await Promise.resolve();

  assert.notEqual(exactSignal, externalController.signal);
  externalController.abort();
  assert.equal(exactSignal.aborted, true);
  resolveExact(exactResult(point));
  await assert.rejects(cancelled, (error) => error.code === "exact_query_cancelled");

  const destroyed = viewer.confirmPoint(point);
  await Promise.resolve();
  viewer.destroy();
  assert.equal(exactSignal.aborted, true);
  resolveExact(exactResult(point));
  await assert.rejects(destroyed, (error) => error.code === "viewer_destroyed");
});

function viewport() {
  return { cssWidth: 800, cssHeight: 500, devicePixelRatio: 2 };
}

function exactResult(point) {
  return {
    authority: "exact_source_record",
    sourceIdentity: point.sourceIdentity,
    pointOrdinal: String(point.pointOrdinal),
    generation: point.generation,
  };
}

class FixtureWorker {
  constructor() {
    this.listeners = new Map();
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  postMessage(message) {
    if (message.type === "cancel") {
      this.emit({
        schema: WORKER_SCHEMA,
        type: "failure",
        operation_id: message.operation_id,
        code: "cancelled",
        message: "cancelled",
        safe_action: "retry",
      });
      return;
    }
    queueMicrotask(() => {
      const deployment = fixtureDeployment();
      this.emit({
        schema: WORKER_SCHEMA,
        type: "state",
        operation_id: message.operation_id,
        phase: "deployment",
        deployment,
      });
      this.emit({
        schema: WORKER_SCHEMA,
        type: "batch",
        operation_id: message.operation_id,
        batch_index: 0,
        point_count: 1,
        payload: transferPayload(7n),
      });
      this.emit({
        schema: WORKER_SCHEMA,
        type: "complete",
        operation_id: message.operation_id,
        deployment,
        metrics: { requestCount: 3 },
        decode: { pointCount: 1 },
      });
    });
  }

  terminate() {}

  emit(data) {
    this.listeners.get("message")?.({ data });
  }
}

class PartialFailureWorker extends FixtureWorker {
  postMessage(message) {
    if (message.type === "cancel") {
      super.postMessage(message);
      return;
    }
    queueMicrotask(() => {
      publishFixtureBatch(this, message);
      this.emit({
        schema: WORKER_SCHEMA,
        type: "failure",
        operation_id: message.operation_id,
        code: "cancelled",
        message: "cancelled after publication",
        safe_action: "retry",
      });
    });
  }
}

class PartialCrashWorker extends FixtureWorker {
  postMessage(message) {
    queueMicrotask(() => {
      publishFixtureBatch(this, message);
      this.listeners.get("error")?.({ message: "fixture post-publication worker crash" });
    });
  }
}

function publishFixtureBatch(worker, message) {
  worker.emit({
    schema: WORKER_SCHEMA,
    type: "state",
    operation_id: message.operation_id,
    phase: "deployment",
    deployment: fixtureDeployment(),
  });
  worker.emit({
    schema: WORKER_SCHEMA,
    type: "batch",
    operation_id: message.operation_id,
    batch_index: 0,
    point_count: 1,
    payload: transferPayload(7n),
  });
}

class ManifestCaptureWorker extends FixtureWorker {
  static current;

  constructor() {
    super();
    this.messages = [];
    ManifestCaptureWorker.current = this;
  }

  postMessage(message) {
    this.messages.push(message);
    super.postMessage(message);
  }
}

class OwnedWorkWorker extends FixtureWorker {
  static current;

  constructor() {
    super();
    this.messages = [];
    OwnedWorkWorker.current = this;
  }

  postMessage(message) {
    this.messages.push(message);
  }

  fail() {
    const operation = this.messages.find((message) => message.type === "start");
    this.emit({
      schema: WORKER_SCHEMA,
      type: "failure",
      operation_id: operation.operation_id,
      code: "cancelled",
      message: "cancelled",
      safe_action: "retry",
    });
  }

  crash() {
    this.listeners.get("error")?.({ message: "fixture worker crashed" });
  }
}

class RecoveringWorker extends FixtureWorker {
  static constructions = 0;

  static reset() {
    RecoveringWorker.constructions = 0;
  }

  constructor() {
    super();
    this.crashes = RecoveringWorker.constructions === 0;
    RecoveringWorker.constructions += 1;
  }

  postMessage(message) {
    if (!this.crashes || message.type === "cancel") {
      super.postMessage(message);
      return;
    }
    queueMicrotask(() => {
      this.listeners.get("error")?.({ message: "fixture pre-publication worker crash" });
    });
  }
}

class CompleteWithoutBatchWorker extends FixtureWorker {
  postMessage(message) {
    queueMicrotask(() => {
      this.emit({
        schema: WORKER_SCHEMA,
        type: "complete",
        operation_id: message.operation_id,
        deployment: fixtureDeployment(),
        metrics: {},
        decode: {},
      });
    });
  }
}

function fixtureDeployment() {
  return {
    schema: "punctra-browser-stream-v1",
    deployment_id: "fixture",
    source_identity: SOURCE,
    source_byte_length: 1_000,
    source_point_count: 10,
    root_display_point_count: 1,
    root_coverage: "sampled",
    world_origin: [500_000, 4_600_000, 100],
    source_bounds: { min: [0, 0, 99], max: [0, 0, 103] },
  };
}

function transferPayload(ordinal) {
  const payload = new ArrayBuffer(32);
  const view = new DataView(payload);
  view.setBigUint64(0, ordinal, true);
  view.setUint16(20, 100, true);
  view.setUint8(22, 2);
  view.setUint16(24, 10, true);
  view.setUint16(26, 20, true);
  view.setUint16(28, 30, true);
  return payload;
}

class FakeRawViewer {
  constructor() {
    this.generation = 1;
    this.data = diagnosticsFixture();
  }

  diagnostics() {
    return this.json();
  }

  resize(cssWidth, cssHeight, dpr) {
    this.data.viewport = {
      ...this.data.viewport,
      css_width: cssWidth,
      css_height: cssHeight,
      device_pixel_ratio: dpr,
      physical_width: Math.round(cssWidth * dpr),
      physical_height: Math.round(cssHeight * dpr),
      surface_bytes: Math.round(cssWidth * dpr) * Math.round(cssHeight * dpr) * 4,
    };
    this.data.frame = null;
    return this.json();
  }

  setVisible(visible) {
    this.data.phase = visible ? "ready" : "hidden";
    return this.json();
  }

  setPerspectiveCamera(...values) {
    this.camera(values, "perspective");
    return this.json();
  }

  setOrthographicCamera(...values) {
    this.camera(values, "orthographic");
    return this.json();
  }

  setDisplayMode(mode) {
    this.data.display_mode = mode;
    this.data.streaming.display_mode = mode;
    return this.json();
  }

  render() {
    this.data.rendered_frames += 1;
    this.data.frame = {
      view_generation: this.generation,
      drawn_points: this.data.streaming.phase === "idle" ? 1_089 : 1,
      draw_calls: 1,
      resident_bytes: 24,
      transient_texture_bytes: 100,
      surface_suboptimal: false,
    };
    this.data.pick = emptyPick();
    return this.json();
  }

  beginStreamBatch(source, points, _x, _y, _z, minimumZ, maximumZ, _batch, payload) {
    assert.equal(payload.byteLength, 32);
    this.data.streaming = {
      phase: "receiving",
      source_identity: source,
      view_id: 16,
      generation: this.generation,
      coverage: "sampled",
      expected_points: points,
      published_points: 1,
      published_batches: 1,
      transferred_bytes: 32,
      retained_record_bytes: 32,
      main_thread_batch_points_high_water: 1,
      main_thread_batch_bytes_high_water: 32,
      world_origin: [500_000, 4_600_000, 100],
      source_z_range: [minimumZ, maximumZ],
      display_mode: this.data.display_mode,
      presentation_version: 1,
    };
    this.data.frame = null;
    return this.json();
  }

  publishStreamBatch() {
    return this.json();
  }

  completeStream() {
    this.data.streaming.phase = "complete";
    return this.json();
  }

  beginPick() {
    this.data.pick = { ...emptyPick(), status: "pending" };
    return this.json();
  }

  pollPick() {
    this.data.pick = {
      status: "hit",
      authority: "provisional_gpu_hint",
      generation: this.generation,
      source_identity: SOURCE,
      point_ordinal: "7",
      batch_key: 1,
      batch_version: 1,
    };
    return this.json();
  }

  setHighlights(source, generation, ordinals) {
    this.data.highlights = {
      generation: Number(generation),
      source_identity: source,
      point_count: ordinals.length,
      authority: "presentation_only",
    };
    return this.json();
  }

  clearHighlights(generation) {
    this.data.highlights = {
      generation: Number(generation),
      source_identity: SOURCE,
      point_count: 0,
      authority: "presentation_only",
    };
    return this.json();
  }

  shutdown() {
    this.data.phase = "shutdown";
    return this.json();
  }

  advanceGeneration() {
    this.generation += 1;
    this.data.streaming.generation = this.generation;
  }

  camera(values, projection) {
    this.data.camera = {
      eye: values.slice(0, 3),
      target: values.slice(3, 6),
      up: values.slice(6, 9),
      projection,
      vertical_field_of_view_radians: projection === "perspective" ? values[9] : null,
      vertical_world_height: projection === "orthographic" ? values[9] : null,
      near_distance: values[10],
      far_distance: values[11],
    };
    this.data.frame = null;
  }

  json() {
    return JSON.stringify(this.data);
  }
}

class BoundedResizeRawViewer extends FakeRawViewer {
  resize(cssWidth, cssHeight, dpr) {
    const width = Math.round(cssWidth * dpr);
    const height = Math.round(cssHeight * dpr);
    if (width > 4_096 || height > 4_096 || width * height > 8_388_608) {
      throw new Error(JSON.stringify({
        code: "resize_viewport",
        message: "fixture viewport exceeds the physical ceiling",
        safe_action: "retry with bounded dimensions",
      }));
    }
    return super.resize(cssWidth, cssHeight, dpr);
  }
}

class FusedRawViewer extends FakeRawViewer {
  constructor(code = "device_lost") {
    super();
    this.code = code;
  }

  setDisplayMode() {
    throw new Error(JSON.stringify({ code: this.code, message: "fixture fused failure" }));
  }
}

class OutdatedRawViewer extends FakeRawViewer {
  setDisplayMode() {
    throw new Error(JSON.stringify({
      code: "surface_outdated",
      message: "fixture surface needs a bounded resize",
    }));
  }
}

class PendingRawViewer extends FakeRawViewer {
  constructor() {
    super();
    this.pickActive = false;
    this.pickBegins = 0;
    this.pickCancellations = 0;
  }

  beginPick() {
    if (this.pickActive) {
      throw new Error(JSON.stringify({ code: "pick_pending", message: "fixture pick is pending" }));
    }
    this.pickActive = true;
    this.pickBegins += 1;
    this.data.pick = { ...emptyPick(), status: "pending" };
    return this.json();
  }

  pollPick() {
    return this.json();
  }

  cancelPick() {
    this.pickActive = false;
    this.pickCancellations += 1;
    this.data.pick = emptyPick();
    return this.json();
  }
}

function diagnosticsFixture() {
  return {
    schema: "punctra-browser-viewer-v1",
    package_version: "0.19.0-alpha.1",
    phase: "ready",
    rendered_frames: 0,
    hidden_frame_skips: 0,
    capabilities: { secure_context: true, webgpu: true, adapter_name: "fixture" },
    limits: {
      points: 8_192,
      batches: 8,
      highlight_points: 32,
      estimated_gpu_bytes: 196_608,
    },
    viewport: {
      css_width: 800,
      css_height: 500,
      device_pixel_ratio: 2,
      physical_width: 1_600,
      physical_height: 1_000,
      surface_bytes: 6_400_000,
    },
    scene: {
      source_identity: GENERATED_SOURCE,
      point_count: 1_089,
      generation: 1,
    },
    streaming: {
      phase: "idle",
      source_identity: null,
      view_id: null,
      generation: null,
      coverage: "none",
      expected_points: 0,
      published_points: 0,
      published_batches: 0,
      transferred_bytes: 0,
      retained_record_bytes: 0,
      main_thread_batch_points_high_water: 0,
      main_thread_batch_bytes_high_water: 0,
      world_origin: null,
      source_z_range: null,
      display_mode: "rgb",
      presentation_version: 0,
    },
    streaming_limits: { retained_record_bytes: 262_144, worker_staging_bytes: 327_680 },
    frame: null,
    pick: emptyPick(),
    camera: {
      eye: [500_000, 4_599_969, 122],
      target: [500_000, 4_600_000, 100],
      up: [0, 0, 1],
      projection: "perspective",
      vertical_field_of_view_radians: Math.PI / 3,
      vertical_world_height: null,
      near_distance: 0.1,
      far_distance: 250,
    },
    display_mode: "rgb",
    highlights: {
      generation: null,
      source_identity: null,
      point_count: 0,
      authority: "presentation_only",
    },
  };
}

function emptyPick() {
  return {
    status: "not_requested",
    authority: "provisional_gpu_hint",
    generation: null,
    source_identity: null,
    point_ordinal: null,
    batch_key: null,
    batch_version: null,
  };
}
