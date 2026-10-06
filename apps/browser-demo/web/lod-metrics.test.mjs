import assert from "node:assert/strict";
import test from "node:test";
import { measureRasterTransition, rasterDensityPointCount } from "./lod-metrics.js";

const red = [255, 0, 0, 255];
const green = [0, 255, 0, 255];
const clear = [0, 0, 0, 255];
const image = (...pixels) => ({ width: pixels.length, height: 1, data: Uint8Array.from(pixels.flat()) });
const controls = {
  outgoing: image(red, red, red, red), incoming: image(green, green, green, green),
  previous: image(red, red, red, red), backgroundRgba: clear, stationary: true,
};

test("stationary metrics enforce coverage, the rounded 25% bound, and whole-image RMSE", () => {
  const passing = measureRasterTransition({ ...controls, candidate: image(red, red, red, green) });
  assert.equal(passing.passed, true);
  assert.equal(passing.changed_common_pixels, 1);
  assert.equal(passing.allowed_changed_common_pixels, 1);
  assert.ok(Math.abs(passing.temporal_rmse - Math.sqrt(2 / 16)) < 1e-12);
  const pop = measureRasterTransition({ ...controls, candidate: image(red, red, green, green) });
  assert.equal(pop.passed, false);
  assert.ok(pop.failures.includes("stationary common-foreground change exceeds 25%"));
  assert.ok(pop.failures.includes("temporal RGBA RMSE exceeds 0.40"));
  const hole = measureRasterTransition({ ...controls, candidate: image(clear, red, red, red) });
  assert.equal(hole.new_clear_pixels, 1);
  assert.equal(hole.passed, false);
});

test("one-byte quantization is stable and movement retains its exact-camera coverage gate", () => {
  assert.equal(measureRasterTransition({ ...controls, candidate: image([254, 0, 0, 255], red, red, red) }).changed_common_pixels, 0);
  const moving = measureRasterTransition({ ...controls, previous: undefined, stationary: false, candidate: image(green, green, green, green) });
  assert.equal(moving.passed, true);
  assert.equal(moving.temporal_rmse, null);
  assert.equal(measureRasterTransition({ ...controls, previous: undefined, stationary: false, candidate: image(clear, green, green, green) }).passed, false);
  assert.throws(() => measureRasterTransition({ ...controls, candidate: image(red) }), /dimensions differ/);
});

test("fractional density rounds once and fully hidden retirement has no pulse", () => {
  const before = [{ point_count: 3, coverage_eighths: 0 }, { point_count: 7, coverage_eighths: 8 }];
  assert.equal(rasterDensityPointCount(before), 7);
  assert.equal(rasterDensityPointCount(before.slice(1)), 7);
  assert.equal(rasterDensityPointCount([{ point_count: 1, coverage_eighths: 3 }, { point_count: 1, coverage_eighths: 3 }]), 1);
  assert.throws(() => rasterDensityPointCount([{ point_count: 1, coverage_eighths: 9 }]), /fraction/);
  assert.throws(() => rasterDensityPointCount([{ point_count: Number.MAX_SAFE_INTEGER, coverage_eighths: 8 }]), /exact integer/);
});
