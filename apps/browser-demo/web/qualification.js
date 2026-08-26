import { RECREATION_REQUIRED_SAFE_ACTIONS } from "./viewer-api.js";

export const QUALIFICATION_LIMITS = deepFreeze({
  firstCoverageMilliseconds: 10_000,
  settledViewMilliseconds: 15_000,
  frameIntervalP95Milliseconds: 50,
  frameSubmissionP95Milliseconds: 16.7,
  cancellationAcknowledgementMilliseconds: 1_000,
  physicalDimensionPixels: 4_096,
  physicalAreaPixels: 8_388_608,
  residentPoints: 8_192,
  residentBytes: 192 * 1024,
  canvasBytes: 32 * 1024 * 1024,
  transientTextureBytes: 64 * 1024 * 1024,
  retainedRecordBytes: 256 * 1024,
  workerStagingBytes: 320 * 1024,
  concurrentResponseBytes: 256 * 1024,
  persistentCacheBytes: 4 * 1024 * 1024,
});

export const QUALIFICATION_WORKLOAD = deepFreeze({
  coverage: "sampled",
  sampledPoints: 4_096,
  publishedBatches: 4,
  transferRecordBytes: 131_072,
  rendererResidentBytes: 98_304,
  warmBinaryRequestCount: 0,
});

export function summarizeSamples(samples) {
  if (!Array.isArray(samples)) throw new TypeError("samples must be an array");
  if (samples.some((sample) => !Number.isFinite(sample) || sample < 0)) {
    throw new TypeError("samples must contain finite nonnegative milliseconds");
  }
  if (samples.length === 0) return emptySummary();
  const ordered = [...samples].sort((left, right) => left - right);
  return Object.freeze({
    count: ordered.length,
    p50: percentile(ordered, 50),
    p95: percentile(ordered, 95),
    max: ordered.at(-1),
  });
}

export function measureForegroundFrames(options) {
  const frameCount = positiveInteger(options?.frameCount, "frameCount");
  const render = requiredFunction(options?.render, "render");
  const requestFrame = requiredFunction(options?.requestAnimationFrame, "requestAnimationFrame");
  const now = requiredFunction(options?.monotonicNow, "monotonicNow");
  return new Promise((resolve, reject) => {
    const measurement = createFrameMeasurement(frameCount, render, requestFrame, now, resolve, reject);
    requestFrame(measurement);
  });
}

export function captureJsHeap(performanceObject = globalThis.performance) {
  const usedBytes = performanceObject?.memory?.usedJSHeapSize;
  if (!Number.isSafeInteger(usedBytes) || usedBytes < 0) {
    return deepFreeze({
      api: "performance.memory.usedJSHeapSize",
      status: "unavailable",
      usedBytes: null,
    });
  }
  return deepFreeze({
    api: "performance.memory.usedJSHeapSize",
    status: "non_standard_observation",
    usedBytes,
  });
}

export function captureEnvironment(options = {}) {
  const navigatorObject = options.navigator ?? globalThis.navigator ?? {};
  const screenObject = options.screen ?? globalThis.screen ?? {};
  const documentObject = options.document ?? globalThis.document ?? {};
  return deepFreeze({
    userAgent: boundedText(navigatorObject.userAgent),
    language: boundedText(navigatorObject.language),
    logicalProcessors: optionalPositiveInteger(navigatorObject.hardwareConcurrency),
    screen: screenFacts(screenObject),
    visibilityState: boundedText(documentObject.visibilityState),
    secureContext: options.secureContext ?? globalThis.isSecureContext === true,
  });
}

export function evaluateQualification(record) {
  const failures = [];
  evaluateLoad("cold", record.cold, failures);
  evaluateLoad("warm", record.warm, failures);
  checkAbove(
    failures,
    record.frames.callbackIntervalMilliseconds.p95,
    QUALIFICATION_LIMITS.frameIntervalP95Milliseconds,
    "foreground frame interval p95",
    "ms",
  );
  checkAbove(
    failures,
    record.frames.submissionMilliseconds.p95,
    QUALIFICATION_LIMITS.frameSubmissionP95Milliseconds,
    "main-thread frame submission p95",
    "ms",
  );
  evaluateCancellation(record.cancellation, failures);
  evaluateViewport(record.viewport, failures);
  evaluateState(record.state, failures);
  checkDifferent(
    failures,
    record.warm.metrics.requestCount,
    QUALIFICATION_WORKLOAD.warmBinaryRequestCount,
    "warm binary network request count",
  );
  return deepFreeze({ passed: failures.length === 0, failures, limits: QUALIFICATION_LIMITS });
}

export function recreationRequiredRecoveryEvidence() {
  return deepFreeze({
    partial_publication: {
      outcome: "viewer_destroyed",
      safe_action: RECREATION_REQUIRED_SAFE_ACTIONS.partialPublication,
      test_scope: "deterministic post-publication Worker failure",
    },
    device_loss: {
      outcome: "viewer_destroyed",
      safe_action: RECREATION_REQUIRED_SAFE_ACTIONS.deviceLoss,
      test_scope: "deterministic facade and raw-viewer failure",
    },
  });
}

