const BUILD_CACHE_TOKEN = encodeURIComponent(
  new URL(import.meta.url).searchParams.get("v") ?? "unversioned",
);

const {
  RECOVERABLE_VIEWER_FAILURE_CODES,
  UNSUPPORTED_INITIALIZATION_CODES,
  failureCause,
  failureState,
  isPreserveViewerFailure,
  preserveViewerFailure,
  preservesCurrentViewer,
} = await import(`./failure-policy.js?v=${BUILD_CACHE_TOKEN}`);
const { runWorkerOperation } = await import(
  `./worker-operation.js?v=${BUILD_CACHE_TOKEN}`
);
const { createDeferredStreamPublication } = await import(
  `./stream-publication.js?v=${BUILD_CACHE_TOKEN}`
);
const {
  appendTransferredOrdinals,
  samePointOrdinals,
} = await import(`./stream-ordinals.js?v=${BUILD_CACHE_TOKEN}`);
const { WORKER_SCHEMA, workerFailure } = await import(
  `./worker-protocol.js?v=${BUILD_CACHE_TOKEN}`
);

const canvas = document.querySelector("#punctra-canvas");
const canvasShell = document.querySelector("#canvas-shell");
const statusBlock = document.querySelector("#status-block");
const statusMessage = document.querySelector("#status-message");
const diagnosticOutput = document.querySelector("#diagnostic-output");
const recoveryBlock = document.querySelector("#recovery-block");
const recoveryMessage = document.querySelector("#recovery-message");
const capabilityFacts = document.querySelector("#capability-facts");
const resourceFacts = document.querySelector("#resource-facts");
const pickFacts = document.querySelector("#pick-facts");
const restartButton = document.querySelector("#restart-button");
const visibilityButton = document.querySelector("#visibility-button");
const pickButton = document.querySelector("#pick-button");
const shutdownButton = document.querySelector("#shutdown-button");

let viewer = null;
let createViewer = null;
let wasmReady = false;
let suspended = false;
let smokeRunning = false;
let smokePassed = false;
let resizeFrame = null;
let smokeRecord = null;
let moduleLoadAttempt = 0;
let streamingFacts = null;
let streamSequence = 0;
let preserveViewerOnRestart = false;

const STREAM_MANIFEST_URL = "./fixtures/v1/deployment.json";

function requestedViewport() {
  const bounds = canvasShell.getBoundingClientRect();
  return {
    cssWidth: Math.max(1, bounds.width),
    cssHeight: Math.max(1, bounds.height),
    dpr: window.devicePixelRatio,
  };
}

