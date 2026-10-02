import assert from "node:assert/strict";
import test from "node:test";

import {
  createOccupancyImage,
  evaluateRepresentativeTiming,
  footprintRectangle,
  measureIsolatedFootprint,
  measureOccupancyTopology,
  sampleForegroundRgba,
} from "./footprint-runner-core.js";

const background = [19, 20, 19, 255];

test("occupancy normalization is color-independent and topology-ready", () => {
  const image = rgbaImage(4, 3, background);
  setPixel(image, 1, 1, [180, 20, 19, 255]);
  setPixel(image, 2, 1, [19, 190, 19, 255]);
  const occupancy = createOccupancyImage(image, background, 2);
  assert.deepEqual(pixel(occupancy, 0, 0), [0, 0, 0, 255]);
  assert.deepEqual(pixel(occupancy, 1, 1), [255, 255, 255, 255]);
  const report = measureOccupancyTopology(image, {
    backgroundRgba: background,
    rectangle: { x: 0, y: 0, width: 4, height: 3 },
  });
  assert.equal(report.metrics.foreground_pixels, 2);
  assert.equal(report.metrics.foreground.component_count, 1);
});

test("isolated footprint chooses the strongest local endpoint and bounded rectangle", () => {
  const image = rgbaImage(12, 12, background);
  setPixel(image, 5, 5, [200, 160, 40, 255]);
  setPixel(image, 6, 5, [110, 90, 30, 255]);
  assert.deepEqual(sampleForegroundRgba(image, [5.5, 5.5], background), [200, 160, 40, 255]);
  assert.deepEqual(footprintRectangle([5.5, 5.5], 4, 12, 12), {
    x: 0,
    y: 0,
    width: 11,
    height: 11,
  });
  const report = measureIsolatedFootprint(image, {
    center: [5.5, 5.5],
    diameterPhysicalPixels: 4,
    backgroundRgba: background,
  });
  assert.equal(report.metrics.radius_pixels, 2);
  assert.deepEqual(report.foreground_rgba, [200, 160, 40, 255]);
});

test("representative timing enforces absolute and predecessor-ratio ceilings", () => {
  const limits = {
    frame_interval_p95_milliseconds: 50,
    frame_submission_p95_milliseconds: 16.7,
    maximum_predecessor_ratio: 2,
  };
  const predecessor = {
    maximum_recreation_frame_interval_p95_milliseconds: 30,
    maximum_recreation_frame_submission_p95_milliseconds: 1,
  };
  assert.equal(evaluateRepresentativeTiming(timing(40, 1.5), predecessor, limits).passed, true);
  assert.deepEqual(evaluateRepresentativeTiming(timing(70, 3), predecessor, limits).failures, [
    "frame_interval_ceiling",
    "frame_interval_predecessor_ratio",
    "frame_submission_predecessor_ratio",
  ]);
});

function timing(interval, submission) {
  return {
    frame_interval_milliseconds: { p95: interval },
    frame_submission_milliseconds: { p95: submission },
  };
}

function rgbaImage(width, height, rgba) {
  const data = new Uint8Array(width * height * 4);
  for (let offset = 0; offset < data.length; offset += 4) data.set(rgba, offset);
  return { width, height, data };
}

function setPixel(image, x, y, rgba) {
  image.data.set(rgba, (y * image.width + x) * 4);
}

function pixel(image, x, y) {
  return Array.from(image.data.subarray((y * image.width + x) * 4, (y * image.width + x + 1) * 4));
}
