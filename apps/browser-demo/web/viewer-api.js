const MODULE_CACHE_TOKEN = encodeURIComponent(
  new URL(import.meta.url).searchParams.get("v") ?? "unversioned",
);
const [
  { appendTransferredOrdinals },
  { runWorkerOperation },
  { WORKER_SCHEMA, workerFailure },
  { CAMERA_PROJECTION_POLICIES },
] = await Promise.all([
  import(`./stream-ordinals.js?v=${MODULE_CACHE_TOKEN}`),
  import(`./worker-operation.js?v=${MODULE_CACHE_TOKEN}`),
  import(`./worker-protocol.js?v=${MODULE_CACHE_TOKEN}`),
  import(`./camera-policy.js?v=${MODULE_CACHE_TOKEN}`),
]);

export const DISPLAY_MODES = Object.freeze([
  "neutral",
  "elevation",
  "rgb",
  "intensity",
  "classification",
]);

export const VIEWER_ERROR_CODES = Object.freeze([
  "invalid_argument",
  "viewer_destroyed",
  "load_busy",
  "render_cancelled",
  "internal",
  "capability_inspection",
  "canvas_surface",
  "device_lost",
  "device_poll",
  "diagnostic_serialization",
  "frame_recording",
  "frame_validation",
  "camera_validation",
  "host_model",
  "initial_viewport",
  "insecure_context",
  "missing_recorded_frame",
  "missing_window",
  "pick_invariant",
  "pick_not_requested",
  "pick_outside_viewport",
  "pick_pending",
  "pick_readback",
  "pick_recording",
  "highlight_validation",
  "presentation_mode",
  "renderer_capability",
  "resize_viewport",
  "scene_planning",
  "scene_publication",
  "scene_validation",
  "stream_publication",
  "stream_validation",
  "stale_generation",
  "display_mode",
  "surface_alpha_mode",
  "surface_configuration",
  "surface_format",
  "surface_lost",
  "surface_occluded",
  "surface_outdated",
  "surface_reconfiguration",
  "surface_timeout",
  "surface_validation",
  "transient_texture_limit",
  "viewer_hidden",
  "viewport_validation",
  "webgpu_adapter",
  "webgpu_device",
  "webgpu_unavailable",
  "manifest_invalid",
  "unsupported_deployment",
  "range_unsupported",
  "cors_headers_hidden",
  "content_encoding",
  "source_changed",
  "range_truncated",
  "range_corrupt",
  "index_incompatible",
  "offline",
  "retry_exhausted",
  "cache_quota",
  "cache_unavailable",
  "cancelled",
  "worker_failed",
  "resource_limit",
  "exact_query_invalid",
  "exact_query_unavailable",
  "exact_query_busy",
  "exact_query_cancelled",
  "exact_query_source_mismatch",
  "exact_query_source_changed",
  "exact_query_incompatible",
  "exact_query_corrupt",
  "exact_query_truncated",
  "exact_query_range_unsupported",
  "exact_query_content_encoding",
  "exact_query_failed",
]);

const ERROR_CODE_SET = new Set(VIEWER_ERROR_CODES);
const RECOVERABLE_CODES = new Set([
  "invalid_argument",
  "load_busy",
  "render_cancelled",
  "camera_validation",
  "highlight_validation",
  "resize_viewport",
  "pick_outside_viewport",
  "pick_pending",
  "pick_not_requested",
  "missing_recorded_frame",
  "stale_generation",
  "display_mode",
  "surface_timeout",
  "surface_occluded",
  "surface_outdated",
  "viewer_hidden",
  "stream_validation",
  "manifest_invalid",
  "unsupported_deployment",
  "range_unsupported",
  "cors_headers_hidden",
  "content_encoding",
  "source_changed",
  "range_truncated",
  "range_corrupt",
  "index_incompatible",
  "offline",
  "retry_exhausted",
  "cache_quota",
  "cache_unavailable",
  "cancelled",
  "worker_failed",
  "resource_limit",
  ...VIEWER_ERROR_CODES.filter((code) => code.startsWith("exact_query_")),
]);
const FUSED_CODES = new Set([
  "canvas_surface",
  "device_lost",
  "device_poll",
  "frame_recording",
  "host_model",
  "pick_invariant",
  "pick_readback",
  "pick_recording",
  "renderer_capability",
  "scene_publication",
  "stream_publication",
  "surface_configuration",
  "surface_lost",
  "surface_reconfiguration",
  "surface_validation",
  "transient_texture_limit",
]);
const ERROR_SCHEMA = "punctra-viewer-error-v1";
const STATE_SCHEMA = "punctra-viewer-state-v1";
const MAX_PICK_POLLS = 180;
const MAX_ERROR_MESSAGE_CHARACTERS = 512;
const LOAD_TIMEOUT_MILLISECONDS = 30_000;
const MAX_POINT_ORDINAL = (1n << 64n) - 1n;
const PARTIAL_PUBLICATION_SAFE_ACTION =
  "Destroy the partially published viewer and explicitly create a new viewer before loading another Source.";
