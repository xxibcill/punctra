import assert from "node:assert/strict";
import test from "node:test";
import { materializeLodFixture } from "./lod-fixture.js";
import { boundaryArgumentFacts, lodBoundaryCases, validateLodArtifact, validateLodBoundaryMatrix, validateLodCapture,
  validateLodEndpointBinding, validateLodNominalPick, validateLodPredecessorRuntime, validatePairedLodTiming, validateQuietTiming,
  validateLodSessionEnvironment, validateLodFunctionalContinuation } from "./lod-records.js";
import { QUALIFICATION_PROFILES } from "./qualification-lane-v0.23.js";
import { summarizeSamples } from "./visual-capture.js";

test("session profiles are closed and separate verify cannot switch screen facts", () => {
  const environments = QUALIFICATION_PROFILES.map(lodEnvironment);
  for (const environment of environments) {
    assert.equal(validateLodSessionEnvironment(environment, structuredClone(environment)).id, environment.qualification_lane);
    for (const mutate of [
      (value) => { value.screen.width = 1921; },
      (value) => { value.screen.pixel_depth_bits = 16; },
      (value) => { value.qualification_lane = "other"; },
      (value) => { value.host.device.gpu = "other"; },
    ]) {
      const changed = structuredClone(environment); mutate(changed);
      assert.throws(() => validateLodSessionEnvironment(changed), /declared|differs/);
    }
  }
  assert.throws(() => validateLodSessionEnvironment(environments[1], environments[0]), /record and verify environments differ/);
});

test("continuation binds observed environments, loaded runtime and the clean packed quickstart", () => {
  for (const profile of QUALIFICATION_PROFILES) {
    const commit = "a".repeat(40), digest = "b".repeat(64);
    const runtime = { package_name: "@punctra/viewer", package_version: "0.23.0-alpha.1",
      artifacts: [{ path: "runtime", sha256: digest }], packed_artifact: { sha256: digest } };
    const record = { pins: { implementation: { commit }, runtime }, environment: lodEnvironment(profile) };
    const environment = { ...profile.runtime.browser, screen: profile.runtime.screen,
      host: profile.runtime.host, visibilityState: "visible", secureContext: true };
    const records = {
      matrix: { implementation_commit: commit, release: "0.23.0-alpha.1", qualified_entries: [{ id: profile.lane.id }] },
      quickstart: { implementation_commit: commit, lane_id: profile.lane.id,
        acceptance: { packageVersion: "0.23.0-alpha.1", disposed: true, packedRuntime: {
          schema: "punctra-browser-packed-runtime-v1", build: "production", serverContract: "punctra-strict-range-v1",
          viewerPackage: "@punctra/viewer", viewerVersion: "0.23.0-alpha.1", viewerArtifactSha256: digest } } },
      functional: { acceptance: { implementation_commit: commit, package_version: "0.23.0-alpha.1",
        runtime_pins: runtime, environment, completed_environment: structuredClone(environment),
        runtime_lane: { lane: profile.lane.id, passed: true, failures: [] },
        final_state: { viewport: profile.runtime.display, capabilities: profile.runtime.capabilities } } },
    };
    assert.doesNotThrow(() => validateLodFunctionalContinuation(record, records));
    for (const mutate of [
      (value) => { value.functional.acceptance.runtime_pins.artifacts[0].sha256 = "c".repeat(64); },
      (value) => { value.quickstart.acceptance.packedRuntime.viewerArtifactSha256 = "c".repeat(64); },
      (value) => { value.quickstart.implementation_commit = "c".repeat(40); },
      (value) => { value.functional.acceptance.environment.screen.width += 1; },
      (value) => { value.functional.acceptance.completed_environment.screen.colorDepth = 16; },
      (value) => { value.functional.acceptance.runtime_lane.passed = false; },
      (value) => { delete value.functional.acceptance.runtime_lane.passed; },
      (value) => { value.functional.acceptance.final_state.capabilities.backend = "other"; },
    ]) {
      const changed = structuredClone(records); mutate(changed);
      assert.throws(() => validateLodFunctionalContinuation(record, changed), /differs|differ/);
    }
  }
});

function lodEnvironment({ lane, runtime }) {
  return { qualification_lane: lane.id, browser_user_agent: lane.browser.user_agent,
    browser_platform: runtime.browser.platform, physical_display_observed: false,
    screen: { width: runtime.screen.width, height: runtime.screen.height,
      color_depth_bits: runtime.screen.colorDepth, pixel_depth_bits: runtime.screen.pixelDepth },
    host: { operating_system: runtime.host.operatingSystem, package: runtime.host.package,
      device: { class: runtime.host.device.class, gpu: runtime.host.device.gpu,
        gpu_cores: runtime.host.device.gpuCores, gpu_class: runtime.host.device.gpuClass,
        metal_support: runtime.host.device.metalSupport } } };
}

