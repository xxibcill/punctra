import assert from "node:assert/strict";
import test from "node:test";

import type {
  BrowserViewer,
  CreateViewerOptions,
  ExactPoint,
  ExactQueryBridge,
  ProvisionalPick,
  SourceLoadResult,
  ViewerCamera,
  ViewerState,
} from "@punctra/viewer";

import { runQuickstartAcceptance } from "./acceptance.ts";
import type { PackedRuntimeProof } from "./packed-runtime.ts";
import { QuickstartController } from "./quickstart.ts";

const packedRuntime: PackedRuntimeProof = {
  schema: "punctra-browser-packed-runtime-v1",
  build: "production",
  serverContract: "punctra-strict-range-v1",
  viewerPackage: "@punctra/viewer",
  viewerVersion: "0.20.0-alpha.1",
  viewerArtifactSha256: "b61e59f0f0b34776158494af272dc684156c24806a0abbefa2c0d2b626e7e834",
};

test("packed quickstart exercises the supported workflow and disposes", async () => {
  const firstViewer = new FakeViewer();
  const recreatedViewer = new FakeViewer();
  const viewers = [firstViewer, recreatedViewer];
  const createOptions: CreateViewerOptions[] = [];
  const snapshots: string[] = [];
  const controller = new QuickstartController({
    canvas: {} as HTMLCanvasElement,
    viewport: { cssWidth: 960, cssHeight: 600, devicePixelRatio: 1 },
    manifestUrl: "https://fixtures.test/fixtures/v1/deployment.json",
    createViewer: async (options) => {
      createOptions.push(options);
      return viewers.shift() as unknown as BrowserViewer;
    },
    createExactBridge: () => ({ confirm: async () => exactPoint() } as ExactQueryBridge),
    publish: (snapshot) => snapshots.push(snapshot.operation),
  });

  const record = await runQuickstartAcceptance(
    controller,
    "https://fixtures.test/fixtures/v1/deployment.json",
    packedRuntime,
  );

  assert.equal(createOptions.length, 2);
  assert.equal(createOptions[0]?.canvas !== undefined, true);
  assert.equal(createOptions[0]?.assets?.cacheKey, "v0.20-quickstart");
  assert.equal(record.schema, "punctra-browser-quickstart-acceptance-v1");
  assert.equal(record.packageVersion, "0.20.0-alpha.1");
  assert.equal(record.displayedPoints, 4_096);
  assert.deepEqual(record.projections, ["orthographic", "perspective"]);
  assert.equal(record.provisionalAuthority, "provisional_gpu_hint");
  assert.equal(record.exactAuthority, "exact_source_record");
  assert.equal(record.recoverableFailureCode, "offline");
  assert.equal(record.retryRetainedViewer, true);
  assert.equal(record.retrySucceeded, true);
  assert.equal(record.recreationFailureCode, "cancelled");
  assert.equal(record.recreationRequired, true);
  assert.equal(record.recreationSucceeded, true);
  assert.equal(record.disposed, true);
  assert.deepEqual(record.packedRuntime, packedRuntime);
  assert.equal(firstViewer.disposals, 1);
  assert.equal(recreatedViewer.disposals, 1);
  assert.equal(controller.state(), null);
  assert(snapshots.includes("Exact Source record confirmed"));
  assert(snapshots.includes("Viewer disposed"));
});

test("a superseded asynchronous mount disposes the late viewer", async () => {
  const first = new FakeViewer();
  const second = new FakeViewer();
  const firstCreation = deferredViewer(first);
  const secondCreation = deferredViewer(second);
  const creations = [firstCreation, secondCreation];
  const controller = quickstartController(async () => creations.shift()!.promise);

  const staleMount = controller.mount();
  const currentMount = controller.mount();
  secondCreation.resolve();
  await currentMount;
  firstCreation.resolve();
  assert.equal(await staleMount, controller.state());
  assert.equal(first.disposals, 1);
  assert.equal(second.disposals, 0);
});

test("disposing during asynchronous mount disposes the unpublished viewer", async () => {
  const viewer = new FakeViewer();
  const creation = deferredViewer(viewer);
  const controller = quickstartController(async () => creation.promise);
  const mount = controller.mount();

  controller.dispose();
  creation.resolve();
  await assert.rejects(mount, /cancelled before publication/);
  assert.equal(viewer.disposals, 1);
  assert.equal(controller.state(), null);
});