const VIEWER_DESTROYED_ABORT = Symbol("viewer_destroyed");
let operationSequence = 0;

export class ViewerError extends Error {
  constructor(code, message, options = {}) {
    const normalizedCode = ERROR_CODE_SET.has(code) ? code : "internal";
    super(boundedMessage(message));
    this.name = "ViewerError";
    this.schema = ERROR_SCHEMA;
    this.code = normalizedCode;
    this.safeAction = boundedMessage(
      options.safeAction
        ?? "Keep the last known safe state, dispose the viewer if it is fused, and retry only after correcting the reported condition.",
    );
    this.recoverable = options.recoverable ?? RECOVERABLE_CODES.has(normalizedCode);
  }
}

export async function createBrowserViewer(options) {
  const bindings = options?.bindings;
  if (typeof bindings?.createViewer !== "function") {
    throw invalidArgument("bindings.createViewer must be a function");
  }
  const canvas = options?.canvas;
  if (!canvas) throw invalidArgument("canvas is required");
  const viewport = viewportInput(options.viewport);
  let raw;
  try {
    raw = await bindings.createViewer(
      canvas,
      viewport.cssWidth,
      viewport.cssHeight,
      viewport.devicePixelRatio,
    );
  } catch (error) {
    throw toViewerError(error, "internal");
  }
  try {
    return new BrowserViewer(raw, options);
  } catch (error) {
    try {
      raw?.shutdown?.();
    } catch {
      // Preserve the construction failure that prevented a usable facade.
    }
    throw toViewerError(error, "internal");
  }
}

class BrowserViewer {
  #raw;
  #diagnostics;
  #state;
  #listeners = new Set();
  #destroyed = false;
  #exactQueryBridge;
  #WorkerConstructor;
  #workerUrl;
  #requestAnimationFrame;
  #cancelAnimationFrame;
  #renderRequest;
  #loadController;
  #exactController;
  #pickController;
  #loadFacts;
  #lastFailure;

  constructor(raw, options = {}) {
    this.#raw = raw;
    this.#exactQueryBridge = options.exactQueryBridge;
    this.#WorkerConstructor = options.WorkerConstructor ?? globalThis.Worker;
    this.#workerUrl = String(options.workerUrl ?? new URL("./stream-worker.js", import.meta.url));
    this.#requestAnimationFrame = options.requestAnimationFrame
      ?? globalThis.requestAnimationFrame?.bind(globalThis)
      ?? ((callback) => globalThis.setTimeout(() => callback(performance.now()), 0));
    this.#cancelAnimationFrame = options.cancelAnimationFrame
      ?? globalThis.cancelAnimationFrame?.bind(globalThis)
      ?? globalThis.clearTimeout.bind(globalThis);
    this.#diagnostics = this.#readRawDiagnostics();
    this.#refreshState();
  }

  state() {
    return this.#state;
  }

