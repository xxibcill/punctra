import assert from "node:assert/strict";
import test from "node:test";
import { materializeLodFixture } from "./lod-fixture.js";
import { validateLodCapture, validateQuietTiming } from "./lod-records.js";
import { summarizeSamples } from "./visual-capture.js";

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
