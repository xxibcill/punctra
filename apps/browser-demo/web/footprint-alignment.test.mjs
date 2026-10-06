import assert from "node:assert/strict";
import test from "node:test";
import { measureFeatureAlignment, measureFeatureComponentAlignment } from "./footprint-alignment.js";

const background = [19, 20, 19, 255];
const rectangle = { x: 4, y: 4, width: 40, height: 40 };

test("alignment rejects translations beyond one pixel despite altered size and opacity", () => {
  const reference = scene(3.5, 1, 0, 0);
  for (const [x, y] of [[0, 0], [1, 0], [-2, 0], [0, 3], [-3, -2]]) {
    const report = measureFeatureAlignment(reference, scene(2.5, 0.75, x, y), rectangle, background);
    assert.deepEqual(report.best_offset_pixels, { x, y });
    assert.equal(report.ambiguous, false);
    assert.equal(report.distance_pixels <= 1, Math.hypot(x, y) <= 1);
    assert(report.correlation > 0.7);
  }
});

test("alignment refuses blank or translation-ambiguous features", () => {
  const blank = image(background);
  assert.equal(measureFeatureAlignment(blank, blank, rectangle, background).correlation, null);
  const uniform = image([80, 90, 100, 255]);
  assert.equal(measureFeatureAlignment(uniform, uniform, rectangle, background).ambiguous, true);
});

test("local alignment detects a weak feature moving or disappearing beside fixed strong content", () => {
  const reference = twoFeatures(0, false);
  assert.equal(measureFeatureComponentAlignment(reference, reference, { x: 4, y: 4, width: 88, height: 40 }, background).passed, true);
  for (const candidate of [twoFeatures(3, false), twoFeatures(0, true)]) {
    const report = measureFeatureComponentAlignment(reference, candidate, { x: 4, y: 4, width: 88, height: 40 }, background);
    assert.equal(report.passed, false);
  }
});

test("local alignment rejects displacement beyond its bounded search", () => {
  const reference = twoFeatures(0, false);
  assert.equal(measureFeatureComponentAlignment(reference, twoFeatures(10, false),
    { x: 4, y: 4, width: 88, height: 40 }, background).passed, false);
});

test("components expose same-tile weak movement and deletion that aggregate registration hides", () => {
  const reference = sameTileScene(0, false);
  for (const candidate of [sameTileScene(3, false), sameTileScene(0, true)]) {
    const aggregate = measureFeatureAlignment(reference, candidate, rectangle, background);
    assert.equal(aggregate.distance_pixels, 0);
    assert(aggregate.correlation > 0.99);
    const components = measureFeatureComponentAlignment(reference, candidate, rectangle, background);
    assert.equal(components.regions.length, 2);
    assert.equal(components.passed, false);
  }
});

test("components cannot silently omit landmarks at the image boundary", () => {
  const reference = image(background);
  reference.data.set([240, 240, 240, 255], 0);
  assert.throws(() => measureFeatureComponentAlignment(reference, reference,
    { x: 0, y: 0, width: 48, height: 48 }, background), /complete search and blur margins/);
});

test("alignment keeps all candidate windows complete and rejects malformed images", () => {
  assert.throws(() => measureFeatureAlignment(scene(3, 1), scene(3, 1),
    { ...rectangle, x: 0 }, background), /complete search margin/);
  assert.throws(() => measureFeatureAlignment(scene(3, 1), { width: 48, height: 48, data: [] },
    rectangle, background), /RGBA8/);
});

function image(color) {
  const data = new Uint8Array(48 * 48 * 4);
  for (let offset = 0; offset < data.length; offset += 4) data.set(color, offset);
  return { width: 48, height: 48, data };
}

function scene(radius, opacity, dx = 0, dy = 0) {
  const result = image(background);
  for (const [cx, cy, color] of [[12.3, 15.6, [240, 70, 80]], [31.1, 13.8, [40, 230, 100]], [23.6, 33.2, [80, 60, 220]]]) {
    for (let y = 0; y < 48; y += 1) {
      for (let x = 0; x < 48; x += 1) {
        if (Math.hypot(x + 0.5 - cx - dx, y + 0.5 - cy - dy) > radius) continue;
        result.data.set([...color.map((value, channel) => Math.round(background[channel]
          + (value - background[channel]) * opacity)), 255], (y * 48 + x) * 4);
      }
    }
  }
  return result;
}

function twoFeatures(dx, omitWeak) {
  const result = { width: 96, height: 48, data: new Uint8Array(96 * 48 * 4) };
  for (let offset = 0; offset < result.data.length; offset += 4) result.data.set(background, offset);
  for (const [cx, cy, color] of [[18, 17, [240, 240, 240]], [21, 31, [230, 230, 230]],
    ...(omitWeak ? [] : [[71 + dx, 21, [45, 40, 35]], [79 + dx, 32, [40, 45, 35]]])]) {
    for (let y = 0; y < result.height; y += 1) {
      for (let x = 0; x < result.width; x += 1) {
        if (Math.hypot(x + 0.5 - cx, y + 0.5 - cy) <= 3) {
          result.data.set([...color, 255], (y * result.width + x) * 4);
        }
      }
    }
  }
  return result;
}

function sameTileScene(dx, omitWeak) {
  const result = image(background);
  for (const [cx, cy, color] of [[13, 13, [240, 240, 240]],
    ...(omitWeak ? [] : [[26 + dx, 26, [45, 40, 35]]])]) {
    for (let y = 0; y < 48; y += 1) {
      for (let x = 0; x < 48; x += 1) {
        if (Math.hypot(x + 0.5 - cx, y + 0.5 - cy) <= 3) {
          result.data.set([...color, 255], (y * 48 + x) * 4);
        }
      }
    }
  }
  return result;
}