test("capture acceptance binds actual renderer controls, exact identities and charged hidden residency", async () => {
  const fixture = await materializeLodFixture({ projection: "perspective" });
  const profile = { physical_width: 640, physical_height: 480 };
  const batches = fixture.batch_point_counts.map((point_count, index) => ({ batch_index: index, key: index + 1,
    version: 2, point_count, state: "resident", presentation_weight_u8: 255 }));
  const record = { source: { source_identity: fixture.source_identity, world_origin: fixture.world_origin, generation: 1, phase: "complete" },
    capture: { facts: { width: 640, height: 480, view_generation: 1, resident_bytes: 3888, drawn_points: 162, draw_calls: 3,
      batches, raster_transitions: batches.map((batch, index) => ({ batch_index: index, key: batch.key, version: 2,
        generation: 1, seed: 9029, step: 3, side: index === 0 ? "outgoing" : "incoming", coverage_eighths: index === 0 ? 5 : 3 })),
      raster_transition_batches: 3, renderer_transient_texture_bytes: 9_830_400,
      point_footprint: { display_size_physical_pixels: 6, nominal_pick_size_physical_pixels: 7, selected: "multisample4x" } } } };
  const inputs = { fixture, profile, generation: 1, version: 2, seed: 9029, step: 3, coarsen: false };
  assert.equal(validateLodCapture(record, inputs), 81);
  for (const mutate of [
    (value) => { value.capture.facts.resident_bytes = 1944; },
    (value) => { value.capture.facts.raster_transitions[0].version = 1; },
    (value) => { value.capture.facts.raster_transitions[1].coverage_eighths = 4; },
    (value) => { value.capture.facts.raster_transition_batches = 2; },
    (value) => { value.capture.facts.renderer_transient_texture_bytes = 67_108_865; },
    (value) => { value.source.source_identity = "a".repeat(64); },
  ]) {
    const tampered = structuredClone(record);
    mutate(tampered);
    assert.throws(() => validateLodCapture(tampered, inputs), /differ|ceiling/);
  }
});

test("artifact role aliases and duplicate negative probes cannot qualify unrelated claims", () => {
  const probes = lodBoundaryCases().map(([id, args]) => ({ id, arguments: boundaryArgumentFacts(args) }));
  validateLodBoundaryMatrix(probes);
  const duplicate = structuredClone(probes);
  duplicate[1] = duplicate[0];
  assert.throws(() => validateLodBoundaryMatrix(duplicate), /matrix/);
  assert.throws(() => validateLodBoundaryMatrix(probes.slice(0, -1)), /matrix/);
  const changed = structuredClone(probes);
  changed[1].arguments[5] = 0;
  assert.throws(() => validateLodBoundaryMatrix(changed), /matrix/);
  const profile = { physical_width: 640, physical_height: 480 };
  const record = { artifact: { path: "candidate.png", kind: "lod_candidate_png", width: 640, height: 480, decoded_byte_length: 1228800 },
    capture: { facts: { width: 640, height: 480 } } };
  const expected = { path: "candidate.png", kind: "lod_candidate_png", profile };
  validateLodArtifact(record, expected);
  assert.throws(() => validateLodArtifact(record, { ...expected, path: "outgoing.png" }), /role/);
  const tiny = structuredClone(record);
  tiny.artifact.width = 1;
  tiny.artifact.height = 1;
  tiny.artifact.decoded_byte_length = 4;
  assert.throws(() => validateLodArtifact(tiny, expected), /dimensions/);
});

test("quiet timing derives percentiles from capture-free samples and enforces predecessor cost", () => {
  const intervals = Array(30).fill(16);
  const submissions = Array(30).fill(0.2);
  const timing = { frame_count: 30, capture_free: true, frame_interval_samples_milliseconds: intervals,
    frame_submission_samples_milliseconds: submissions, frame_interval: summarizeSamples(intervals), frame_submission: summarizeSamples(submissions) };
  const limits = { frame_callback_p95_milliseconds: 50, frame_submission_p95_milliseconds: 16.7, canonical_predecessor_p95_ratio: 2 };
  validateQuietTiming(timing, limits, { frame_interval: 16.7, frame_submission: 0.2 });
  assert.throws(() => validateQuietTiming(timing, limits, { frame_interval: 5, frame_submission: 0.2 }), /predecessor/);
  const tampered = structuredClone(timing);
  tampered.frame_interval.p95 = 1;
  assert.throws(() => validateQuietTiming(tampered, limits), /samples/);
});

