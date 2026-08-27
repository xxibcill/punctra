const BUILD_CACHE_TOKEN = encodeURIComponent(
  new URL(import.meta.url).searchParams.get("v") ?? "unversioned",
);

const [
  { DISPLAY_MODES, ViewerError, createViewer },
  { createInputNormalizer },
  { createLasExactQueryBridge },
  {
    captureEnvironment,
    captureJsHeap,
    evaluateQualificationLane,
    evaluateQualification,
    measureForegroundFrames,
    QUALIFICATION_LIMITS,
    QUALIFICATION_WORKLOAD,
    recreationRequiredRecoveryEvidence,
  },
] = await Promise.all([
  import("@punctra/viewer"),
  import("@punctra/viewer/input"),
  import("@punctra/viewer/exact-query"),
  import(`./qualification.js?v=${BUILD_CACHE_TOKEN}`),
]);
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
const displaySelect = document.querySelector("#display-mode");
const projectionButton = document.querySelector("#projection-button");
const clearButton = document.querySelector("#clear-button");
const shutdownButton = document.querySelector("#shutdown-button");

const STREAM_MANIFEST_URL = "./fixtures/v1/deployment.json";
const QUALIFICATION_WORKER_URL = new URL("./qualification-worker.js", import.meta.url);
const EXACT_QUERY_AUTHORITY = "exact_source_record";
const ACCEPTANCE_SCHEMA = "punctra-browser-qualification-v1";
const HOST_CAMERA_PROJECTION_POLICIES = Object.freeze({
  perspective: Object.freeze({
    extentProperty: "verticalFieldOfViewRadians",
    visibleHeight: perspectiveVisibleHeight,
    zoom: (camera, factor) => ({
      ...camera,
      eye: add(camera.target, scaleVector(subtract(camera.eye, camera.target), factor)),
    }),
    alternate: (camera) => cameraWithProjection(
      camera,
      "orthographic",
      perspectiveVisibleHeight(camera),
    ),
  }),
  orthographic: Object.freeze({
    extentProperty: "verticalWorldHeight",
    visibleHeight: (camera) => camera.verticalWorldHeight,
    zoom: (camera, factor) => ({
      ...camera,
      verticalWorldHeight: Math.max(0.01, camera.verticalWorldHeight * factor),
    }),
    alternate: (camera) => cameraWithProjection(camera, "perspective", Math.PI / 3),
  }),
});
let viewer;
let viewerSubscription;
let inputNormalizer;
let suspended = false;
let smokeRunning = false;
let smokePassed = false;
let smokeRecord;
let inputAcceptanceFacts;
let latestLoad;
let exactPoint;
let resizeFrame;