  subscribe(listener) {
    return this.#execute(() => {
      this.#ensureActive();
      if (typeof listener !== "function") throw invalidArgument("state listener must be a function");
      this.#listeners.add(listener);
      listener(this.#state);
      return () => this.#listeners.delete(listener);
    });
  }

  resize(viewport) {
    return this.#execute(() => {
      const value = viewportInput(viewport);
      return this.#callRaw(
        "resize",
        value.cssWidth,
        value.cssHeight,
        value.devicePixelRatio,
      );
    });
  }

  setVisible(visible) {
    return this.#execute(() => {
      if (typeof visible !== "boolean") throw invalidArgument("visible must be boolean");
      this.#callRaw("setVisible", visible);
      if (!visible) this.#cancelScheduledRender("scheduled render was cancelled because the viewer was hidden");
      return this.#state;
    });
  }

  setCamera(camera) {
    return this.#execute(() => {
      const value = cameraInput(camera);
      const policy = cameraProjectionPolicy(value.projection);
      const shared = [
        ...value.eye,
        ...value.target,
        ...value.up,
      ];
      return this.#callRaw(
        policy.rawMethod,
        ...shared,
        value[policy.extentProperty],
        value.nearDistance,
        value.farDistance,
      );
    });
  }

  setDisplayMode(mode) {
    return this.#execute(() => {
      if (!DISPLAY_MODES.includes(mode)) {
        throw invalidArgument(`display mode must be one of ${DISPLAY_MODES.join(", ")}`);
      }
      return this.#callRaw("setDisplayMode", mode);
    });
  }

  render() {
    return this.#callRaw("render");
  }

  requestRender() {
    this.#ensureActive();
    if (this.#renderRequest) return this.#renderRequest.promise;
    let resolve;
    let reject;
    const promise = new Promise((resolvePromise, rejectPromise) => {
      resolve = resolvePromise;
      reject = rejectPromise;
    });
    const id = this.#requestAnimationFrame(() => {
      const request = this.#renderRequest;
      this.#renderRequest = undefined;
      if (!request) return;
      try {
        request.resolve(this.render());
      } catch (error) {
        request.reject(toViewerError(error));
      }
    });
    this.#renderRequest = { id, promise, resolve, reject };
    this.#refreshState();
    return promise;
  }

  async loadSource(options) {
    return this.#executeAsync(() => this.#loadSource(options));
  }

  async #loadSource(options) {
    this.#ensureActive();
    if (this.#loadController) throw new ViewerError("load_busy", "one Source load is already active");
    if (typeof this.#WorkerConstructor !== "function") {
      throw new ViewerError("worker_failed", "Web Worker construction is unavailable");
    }
    const manifestUrl = requiredString(options?.manifestUrl, "manifestUrl");
    const cacheMode = cacheModeInput(options?.cacheMode ?? "none");
    const credentials = credentialsInput(options?.credentials ?? "same-origin");
    const invalidate = options?.invalidate === true;
    const controller = linkedAbortController(options?.signal);
    this.#loadController = controller;
    const operationId = `punctra-viewer-${Date.now()}-${operationSequence}`;
    operationSequence += 1;
    const workerUrl = `${this.#workerUrl}${this.#workerUrl.includes("?") ? "&" : "?"}operation=${encodeURIComponent(operationId)}`;
    let deployment;
    let begun = false;
    let mainThreadMillisecondsHighWater = 0;
    const pointOrdinals = [];

    try {
      const result = await runWorkerOperation({
        WorkerConstructor: this.#WorkerConstructor,
        workerUrl,
        workerName: operationId,
        timeoutMilliseconds: LOAD_TIMEOUT_MILLISECONDS,
        timeoutFailure: workerFailure("viewer Source worker did not complete within 30 seconds"),
        errorFailure: (event) => workerFailure(event.message),
        messageErrorFailure: workerFailure("the browser could not deserialize a worker message"),
        signal: controller.signal,
        cancellationMessage: {
          schema: WORKER_SCHEMA,
          type: "cancel",
          operation_id: operationId,
        },
        initialMessage: {
          schema: WORKER_SCHEMA,
          type: "start",
          operation_id: operationId,
          manifest_url: manifestUrl,
          cache_mode: cacheMode,
          invalidate,
          credentials,
        },
        onMessage: (message, controls) => {
          if (this.#destroyed) {
            controls.reject(new ViewerError("viewer_destroyed", "viewer was destroyed during Source loading"));
            return;
          }
          if (message?.schema !== WORKER_SCHEMA || message.operation_id !== operationId) return;
          if (message.type === "failure") {
            controls.reject(message);
          } else if (message.type === "state") {
            if (message.phase === "deployment") deployment = message.deployment;
          } else if (message.type === "batch") {
            const started = performance.now();
            this.#publishWorkerBatch(deployment, message, begun);
            begun = true;
            appendTransferredOrdinals(pointOrdinals, message.payload);
            this.render();
            mainThreadMillisecondsHighWater = Math.max(
              mainThreadMillisecondsHighWater,
              performance.now() - started,
            );
          } else if (message.type === "complete") {
            this.#callRaw("completeStream");
            const state = this.render();
            controls.resolve({
              deployment: message.deployment,
              metrics: message.metrics,
              decode: message.decode,
              pointOrdinals,
              mainThreadMillisecondsHighWater,
              state,
            });
          }
        },
      });
      this.#loadFacts = {
        deployment: result.deployment,
        metrics: result.metrics,
        decode: result.decode,
        mainThreadMillisecondsHighWater: result.mainThreadMillisecondsHighWater,
      };
      this.#loadController = undefined;
      controller.dispose();
      this.#refreshState();
      return deepFreeze({ ...result, state: this.#state });
    } catch (error) {
      if (this.#destroyed) {
        throw new ViewerError("viewer_destroyed", "viewer was destroyed during Source loading");
      }
      const viewerError = toViewerError(error, begun ? "stream_publication" : "worker_failed");
      if (!begun) throw viewerError;
      const fusedError = new ViewerError(viewerError.code, viewerError.message, {
        safeAction: PARTIAL_PUBLICATION_SAFE_ACTION,
        recoverable: false,
      });
      this.#fuseViewer(fusedError);
      throw fusedError;
    } finally {
      if (this.#loadController === controller) {
        this.#loadController = undefined;
        controller.dispose();
        if (!this.#destroyed) this.#refreshState();
      }
    }
  }

  async pick(request) {
    return this.#executeAsync(() => this.#pick(request));
  }

  async #pick(request) {
    this.#ensureActive();
    if (this.#pickController) {
      throw new ViewerError("pick_pending", "one provisional pick is already active");
    }
    const x = pickCoordinate(request?.x, this.#state.viewport.physicalWidth, "pick x");
    const y = pickCoordinate(request?.y, this.#state.viewport.physicalHeight, "pick y");
    const controller = linkedAbortController(request?.signal);
    this.#pickController = controller;
    let rawPickActive = false;
    try {
      assertNotCancelled(controller.signal, "cancelled");
      this.#callRaw("beginPick", x, y);
      rawPickActive = true;
      for (let attempt = 0; attempt < MAX_PICK_POLLS; attempt += 1) {
        await animationFrame(
          this.#requestAnimationFrame,
          this.#cancelAnimationFrame,
          controller.signal,
        );
        const state = this.#callRaw("pollPick");
        if (state.pick.status === "miss") {
          rawPickActive = false;
          return null;
        }
        if (state.pick.status === "hit") {
          rawPickActive = false;
          return deepFreeze({ ...state.pick });
        }
      }
      throw new ViewerError("pick_pending", "provisional pick exceeded 180 animation frames");
    } finally {
      if (rawPickActive && !this.#destroyed) this.#callRaw("cancelPick");
      if (this.#pickController === controller) this.#pickController = undefined;
      controller.dispose();
    }
  }

  setHighlights(points, generation = this.#state.generation) {
    return this.#execute(() => {
      this.#ensureActive();
      if (!Array.isArray(points)) throw invalidArgument("highlights must be an array");
      const pointLimit = this.#state.resources.highlightPointLimit;
      if (points.length > pointLimit) {
        throw invalidArgument(`highlights exceed the ${pointLimit}-Point ceiling`);
      }
      if (points.length === 0) {
        return this.#callRaw(
          "clearHighlights",
          BigInt(positiveInteger(generation, "generation")),
        );
      }
      const identities = points.map(pointIdentityInput);
      const sourceIdentity = identities[0]?.sourceIdentity ?? this.#state.source?.identity;
      if (!sourceIdentity) throw invalidArgument("the active View has no Source identity");
      if (identities.some((point) => point.sourceIdentity !== sourceIdentity)) {
        throw invalidArgument("all highlights must belong to one Source");
      }
      const ordinals = new BigUint64Array(identities.map((point) => point.pointOrdinal));
      return this.#callRaw(
        "setHighlights",
        sourceIdentity,
        BigInt(positiveInteger(generation, "generation")),
        ordinals,
      );
    });
  }

  clearHighlights(generation = this.#state.generation) {
    return this.setHighlights([], generation);
  }

  async confirmPoint(point, options = {}) {
    return this.#executeAsync(() => this.#confirmPoint(point, options));
  }

  async #confirmPoint(point, options) {
    this.#ensureActive();
    if (typeof this.#exactQueryBridge?.confirm !== "function") {
      throw new ViewerError("exact_query_unavailable", "no exact-Query bridge was supplied");
    }
    if (this.#exactController) {
      throw new ViewerError("exact_query_busy", "one exact-Query handoff is already active");
    }
    const identity = pointIdentityInput(point);
    const generation = positiveInteger(point?.generation ?? this.#state.generation, "generation");
    this.#requireCurrentPoint(identity, generation);
    const controller = linkedAbortController(options.signal);
    this.#exactController = controller;
    try {
      assertNotCancelled(controller.signal, "exact_query_cancelled");
      let result;
      try {
        result = await this.#exactQueryBridge.confirm({
          sourceIdentity: identity.sourceIdentity,
          pointOrdinal: identity.pointOrdinal,
          generation,
          signal: controller.signal,
        });
      } catch (error) {
        this.#ensureActive();
        assertNotCancelled(controller.signal, "exact_query_cancelled");
        throw toViewerError(error, "exact_query_failed");
      }
      this.#ensureActive();
      assertNotCancelled(controller.signal, "exact_query_cancelled");
      this.#requireCurrentPoint(identity, generation);
      if (!matchesExactPoint(result, identity, generation)) {
        throw new ViewerError("exact_query_source_mismatch", "exact bridge returned a mismatched Point result");
      }
      return deepFreeze(result);
    } finally {
      if (this.#exactController === controller) this.#exactController = undefined;
      controller.dispose();
    }
  }

  destroy() {
    if (this.#destroyed) return;
    const renderFailure = this.#cancelScheduledRender(
      "scheduled render was cancelled by viewer destruction",
      false,
    );
    this.#destroyViewer(renderFailure);
  }

  #destroyViewer(failure) {
    if (this.#destroyed) return;
    this.#destroyed = true;
    const controllers = [this.#loadController, this.#exactController, this.#pickController];
    this.#loadController = undefined;
    this.#exactController = undefined;
    this.#pickController = undefined;
    for (const controller of controllers) {
      controller?.abort(VIEWER_DESTROYED_ABORT);
      controller?.dispose();
    }
    try {
      this.#diagnostics = parseDiagnostics(this.#raw.shutdown());
    } catch {
      // A fused raw viewer already owns no safe continuation.
    }
    this.#refreshState(failure);
    this.#listeners.clear();
  }

  #publishWorkerBatch(deployment, message, begun) {
    if (!deployment?.source_bounds || !(message.payload instanceof ArrayBuffer)) {
      throw new ViewerError("stream_validation", "worker batch preceded a complete deployment binding");
    }
    const payload = new Uint8Array(message.payload);
    if (!begun) {
      this.#cancelScheduledRender("scheduled render was cancelled by a new Source generation");
      const [x, y, z] = deployment.world_origin;
      this.#callRaw(
        "beginStreamBatch",
        deployment.source_identity,
        deployment.root_display_point_count,
        x,
        y,
        z,
        deployment.source_bounds.min[2],
        deployment.source_bounds.max[2],
        message.batch_index,
        payload,
      );
    } else {
      this.#callRaw("publishStreamBatch", message.batch_index, payload);
    }
  }

  #callRaw(method, ...arguments_) {
    this.#ensureActive();
    try {
      this.#diagnostics = parseDiagnostics(this.#raw[method](...arguments_));
      this.#refreshState();
      return this.#state;
    } catch (error) {
      const viewerError = toViewerError(error);
      if (FUSED_CODES.has(viewerError.code)) {
        this.#fuseViewer(viewerError);
      } else {
        this.#refreshState(viewerError);
      }
      throw viewerError;
    }
  }

  #readRawDiagnostics() {
    try {
      return parseDiagnostics(this.#raw.diagnostics());
    } catch (error) {
      throw toViewerError(error);
    }
  }

  #refreshState(failure) {
    if (failure) this.#lastFailure = failure;
    this.#state = publicState(
      this.#diagnostics,
      this.#destroyed,
      this.#renderRequest !== undefined,
      this.#loadController !== undefined,
      this.#loadFacts,
      this.#lastFailure,
    );
    for (const listener of this.#listeners) {
      try {
        listener(this.#state);
      } catch (error) {
        globalThis.reportError?.(error);
      }
    }
  }

  #ensureActive() {
    if (!this.#destroyed) return;
    const failure = new ViewerError("viewer_destroyed", "viewer has been destroyed");
    this.#recordFailure(failure);
    throw failure;
  }

  #fuseViewer(failure) {
    this.#cancelScheduledRender("scheduled render was cancelled by a fused viewer failure", false);
    this.#destroyViewer(failure);
  }

  #execute(operation) {
    try {
      return operation();
    } catch (error) {
      throw this.#recordFailure(error);
    }
  }

  async #executeAsync(operation) {
    try {
      return await operation();
    } catch (error) {
      throw this.#recordFailure(error);
    }
  }

  #recordFailure(error) {
    const viewerError = toViewerError(error);
    if (this.#lastFailure !== viewerError) this.#refreshState(viewerError);
    return viewerError;
  }

  #cancelScheduledRender(message, publishFailure = true) {
    if (!this.#renderRequest) return undefined;
    const request = this.#renderRequest;
    const failure = new ViewerError("render_cancelled", message);
    this.#renderRequest = undefined;
    this.#cancelAnimationFrame(request.id);
    request.reject(failure);
    if (publishFailure) this.#refreshState(failure);
    return failure;
  }

  #requireCurrentPoint(identity, generation) {
    if (generation !== this.#state.generation) {
      throw new ViewerError("stale_generation", "Point belongs to a stale View generation");
    }
    if (identity.sourceIdentity !== this.#state.source?.identity) {
      throw new ViewerError("exact_query_source_mismatch", "Point Source is not active");
    }
  }
}

