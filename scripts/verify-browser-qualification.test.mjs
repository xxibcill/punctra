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
} from "./verify-browser-qualification.mjs";

const changelogUrl = new URL("../CHANGELOG.md", import.meta.url);
const matrixUrl = new URL("../docs/releases/v0.19-browser-matrix.json", import.meta.url);
const releaseRecordUrl = new URL("../docs/releases/v0.19.0.md", import.meta.url);
const [changelog, matrixSource, releaseRecord] = await Promise.all([
  readFile(changelogUrl, "utf8"),
  readFile(matrixUrl, "utf8"),
  readFile(releaseRecordUrl, "utf8"),
]);
const matrix = JSON.parse(matrixSource);
const implementationCommit = releaseImplementationCommit(releaseRecord);

test("the checked-in browser qualification matrix derives a passing result", () => {
  assert.equal(verifyBrowserQualificationMatrix(matrix, implementationCommit), true);
});

test("the qualification runtime package matches its packed and source files", () => {
  assert.doesNotThrow(verifyQualificationRuntimeTree);
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

test("observed render output is required by the evaluator", () => {
  const tampered = structuredClone(matrix);
  tampered.qualified_entries[0].observations.render.drawn_points = 1;
  assert.throws(
    () => verifyBrowserQualificationMatrix(tampered, implementationCommit),
    /observed render output must match the qualified workload/,
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
