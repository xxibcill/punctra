import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

import {
  QUALIFICATION_LIMITS,
  evaluateQualification,
  recreationRequiredRecoveryEvidence,
} from "../apps/browser-demo/web/qualification.js";

const matrixUrl = new URL("../docs/releases/v0.19-browser-matrix.json", import.meta.url);

export function verifyBrowserQualificationMatrix(matrix) {
  assert.equal(matrix.schema, "punctra-browser-qualification-matrix-v1");
  assert.equal(matrix.release, "0.19.0-alpha.1");
  assert.match(matrix.observed_on, /^\d{4}-\d{2}-\d{2}$/);
  assert.equal(matrix.qualified_entries.length, 1);
  assert.ok(matrix.unqualified_entries.length >= 1);

  const entry = matrix.qualified_entries[0];
  const observations = entry.observations;
  assert.equal(entry.status, "repository_qualified_exact_lane");
  assert.equal(entry.browser.surface, "Codex in-app browser");
  assert.match(entry.browser.user_agent, /Chrome\/151\.0\.0\.0/);
  assert.equal(entry.operating_system.architecture, "arm64");
  assert.equal(entry.webgpu.backend, "BrowserWebGpu");
  assert.equal(entry.webgpu.render_attachment, true);
  assert.equal(entry.workload.coverage, "sampled");
  assert.equal(entry.workload.displayed_points, 4_096);
  assert.equal(entry.workload.displayed_batches, 4);

  assertNonnegativeNumbers(observations, "observations");
  assert.equal(observations.acceptance_schema, "punctra-browser-qualification-v1");
  assert.deepEqual(observations.limits, QUALIFICATION_LIMITS);
  assert.deepEqual(
    observations.recovery.recreation_required,
    recreationRequiredRecoveryEvidence(),
  );
  for (const outcome of [
    "invalid_resize_preserved_viewport",
    "dpr_change_and_restore",
    "hidden_frame_skipped_and_resumed",
    "prepublication_worker_crash_preserved_viewer",
    "prepublication_offline_failure_preserved_viewer",
    "warm_cache_recreation_zero_binary_requests",
    "stale_generation_rejected",
    "partial_publication_failure_fuses_in_deterministic_tests",
    "device_loss_fuses_in_deterministic_tests",
  ]) {
    assert.equal(observations.recovery[outcome], true, `${outcome} must pass`);
  }
  assert.equal(observations.recovery.physical_device_loss_forced, false);
  assert.equal(observations.recovery.memory_pressure_forced, false);
  verifyTransportObservations(observations);
  verifyFrameObservations(observations.foreground_frames);
  verifyResourceObservations(entry);

  const evaluation = evaluateQualification(evaluationRecord(entry));
  assert.deepEqual(
    evaluation.failures,
    [],
    `recorded observations violate qualification limits: ${evaluation.failures.join("; ")}`,
  );
  assert.equal(observations.passed, evaluation.passed);

  assert.equal(matrix.external_evidence.independent_adopter, false);
  assert.equal(matrix.external_evidence.registry_install, false);
  assert.equal(matrix.external_evidence.support_qualified, false);
  assert.equal(matrix.external_evidence.release_candidate, false);
  return true;
}

function evaluationRecord(entry) {
  const observations = entry.observations;
  const [physicalWidth, physicalHeight] = entry.display.physical_viewport;
  return {
    cold: loadRecord(observations.cold),
    warm: loadRecord(observations.warm),
    frames: {
      callbackIntervalMilliseconds: {
        p95: observations.foreground_frames.callback_interval_p95_milliseconds,
      },
      submissionMilliseconds: {
        p95: observations.foreground_frames.submission_p95_milliseconds,
      },
    },
    cancellation: {
      acknowledgementMilliseconds:
        observations.recovery.cancellation_acknowledgement_milliseconds,
    },
    viewport: {
      physicalWidth,
      physicalHeight,
      surfaceBytes: entry.display.canvas_bytes,
    },
    state: {
      source: {
        publishedPoints: entry.workload.displayed_points,
        publishedBatches: entry.workload.displayed_batches,
        retainedRecordBytes: observations.resources.retained_record_bytes,
      },
      render: {
        residentBytes: observations.resources.renderer_resident_bytes,
        transientTextureBytes: observations.resources.transient_texture_bytes,
      },
    },
  };
}

function loadRecord(load) {
  return {
    timings: {
      firstCoverageMilliseconds: load.first_coverage_milliseconds,
      settledViewMilliseconds: load.settled_view_milliseconds,
    },
    metrics: {
      requestCount: load.binary_requests,
      concurrentResponseBytesHighWater: load.concurrent_response_bytes_high_water,
      decodedStagingBytesHighWater: load.worker_staging_bytes_high_water,
      cacheBytes: load.verified_cache_bytes,
    },
  };
}

function verifyTransportObservations(observations) {
  const { cold, warm } = observations;
  assert.equal(cold.binary_requests, 3);
  assert.equal(cold.requested_bytes, cold.received_bytes);
  assert.equal(cold.verified_cache_bytes, 0);
  assert.equal(warm.binary_requests, 0);
  assert.equal(warm.requested_bytes, 0);
  assert.equal(warm.received_bytes, 0);
  assert.equal(warm.concurrent_response_bytes_high_water, 0);
  assert.equal(warm.verified_cache_hits, cold.binary_requests);
  assert.equal(warm.verified_cache_bytes, cold.received_bytes);
}

function verifyFrameObservations(frames) {
  assert.equal(frames.sample_count, 30);
  assertOrderedSummary(
    frames.callback_interval_p50_milliseconds,
    frames.callback_interval_p95_milliseconds,
    frames.callback_interval_max_milliseconds,
    "callback interval",
  );
  assertOrderedSummary(
    frames.submission_p50_milliseconds,
    frames.submission_p95_milliseconds,
    frames.submission_max_milliseconds,
    "submission",
  );
}

function assertOrderedSummary(p50, p95, maximum, label) {
  assert.ok(p50 <= p95, `${label} p50 must not exceed p95`);
  assert.ok(p95 <= maximum, `${label} p95 must not exceed max`);
}

function verifyResourceObservations(entry) {
  const resources = entry.observations.resources;
  const [physicalWidth, physicalHeight] = entry.display.physical_viewport;
  assert.equal(entry.display.canvas_bytes, physicalWidth * physicalHeight * 4);
  assert.equal(resources.javascript_heap_api, "performance.memory.usedJSHeapSize");
  assert.equal(resources.javascript_heap_status, "non_standard_observation");
  assert.equal(resources.process_resident_bytes, null);
  assert.equal(resources.physical_cache_allocation_bytes, null);
  assert.equal(resources.physical_gpu_allocation_bytes, null);
}

function assertNonnegativeNumbers(value, path) {
  if (typeof value === "number") {
    assert.ok(Number.isFinite(value) && value >= 0, `${path} must be finite and nonnegative`);
    return;
  }
  if (value === null || typeof value !== "object") return;
  for (const [key, nested] of Object.entries(value)) {
    assertNonnegativeNumbers(nested, `${path}.${key}`);
  }
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const matrix = JSON.parse(await readFile(matrixUrl, "utf8"));
  verifyBrowserQualificationMatrix(matrix);
  console.log("browser qualification matrix passed");
}