function publicState(diagnostics, destroyed, renderScheduled, loadActive, loadFacts, failure) {
  const stream = diagnostics.streaming;
  const streamActive = stream.phase !== "idle";
  const frame = diagnostics.frame;
  const camera = diagnostics.camera;
  const source = streamActive
    ? {
        identity: stream.source_identity,
        coverage: stream.coverage,
        expectedPoints: stream.expected_points,
        publishedPoints: stream.published_points,
        publishedBatches: stream.published_batches,
        retainedRecordBytes: stream.retained_record_bytes,
      }
    : {
        identity: diagnostics.scene.source_identity,
        coverage: "generated_fixture",
        expectedPoints: diagnostics.scene.point_count,
        publishedPoints: diagnostics.scene.point_count,
        publishedBatches: 1,
        retainedRecordBytes: 0,
      };
  return deepFreeze({
    schema: STATE_SCHEMA,
    packageVersion: diagnostics.package_version,
    lifecycle: destroyed ? "destroyed" : diagnostics.phase,
    generation: stream.generation ?? diagnostics.scene.generation,
    source,
    viewport: {
      cssWidth: diagnostics.viewport.css_width,
      cssHeight: diagnostics.viewport.css_height,
      devicePixelRatio: diagnostics.viewport.device_pixel_ratio,
      physicalWidth: diagnostics.viewport.physical_width,
      physicalHeight: diagnostics.viewport.physical_height,
      surfaceBytes: diagnostics.viewport.surface_bytes,
    },
    camera: {
      eye: camera.eye,
      target: camera.target,
      up: camera.up,
      projection: camera.projection,
      verticalFieldOfViewRadians: camera.vertical_field_of_view_radians,
      verticalWorldHeight: camera.vertical_world_height,
      nearDistance: camera.near_distance,
      farDistance: camera.far_distance,
    },
    displayMode: diagnostics.display_mode,
    render: {
      scheduled: renderScheduled,
      renderedFrames: diagnostics.rendered_frames,
      hiddenFrameSkips: diagnostics.hidden_frame_skips,
      drawnPoints: frame?.drawn_points ?? 0,
      drawCalls: frame?.draw_calls ?? 0,
      residentBytes: frame?.resident_bytes ?? 0,
      transientTextureBytes: frame?.transient_texture_bytes ?? 0,
      surfaceSuboptimal: frame?.surface_suboptimal ?? false,
    },
    pick: {
      status: diagnostics.pick.status,
      authority: diagnostics.pick.authority,
      generation: diagnostics.pick.generation,
      sourceIdentity: diagnostics.pick.source_identity,
      pointOrdinal: diagnostics.pick.point_ordinal,
      batchKey: diagnostics.pick.batch_key,
      batchVersion: diagnostics.pick.batch_version,
    },
    highlights: {
      generation: diagnostics.highlights.generation,
      sourceIdentity: diagnostics.highlights.source_identity,
      pointCount: diagnostics.highlights.point_count,
      authority: diagnostics.highlights.authority,
    },
    resources: {
      pointLimit: diagnostics.limits.points,
      batchLimit: diagnostics.limits.batches,
      highlightPointLimit: diagnostics.limits.highlight_points,
      residentByteLimit: diagnostics.limits.estimated_gpu_bytes,
      retainedRecordByteLimit: diagnostics.streaming_limits.retained_record_bytes,
      workerStagingByteLimit: diagnostics.streaming_limits.worker_staging_bytes,
    },
    capabilities: diagnostics.capabilities,
    load: {
      active: loadActive,
      facts: loadFacts ?? null,
    },
    failure: failure
      ? {
          code: failure.code,
          message: failure.message,
          safeAction: failure.safeAction,
          recoverable: failure.recoverable,
        }
      : null,
  });
}