function requestedViewport() {
  const bounds = canvasShell.getBoundingClientRect();
  return {
    cssWidth: Math.max(1, bounds.width),
    cssHeight: Math.max(1, bounds.height),
    devicePixelRatio: window.devicePixelRatio,
  };
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes)) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 ** 2).toFixed(2)} MiB`;
}

function replaceFacts(list, entries) {
  list.replaceChildren(...entries.map(([label, value]) => {
    const row = document.createElement("div");
    const term = document.createElement("dt");
    const detail = document.createElement("dd");
    term.textContent = label;
    detail.textContent = String(value);
    row.append(term, detail);
    return row;
  }));
}

function publishState(state) {
  diagnosticOutput.textContent = JSON.stringify(
    { state, acceptance: smokeRecord ?? null, exact_point: exactPoint ?? null },
    null,
    2,
  );
  const capabilities = state.capabilities;
  replaceFacts(capabilityFacts, [
    ["Secure context", capabilities.secure_context ? "available" : "unavailable"],
    ["WebGPU", capabilities.webgpu ? "available" : "unavailable"],
    ["Browser", capabilities.browser_user_agent ?? "unreported"],
    ["Adapter", capabilities.adapter_name ?? "unreported"],
    ["Backend", capabilities.backend ?? "unreported"],
    ["Surface", capabilities.surface_format ?? "unreported"],
    ["Projection", state.camera.projection],
    ["Display", state.displayMode],
    ["Physical viewport", `${state.viewport.physicalWidth} × ${state.viewport.physicalHeight}`],
  ]);
  replaceFacts(resourceFacts, [
    ["Resident Points", `${state.source.publishedPoints} / ${state.resources.pointLimit}`],
    ["Logical vertex bytes", `${formatBytes(state.render.residentBytes)} / ${formatBytes(state.resources.residentByteLimit)}`],
    ["Decoded record bytes", `${formatBytes(state.source.retainedRecordBytes)} / ${formatBytes(state.resources.retainedRecordByteLimit)}`],
    ["Stream Coverage", state.source.coverage],
    ["Range requested bytes", formatBytes(latestLoad?.metrics.requestedBytes)],
    ["Range received bytes", formatBytes(latestLoad?.metrics.receivedBytes)],
    ["Verified cache bytes", formatBytes(latestLoad?.metrics.cacheBytes)],
    ["Worker staging high-water", formatBytes(latestLoad?.metrics.decodedStagingBytesHighWater)],
    ["Canvas bytes", formatBytes(state.viewport.surfaceBytes)],
    ["Transient texture bytes", formatBytes(state.render.transientTextureBytes)],
    ["Rendered frames", state.render.renderedFrames],
  ]);
  replaceFacts(pickFacts, [
    ["Pick state", state.pick.status.replaceAll("_", " ")],
    ["Point ordinal", state.pick.pointOrdinal ?? "—"],
    ["Generation / batch / version", state.pick.generation === null
      ? "—"
      : `${state.pick.generation} / ${state.pick.batchKey} / ${state.pick.batchVersion}`],
    ["Highlights", `${state.highlights.pointCount} presentation-only`],
    ["Exact authority", exactPoint?.authority ?? "not confirmed"],
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
  displaySelect.disabled = !enabled;
  projectionButton.disabled = !enabled;
  clearButton.disabled = !enabled;
  shutdownButton.disabled = !enabled;
}

function failureRecord(error) {
  if (error instanceof ViewerError) return error;
  return new ViewerError("internal", error?.message ?? String(error));
}

function publishFailure(error, disableControls = false) {
  const failure = failureRecord(error);
  if (disableControls) setControls(false);
  setHarnessState(
    failure.code === "webgpu_unavailable" || failure.code === "insecure_context"
      ? "unsupported"
      : "failed",
    `FAIL — ${failure.message}`,
    failure.safeAction,
  );
  diagnosticOutput.textContent = JSON.stringify({
    schema: failure.schema,
    code: failure.code,
    message: failure.message,
    safe_action: failure.safeAction,
    recoverable: failure.recoverable,
  }, null, 2);
}

function assertFact(condition, message) {
  if (!condition) throw new Error(`Browser acceptance invariant failed: ${message}`);
}

async function initializeViewer(assetOverrides = {}) {
  const exactQueryBridge = createLasExactQueryBridge({
    manifestUrl: STREAM_MANIFEST_URL,
    credentials: "same-origin",
  });
  viewer = await createViewer({
    canvas,
    viewport: requestedViewport(),
    exactQueryBridge,
    assets: {
      cacheKey: BUILD_CACHE_TOKEN,
      ...assetOverrides,
    },
  });
  viewerSubscription = viewer.subscribe(publishState);
  inputNormalizer = createInputNormalizer(canvas, applyNormalizedInput, {
    preventDefault: true,
  });
  suspended = false;
  latestLoad = undefined;
  exactPoint = undefined;
  visibilityButton.textContent = "Suspend rendering";
  projectionButton.textContent = "Orthographic";
  displaySelect.value = viewer.state().displayMode;
  setControls(true);
  return viewer.state();
}

function discardViewer() {
  viewerSubscription?.();
  viewerSubscription = undefined;
  inputNormalizer?.dispose();
  inputNormalizer = undefined;
  viewer?.dispose();
  viewer = undefined;
}

async function loadQualificationHost() {
  try {
    const response = await fetch("./qualification-host.json", {
      cache: "no-store",
      credentials: "same-origin",
    });
    if (!response.ok) return null;
    return await response.json();
  } catch {
    return null;
  }
}

async function runSmokePath() {
  smokeRunning = true;
  smokePassed = false;
  smokeRecord = { schema: ACCEPTANCE_SCHEMA };
  const environment = captureEnvironment({ host: await loadQualificationHost() });
  const heap = { before: captureJsHeap(performance) };
  setHarnessState("checking", "Running browser/device qualification checks…");
  const initial = await initializeViewer();
  const runtimeLane = evaluateQualificationLane(environment, initial);
  assertFact(
    runtimeLane.passed,
    `exact qualification lane: ${runtimeLane.failures.join("; ")}`,
  );
  let state = viewer.render();
  assertFact(state.packageVersion === "0.19.0-alpha.1", "v0.19 package version");
  assertFact(state.capabilities.secure_context === true, "secure context");
  assertFact(state.capabilities.webgpu === true, "WebGPU capability");
  assertFact(state.source.publishedPoints === 1_089, "generated fixture Points");
  assertFact(state.resources.pointLimit === 8_192, "Point ceiling");
  assertFact(state.resources.highlightPointLimit === 32, "highlight ceiling");

  const resized = {
    cssWidth: Math.max(1, initial.viewport.cssWidth * 0.75),
    cssHeight: Math.max(1, initial.viewport.cssHeight * 0.75),
    devicePixelRatio: initial.viewport.devicePixelRatio,
  };
  const resizeBeforeFailure = viewer.state().viewport;
  const resizeFailure = expectSynchronousCode(() => viewer.resize({
    cssWidth: QUALIFICATION_LIMITS.physicalDimensionPixels,
    cssHeight: QUALIFICATION_LIMITS.physicalDimensionPixels,
    devicePixelRatio: 4,
  }), "resize_viewport");
  assertFact(
    sameViewport(viewer.state().viewport, resizeBeforeFailure),
    "over-limit resize preserves the prior viewport",
  );
  state = viewer.resize(resized);
  assertFact(state.viewport.physicalWidth === Math.round(resized.cssWidth * resized.devicePixelRatio), "bounded resize");
  const alternateDpr = resized.devicePixelRatio === 1 ? 1.25 : 1;
  state = viewer.resize({ ...resized, devicePixelRatio: alternateDpr });
  assertFact(state.viewport.devicePixelRatio === alternateDpr, "bounded DPR change");
  state = viewer.resize(resized);
  viewer.render();
  const hiddenSkipsBefore = viewer.state().render.hiddenFrameSkips;
  viewer.pause();
  state = viewer.render();
  assertFact(state.lifecycle === "hidden", "hidden lifecycle");
  assertFact(state.render.hiddenFrameSkips === hiddenSkipsBefore + 1, "hidden frame skip");
  viewer.resume();
  viewer.render();
  const lifecycleRecovery = {
    invalid_resize_code: resizeFailure.code,
    invalid_resize_safe_action: resizeFailure.safeAction,
    prior_viewport_preserved: true,
    alternate_device_pixel_ratio: alternateDpr,
    hidden_frame_skips: 1,
    resumed: viewer.state().lifecycle === "ready",
  };

  const generatedPick = await pickCentre();
  assertFact(generatedPick?.pointOrdinal === "544", "generated centre pick identity");
  const generatedEvidence = { state: viewer.state(), pick: generatedPick };
  const destroyedViewer = viewer;
  const stalePresentation = destroyedViewer.requestRender();
  discardViewer();
  await expectCode(stalePresentation, "render_cancelled");
  expectSynchronousCode(() => destroyedViewer.render(), "viewer_destroyed");
  assertFact(viewer === undefined, "explicit viewer disposal");
  const destructionEvidence = {
    stale_presentation_cancelled: true,
    work_after_destruction_rejected: true,
  };

  await initializeViewer({ workerUrl: QUALIFICATION_WORKER_URL });
  const workerRecovery = await exercisePrepublicationRecovery({
    manifestUrl: `${STREAM_MANIFEST_URL}?worker_fault=crash`,
    expectedCode: "worker_failed",
    label: "pre-publication worker crash",
    retryManifestUrl: STREAM_MANIFEST_URL,
  });
  discardViewer();

  await initializeViewer();
  const cancellation = await cancellationProbe();
  const networkRecovery = await exercisePrepublicationRecovery({
    manifestUrl: `${STREAM_MANIFEST_URL}?fault=disconnect`,
    expectedCode: "offline",
    label: "pre-publication offline failure",
  });
  setHarnessState("checking", "Loading the cold immutable deployment through the public viewer API…");
  const cold = await viewer.loadSource({
    manifestUrl: STREAM_MANIFEST_URL,
    cacheMode: "persistent",
    invalidate: true,
    credentials: "same-origin",
  });
  verifyStreamingResult(cold, "cold");
  heap.afterCold = captureJsHeap(performance);

  discardViewer();
  await initializeViewer();
  setHarnessState("checking", "Recreating the viewer and proving identity-safe warm delivery…");
  const warm = await viewer.loadSource({
    manifestUrl: STREAM_MANIFEST_URL,
    cacheMode: "persistent",
    credentials: "same-origin",
  });
  verifyStreamingResult(warm, "warm");
  assertFact(warm.metrics.requestCount === 0, "warm binary network request count");
  assertFact(sameOrdinals(cold.pointOrdinals, warm.pointOrdinals), "cold/warm Point identities");
  latestLoad = warm;
  heap.afterWarm = captureJsHeap(performance);

  const displayEvidence = [];
  for (const mode of DISPLAY_MODES) {
    viewer.setDisplayMode(mode);
    state = viewer.render();
    assertFact(state.displayMode === mode, `${mode} display mode`);
    displayEvidence.push({ mode, generation: state.generation, drawnPoints: state.render.drawnPoints });
  }

  const perspective = cameraInputFromState(viewer.state());
  const verticalWorldHeight = perspectiveVisibleHeight(perspective);
  viewer.setCamera({
    projection: "orthographic",
    eye: perspective.eye,
    target: perspective.target,
    up: perspective.up,
    verticalWorldHeight,
    nearDistance: perspective.nearDistance,
    farDistance: perspective.farDistance,
  });
  assertFact(viewer.render().camera.projection === "orthographic", "orthographic camera");
  viewer.setCamera(perspective);
  assertFact(viewer.render().camera.projection === "perspective", "perspective camera");
  const normalizedInput = await exerciseNormalizedInput();

  const provisional = await pickResidentPoint();
  assertFact(provisional !== undefined, "streamed provisional pick");
  assertFact(provisional?.sourceIdentity === warm.deployment.source_identity, "streamed provisional Source identity");
  viewer.setHighlights([provisional], provisional.generation);
  assertFact(viewer.render().highlights.pointCount === 1, "presentation-only highlight");
  exactPoint = await viewer.confirmPoint(provisional);
  assertFact(exactPoint.authority === EXACT_QUERY_AUTHORITY, "exact Source record authority");
  assertFact(exactPoint.pointOrdinal === provisional.pointOrdinal, "exact/provisional Point identity");

  const cancelledQuery = new AbortController();
  cancelledQuery.abort();
  await expectCode(
    viewer.confirmPoint(provisional, { signal: cancelledQuery.signal }),
    "exact_query_cancelled",
  );

  const nextGeneration = await viewer.loadSource({
    manifestUrl: STREAM_MANIFEST_URL,
    cacheMode: "persistent",
    credentials: "same-origin",
  });
  verifyStreamingResult(nextGeneration, "generation retry");
  assertFact(
    nextGeneration.state.pick.status === "not_requested",
    "generation replacement clears provisional pick state",
  );
  assertFact(
    nextGeneration.state.highlights.pointCount === 0,
    "generation replacement clears presentation highlights",
  );
  const replacementPick = await pickResidentPoint();
  assertFact(replacementPick !== undefined, "replacement-generation provisional pick");
  viewer.setHighlights([replacementPick], replacementPick.generation);
  assertFact(viewer.state().highlights.pointCount === 1, "replacement-generation presentation highlight");
  viewer.clearHighlights();
  assertFact(viewer.state().highlights.pointCount === 0, "explicit highlight clear");
  await expectCode(viewer.confirmPoint(provisional), "stale_generation");
  const generationRecovery = {
    provisional_pick_cleared: true,
    presentation_highlights_cleared: true,
    stale_exact_request_rejected: true,
  };

  const frames = await measureForegroundFrames({
    frameCount: 30,
    render: () => viewer.render(),
    requestAnimationFrame: window.requestAnimationFrame.bind(window),
    monotonicNow: performance.now.bind(performance),
  });
  heap.afterFrames = captureJsHeap(performance);
  const finalState = viewer.state();
  const recovery = {
    lifecycle: lifecycleRecovery,
    worker: workerRecovery,
    network: networkRecovery,
  };
  const qualification = evaluateQualification({
    cold,
    warm,
    frames,
    cancellation: {
      acknowledgementMilliseconds: cancellation.acknowledgement_milliseconds,
    },
    viewport: finalState.viewport,
    state: finalState,
    recovery,
  });
  assertFact(
    qualification.passed,
    `qualification ceilings: ${qualification.failures.join("; ")}`,
  );

  smokeRecord = {
    schema: ACCEPTANCE_SCHEMA,
    package_version: finalState.packageVersion,
    environment,
    runtime_lane: runtimeLane,
    heap: completeHeapRecord(heap),
    generated: generatedEvidence,
    destruction: destructionEvidence,
    recovery: {
      ...recovery,
      generation: generationRecovery,
      recreation_required: recreationRequiredRecoveryEvidence(),
      physical_device_loss: "not forced on the physical adapter",
      memory_pressure: "no portable signal; independent fixed ceilings enforced",
    },
    cancellation,
    workload: observedWorkload(warm),
    render: {
      coverage: finalState.source.coverage,
      drawn_points: finalState.render.drawnPoints,
    },
    cold: compactLoad(cold),
    warm: compactLoad(warm),
    foreground_frames: frames,
    qualification,
    display_modes: displayEvidence,
    projections: ["orthographic", "perspective"],
    input_normalizer: normalizedInput,
    provisional,
    exact: exactPoint,
    stale_generation_rejected: true,
    cancelled_query_rejected: true,
    final_state: finalState,
    nonclaims: [
      "no arbitrary Source or Query support",
      "no npm registry publication, production hosting, or offline-first support",
      "no browser or device support beyond the exact recorded qualification lane",
      "no physical cache, process RSS, driver, or GPU allocation claim",
      "no independent adoption, stable API, visual-quality, support, or release-candidate claim",
    ],
  };
  smokePassed = true;
  smokeRunning = false;
  publishState(viewer.state());
  setHarnessState(
    "passed",
    "PASS — the declared browser/device lane satisfied functional, latency, resource, and recovery qualification locally.",
  );
}

async function cancellationProbe() {
  const controller = new AbortController();
  const started = performance.now();
  const operation = viewer.loadSource({
    manifestUrl: `${STREAM_MANIFEST_URL}?delay_ms=200`,
    cacheMode: "none",
    credentials: "same-origin",
    signal: controller.signal,
  });
  controller.abort();
  await expectCode(operation, "cancelled");
  const acknowledgementMilliseconds = performance.now() - started;
  assertFact(acknowledgementMilliseconds <= 1_000, "load cancellation deadline");
  return { code: "cancelled", acknowledgement_milliseconds: acknowledgementMilliseconds, limit_milliseconds: 1_000 };
}

async function exercisePrepublicationRecovery({ manifestUrl, expectedCode, label, retryManifestUrl }) {
  const generationBeforeFailure = viewer.state().generation;
  const failure = await expectCode(viewer.loadSource({
    manifestUrl,
    cacheMode: "none",
    credentials: "same-origin",
  }), expectedCode);
  assertFact(failure.recoverable === true, `${label} is recoverable`);
  assertFact(viewer.state().lifecycle === "ready", `${label} retains viewer`);
  assertFact(viewer.state().generation === generationBeforeFailure, `${label} preserves generation`);
  const evidence = {
    code: failure.code,
    recoverable: failure.recoverable,
    viewer_retained: true,
    generation_preserved: true,
  };
  if (retryManifestUrl !== undefined) {
    const retry = await viewer.loadSource({
      manifestUrl: retryManifestUrl,
      cacheMode: "none",
      credentials: "same-origin",
    });
    verifyStreamingResult(retry, `${label} retry`);
    evidence.retry_succeeded = true;
  }
  return evidence;
}

function verifyStreamingResult(result, label) {
  assertFact(
    result.deployment.deployment_id === QUALIFICATION_WORKLOAD.deploymentId,
    `${label} deployment identity`,
  );
  assertFact(
    result.deployment.source_identity === QUALIFICATION_WORKLOAD.sourceIdentity,
    `${label} Source identity`,
  );
  assertFact(
    result.deployment.source_point_count === QUALIFICATION_WORKLOAD.sourcePoints,
    `${label} Source point count`,
  );
  assertFact(
    result.deployment.root_coverage === QUALIFICATION_WORKLOAD.coverage,
    `${label} deployment Coverage`,
  );
  assertFact(
    result.state.source.identity === QUALIFICATION_WORKLOAD.sourceIdentity,
    `${label} state Source identity`,
  );
  assertFact(
    result.state.source.expectedPoints === QUALIFICATION_WORKLOAD.sourcePoints,
    `${label} state Source point count`,
  );
  assertFact(
    result.state.source.coverage === QUALIFICATION_WORKLOAD.coverage,
    `${label} Sampled Coverage`,
  );
  assertFact(
    result.state.source.publishedPoints === QUALIFICATION_WORKLOAD.sampledPoints,
    `${label} published Points`,
  );
  assertFact(
    result.state.source.publishedBatches === QUALIFICATION_WORKLOAD.publishedBatches,
    `${label} published batches`,
  );
  assertFact(
    result.state.source.retainedRecordBytes === QUALIFICATION_WORKLOAD.transferRecordBytes,
    `${label} retained records`,
  );
  assertFact(
    result.state.render.drawnPoints === QUALIFICATION_WORKLOAD.sampledPoints,
    `${label} drawn Points`,
  );
  assertFact(
    result.state.render.residentBytes === QUALIFICATION_WORKLOAD.rendererResidentBytes,
    `${label} GPU vertex bytes`,
  );
  assertFact(
    result.pointOrdinals.length === QUALIFICATION_WORKLOAD.sampledPoints,
    `${label} Point identities`,
  );
  assertFact(
    result.metrics.concurrentResponseBytesHighWater <= QUALIFICATION_LIMITS.concurrentResponseBytes,
    `${label} response ceiling`,
  );
  assertFact(
    result.metrics.decodedStagingBytesHighWater <= QUALIFICATION_LIMITS.workerStagingBytes,
    `${label} staging ceiling`,
  );
  assertFact(
    result.metrics.transferredBytes === QUALIFICATION_WORKLOAD.transferRecordBytes,
    `${label} transfer-v2 bytes`,
  );
}

function observedWorkload(result) {
  return {
    deployment_id: result.deployment.deployment_id,
    source_identity: result.deployment.source_identity,
    source_points: result.deployment.source_point_count,
    coverage: result.state.source.coverage,
    displayed_points: result.state.source.publishedPoints,
    displayed_batches: result.state.source.publishedBatches,
  };
}

function compactLoad(result) {
  return {
    deployment: result.deployment,
    metrics: result.metrics,
    decode: result.decode,
    timings: result.timings,
    ordinal_count: result.pointOrdinals.length,
    main_thread_milliseconds_high_water: result.mainThreadMillisecondsHighWater,
    generation: result.state.generation,
  };
}

async function pickCentre() {
  const state = viewer.state();
  return viewer.pick({
    x: Math.floor(state.viewport.physicalWidth / 2),
    y: Math.floor(state.viewport.physicalHeight / 2),
  });
}

async function pickResidentPoint() {
  const state = viewer.state();
  const fractions = [0.5, 0.35, 0.65, 0.2, 0.8, 0.1, 0.9];
  for (const yFraction of fractions) {
    for (const xFraction of fractions) {
      const pick = await viewer.pick({
        x: Math.floor(state.viewport.physicalWidth * xFraction),
        y: Math.floor(state.viewport.physicalHeight * yFraction),
      });
      if (pick) return pick;
    }
  }
  return undefined;
}

function sameOrdinals(left, right) {
  return left.length === right.length && left.every((ordinal, index) => ordinal === right[index]);
}

async function expectCode(promise, code) {
  try {
    await promise;
  } catch (error) {
    assertFact(error instanceof ViewerError && error.code === code, `${code} failure classification`);
    return error;
  }
  throw new Error(`Browser acceptance invariant failed: expected ${code}`);
}

function expectSynchronousCode(operation, code) {
  try {
    operation();
  } catch (error) {
    assertFact(error instanceof ViewerError && error.code === code, `${code} failure classification`);
    return error;
  }
  throw new Error(`Browser acceptance invariant failed: expected ${code}`);
}

function sameViewport(left, right) {
  return left.cssWidth === right.cssWidth
    && left.cssHeight === right.cssHeight
    && left.devicePixelRatio === right.devicePixelRatio
    && left.physicalWidth === right.physicalWidth
    && left.physicalHeight === right.physicalHeight;
}

function completeHeapRecord(observations) {
  const available = Object.values(observations)
    .map((observation) => observation.usedBytes)
    .filter((usedBytes) => usedBytes !== null);
  return {
    ...observations,
    highWaterBytes: available.length === 0 ? null : Math.max(...available),
    processResidentBytes: null,
    physicalCacheAllocationBytes: null,
    physicalGpuAllocationBytes: null,
  };
}

function cameraInputFromState(state) {
  const camera = state.camera;
  const policy = cameraProjectionPolicy(camera.projection);
  return cameraWithProjection(camera, camera.projection, camera[policy.extentProperty]);
}

function cameraWithProjection(camera, projection, extent) {
  const policy = cameraProjectionPolicy(projection);
  return {
    projection,
    eye: [...camera.eye],
    target: [...camera.target],
    up: [...camera.up],
    [policy.extentProperty]: extent,
    nearDistance: camera.nearDistance,
    farDistance: camera.farDistance,
  };
}

function cameraProjectionPolicy(projection) {
  return HOST_CAMERA_PROJECTION_POLICIES[projection];
}

function perspectiveVisibleHeight(camera) {
  const radius = length(subtract(camera.eye, camera.target));
  return 2 * radius * Math.tan(camera.verticalFieldOfViewRadians / 2);
}

function applyNormalizedInput(input) {
  if (!viewer || suspended || (smokeRunning && inputAcceptanceFacts === undefined)) return;
  try {
    const camera = cameraInputFromState(viewer.state());
    const next = input.kind === "orbit"
      ? orbitCamera(camera, input.deltaX, input.deltaY)
      : input.kind === "pan"
        ? panCamera(camera, input.deltaX, input.deltaY, viewer.state().viewport.physicalHeight)
        : input.kind === "zoom"
          ? zoomCamera(camera, input.delta)
          : keyboardCamera(camera, input.code);
    if (!next) return;
    viewer.setCamera(next);
    recordInputAcceptance(input, next);
    void viewer.requestRender().catch((error) => publishFailure(error));
    projectionButton.textContent = next.projection === "perspective" ? "Orthographic" : "Perspective";
  } catch (error) {
    publishFailure(error);
  }
}

async function exerciseNormalizedInput() {
  const initialCamera = cameraInputFromState(viewer.state());
  inputAcceptanceFacts = [];
  try {
    const preventedEvents = [
      exercisePointerInput(),
      exerciseTouchInput(),
      exerciseWheelInput(),
      exerciseKeyboardInput(),
    ];
    await viewer.requestRender();

    const channels = new Set(inputAcceptanceFacts.map((fact) => fact.channel));
    for (const channel of ["pointer", "touch", "wheel", "keyboard"]) {
      assertFact(channels.has(channel), `normalized ${channel} input`);
    }
    for (const [channel, event] of preventedEvents) {
      assertFact(event.defaultPrevented, `normalized ${channel} default prevention`);
    }
    return inputAcceptanceFacts;
  } finally {
    inputAcceptanceFacts = undefined;
    viewer.setCamera(initialCamera);
    viewer.render();
  }
}

function exercisePointerInput() {
  dispatchPointer("pointerdown", {
    pointerId: 101,
    pointerType: "mouse",
    clientX: 100,
    clientY: 100,
    buttons: 1,
  });
  const move = dispatchPointer("pointermove", {
    pointerId: 101,
    pointerType: "mouse",
    clientX: 112,
    clientY: 107,
    buttons: 1,
  });
  dispatchPointer("pointerup", {
    pointerId: 101,
    pointerType: "mouse",
    clientX: 112,
    clientY: 107,
  });
  return ["pointer", move];
}

function exerciseTouchInput() {
  dispatchPointer("pointerdown", {
    pointerId: 201,
    pointerType: "touch",
    clientX: 100,
    clientY: 100,
    buttons: 1,
  });
  dispatchPointer("pointerdown", {
    pointerId: 202,
    pointerType: "touch",
    clientX: 120,
    clientY: 100,
    buttons: 1,
  });
  const move = dispatchPointer("pointermove", {
    pointerId: 202,
    pointerType: "touch",
    clientX: 140,
    clientY: 100,
    buttons: 1,
  });
  dispatchPointer("pointerup", { pointerId: 201, pointerType: "touch" });
  dispatchPointer("pointerup", { pointerId: 202, pointerType: "touch" });
  return ["touch", move];
}

function exerciseWheelInput() {
  const event = new WheelEvent("wheel", {
    bubbles: true,
    cancelable: true,
    deltaY: 100,
    deltaMode: WheelEvent.DOM_DELTA_PIXEL,
  });
  canvas.dispatchEvent(event);
  return ["wheel", event];
}

function exerciseKeyboardInput() {
  const event = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    code: "KeyP",
  });
  canvas.dispatchEvent(event);
  return ["keyboard", event];
}

function dispatchPointer(type, options) {
  const event = new PointerEvent(type, {
    bubbles: true,
    cancelable: true,
    ...options,
  });
  canvas.dispatchEvent(event);
  return event;
}

function recordInputAcceptance(input, camera) {
  if (inputAcceptanceFacts === undefined) return;
  const channel = input.kind === "keyboard"
    ? "keyboard"
    : input.source === "touch"
      ? "touch"
      : input.source === "wheel"
        ? "wheel"
        : "pointer";
  inputAcceptanceFacts.push(Object.freeze({
    channel,
    kind: input.kind,
    source: input.source ?? "keyboard",
    camera_projection: camera.projection,
  }));
}

function orbitCamera(camera, horizontalPixels, verticalPixels) {
  const offset = subtract(camera.eye, camera.target);
  const radius = length(offset);
  const azimuth = Math.atan2(offset[1], offset[0]) - horizontalPixels * 0.006;
  let elevation = Math.asin(offset[2] / radius) + verticalPixels * 0.006;
  elevation = Math.max(0.08, Math.min(1.48, elevation));
  const horizontalRadius = radius * Math.cos(elevation);
  const eye = [
    camera.target[0] + horizontalRadius * Math.cos(azimuth),
    camera.target[1] + horizontalRadius * Math.sin(azimuth),
    camera.target[2] + radius * Math.sin(elevation),
  ];
  return { ...camera, eye };
}

function panCamera(camera, horizontalPixels, verticalPixels, viewportHeight) {
  const forward = normalize(subtract(camera.target, camera.eye));
  const right = normalize(cross(forward, camera.up));
  const up = normalize(cross(right, forward));
  const verticalHeight = cameraProjectionPolicy(camera.projection).visibleHeight(camera);
  const scale = verticalHeight / Math.max(1, viewportHeight);
  const movement = add(scaleVector(right, -horizontalPixels * scale), scaleVector(up, verticalPixels * scale));
  return { ...camera, eye: add(camera.eye, movement), target: add(camera.target, movement) };
}

function zoomCamera(camera, lines) {
  const factor = Math.exp(lines * 0.12);
  return cameraProjectionPolicy(camera.projection).zoom(camera, factor);
}

function keyboardCamera(camera, code) {
  if (code !== "KeyP") return undefined;
  return cameraProjectionPolicy(camera.projection).alternate(camera);
}

function add(left, right) {
  return left.map((value, axis) => value + right[axis]);
}

function subtract(left, right) {
  return left.map((value, axis) => value - right[axis]);
}

function scaleVector(vector, scale) {
  return vector.map((value) => value * scale);
}

function length(vector) {
  return Math.hypot(...vector);
}

function normalize(vector) {
  const magnitude = length(vector);
  return vector.map((value) => value / magnitude);
}

function cross(left, right) {
  return [
    left[1] * right[2] - left[2] * right[1],
    left[2] * right[0] - left[0] * right[2],
    left[0] * right[1] - left[1] * right[0],
  ];
}

async function start() {
  if (!window.isSecureContext || !navigator.gpu) {
    publishFailure(new ViewerError(
      window.isSecureContext ? "webgpu_unavailable" : "insecure_context",
      window.isSecureContext ? "WebGPU is unavailable" : "a secure context is required",
    ), true);
    return;
  }
  try {
    await runSmokePath();
  } catch (error) {
    smokeRunning = false;
    smokePassed = false;
    publishFailure(error, !failureRecord(error).recoverable);
  }
}

async function restart() {
  if (smokeRunning) return;
  discardViewer();
  await start();
}

function toggleVisibility() {
  if (!viewer || smokeRunning) return;
  try {
    suspended = !suspended;
    if (suspended) viewer.pause();
    else viewer.resume();
    visibilityButton.textContent = suspended ? "Resume rendering" : "Suspend rendering";
    if (!suspended) viewer.render();
  } catch (error) {
    publishFailure(error);
  }
}

async function checkPick() {
  if (!viewer || smokeRunning || suspended) return;
  try {
    const pick = await pickResidentPoint();
    if (!pick) throw new ViewerError("pick_invariant", "no resident Point was hit by the bounded pick probe");
    viewer.setHighlights([pick], pick.generation);
    exactPoint = viewer.state().source.coverage === "sampled"
      ? await viewer.confirmPoint(pick)
      : undefined;
    viewer.render();
    setHarnessState("passed", exactPoint
      ? "READY — provisional pick highlighted and exactly confirmed."
      : "READY — generated provisional pick highlighted.");
  } catch (error) {
    publishFailure(error);
  }
}

function changeDisplay() {
  if (!viewer || smokeRunning) return;
  try {
    viewer.setDisplayMode(displaySelect.value);
    viewer.render();
  } catch (error) {
    publishFailure(error);
  }
}

function toggleProjection() {
  if (!viewer || smokeRunning) return;
  applyNormalizedInput({ kind: "keyboard", code: "KeyP" });
}

function clearHighlight() {
  if (!viewer || smokeRunning) return;
  try {
    exactPoint = undefined;
    viewer.clearHighlights();
    viewer.render();
  } catch (error) {
    publishFailure(error);
  }
}

function shutdown() {
  if (!viewer || smokeRunning) return;
  discardViewer();
  setControls(false);
  setHarnessState("passed", "SHUT DOWN — recreate the viewer before more work.");
}

function scheduleResize() {
  if (!viewer || smokeRunning || suspended || resizeFrame !== undefined) return;
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = undefined;
    try {
      viewer.resize(requestedViewport());
      viewer.render();
    } catch (error) {
      publishFailure(error);
    }
  });
}

function synchronizeDocumentVisibility() {
  if (!viewer || smokeRunning) return;
  try {
    const visible = document.visibilityState === "visible" && !suspended;
    if (visible) viewer.resume();
    else viewer.pause();
    if (visible) viewer.render();
  } catch (error) {
    publishFailure(error, true);
  }
}

restartButton.addEventListener("click", () => void restart());
visibilityButton.addEventListener("click", toggleVisibility);
pickButton.addEventListener("click", () => void checkPick());
displaySelect.addEventListener("change", changeDisplay);
projectionButton.addEventListener("click", toggleProjection);
clearButton.addEventListener("click", clearHighlight);
shutdownButton.addEventListener("click", shutdown);
document.addEventListener("visibilitychange", synchronizeDocumentVisibility);
new ResizeObserver(scheduleResize).observe(canvasShell);

window.__PUNCTRA_BROWSER_VIEWER_API__ = {
  state: () => viewer?.state() ?? null,
  smoke: () => smokeRecord ?? null,
  harness: () => document.body.dataset.browserSmoke,
};
window.__PUNCTRA_BROWSER_SDK__ = window.__PUNCTRA_BROWSER_VIEWER_API__;
window.__PUNCTRA_BROWSER_FOUNDATION__ = window.__PUNCTRA_BROWSER_VIEWER_API__;
window.__PUNCTRA_BROWSER_STREAMING__ = window.__PUNCTRA_BROWSER_VIEWER_API__;

void start();
