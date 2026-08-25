import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import { QUALIFICATION_LIMITS } from "../apps/browser-demo/web/qualification.js";

const matrixUrl = new URL("../docs/releases/v0.19-browser-matrix.json", import.meta.url);
const matrix = JSON.parse(await readFile(matrixUrl, "utf8"));

assert.equal(matrix.schema, "punctra-browser-qualification-matrix-v1");
assert.equal(matrix.release, "0.19.0-alpha.1");
assert.match(matrix.observed_on, /^\d{4}-\d{2}-\d{2}$/);
assert.equal(matrix.qualified_entries.length, 1);
assert.ok(matrix.unqualified_entries.length >= 1);

const entry = matrix.qualified_entries[0];
assert.equal(entry.status, "repository_qualified_exact_lane");
assert.equal(entry.browser.surface, "Codex in-app browser");
assert.match(entry.browser.user_agent, /Chrome\/151\.0\.0\.0/);
assert.equal(entry.operating_system.architecture, "arm64");
assert.equal(entry.webgpu.backend, "BrowserWebGpu");
assert.equal(entry.webgpu.render_attachment, true);
assert.equal(entry.workload.coverage, "sampled");
assert.equal(entry.workload.displayed_points, 4_096);
assert.equal(entry.observations.cold.binary_requests, 3);
assert.equal(entry.observations.warm.binary_requests, 0);
assert.equal(entry.observations.warm.verified_cache_hits, 3);
assert.equal(entry.observations.recovery.stale_generation_rejected, true);
assert.equal(entry.observations.recovery.physical_device_loss_forced, false);
assert.equal(entry.observations.recovery.memory_pressure_forced, false);
assert.deepEqual(entry.observations.limits, QUALIFICATION_LIMITS);
assert.equal(entry.observations.passed, true);

for (const value of [
  entry.observations.cold.first_coverage_milliseconds,
  entry.observations.cold.settled_view_milliseconds,
  entry.observations.foreground_frames.callback_interval_p95_milliseconds,
  entry.observations.foreground_frames.submission_p95_milliseconds,
  entry.observations.recovery.cancellation_acknowledgement_milliseconds,
]) {
  assert.ok(Number.isFinite(value) && value >= 0);
}

assert.equal(matrix.external_evidence.independent_adopter, false);
assert.equal(matrix.external_evidence.support_qualified, false);
assert.equal(matrix.external_evidence.release_candidate, false);

console.log("browser qualification matrix passed");