function createFrameMeasurement(frameCount, render, requestFrame, now, resolve, reject) {
  const callbackIntervals = [];
  const submissionTimes = [];
  let previousTimestamp;
  return function measureFrame(timestamp) {
    try {
      if (previousTimestamp !== undefined) callbackIntervals.push(timestamp - previousTimestamp);
      previousTimestamp = timestamp;
      const started = now();
      render();
      submissionTimes.push(now() - started);
      if (submissionTimes.length < frameCount) requestFrame(measureFrame);
      else resolve(frameMeasurement(frameCount, callbackIntervals, submissionTimes));
    } catch (error) {
      reject(error);
    }
  };
}

function frameMeasurement(frameCount, callbackIntervals, submissionTimes) {
  return deepFreeze({
    frameCount,
    callbackIntervalMilliseconds: summarizeSamples(callbackIntervals),
    submissionMilliseconds: summarizeSamples(submissionTimes),
  });
}

function evaluateLoad(label, load, failures) {
  checkAbove(
    failures,
    load.timings.firstCoverageMilliseconds,
    QUALIFICATION_LIMITS.firstCoverageMilliseconds,
    `${label} first sampled Coverage`,
    "ms",
  );
  checkAbove(
    failures,
    load.timings.settledViewMilliseconds,
    QUALIFICATION_LIMITS.settledViewMilliseconds,
    `${label} settled View`,
    "ms",
  );
  checkAbove(
    failures,
    load.metrics.concurrentResponseBytesHighWater,
    QUALIFICATION_LIMITS.concurrentResponseBytes,
    `${label} concurrent response bytes`,
    "bytes",
  );
  checkAbove(
    failures,
    load.metrics.decodedStagingBytesHighWater,
    QUALIFICATION_LIMITS.workerStagingBytes,
    `${label} worker staging bytes`,
    "bytes",
  );
  checkAbove(
    failures,
    load.metrics.cacheBytes,
    QUALIFICATION_LIMITS.persistentCacheBytes,
    `${label} verified cache bytes`,
    "bytes",
  );
}

function evaluateCancellation(cancellation, failures) {
  checkAbove(
    failures,
    cancellation.acknowledgementMilliseconds,
    QUALIFICATION_LIMITS.cancellationAcknowledgementMilliseconds,
    "cancellation acknowledgement",
    "ms",
  );
}

function evaluateViewport(viewport, failures) {
  checkAbove(
    failures,
    Math.max(viewport.physicalWidth, viewport.physicalHeight),
    QUALIFICATION_LIMITS.physicalDimensionPixels,
    "physical canvas dimension",
    "px",
  );
  checkAbove(
    failures,
    viewport.physicalWidth * viewport.physicalHeight,
    QUALIFICATION_LIMITS.physicalAreaPixels,
    "physical canvas area",
    "px",
  );
  checkAbove(failures, viewport.surfaceBytes, QUALIFICATION_LIMITS.canvasBytes, "canvas bytes", "bytes");
}

function evaluateState(state, failures) {
  checkDifferent(
    failures,
    state.source.publishedPoints,
    QUALIFICATION_WORKLOAD.sampledPoints,
    "published Points",
  );
  checkDifferent(
    failures,
    state.source.publishedBatches,
    QUALIFICATION_WORKLOAD.publishedBatches,
    "published batches",
  );
  checkDifferent(
    failures,
    state.source.retainedRecordBytes,
    QUALIFICATION_WORKLOAD.transferRecordBytes,
    "retained record bytes",
  );
  checkDifferent(
    failures,
    state.render.residentBytes,
    QUALIFICATION_WORKLOAD.rendererResidentBytes,
    "renderer resident bytes",
  );
  checkAbove(failures, state.source.publishedPoints, QUALIFICATION_LIMITS.residentPoints, "resident Points", "Points");
  checkAbove(failures, state.source.retainedRecordBytes, QUALIFICATION_LIMITS.retainedRecordBytes, "retained record bytes", "bytes");
  checkAbove(failures, state.render.residentBytes, QUALIFICATION_LIMITS.residentBytes, "renderer resident bytes", "bytes");
  checkAbove(
    failures,
    state.render.transientTextureBytes,
    QUALIFICATION_LIMITS.transientTextureBytes,
    "transient texture bytes",
    "bytes",
  );
}

function checkAbove(failures, actual, limit, label, unit) {
  if (Number.isFinite(actual) && actual <= limit) return;
  failures.push(`${label} exceeded ${limit} ${unit}`);
}

function checkDifferent(failures, actual, expected, label) {
  if (actual === expected) return;
  failures.push(`${label} differed from ${expected}`);
}

function percentile(ordered, percentage) {
  const rank = Math.max(0, Math.ceil((percentage / 100) * ordered.length) - 1);
  return ordered[rank];
}

function emptySummary() {
  return Object.freeze({ count: 0, p50: null, p95: null, max: null });
}

function screenFacts(value) {
  return {
    width: optionalNonnegativeInteger(value.width),
    height: optionalNonnegativeInteger(value.height),
    colorDepth: optionalNonnegativeInteger(value.colorDepth),
    pixelDepth: optionalNonnegativeInteger(value.pixelDepth),
  };
}

function boundedText(value) {
  return typeof value === "string" ? value.slice(0, 512) : null;
}

function optionalPositiveInteger(value) {
  return Number.isSafeInteger(value) && value > 0 ? value : null;
}

function optionalNonnegativeInteger(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : null;
}

function positiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) throw new TypeError(`${label} must be a positive integer`);
  return value;
}

function requiredFunction(value, label) {
  if (typeof value !== "function") throw new TypeError(`${label} must be a function`);
  return value;
}

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    for (const nested of Object.values(value)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}