function cameraInput(value) {
  if (!value || typeof value !== "object") throw invalidArgument("camera must be an object");
  const projection = value.projection;
  const policy = cameraProjectionPolicy(projection);
  const camera = {
    projection,
    eye: finiteTriple(value.eye, "camera eye"),
    target: finiteTriple(value.target, "camera target"),
    up: finiteTriple(value.up, "camera up"),
    nearDistance: positiveNumber(value.nearDistance, "nearDistance"),
    farDistance: positiveNumber(value.farDistance, "farDistance"),
  };
  if (camera.farDistance <= camera.nearDistance) {
    throw invalidArgument("farDistance must be greater than nearDistance");
  }
  camera[policy.extentProperty] = positiveNumber(
    value[policy.extentProperty],
    policy.extentProperty,
  );
  return camera;
}

function cameraProjectionPolicy(projection) {
  const policy = CAMERA_PROJECTION_POLICIES[projection];
  if (!policy) throw invalidArgument("camera projection must be perspective or orthographic");
  return policy;
}

function viewportInput(value) {
  if (!value || typeof value !== "object") throw invalidArgument("viewport must be an object");
  return {
    cssWidth: positiveNumber(value.cssWidth, "cssWidth"),
    cssHeight: positiveNumber(value.cssHeight, "cssHeight"),
    devicePixelRatio: positiveNumber(value.devicePixelRatio, "devicePixelRatio"),
  };
}

