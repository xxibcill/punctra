import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { QUALIFICATION_LIMITS } from "../apps/browser-demo/web/qualification.js";
import {
  changelogImplementationCommit,
  releaseImplementationCommit,
  releaseVerifierSha256,
  verifyBrowserQualificationMatrix,
  verifyImplementationCommit,
  verifyQualificationRuntimeTree,
  verifyUnqualifiedEntries,
  verifyWorkloadObservations,
} from "./verify-browser-qualification.mjs";

const changelogUrl = new URL("../CHANGELOG.md", import.meta.url);
const matrixUrl = new URL("../docs/releases/v0.21-browser-matrix.json", import.meta.url);
const releaseRecordUrl = new URL("../docs/releases/v0.21.0.md", import.meta.url);
const verifierUrl = new URL("./verify-browser-qualification.mjs", import.meta.url);
const [changelog, matrixSource, releaseRecord, verifierSource] = await Promise.all([
  readFile(changelogUrl, "utf8"),
  readFile(matrixUrl, "utf8"),
  readFile(releaseRecordUrl, "utf8"),
  readFile(verifierUrl, "utf8"),
]);
const matrix = JSON.parse(matrixSource);
const implementationCommit = releaseImplementationCommit(releaseRecord);

test("the checked-in browser qualification matrix derives a passing result", () => {
  assert.equal(verifyBrowserQualificationMatrix(matrix, implementationCommit), true);
});

test("the exact unqualified platform classes are frozen", () => {
  assert.doesNotThrow(() => verifyUnqualifiedEntries(matrix.unqualified_entries));
  for (const mutate of [
    (entries) => { entries.shift(); },
    (entries) => { entries[0].browser = "Other Chrome"; },
    (entries) => { entries[1].reason = "Not recorded"; },
    (entries) => { entries.push({ browser: "Other", reason: "Not run" }); },
  ]) {
    const entries = structuredClone(matrix.unqualified_entries);
    mutate(entries);
    assert.throws(
      () => verifyUnqualifiedEntries(entries),
      /unqualified platform classes must match the frozen v0.21 matrix/,
    );
  }
});

test("the qualification runtime package matches its packed and source files", () => {
  assert.doesNotThrow(verifyQualificationRuntimeTree);
});

test("the implementation pin includes the qualification verifier", () => {
  assert.match(verifierSource, /^\s+"scripts\/verify-browser-qualification\.mjs",$/m);
});

test("an over-limit observation fails even when the recorded pass flag is true", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.cold.first_coverage_milliseconds =
    QUALIFICATION_LIMITS.firstCoverageMilliseconds + 1;

  assert.throws(
    () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
    /cold first sampled Coverage exceeded 10000 ms/,
  );
});

test("the recorded pass flag must agree with the derived result", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.passed = false;
  assert.throws(() => verifyBrowserQualificationMatrix(tampered, implementationCommit));
});

test("cold network delivery is not mislabeled as a cache hit", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.cold.verified_cache_bytes = 172_696;
  assert.throws(() => verifyBrowserQualificationMatrix(tampered, implementationCommit));
});

test("the JSON matrix and Markdown record pin the same implementation", () => {
  assert.equal(matrix.implementation_commit, implementationCommit);
  assert.throws(
    () => verifyBrowserQualificationMatrix(matrix, "0000000000000000000000000000000000000000"),
  );
});

test("generation preservation is independently required for pre-publication recovery", () => {
  for (const field of [
    "prepublication_worker_generation_preserved",
    "prepublication_offline_generation_preserved",
  ]) {
    const tampered = structuredClone(matrix);
    tampered.qualified_entries[0].observations.recovery[field] = false;
    assert.throws(
      () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
      new RegExp(`${field} must pass`),
    );
  }
});

test("cancellation evidence requires viewer and frame retention", () => {
  for (const field of ["cancellation_viewer_retained", "cancellation_frame_retained"]) {
    const tampered = structuredClone(matrix);
    tampered.qualified_entries[0].observations.recovery[field] = false;
    assert.throws(
      () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
      new RegExp(`${field} must pass`),
    );
  }
});

test("observed workload identity is bound to the qualified deployment", () => {
  for (const field of ["deployment_id", "source_identity", "source_points"]) {
    const tampered = structuredClone(matrix);
    tampered.qualified_entries[0].observations.workload[field] = field === "source_points"
      ? 1
      : "other";
    assert.throws(
      () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
      /observed workload identity must match the qualified deployment/,
    );
  }
});

test("cold and warm loads preserve exact workload and transport evidence", () => {
  const missingWorkload = structuredClone(matrix);
  delete missingWorkload.qualified_entries[0].observations.cold.workload;
  assert.throws(
    () => verifyBrowserQualificationMatrix(missingWorkload, implementationCommit),
    /cold load workload facts must match the qualified deployment/,
  );

  const missingTiming = structuredClone(matrix);
  delete missingTiming.qualified_entries[0].observations.warm.main_thread_batch_milliseconds_high_water;
  assert.throws(
    () => verifyBrowserQualificationMatrix(missingTiming, implementationCommit),
    /warm load must preserve main_thread_batch_milliseconds_high_water/,
  );

  const mismatchedTransfer = structuredClone(matrix);
  mismatchedTransfer.qualified_entries[0].observations.cold.workload.transferred_bytes = 0;
  assert.throws(
    () => verifyBrowserQualificationMatrix(mismatchedTransfer, implementationCommit),
    /cold load workload facts must match the qualified deployment/,
  );

  const mismatchedNetwork = structuredClone(matrix);
  mismatchedNetwork.qualified_entries[0].observations.cold.requested_bytes = 0;
  assert.throws(
    () => verifyBrowserQualificationMatrix(mismatchedNetwork, implementationCommit),
    /Expected values to be strictly equal/,
  );

  const mismatchedWarmIdentity = structuredClone(matrix);
  mismatchedWarmIdentity.qualified_entries[0].observations.warm.workload.source_identity = "other";
  assert.throws(
    () => verifyBrowserQualificationMatrix(mismatchedWarmIdentity, implementationCommit),
    /warm load workload facts must match the qualified deployment/,
  );
});