test("paired fallback captures reject camera and diameter substitutions", async () => {
  const fixture = await materializeLodFixture({ projection: "orthographic" });
  const camera = { ...fixture.camera, vertical_field_of_view_radians: null };
  const capture = { camera, capture: { facts: { point_footprint: { display_size_physical_pixels: 6 } } } };
  const frame = { candidate: structuredClone(capture), outgoing: structuredClone(capture), incoming: structuredClone(capture) };
  validateLodEndpointBinding(frame, fixture.camera);
  for (const mutate of [
    (value) => { value.outgoing.camera.target[0] += 0.25; },
    (value) => { for (const role of ["candidate", "outgoing", "incoming"]) value[role].camera.eye[1] += 1; },
    (value) => { value.incoming.capture.facts.point_footprint.display_size_physical_pixels = 5; },
  ]) {
    const changed = structuredClone(frame);
    mutate(changed);
    assert.throws(() => validateLodEndpointBinding(changed, fixture.camera), /camera|diameters/);
  }
});

test("canonical picks bind authored centers, allowed neighbors and exact resident identities", () => {
  const expected = { source_identity: "a".repeat(64), generation: 1, batch_key: 3, batch_version: 2, point_ordinal: "104" };
  const pick = { center: [320, 240], pixel: [319, 241], expected,
    observed: { ...expected, authority: "provisional_gpu_hint", status: "hit" } };
  validateLodNominalPick(pick, expected, [320, 240], 1);
  for (const mutate of [
    (value) => { value.observed.batch_key = 4; },
    (value) => { value.observed.batch_version = 1; },
    (value) => { value.expected.batch_key = 4; },
    (value) => { value.center[0] += 1; },
    (value) => { value.pixel[0] = 318; },
    (value) => { value.pixel[1] = 240.5; },
  ]) {
    const changed = structuredClone(pick);
    mutate(changed);
    assert.throws(() => validateLodNominalPick(changed, expected, [320, 240], 1), /pick/);
  }
  assert.throws(() => validateLodNominalPick(pick, expected, [320, 240]), /pixel/);
});

test("paired runtime pins cannot substitute a current module, alias a path or omit an artifact", () => {
  const relatives = ["package.json", "pkg/browser_demo.js", "pkg/browser_demo_bg.wasm"];
  const frozen = { package_name: "@punctra/viewer", package_version: "0.22.0-alpha.1",
    artifacts: relatives.map((path, index) => ({ path, byte_length: 100 + index, sha256: String(index).repeat(64) })) };
  const paired = { ...frozen, artifacts: frozen.artifacts.map((artifact) => ({ ...artifact,
    path: `target/predecessors/v0.22/node_modules/@punctra/viewer/${artifact.path}` })) };
  validateLodPredecessorRuntime(paired, frozen);
  for (const mutate of [
    (value) => { value.package_version = "0.23.0-alpha.1"; },
    (value) => { value.artifacts[1].sha256 = "a".repeat(64); },
    (value) => { value.artifacts[1].path = value.artifacts[0].path; },
    (value) => { value.artifacts.pop(); },
  ]) {
    const changed = structuredClone(paired);
    mutate(changed);
    assert.throws(() => validateLodPredecessorRuntime(changed, frozen), /predecessor/);
  }
});

test("paired cadence does not waive absolute costs, a real two-times regression or an unmeasurable control", () => {
  const timing = (interval, submission) => {
    const intervals = Array(30).fill(interval), submissions = Array(30).fill(submission);
    return { frame_count: 30, capture_free: true, frame_interval_samples_milliseconds: intervals,
      frame_submission_samples_milliseconds: submissions, frame_interval: summarizeSamples(intervals), frame_submission: summarizeSamples(submissions) };
  };
  const limits = { frame_callback_p95_milliseconds: 50, frame_submission_p95_milliseconds: 16.7, canonical_predecessor_p95_ratio: 2 };
  validatePairedLodTiming(timing(34, 0.3), timing(34.5, 0.3), limits, "same session");
  assert.throws(() => validatePairedLodTiming(timing(51, 0.3), timing(34.5, 0.3), limits, "absolute"), /ceiling/);
  assert.throws(() => validatePairedLodTiming(timing(34, 0.7), timing(34.5, 0.3), limits, "regression"), /predecessor/);
  assert.throws(() => validatePairedLodTiming(timing(34, 0.3), timing(34.5, 0), limits, "zero"), /measurable/);
});
