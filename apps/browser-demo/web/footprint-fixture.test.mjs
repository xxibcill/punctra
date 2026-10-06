import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { materializeFootprintFixture, validateFootprintSampleBinding, validateIsolatedFootprintFixture } from "./footprint-fixture.js";
import { decodeTransferV2, materializeVisualTrial } from "./visual-corpus.js";

const visual = JSON.parse(await readFile(new URL("./fixtures/visual-v1/corpus.json", import.meta.url)));
const footprint = JSON.parse(await readFile(new URL("./fixtures/footprint-v1/corpus.json", import.meta.url)));

test("dedicated fixtures preserve exact Source Points and isolate every DPR trial", async () => {
  for (const focused of footprint.focused_trials) {
    const inherited = await materializeVisualTrial(visual, focused.id);
    const fixture = await materializeFootprintFixture(inherited, focused);
    const original = inherited.batches.flatMap((batch) => decodeTransferV2(batch));
    const points = decodeTransferV2(fixture.batches[0]);
    assert.deepEqual(points, original.filter(({ ordinal }) => focused.isolated_ordinals.includes(ordinal)));
    assert.equal(fixture.source_identity, inherited.source_identity);
    assert.equal(fixture.point_count, inherited.point_count);
    assert.equal(fixture.source.expected_view.settled_resident_points, points.length);
    assert.deepEqual(fixture.trial.temporal_trace, { kind: "static" });
    for (const profile of [footprint.canonical_profile, ...footprint.scale_profiles]) {
      const samples = validateIsolatedFootprintFixture(fixture, profile);
      assert.equal(samples.length, focused.isolated_ordinals.length);
      assert(samples.every(({ nearest_separation_pixels }) => nearest_separation_pixels > 7));
    }
  }
});

test("preflight rejects the inherited overlapping kernel targets", async () => {
  const focused = footprint.focused_trials[0];
  const inherited = await materializeVisualTrial(visual, focused.id);
  assert.throws(() => validateIsolatedFootprintFixture(inherited, footprint.canonical_profile),
    /overlaps|clipped/);
});

test("preflight rejects an unobstructed target whose measurement would be clipped", async () => {
  const focused = footprint.focused_trials[0];
  const inherited = await materializeVisualTrial(visual, focused.id);
  const fixture = await materializeFootprintFixture(inherited, focused);
  fixture.camera = { ...fixture.camera, vertical_field_of_view_radians: 0.05 };
  assert.throws(() => validateIsolatedFootprintFixture(fixture, footprint.canonical_profile), /clipped/);
});

test("fixture generation rejects missing Points and unrecognized sampling recipes", async () => {
  const focused = footprint.focused_trials[0];
  const inherited = await materializeVisualTrial(visual, focused.id);
  await assert.rejects(materializeFootprintFixture(inherited, { ...focused, isolated_ordinals: [9_999] }), /absent/);
  await assert.rejects(materializeFootprintFixture(inherited, { ...focused, fixture: "unknown" }), /recipe/);
});

test("metric bindings reject diluted windows, changed radii, and displaced centers", async () => {
  const focused = footprint.focused_trials[0];
  const fixture = await materializeFootprintFixture(await materializeVisualTrial(visual, focused.id), focused);
  const isolated = validateIsolatedFootprintFixture(fixture, footprint.canonical_profile)[0];
  const sample = {
    ordinal: isolated.ordinal,
    candidate: {
      center: [isolated.projected.exact_x, isolated.projected.exact_y],
      rectangle: isolated.rectangle,
      radius_pixels: 3,
    },
  };
  validateFootprintSampleBinding(sample, isolated, 6);
  for (const field of ["rectangle", "radius", "center"]) {
    const forged = structuredClone(sample);
    if (field === "rectangle") forged.candidate.rectangle.width += 20;
    if (field === "radius") forged.candidate.radius_pixels = 2;
    if (field === "center") forged.candidate.center[0] += 1;
    assert.throws(() => validateFootprintSampleBinding(forged, isolated, 6), new RegExp(field));
  }
});