function pointIdentityInput(value) {
  if (!value || typeof value !== "object" || !/^[0-9a-f]{64}$/.test(value.sourceIdentity ?? "")) {
    throw invalidArgument("Point identity requires a 64-character lowercase Source identity");
  }
  if (typeof value.pointOrdinal === "number" && !Number.isSafeInteger(value.pointOrdinal)) {
    throw invalidArgument("Point ordinal numbers must be safe integers");
  }
  let pointOrdinal;
  try {
    pointOrdinal = BigInt(value.pointOrdinal);
  } catch {
    throw invalidArgument("Point ordinal must be a nonnegative integer");
  }
  if (pointOrdinal < 0n) throw invalidArgument("Point ordinal must be nonnegative");
  if (pointOrdinal > MAX_POINT_ORDINAL) {
    throw invalidArgument("Point ordinal must fit in an unsigned 64-bit integer");
  }
  return { sourceIdentity: value.sourceIdentity, pointOrdinal };
}

function parseDiagnostics(value) {
  if (typeof value !== "string") throw new ViewerError("internal", "raw viewer returned non-string diagnostics");
  try {
    return JSON.parse(value);
  } catch {
    throw new ViewerError("diagnostic_serialization", "raw viewer diagnostics are invalid JSON");
  }
}

function toViewerError(error, fallbackCode = "internal") {
  if (error instanceof ViewerError) return error;
  let record = error;
  if (typeof error === "string" || typeof error?.message === "string") {
    const text = typeof error === "string" ? error : error.message;
    try {
      record = JSON.parse(text);
    } catch {
      record = { code: error?.code ?? fallbackCode, message: text };
    }
  }
  const code = ERROR_CODE_SET.has(record?.code) ? record.code : fallbackCode;
  return new ViewerError(code, record?.message ?? String(error), {
    safeAction: record?.safe_action ?? record?.safeAction,
    recoverable: record?.recoverable,
  });
}

