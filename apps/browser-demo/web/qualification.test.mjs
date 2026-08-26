import assert from "node:assert/strict";
import test from "node:test";

import {
  QUALIFICATION_LIMITS,
  captureEnvironment,
  captureJsHeap,
  evaluateQualification,
  measureForegroundFrames,
  recreationRequiredRecoveryEvidence,
  summarizeSamples,
} from "./qualification.js";
import { RECREATION_REQUIRED_SAFE_ACTIONS } from "./viewer-api.js";

test("sample summaries use deterministic nearest-rank percentiles", () => {
  assert.deepEqual(summarizeSamples([7, 1, 5, 3, 9]), {
    count: 5,
    p50: 5,
    p95: 9,
    max: 9,
  });
  assert.deepEqual(summarizeSamples([]), {
    count: 0,
    p50: null,
    p95: null,
    max: null,
  });
  assert.throws(() => summarizeSamples([1, Number.NaN]), /finite nonnegative/);
});

test("foreground frame measurement separates callback cadence from submission work", async () => {
  const callbacks = [];
  const clock = [10, 12, 20, 23, 30, 34];
  const measurement = measureForegroundFrames({
    frameCount: 3,
    render: () => {},
    requestAnimationFrame: (callback) => {
      callbacks.push(callback);
      return callbacks.length;
    },
    monotonicNow: () => clock.shift(),
  });

  callbacks.shift()(100);
  callbacks.shift()(116);
  callbacks.shift()(149);

  assert.deepEqual(await measurement, {
    frameCount: 3,
    callbackIntervalMilliseconds: {
      count: 2,
      p50: 16,
      p95: 33,
      max: 33,
    },
    submissionMilliseconds: {
      count: 3,
      p50: 3,
      p95: 4,
      max: 4,
    },
  });
});

test("JavaScript heap observations are explicit when the non-standard API is absent", () => {
  assert.deepEqual(captureJsHeap({}), {
    api: "performance.memory.usedJSHeapSize",
    status: "unavailable",
    usedBytes: null,
  });
  assert.deepEqual(captureJsHeap({ memory: { usedJSHeapSize: 42_000 } }), {
    api: "performance.memory.usedJSHeapSize",
    status: "non_standard_observation",
    usedBytes: 42_000,
  });
});

test("environment capture bounds caller-visible browser facts", () => {
  const environment = captureEnvironment({
    navigator: {
      userAgent: "Fixture Browser/19",
      language: "en-US",
      hardwareConcurrency: 12,
    },
    screen: { width: 1_920, height: 1_080, colorDepth: 30, pixelDepth: 30 },
    document: { visibilityState: "visible" },
    secureContext: true,
  });

  assert.deepEqual(environment, {
    userAgent: "Fixture Browser/19",
    language: "en-US",
    logicalProcessors: 12,
    screen: { width: 1_920, height: 1_080, colorDepth: 30, pixelDepth: 30 },
    visibilityState: "visible",
    secureContext: true,
  });
  assert.equal(Object.isFrozen(environment), true);
});

test("qualification evaluation reports every violated fixed ceiling and exact fact", () => {
  const passing = qualificationFixture();
  assert.deepEqual(evaluateQualification(passing), {
    passed: true,
    failures: [],
    limits: QUALIFICATION_LIMITS,
  });

  const result = evaluateQualification({
    ...passing,
    cold: {
      ...passing.cold,
      timings: {
        ...passing.cold.timings,
        firstCoverageMilliseconds: QUALIFICATION_LIMITS.firstCoverageMilliseconds + 1,
      },
    },
    warm: {
      ...passing.warm,
      metrics: { ...passing.warm.metrics, requestCount: 1 },
    },
  });

  assert.equal(result.passed, false);
  assert.deepEqual(result.failures, [
    "cold first sampled Coverage exceeded 10000 ms",
    "warm binary network request count differed from 0",
  ]);
});

test("recreation-required evidence names the outcome and safe host action", () => {
  const evidence = recreationRequiredRecoveryEvidence();
  assert.deepEqual(evidence, {
    partial_publication: {
      outcome: "viewer_destroyed",
      safe_action: "Dispose the fused viewer and create a new one before any Source load.",
      test_scope: "deterministic post-publication Worker failure",
    },
    device_loss: {
      outcome: "viewer_destroyed",
      safe_action: "Dispose the fused viewer and explicitly recreate the viewer and device.",
      test_scope: "deterministic facade and raw-viewer failure",
    },
  });
  assert.equal(Object.isFrozen(evidence), true);
  assert.equal(Object.isFrozen(evidence.partial_publication), true);
  assert.equal(Object.isFrozen(evidence.device_loss), true);
  assert.equal(
    evidence.partial_publication.safe_action,
    RECREATION_REQUIRED_SAFE_ACTIONS.partialPublication,
  );
  assert.equal(evidence.device_loss.safe_action, RECREATION_REQUIRED_SAFE_ACTIONS.deviceLoss);
});

function qualificationFixture() {
  return {
    cold: loadFixture(),
    warm: loadFixture(),
    frames: {
      callbackIntervalMilliseconds: { p95: 16.7 },
      submissionMilliseconds: { p95: 0.5 },
    },
    cancellation: { acknowledgementMilliseconds: 10 },
    viewport: { physicalWidth: 1_600, physicalHeight: 1_000, surfaceBytes: 6_400_000 },
    state: {
      source: { publishedPoints: 4_096, publishedBatches: 4, retainedRecordBytes: 131_072 },
      render: { residentBytes: 98_304, transientTextureBytes: 12_800_000 },
    },
  };
}

function loadFixture() {
  return {
    timings: {
      firstCoverageMilliseconds: 100,
      settledViewMilliseconds: 200,
      mainThreadBatchMillisecondsHighWater: 0.5,
    },
    metrics: {
      requestCount: 0,
      concurrentResponseBytesHighWater: 172_032,
      decodedStagingBytesHighWater: 204_800,
      cacheBytes: 172_696,
    },
  };
}