test("workload evidence is bound to the checked-in deployment bytes", () => {
  const tampered = structuredClone(matrix.qualified_entries[0]);
  tampered.workload.source_points = 1;
  tampered.observations.workload.source_points = 1;

  assert.throws(
    () => verifyWorkloadObservations(tampered),
    /recorded Source point count must match the checked-in deployment/,
  );
});

test("observed render output is required by the evaluator", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.render.drawn_points = 1;
  assert.throws(
    () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
    /observed render output must match the qualified workload/,
  );
});

test("qualification evidence preserves environment and heap phase fields", () => {
  const missingEnvironment = structuredClone(matrix);
  delete missingEnvironment.qualified_entries[0].observations.environment.visibility_state;
  assert.throws(
    () => verifyBrowserQualificationMatrix(missingEnvironment, implementationCommit),
    /recorded browser environment must include the declared runtime facts/,
  );

  const missingHeapPhase = structuredClone(matrix);
  delete missingHeapPhase.qualified_entries[0].observations.resources.javascript_heap_before_bytes;
  assert.throws(
    () => verifyBrowserQualificationMatrix(missingHeapPhase, implementationCommit),
    /resources must preserve javascript_heap_before_bytes/,
  );
});

test("JavaScript heap status and high-water agree with phase observations", () => {
  const available = structuredClone(matrix);
  const availableResources = available.qualified_entries[0].observations.resources;
  availableResources.javascript_heap_status = "non_standard_observation";
  availableResources.javascript_heap_before_bytes = 100;
  availableResources.javascript_heap_after_cold_bytes = 120;
  availableResources.javascript_heap_after_warm_bytes = 110;
  availableResources.javascript_heap_after_frames_bytes = 130;
  availableResources.javascript_heap_high_water_bytes = 130;
  assert.doesNotThrow(() => verifyBrowserQualificationMatrix(available, implementationCommit));

  const missingHighWater = structuredClone(matrix);
  delete missingHighWater.qualified_entries[0].observations.resources.javascript_heap_high_water_bytes;
  assert.throws(
    () => verifyBrowserQualificationMatrix(missingHighWater, implementationCommit),
    /resources must preserve javascript_heap_high_water_bytes/,
  );

  const inconsistentStatus = structuredClone(matrix);
  inconsistentStatus.qualified_entries[0].observations.resources.javascript_heap_status =
    "non_standard_observation";
  inconsistentStatus
    .qualified_entries[0].observations.resources.javascript_heap_before_bytes = null;
  assert.throws(
    () => verifyBrowserQualificationMatrix(inconsistentStatus, implementationCommit),
    /non-standard JavaScript heap observations must include every numeric phase/,
  );

  const inconsistentHighWater = structuredClone(available);
  inconsistentHighWater.qualified_entries[0].observations.resources.javascript_heap_high_water_bytes =
    131;
  assert.throws(
    () => verifyBrowserQualificationMatrix(inconsistentHighWater, implementationCommit),
    /JavaScript heap high-water must match the phase observations/,
  );
});

test("the JSON matrix and Markdown record pin the same qualification verifier", () => {
  assert.equal(matrix.verifier_sha256, releaseVerifierSha256(releaseRecord));
  assert.match(matrix.verifier_sha256, /^[0-9a-f]{64}$/);
});

test("the changelog pins the same implementation as the evidence records", () => {
  assert.equal(changelogImplementationCommit(changelog), implementationCommit);
});

test("matching records cannot pin a nonexistent implementation commit", () => {
  const nonexistentCommit = "f".repeat(40);
  const tampered = structuredClone(matrix);
  tampered.implementation_commit = nonexistentCommit;

  assert.throws(
    () => verifyBrowserQualificationMatrix(tampered, nonexistentCommit),
    /does not resolve to a repository commit/,
  );
});

test("the implementation pin rejects later changes to qualified browser files", () => {
  assert.throws(
    () => verifyImplementationCommit("7c3ceec11fc2cc4d3eae5db9ebd2399271c0cb18"),
    /qualified implementation files changed after/,
  );
});

test("the implementation pin covers every executable application tree", () => {
  assert.match(verifierSource, /^\s*"apps",$/m);
  assert.doesNotMatch(verifierSource, /^\s*"apps\/browser-demo\/web",$/m);
});

test("the qualified entry rejects exact lane and workload drift", () => {
  const mutations = [
    ["operating-system version", (entry) => { entry.operating_system.version = "0.0"; }],
    ["operating-system build", (entry) => { entry.operating_system.build = "tampered"; }],
    ["device GPU", (entry) => { entry.device.gpu = "Different GPU"; }],
    ["device class", (entry) => { entry.device.class = "Different device"; }],
    ["display DPR", (entry) => { entry.display.device_pixel_ratio = 1; }],
    ["display path", (entry) => { entry.display.display_path = "external display"; }],
    ["deployment identity", (entry) => { entry.workload.deployment_id = "other"; }],
    ["Source identity", (entry) => { entry.workload.source_identity = "00".repeat(32); }],
  ];

  for (const [label, mutate] of mutations) {
    const tampered = structuredClone(matrix);
    mutate(tampered.qualified_entries[0]);
    assert.throws(
      () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
      undefined,
      `${label} drift must fail verification`,
    );
  }
});