function matchesExactPoint(result, identity, generation) {
  if (result?.sourceIdentity !== identity.sourceIdentity
    || result?.generation !== generation
    || result?.authority !== "exact_source_record") {
    return false;
  }
  try {
    return BigInt(result.pointOrdinal) === identity.pointOrdinal;
  } catch {
    return false;
  }
}

function linkedAbortController(externalSignal) {
  const controller = new AbortController();
  const abort = () => controller.abort(externalSignal?.reason);
  externalSignal?.addEventListener("abort", abort, { once: true });
  if (externalSignal?.aborted) abort();
  controller.dispose = () => externalSignal?.removeEventListener("abort", abort);
  return controller;
}

function animationFrame(requestAnimationFrame, cancelAnimationFrame, signal) {
  return new Promise((resolve, reject) => {
    assertNotCancelled(signal, "cancelled");
    let settled = false;
    let frameId;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener("abort", onAbort);
      callback(value);
    };
    const onAbort = () => {
      cancelAnimationFrame(frameId);
      finish(reject, cancellationFailure(signal, "cancelled"));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
    frameId = requestAnimationFrame(() => finish(resolve));
  });
}

function assertNotCancelled(signal, code) {
  if (signal?.aborted) throw cancellationFailure(signal, code);
}

function cancellationFailure(signal, code) {
  return signal?.reason === VIEWER_DESTROYED_ABORT
    ? new ViewerError("viewer_destroyed", "viewer has been destroyed")
    : new ViewerError(code, "operation was cancelled");
}

