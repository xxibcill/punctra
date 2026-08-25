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

test("runtime error codes and display modes agree with TypeScript declarations", async () => {
  const declaration = await readFile(new URL("viewer-api.d.ts", import.meta.url), "utf8");
  const errorBlock = declaration.match(/export type ViewerErrorCode =([\s\S]*?);\n\nexport const/)[1];
  const declaredErrors = [...errorBlock.matchAll(/"([a-z0-9_]+)"/g)].map((match) => match[1]);
  const displayBlock = declaration.match(/export type DisplayMode =([\s\S]*?);/)[1];
  const declaredModes = [...displayBlock.matchAll(/"([a-z]+)"/g)].map((match) => match[1]);

  assert.deepEqual(declaredErrors, VIEWER_ERROR_CODES);
  assert.deepEqual(declaredModes, DISPLAY_MODES);
  assert.equal(new Set(VIEWER_ERROR_CODES).size, VIEWER_ERROR_CODES.length);
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
  assert.ok(observed.length >= 4);
  assert.equal(unsubscribe(), true);

  viewer.destroy();
  viewer.destroy();
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

  const pick = await viewer.pick({ x: 10, y: 20 });
  assert.equal(pick.sourceIdentity, SOURCE);
  assert.equal(pick.pointOrdinal, "7");
  viewer.setHighlights([pick], pick.generation);
  assert.equal(viewer.state().highlights.pointCount, 1);
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
        type: "failure",
        operation_id: message.operation_id,
        code: "cancelled",
        message: "cancelled after publication",
        safe_action: "retry",
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

class FusedRawViewer extends FakeRawViewer {
  setDisplayMode() {
    throw new Error(JSON.stringify({ code: "device_lost", message: "fixture device loss" }));
  }
}

function diagnosticsFixture() {
  return {
    schema: "punctra-browser-viewer-v1",
    package_version: "0.17.0-alpha.1",
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
    scene: { point_count: 1_089, generation: 1 },
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