test("a superseded mount rejection is reported as cancellation", async () => {
  const firstCreation = deferred<BrowserViewer>();
  const second = new FakeViewer();
  const secondCreation = deferred<BrowserViewer>();
  const creations = [firstCreation.promise, secondCreation.promise];
  const controller = quickstartController(async () => creations.shift()!);

  const staleMount = controller.mount();
  const currentMount = controller.mount();
  firstCreation.reject(new Error("stale initialization failure"));
  await assert.rejects(staleMount, (error) => error instanceof DOMException && error.name === "AbortError");
  secondCreation.resolve(second as unknown as BrowserViewer);
  await currentMount;
  assert.equal(controller.state(), second.state());
});

test("a pick resolving after remount cannot select a Point in the new viewer", async () => {
  const first = new FakeViewer();
  const second = new FakeViewer();
  const pendingPick = deferred<ProvisionalPick>();
  first.pick = async () => pendingPick.promise;
  const viewers = [first, second];
  const controller = quickstartController(
    async () => viewers.shift() as unknown as BrowserViewer,
  );

  await controller.mount();
  const stalePick = controller.pick(10, 10);
  await controller.mount();
  pendingPick.resolve(provisionalPick(1));

  await assert.rejects(stalePick, (error) => error instanceof DOMException && error.name === "AbortError");
  assert.throws(() => controller.highlightSelected(), /Pick a resident display Point/);
});

const SOURCE_IDENTITY = "c459ff39717b7d6994aaebf344641f5a3add7faf65e249b85933ebd066d1c26e";

class FakeViewer {
  data = viewerState();
  listeners = new Set<(state: ViewerState) => void>();
  disposals = 0;

  state(): ViewerState {
    return this.data;
  }

  subscribe(listener: (state: ViewerState) => void): () => boolean {
    this.listeners.add(listener);
    listener(this.data);
    return () => this.listeners.delete(listener);
  }

  render(): ViewerState {
    return this.publish({
      render: { ...this.data.render, renderedFrames: this.data.render.renderedFrames + 1 },
    });
  }

  async loadSource(options: { manifestUrl: string; signal?: AbortSignal }): Promise<SourceLoadResult> {
    if (options.manifestUrl.includes("delay_ms")) {
      await new Promise<never>((_resolve, reject) => {
        options.signal?.addEventListener("abort", () => reject({ code: "cancelled" }), { once: true });
      });
    }
    if (options.manifestUrl.includes("fault=disconnect")) {
      throw viewerFailure("offline", true);
    }
    if (options.manifestUrl.includes("acceptance_phase=partial-publication")) {
      this.publish({
        generation: this.data.generation + 1,
        source: {
          ...this.data.source,
          identity: SOURCE_IDENTITY,
          coverage: "sampled",
          expectedPoints: 4_096,
          publishedPoints: 1_024,
          publishedBatches: 1,
          retainedRecordBytes: 32_768,
        },
      });
      assert.equal(options.signal?.aborted, true);
      this.publish({ lifecycle: "destroyed" });
      throw viewerFailure("cancelled", false);
    }
    const state = this.publish({
      generation: this.data.generation + 1,
      source: {
        ...this.data.source,
        identity: SOURCE_IDENTITY,
        coverage: "sampled",
        expectedPoints: 4_096,
        publishedPoints: 4_096,
        publishedBatches: 4,
        retainedRecordBytes: 131_072,
      },
    });
    return {
      deployment: {},
      metrics: {},
      decode: {},
      pointOrdinals: [0],
      timings: {
        firstCoverageMilliseconds: 1,
        settledViewMilliseconds: 2,
        mainThreadBatchMillisecondsHighWater: 0.5,
      },
      mainThreadMillisecondsHighWater: 0.5,
      state,
    };
  }

  setDisplayMode(displayMode: ViewerState["displayMode"]): ViewerState {
    return this.publish({ displayMode });
  }

  resize(viewport: { cssWidth: number; cssHeight: number; devicePixelRatio: number }): ViewerState {
    return this.publish({
      viewport: {
        ...viewport,
        physicalWidth: Math.round(viewport.cssWidth * viewport.devicePixelRatio),
        physicalHeight: Math.round(viewport.cssHeight * viewport.devicePixelRatio),
        surfaceBytes: Math.round(viewport.cssWidth * viewport.devicePixelRatio)
          * Math.round(viewport.cssHeight * viewport.devicePixelRatio) * 4,
      },
    });
  }

  setCamera(camera: ViewerCamera): ViewerState {
    return this.publish({ camera: cameraState(camera) });
  }

  requestRender(): Promise<ViewerState> {
    return Promise.resolve(this.render());
  }

  async pick(): Promise<ProvisionalPick> {
    return provisionalPick(this.data.generation);
  }

  setHighlights(): ViewerState {
    return this.publish({
      highlights: {
        generation: this.data.generation,
        sourceIdentity: SOURCE_IDENTITY,
        pointCount: 1,
        authority: "presentation_only",
      },
    });
  }