function parseDiagnostics(json) {
  return JSON.parse(json);
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes)) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 ** 2).toFixed(2)} MiB`;
}

function replaceFacts(list, entries) {
  list.replaceChildren(
    ...entries.map(([label, value]) => {
      const row = document.createElement("div");
      const term = document.createElement("dt");
      const detail = document.createElement("dd");
      term.textContent = label;
      detail.textContent = String(value);
      row.append(term, detail);
      return row;
    }),
  );
}

function publishDiagnostics(diagnostics) {
  diagnosticOutput.textContent = JSON.stringify(diagnostics, null, 2);
  replaceFacts(capabilityFacts, [
    ["Secure context", diagnostics.capabilities.secure_context ? "available" : "unavailable"],
    ["WebGPU", diagnostics.capabilities.webgpu ? "available" : "unavailable"],
    ["Browser", diagnostics.capabilities.browser_user_agent],
    ["Adapter", diagnostics.capabilities.adapter_name],
    ["Backend", diagnostics.capabilities.backend],
    ["Surface", diagnostics.capabilities.surface_format],
    ["Composite alpha", diagnostics.capabilities.composite_alpha_mode],
    [
      "Render attachment",
      diagnostics.capabilities.surface_format_support.render_attachment ? "available" : "unavailable",
    ],
    [
      "Blendable surface",
      diagnostics.capabilities.surface_format_support.blendable ? "available" : "unavailable",
    ],
    ["Physical viewport", `${diagnostics.viewport.physical_width} × ${diagnostics.viewport.physical_height}`],
  ]);
  const streamActive = diagnostics.streaming.phase !== "idle";
  const residentPoints = streamActive
    ? diagnostics.streaming.published_points
    : diagnostics.scene.point_count;
  const residentBytes = streamActive
    ? diagnostics.frame?.resident_bytes
    : diagnostics.scene.estimated_gpu_bytes;
  replaceFacts(resourceFacts, [
    ["Resident Points", `${residentPoints} / ${diagnostics.limits.points}`],
    ["Logical vertex bytes", `${formatBytes(residentBytes)} / ${formatBytes(diagnostics.limits.estimated_gpu_bytes)}`],
    ["Stream Coverage", streamActive ? diagnostics.streaming.coverage : "generated fixture"],
    ["Range requested bytes", formatBytes(streamingFacts?.requestedBytes)],
    ["Range received bytes", formatBytes(streamingFacts?.receivedBytes)],
    ["Verified cache bytes", formatBytes(streamingFacts?.cacheBytes)],
    ["Worker staging high-water", formatBytes(streamingFacts?.decodedStagingBytesHighWater)],
    ["Main-task batch high-water", formatBytes(diagnostics.streaming.main_thread_batch_bytes_high_water)],
    ["Surface bytes / pixel", diagnostics.limits.surface_bytes_per_pixel],
    ["Canvas bytes", formatBytes(diagnostics.viewport.surface_bytes)],
    ["Transient texture bytes", formatBytes(diagnostics.frame?.transient_texture_bytes)],
    ["Presentation latency hint", `${diagnostics.limits.presentation_latency_frames} frames`],
    ["Rendered frames", diagnostics.rendered_frames],
    ["Hidden frame skips", diagnostics.hidden_frame_skips],
  ]);
  replaceFacts(pickFacts, [
    ["Pick state", diagnostics.pick.status.replaceAll("_", " ")],
    ["Point ordinal", diagnostics.pick.point_ordinal ?? "—"],
    ["Generation / batch / version", diagnostics.pick.generation === null
      ? "—"
      : `${diagnostics.pick.generation} / ${diagnostics.pick.batch_key} / ${diagnostics.pick.batch_version}`],
  ]);
}

function setHarnessState(state, message, safeAction = "") {
  document.body.dataset.browserSmoke = state;
  statusBlock.dataset.state = state;
  statusMessage.textContent = message;
  recoveryBlock.hidden = safeAction.length === 0;
  recoveryMessage.textContent = safeAction;
}

function setControls(enabled) {
  visibilityButton.disabled = !enabled;
  pickButton.disabled = !enabled;
  shutdownButton.disabled = !enabled;
}

function discardViewer() {
  const currentViewer = viewer;
  viewer = null;
  try {
    currentViewer?.shutdown();
  } catch {
    // A failed or fused viewer is already unavailable to the host.
  }
}

function failureRecord(error) {
  if (error?.schema === "punctra-browser-failure-v1") return error;
  if (error?.schema === WORKER_SCHEMA && error?.type === "failure") return error;
  const message = typeof error === "string" ? error : String(error?.message ?? error);
  try {
    return JSON.parse(message);
  } catch {
    return {
      schema: "punctra-browser-failure-v1",
      code: "browser_module",
      message,
      safe_action: "Build the browser package again, serve it from localhost, and recreate the viewer.",
    };
  }
}

function publishFailure(error, { disableControls = false, state } = {}) {
  const record = failureRecord(error);
  const publishedState = state ?? failureState(record);
  diagnosticOutput.textContent = JSON.stringify(record, null, 2);
  if (disableControls) setControls(false);
  const label = publishedState === "unsupported" ? "UNSUPPORTED" : "FAILED";
  setHarnessState(publishedState, `${label} — ${record.message}`, record.safe_action);
}

function assertFact(condition, message) {
  if (!condition) throw new Error(`Browser acceptance invariant failed: ${message}`);
}

function verifyFailureStateClassification() {
  for (const code of UNSUPPORTED_INITIALIZATION_CODES) {
    assertFact(failureState({ code }) === "unsupported", `${code} unsupported classification`);
  }
  assertFact(failureState({ code: "browser_module" }) === "failed", "module failure classification");
  assertFact(failureState({ code: "scene_publication" }) === "failed", "logic failure classification");
  for (const code of RECOVERABLE_VIEWER_FAILURE_CODES) {
    assertFact(preservesCurrentViewer({ code }), `${code} preserves the current viewer`);
  }
  assertFact(
    !preservesCurrentViewer({ code: "surface_lost" }),
    "surface loss requires viewer recreation",
  );
}

function verifyCapabilityDiagnostics(diagnostics) {
  const capabilities = diagnostics.capabilities;
  assertFact(capabilities.secure_context === true, "secure-context capability");
  assertFact(capabilities.webgpu === true, "WebGPU capability");
  assertFact(capabilities.adapter_name.length > 0, "adapter name");
  assertFact(capabilities.backend === "BrowserWebGpu", "browser WebGPU backend");
  assertFact(capabilities.device_type.length > 0, "adapter device type");
  assertFact(capabilities.surface_format.length > 0, "surface format");
  assertFact(
    ["Opaque", "PreMultiplied"].includes(capabilities.composite_alpha_mode),
    "supported composite alpha mode",
  );
  assertFact(capabilities.present_mode === "fifo", "FIFO presentation");
  assertFact(
    capabilities.surface_format_support.render_attachment === true,
    "render-attachment surface",
  );
  assertFact(capabilities.surface_format_support.blendable === true, "blendable surface format");
  assertFact(capabilities.required_feature_count === 0, "WebGPU core features only");
  assertFact(
    capabilities.adapter_max_buffer_size >= 268_435_456,
    "default adapter buffer limit",
  );
  assertFact(
    capabilities.adapter_max_texture_dimension_2d >= 8_192,
    "default adapter texture-dimension limit",
  );
  assertFact(capabilities.adapter_max_bind_groups >= 4, "default adapter bind-group limit");
  assertFact(capabilities.adapter_max_vertex_buffers >= 8, "default adapter vertex-buffer limit");
  assertFact(
    capabilities.adapter_max_color_attachments >= 8,
    "default adapter color-attachment limit",
  );
}

async function initializeViewer() {
  const requested = requestedViewport();
  const next = await createViewer(
    canvas,
    requested.cssWidth,
    requested.cssHeight,
    requested.dpr,
  );
  viewer = next;
  suspended = false;
  streamingFacts = null;
  visibilityButton.textContent = "Suspend rendering";
  pickButton.textContent = "Check centre pick";
  setControls(true);
  return requested;
}

async function pollCentrePick() {
  let diagnostics = parseDiagnostics(viewer.diagnostics());
  const x = Math.floor(diagnostics.viewport.physical_width / 2);
  const y = Math.floor(diagnostics.viewport.physical_height / 2);
  publishDiagnostics(parseDiagnostics(viewer.beginPick(x, y)));

  for (let attempt = 0; attempt < 180; attempt += 1) {
    await new Promise((resolve) => requestAnimationFrame(resolve));
    diagnostics = parseDiagnostics(viewer.pollPick());
    publishDiagnostics(diagnostics);
    if (diagnostics.pick.status !== "pending") return diagnostics;
  }
  throw new Error("Browser acceptance invariant failed: provisional pick remained pending");
}

function verifyBoundedResize(initialViewport) {
  const requested = {
    cssWidth: Math.max(1, initialViewport.cssWidth * 0.75),
    cssHeight: Math.max(1, initialViewport.cssHeight * 0.75),
    dpr: initialViewport.dpr,
  };
  const diagnostics = parseDiagnostics(
    viewer.resize(requested.cssWidth, requested.cssHeight, requested.dpr),
  );
  const viewport = diagnostics.viewport;
  assertFact(viewport.css_width === requested.cssWidth, "resized CSS width");
  assertFact(viewport.css_height === requested.cssHeight, "resized CSS height");
  assertFact(viewport.device_pixel_ratio === requested.dpr, "resized device-pixel ratio");
  assertFact(
    viewport.physical_width === Math.round(requested.cssWidth * requested.dpr),
    "resized physical width",
  );
  assertFact(
    viewport.physical_height === Math.round(requested.cssHeight * requested.dpr),
    "resized physical height",
  );
  assertFact(
    viewport.surface_bytes
      === viewport.physical_width * viewport.physical_height * diagnostics.limits.surface_bytes_per_pixel,
    "resized surface accounting",
  );
  assertFact(diagnostics.frame === null, "resize discards the recorded frame");
  publishDiagnostics(parseDiagnostics(viewer.render()));
  return viewport;
}

async function runSmokePath() {
  smokeRunning = true;
  smokePassed = false;
  setHarnessState("checking", "Running bounded browser lifecycle checks…");
  const initialViewport = await initializeViewer();

  let diagnostics = parseDiagnostics(viewer.render());
  publishDiagnostics(diagnostics);
  assertFact(diagnostics.schema === "punctra-browser-streaming-v1", "diagnostic schema");
  assertFact(diagnostics.package_version === "0.16.0-alpha.1", "browser package version");
  verifyFailureStateClassification();
  verifyCapabilityDiagnostics(diagnostics);
  assertFact(diagnostics.scene.point_count === 1089, "fixed scene Point count");
  assertFact(diagnostics.scene.initial_requests === 1, "initial planner request");
  assertFact(diagnostics.scene.retained_nodes === 1, "settled planner retention");
  assertFact(diagnostics.scene.generation === 1, "View generation");
  assertFact(diagnostics.scene.batch_version === 1, "batch version");
  assertFact(diagnostics.scene.estimated_gpu_bytes === 26_136, "fixed scene logical bytes");
  assertFact(diagnostics.frame.drawn_points === 1089, "drawn Point count");
  assertFact(diagnostics.frame.draw_calls === 1, "single generated draw call");
  assertFact(diagnostics.frame.resident_bytes === 26_136, "fixed resident bytes");
  assertFact(diagnostics.limits.estimated_gpu_bytes === 196_608, "logical byte ceiling");
  assertFact(diagnostics.limits.points === 8_192, "Point ceiling");
  assertFact(diagnostics.limits.batches === 8, "batch ceiling");
  assertFact(diagnostics.limits.highlight_points === 32, "highlight ceiling");
  assertFact(diagnostics.limits.canvas_dimension === 4_096, "canvas dimension ceiling");
  assertFact(diagnostics.limits.canvas_pixels === 8_388_608, "canvas area ceiling");
  assertFact(diagnostics.limits.device_pixel_ratio === 4, "device-pixel-ratio ceiling");
  assertFact(
    diagnostics.limits.renderer_transient_bytes === 67_108_864,
    "transient texture ceiling",
  );
  assertFact(
    diagnostics.viewport.surface_bytes
      <= diagnostics.limits.canvas_pixels * diagnostics.limits.surface_bytes_per_pixel,
    "canvas byte ceiling",
  );
  assertFact(diagnostics.limits.surface_bytes_per_pixel === 4, "surface byte factor");
  assertFact(diagnostics.limits.presentation_latency_frames === 2, "presentation latency hint");
  assertFact(
    diagnostics.streaming_limits.cancellation_milliseconds === 1_000,
    "cancellation acknowledgement ceiling",
  );
  assertFact(
    diagnostics.frame.transient_texture_bytes === diagnostics.viewport.surface_bytes,
    "exact pre-pick transient texture accounting",
  );

  const resizedViewport = verifyBoundedResize(initialViewport);

  viewer.setVisible(false);
  diagnostics = parseDiagnostics(viewer.render());
  assertFact(diagnostics.phase === "hidden", "hidden phase");
  assertFact(diagnostics.hidden_frame_skips === 1, "hidden frame suppression");
  viewer.setVisible(true);
  publishDiagnostics(parseDiagnostics(viewer.render()));

  diagnostics = await pollCentrePick();
  assertFact(diagnostics.pick.status === "hit", "centre provisional pick");
  assertFact(diagnostics.pick.point_ordinal === 544, "centre Point identity");
  assertFact(diagnostics.pick.generation === 1, "pick generation");
  assertFact(diagnostics.pick.batch_key === 1, "pick batch key");
  assertFact(diagnostics.pick.batch_version === 1, "pick batch version");
  assertFact(
    diagnostics.frame.transient_texture_bytes
      === diagnostics.viewport.physical_width * diagnostics.viewport.physical_height * 8,
    "exact post-pick transient texture accounting",
  );

  smokeRecord = {
    state: "pick_verified",
    browser: diagnostics.capabilities.browser_user_agent,
    platform: diagnostics.capabilities.browser_platform,
    scene: diagnostics.scene,
    frame: diagnostics.frame,
    pick: diagnostics.pick,
    viewport: diagnostics.viewport,
    resized_viewport: resizedViewport,
  };

  parseDiagnostics(viewer.shutdown());
  let shutdownRejected = false;
  try {
    viewer.render();
  } catch (error) {
    shutdownRejected = failureRecord(error).code === "host_model";
  }
  assertFact(shutdownRejected, "fused shutdown");
  smokeRecord.shutdown_rejected = shutdownRejected;

  await initializeViewer();
  diagnostics = parseDiagnostics(viewer.render());
  publishDiagnostics(diagnostics);
  assertFact(diagnostics.phase === "ready", "explicit recreation");
  const streaming = await runStreamingSmoke();
  smokeRecord.streaming = streaming;
  pickButton.disabled = true;
  pickButton.textContent = "Remote pick deferred";
  setHarnessState(
    "passed",
    "PASS — WebGPU lifecycle, bounded remote ranges, worker decode, and warm-cache isolation verified locally.",
  );
  smokePassed = true;
  smokeRunning = false;
  preserveViewerOnRestart = false;
}

async function runStreamingSmoke() {
  setHarnessState("checking", "Proving that an in-flight Fetch cancels within the fixed deadline…");
  const cancellation = await runCancellationProbe();
  setHarnessState("checking", "Streaming the cold immutable LAS deployment through one bounded worker…");
  const cold = await runWorkerStream({ cacheMode: "persistent", invalidate: true });
  verifyStreamingResult(cold, "cold");

  discardViewer();
  await initializeViewer();
  setHarnessState("checking", "Recreating the worker and proving identity-safe warm-cache delivery…");
  const warm = await runWorkerStream({ cacheMode: "persistent", invalidate: false });
  verifyStreamingResult(warm, "warm");
  assertFact(
    cold.deployment.source_identity === warm.deployment.source_identity,
    "cold and warm Source identity",
  );
  const ordinalIdentity = {
    matches: samePointOrdinals(cold.point_ordinals, warm.point_ordinals),
    point_count: warm.point_ordinals.length,
  };
  assertFact(ordinalIdentity.matches, "cold and warm Point ordinals");
  assertFact(warm.metrics.requestCount === 0, "warm binary network request count");
  assertFact(warm.metrics.cacheHits === 3, "warm verified cache hits");
  publishAcceptanceEvidence(warm.renderer, cancellation, cold, warm, ordinalIdentity);
  return { cancellation, cold, warm, ordinal_identity: ordinalIdentity };
}

function publishAcceptanceEvidence(renderer, cancellation, cold, warm, ordinalIdentity) {
  diagnosticOutput.textContent = JSON.stringify(
    {
      schema: "punctra-browser-streaming-acceptance-v1",
      renderer,
      streaming: {
        cancellation,
        cold: compactStreamResult(cold),
        warm: compactStreamResult(warm),
        ordinal_identity: ordinalIdentity,
      },
    },
    null,
    2,
  );
}

function runCancellationProbe() {
  const operationId = `browser-v016-cancel-${Date.now()}-${streamSequence}`;
  streamSequence += 1;
  let cancelStarted;
  return runWorkerOperation({
    workerUrl: `./stream-worker.js?v=${BUILD_CACHE_TOKEN}-${streamSequence}`,
    workerName: operationId,
    timeoutMilliseconds: 5_000,
    timeoutFailure: new Error("stream worker did not acknowledge cancellation"),
    errorFailure: (event) => new Error(event.message),
    messageErrorFailure: new Error("the browser could not deserialize the cancellation response"),
    initialMessage: {
      schema: WORKER_SCHEMA,
      type: "start",
      operation_id: operationId,
      manifest_url: `${STREAM_MANIFEST_URL}?delay_ms=200`,
      cache_mode: "none",
      invalidate: false,
      credentials: "same-origin",
    },
    onMessage(message, controls) {
      if (message?.schema !== WORKER_SCHEMA || message.operation_id !== operationId) return;
      if (message.type === "state" && message.phase === "starting" && cancelStarted === undefined) {
        cancelStarted = performance.now();
        controls.postMessage({
          schema: WORKER_SCHEMA,
          type: "cancel",
          operation_id: operationId,
        });
      } else if (message.type === "failure") {
        assertFact(cancelStarted !== undefined, "cancellation followed worker start");
        assertFact(message.code === "cancelled", "deterministic cancellation code");
        const acknowledgementMilliseconds = performance.now() - cancelStarted;
        assertFact(
          acknowledgementMilliseconds <= 1_000,
          "cancellation acknowledgement deadline",
        );
        controls.resolve({
          code: message.code,
          acknowledgement_milliseconds: acknowledgementMilliseconds,
          limit_milliseconds: 1_000,
        });
      } else if (message.type === "complete") {
        controls.reject(new Error("cancelled stream operation completed"));
      }
    },
  });
}

function compactStreamResult(result) {
  return {
    deployment: result.deployment,
    metrics: result.metrics,
    decode: result.decode,
    ordinal_count: result.point_ordinals.length,
    main_thread_milliseconds_high_water: result.main_thread_milliseconds_high_water,
  };
}

function runWorkerStream({ cacheMode, invalidate }) {
  const operationId = `browser-v016-${Date.now()}-${streamSequence}`;
  streamSequence += 1;
  let deployment;
  const publication = createDeferredStreamPublication({
    viewer,
    assertFact,
    publishDiagnostics,
    parseDiagnostics,
  });
  let mainThreadMillisecondsHighWater = 0;
  const pointOrdinals = [];
  return runWorkerOperation({
    workerUrl: `./stream-worker.js?v=${BUILD_CACHE_TOKEN}-${streamSequence}`,
    workerName: operationId,
    timeoutMilliseconds: 30_000,
    timeoutFailure: workerFailure("stream worker did not complete within 30 seconds"),
    errorFailure: (event) => workerFailure(event.message),
    messageErrorFailure: workerFailure("the browser could not deserialize a worker message"),
    initialMessage: {
      schema: WORKER_SCHEMA,
      type: "start",
      operation_id: operationId,
      manifest_url: STREAM_MANIFEST_URL,
      cache_mode: cacheMode,
      invalidate,
      credentials: "same-origin",
    },
    onMessage(message, controls) {
      if (message?.schema !== WORKER_SCHEMA || message.operation_id !== operationId) return;
      if (message.type === "failure") {
        controls.reject(message);
      } else if (message.type === "state") {
        if (message.phase === "deployment") {
          deployment = message.deployment;
          publication.acceptDeployment(deployment);
        }
        streamingFacts = message.metrics ?? streamingFacts;
      } else if (message.type === "batch") {
        const started = performance.now();
        publication.publishBatch(message);
        appendTransferredOrdinals(pointOrdinals, message.payload);
        mainThreadMillisecondsHighWater = Math.max(
          mainThreadMillisecondsHighWater,
          performance.now() - started,
        );
      } else if (message.type === "complete") {
        const diagnostics = publication.complete();
        streamingFacts = message.metrics;
        publishDiagnostics(diagnostics);
        controls.resolve({
          deployment: message.deployment,
          metrics: message.metrics,
          decode: message.decode,
          point_ordinals: pointOrdinals,
          main_thread_milliseconds_high_water: mainThreadMillisecondsHighWater,
          renderer: diagnostics,
        });
      }
    },
  }).catch((error) => {
    if (!publication.hasBegun()) throw preserveViewerFailure(error);
    throw error;
  });
}

function verifyStreamingResult(result, disposition) {
  const metrics = result.metrics;
  const diagnostics = result.renderer;
  assertFact(diagnostics.streaming.phase === "complete", `${disposition} stream completion`);
  assertFact(
    diagnostics.streaming.source_identity === result.deployment.source_identity,
    `${disposition} renderer Source identity`,
  );
  assertFact(diagnostics.streaming.coverage === "sampled", `${disposition} Sampled Coverage`);
  assertFact(diagnostics.streaming.expected_points === 4_096, `${disposition} expected Points`);
  assertFact(diagnostics.streaming.published_points === 4_096, `${disposition} published Points`);
  assertFact(result.point_ordinals.length === 4_096, `${disposition} captured Point ordinals`);
  assertFact(diagnostics.streaming.published_batches === 4, `${disposition} published batches`);
  assertFact(diagnostics.frame.drawn_points === 4_096, `${disposition} drawn Points`);
  assertFact(diagnostics.frame.draw_calls === 4, `${disposition} draw calls`);
  assertFact(diagnostics.frame.resident_bytes === 98_304, `${disposition} resident bytes`);
  assertFact(metrics.concurrentResponseBytesHighWater <= 262_144, `${disposition} response-byte ceiling`);
  assertFact(metrics.queuedRangesHighWater <= 2, `${disposition} queue-count ceiling`);
  assertFact(metrics.queuedRangeBytesHighWater <= 524_288, `${disposition} queue-byte ceiling`);
  assertFact(metrics.decodedStagingBytesHighWater <= 327_680, `${disposition} decode staging ceiling`);
  assertFact(metrics.transferredBatches <= 8, `${disposition} transfer-batch ceiling`);
  assertFact(metrics.logicalCacheEntries <= 64, `${disposition} cache-entry ceiling`);
  assertFact(result.decode.intensityMinimum === 22, `${disposition} decoded intensity minimum`);
  assertFact(result.decode.intensityMaximum === 65_519, `${disposition} decoded intensity maximum`);
  assertFact(
    result.decode.classificationMinimum === 2 && result.decode.classificationMaximum === 2,
    `${disposition} decoded Ground classification`,
  );
  assertFact(
    metrics.sourceNetworkBytes < result.deployment.source_byte_length,
    `${disposition} first frame precedes complete Source transfer`,
  );
}

async function start() {
  if (!window.isSecureContext || !navigator.gpu) {
    const missing = !window.isSecureContext ? "secure context" : "WebGPU";
    publishFailure(
      {
        schema: "punctra-browser-failure-v1",
        code: "browser_capability",
        message: `${missing} is unavailable.`,
        safe_action: "Serve this page from localhost or HTTPS in a WebGPU-capable browser, then recreate the viewer.",
      },
      { disableControls: true, state: "unsupported" },
    );
    return;
  }

  try {
    if (!wasmReady) {
      const attempt = moduleLoadAttempt;
      moduleLoadAttempt += 1;
      const browserBindings = await import(
        `./pkg/browser_demo.js?v=${BUILD_CACHE_TOKEN}-${attempt}`
      );
      await browserBindings.default({
        module_or_path: new URL(
          `./pkg/browser_demo_bg.wasm?v=${BUILD_CACHE_TOKEN}-${attempt}`,
          import.meta.url,
        ),
      });
      createViewer = browserBindings.createViewer;
      wasmReady = true;
    }
    await runSmokePath();
  } catch (error) {
    handleSmokeFailure(error);
  }
}

async function restart() {
  if (smokeRunning) return;
  if (!smokePassed) {
    if (preserveViewerOnRestart && viewer) {
      await retryStreamingSmoke();
      return;
    }
    discardViewer();
    await start();
    return;
  }
  try {
    discardViewer();
    await initializeViewer();
    const diagnostics = parseDiagnostics(viewer.render());
    publishDiagnostics(diagnostics);
    setHarnessState("passed", "READY — viewer explicitly recreated.");
    preserveViewerOnRestart = false;
  } catch (error) {
    publishFailure(error, { disableControls: true });
  }
}

async function retryStreamingSmoke() {
  smokeRunning = true;
  preserveViewerOnRestart = false;
  try {
    const streaming = await runStreamingSmoke();
    smokeRecord.streaming = streaming;
    pickButton.disabled = true;
    pickButton.textContent = "Remote pick deferred";
    smokePassed = true;
    smokeRunning = false;
    setHarnessState(
      "passed",
      "PASS — the replacement worker completed against the preserved viewer and fresh View generation.",
    );
  } catch (error) {
    handleSmokeFailure(error);
  }
}

function handleSmokeFailure(error) {
  smokeRunning = false;
  smokePassed = false;
  const record = failureRecord(failureCause(error));
  const preserveViewer = preservesCurrentViewer(record, error);
  preserveViewerOnRestart = preserveViewer
    && (record.code === "worker_failed" || isPreserveViewerFailure(error));
  if (!preserveViewer) discardViewer();
  publishFailure(record, { disableControls: !preserveViewer });
}

async function toggleVisibility() {
  if (!viewer || smokeRunning) return;
  suspended = !suspended;
  try {
    publishDiagnostics(parseDiagnostics(viewer.setVisible(!suspended)));
    visibilityButton.textContent = suspended ? "Resume rendering" : "Suspend rendering";
    if (!suspended) publishDiagnostics(parseDiagnostics(viewer.render()));
  } catch (error) {
    publishFailure(error);
  }
}

async function checkPick() {
  if (!viewer || smokeRunning || suspended) return;
  try {
    const diagnostics = await pollCentrePick();
    assertFact(diagnostics.pick.status === "hit", "manual centre provisional pick");
    setHarnessState("passed", "READY — centre provisional pick retained the recorded identity.");
  } catch (error) {
    publishFailure(error);
  }
}

function shutdown() {
  if (!viewer || smokeRunning) return;
  publishDiagnostics(parseDiagnostics(viewer.shutdown()));
  viewer = null;
  suspended = false;
  setControls(false);
  setHarnessState("passed", "SHUT DOWN — recreate the viewer before more work.");
}

function scheduleResize() {
  if (!viewer || smokeRunning || suspended || resizeFrame !== null) return;
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = null;
    try {
      const requested = requestedViewport();
      viewer.resize(requested.cssWidth, requested.cssHeight, requested.dpr);
      publishDiagnostics(parseDiagnostics(viewer.render()));
    } catch (error) {
      publishFailure(error);
    }
  });
}

function synchronizeDocumentVisibility() {
  if (!viewer || smokeRunning) return;
  try {
    const visible = document.visibilityState === "visible" && !suspended;
    viewer.setVisible(visible);
    if (visible) publishDiagnostics(parseDiagnostics(viewer.render()));
  } catch (error) {
    publishFailure(error, { disableControls: true });
  }
}

restartButton.addEventListener("click", restart);
visibilityButton.addEventListener("click", toggleVisibility);
pickButton.addEventListener("click", checkPick);
shutdownButton.addEventListener("click", shutdown);
document.addEventListener("visibilitychange", synchronizeDocumentVisibility);
new ResizeObserver(scheduleResize).observe(canvasShell);

window.__PUNCTRA_BROWSER_FOUNDATION__ = {
  diagnostics: () => viewer ? parseDiagnostics(viewer.diagnostics()) : null,
  smoke: () => smokeRecord,
  state: () => document.body.dataset.browserSmoke,
};

window.__PUNCTRA_BROWSER_STREAMING__ = {
  diagnostics: () => viewer ? parseDiagnostics(viewer.diagnostics()) : null,
  smoke: () => smokeRecord?.streaming ?? null,
  state: () => document.body.dataset.browserSmoke,
};

start();
