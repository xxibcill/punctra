import assert from "node:assert/strict";
import test from "node:test";

import {
  BASELINE_INPUTS_SCHEMA,
  createBaselineInputsManifest,
  encodeBaselineInputsManifest,
  validateBaselineInputsManifest,
} from "./visual-baseline-inputs.js";

test("baseline-input manifest deterministically binds record PNGs and packed runtime bytes", () => {
  const inputs = fixtureInputs();
  const first = createBaselineInputsManifest(inputs);
  const second = createBaselineInputsManifest(inputs);
  assert.deepEqual(second, first);
  assert.equal(first.schema, BASELINE_INPUTS_SCHEMA);
  assert.equal(Object.hasOwn(first, "implementation_commit"), false);
  assert.deepEqual(first.canonical_baselines.map(({ path }) => path), [
    "apps/browser-demo/web/fixtures/visual-v1/baselines/trial-a.png",
    "apps/browser-demo/web/fixtures/visual-v1/baselines/trial-b.png",
  ]);
  assert.deepEqual(encodeBaselineInputsManifest(first), encodeBaselineInputsManifest(second));
});

test("baseline-input validation rejects runtime, image, path, and commit tampering", () => {
  const inputs = fixtureInputs();
  const manifest = createBaselineInputsManifest(inputs);
  const runtime = structuredClone(manifest);
  runtime.package_artifact.runtime_artifacts[0].sha256 = "f".repeat(64);
  assert.throws(() => validateBaselineInputsManifest(runtime, inputs), /runtime artifacts differ/);
  const image = structuredClone(manifest);
  image.canonical_baselines[0].decoded_sha256 = "f".repeat(64);
  assert.throws(() => validateBaselineInputsManifest(image, inputs), /identity differs/);
  const moved = structuredClone(manifest);
  moved.canonical_baselines[0].path = "apps/browser-demo/web/fixtures/visual-v1/baselines/moved.png";
  assert.throws(() => validateBaselineInputsManifest(moved, inputs), /binding differs/);
  const commit = structuredClone(manifest);
  commit.implementation_commit = "0".repeat(40);
  assert.throws(() => validateBaselineInputsManifest(commit, inputs), /must not contain an implementation commit/);
});

function fixtureInputs() {
  const trials = [
    { id: "trial-a", baseline_path: "./baselines/trial-a.png" },
    { id: "trial-b", baseline_path: "./baselines/trial-b.png" },
  ];
  const artifacts = trials.map((trial, index) => ({
    kind: "baseline_png",
    trial_id: trial.id,
    path: `apps/browser-demo/web/fixtures/visual-v1/baselines/${trial.id}.png`,
    width: 640,
    height: 480,
    encoded_byte_length: 1_000 + index,
    encoded_sha256: String(index + 1).repeat(64),
    decoded_byte_length: 1_228_800,
    decoded_sha256: String(index + 3).repeat(64),
  }));
  const runtimeArtifacts = [{
    path: "apps/browser-demo/web/pkg/browser_demo.js",
    byte_length: 1_000,
    sha256: "a".repeat(64),
  }];
  return {
    release: "0.21.0-alpha.1",
    trials,
    artifacts,
    runtimeArtifacts,
  };
}