  clearHighlights(): ViewerState {
    return this.publish({
      highlights: { ...this.data.highlights, pointCount: 0 },
    });
  }

  confirmPoint(): Promise<ExactPoint> {
    return Promise.resolve(exactPoint(this.data.generation));
  }

  pause(): ViewerState {
    return this.publish({ lifecycle: "hidden" });
  }

  resume(): ViewerState {
    return this.publish({ lifecycle: "ready" });
  }

  dispose(): void {
    this.disposals += 1;
  }

  publish(changes: Partial<ViewerState>): ViewerState {
    this.data = { ...this.data, ...changes } as ViewerState;
    for (const listener of this.listeners) listener(this.data);
    return this.data;
  }
}

function viewerFailure(code: "offline" | "cancelled", recoverable: boolean) {
  return {
    schema: "punctra-viewer-error-v1",
    code,
    recoverable,
    safeAction: recoverable
      ? "Correct the reported condition and retry."
      : "Dispose the fused viewer and create a new one before any Source load.",
  };
}

function quickstartController(createViewer: (options: CreateViewerOptions) => Promise<BrowserViewer>) {
  return new QuickstartController({
    canvas: {} as HTMLCanvasElement,
    viewport: { cssWidth: 960, cssHeight: 600, devicePixelRatio: 1 },
    manifestUrl: "https://fixtures.test/fixtures/v1/deployment.json",
    createViewer,
    createExactBridge: () => ({ confirm: async () => exactPoint() } as ExactQueryBridge),
    publish: () => {},
  });
}

function deferredViewer(viewer: FakeViewer) {
  let resolve!: () => void;
  const promise = new Promise<BrowserViewer>((settle) => {
    resolve = () => settle(viewer as unknown as BrowserViewer);
  });
  return { promise, resolve };
}

function deferred<Value>() {
  let resolve!: (value: Value) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<Value>((settle, fail) => {
    resolve = settle;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function viewerState(): ViewerState {
  return {
    schema: "punctra-viewer-state-v1",
    packageVersion: "0.20.0-alpha.1",
    lifecycle: "ready",
    generation: 1,
    source: {
      identity: null,
      coverage: "none",
      expectedPoints: 0,
      publishedPoints: 0,
      publishedBatches: 0,
      retainedRecordBytes: 0,
    },
    viewport: {
      cssWidth: 960,
      cssHeight: 600,
      devicePixelRatio: 1,
      physicalWidth: 960,
      physicalHeight: 600,
      surfaceBytes: 2_304_000,
    },
    camera: cameraState({
      projection: "perspective",
      eye: [0, -10, 10],
      target: [0, 0, 0],
      up: [0, 0, 1],
      verticalFieldOfViewRadians: Math.PI / 3,
      nearDistance: 0.1,
      farDistance: 100,
    }),
    displayMode: "rgb",
    render: {
      scheduled: false,
      renderedFrames: 0,
      hiddenFrameSkips: 0,
      drawnPoints: 0,
      drawCalls: 0,
      residentBytes: 98_304,
      transientTextureBytes: 0,
      surfaceSuboptimal: false,
    },
    pick: {
      status: "not_requested",
      authority: "provisional_gpu_hint",
      generation: null,
      sourceIdentity: null,
      pointOrdinal: null,
      batchKey: null,
      batchVersion: null,
    },
    highlights: {
      generation: null,
      sourceIdentity: null,
      pointCount: 0,
      authority: "presentation_only",
    },
    resources: {
      pointLimit: 8_192,
      batchLimit: 8,
      highlightPointLimit: 32,
      residentByteLimit: 196_608,
      retainedRecordByteLimit: 262_144,
      workerStagingByteLimit: 327_680,
    },
    capabilities: {},
    load: { active: false, facts: null },
    failure: null,
  };
}

function cameraState(camera: ViewerCamera): ViewerState["camera"] {
  return camera.projection === "perspective"
    ? { ...camera, verticalWorldHeight: null }
    : { ...camera, verticalFieldOfViewRadians: null };
}

function provisionalPick(generation: number): ProvisionalPick {
  return {
    status: "hit",
    authority: "provisional_gpu_hint",
    sourceIdentity: SOURCE_IDENTITY,
    pointOrdinal: "7",
    generation,
    batchKey: 1,
    batchVersion: 1,
  };
}

function exactPoint(generation = 2): ExactPoint {
  return {
    authority: "exact_source_record",
    sourceIdentity: SOURCE_IDENTITY,
    pointOrdinal: "7",
    generation,
    ticks: [0, 0, 0],
    position: [500_000, 4_600_000, 100],
    intensity: 7,
    classification: 2,
    rgb: [7, 7, 7],
  };
}