function finiteTriple(value, label) {
  if (!Array.isArray(value) || value.length !== 3 || !value.every(Number.isFinite)) {
    throw invalidArgument(`${label} must contain three finite numbers`);
  }
  return [...value];
}

function positiveNumber(value, label) {
  if (!Number.isFinite(value) || value <= 0) throw invalidArgument(`${label} must be positive and finite`);
  return value;
}

function nonnegativeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw invalidArgument(`${label} must be a nonnegative safe integer`);
  return value;
}

function pickCoordinate(value, dimension, label) {
  const coordinate = nonnegativeInteger(value, label);
  if (coordinate >= dimension) {
    throw new ViewerError(
      "pick_outside_viewport",
      `${label} ${coordinate} is outside the ${dimension}-pixel viewport dimension`,
      { safeAction: "Choose a physical pixel inside the current viewport and retry." },
    );
  }
  return coordinate;
}

function positiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) throw invalidArgument(`${label} must be a positive safe integer`);
  return value;
}

function requiredString(value, label) {
  if (typeof value !== "string" || value.length === 0) throw invalidArgument(`${label} must be a nonempty string`);
  return value;
}

function cacheModeInput(value) {
  if (!["none", "memory", "persistent"].includes(value)) throw invalidArgument("cacheMode is unsupported");
  return value;
}

function credentialsInput(value) {
  if (!["omit", "same-origin", "include"].includes(value)) throw invalidArgument("credentials mode is unsupported");
  return value;
}

function invalidArgument(message) {
  return new ViewerError("invalid_argument", message, {
    safeAction: "Keep the current viewer state, correct the caller input, and retry.",
  });
}

function boundedMessage(value) {
  const message = String(value ?? "browser viewer failure");
  return message.length <= MAX_ERROR_MESSAGE_CHARACTERS
    ? message
    : `${message.slice(0, MAX_ERROR_MESSAGE_CHARACTERS - 1)}…`;
}

function deepFreeze(value) {
  if (!value || typeof value !== "object" || Object.isFrozen(value)) return value;
  for (const child of Object.values(value)) deepFreeze(child);
  return Object.freeze(value);
}
