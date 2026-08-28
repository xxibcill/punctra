import assert from "node:assert/strict";
import test from "node:test";

import {
  HARD_TOLERANCE_CAPS,
  compareCanonicalImages,
  compareTemporalSequence,
  createDifferenceImage,
  measureCoverage,
  summarizeTemporalPairs,
  validateToleranceProfile,
  writeDifferenceImage,
} from "./visual-comparison.js";
import { decodeRgba8Png, encodeRgba8Png } from "./visual-png.js";

const BACKGROUND = [19, 20, 19, 255];
const EXACT = Object.freeze({
  channel_threshold: 0,
  maximum_channel_delta: 0,
  mean_channel_delta: 0,
  rms_channel_delta: 0,
  p95_channel_delta: 0,
  unstable_pixel_fraction: 0,
  coverage_fraction_delta: 0,
  feature_occupancy_fraction_delta: 0,
  feature_centroid_distance_pixels: 0,
});
const BOUNDED = Object.freeze({ ...HARD_TOLERANCE_CAPS });

test("canonical comparison reports exact pixels, Coverage, bounds, centroid, and ROI occupancy", () => {
  const image = solidImage(4, 4, BACKGROUND);
  setPixel(image, 1, 1, [240, 80, 20, 255]);
  setPixel(image, 2, 1, [240, 80, 20, 255]);

  const report = compareCanonicalImages(image, cloneImage(image), {
    toleranceProfile: EXACT,
    backgroundRgba: BACKGROUND,
    features: [{
      id: "two-pixel-feature",
      rectangle: { x: 1, y: 1, width: 2, height: 1 },
      minimum_foreground_pixels: 2,
    }],
  });

  assert.equal(report.passed, true);
  assert.deepEqual(report.dimensions, {
    match: true,
    reference: { width: 4, height: 4 },
    candidate: { width: 4, height: 4 },
    pixel_count: 16,
    channel_count: 64,
  });
  assert.deepEqual(report.pixels, {
    exact: 16,
    unstable: 0,
    unstable_fraction: 0,
    unstable_bbox: null,
  });
  assert.equal(report.difference_regions.mask.kind, "reference_exact_background_rgba8-v1");
  assert.equal(report.difference_regions.background.pixel_count, 14);
  assert.equal(report.difference_regions.foreground.pixel_count, 2);
  assert.equal(report.difference_regions.background.unstable_pixels, 0);
  assert.equal(report.difference_regions.foreground.channels.maximum_absolute_delta, 0);
  assert.deepEqual(report.coverage.candidate.bbox, { x: 1, y: 1, width: 2, height: 1 });
  assert.deepEqual(report.coverage.candidate.centroid, { x: 1.5, y: 1 });
  assert.equal(report.features[0].candidate.foreground_pixels, 2);
  assert.equal(report.features[0].passed, true);
});

test("bounded comparison accepts sub-threshold channel variation and reports it", () => {
  const reference = solidImage(100, 100, BACKGROUND);
  const candidate = cloneImage(reference);
  candidate.data[0] += 2;

  const report = compareCanonicalImages(reference, candidate, {
    toleranceProfile: BOUNDED,
    backgroundRgba: BACKGROUND,
    features: [],
  });

  assert.equal(report.passed, true);
  assert.equal(report.channels.maximum_absolute_delta, 2);
  assert.equal(report.pixels.unstable, 0);
});

test("comparison fails independent maximum and unstable-pixel gates with a bounded bbox", () => {
  const reference = solidImage(100, 100, BACKGROUND);
  const candidate = cloneImage(reference);
  setPixel(candidate, 31, 42, [29, 20, 19, 255]);

  const report = compareCanonicalImages(reference, candidate, {
    toleranceProfile: BOUNDED,
    backgroundRgba: BACKGROUND,
    features: [],
  });

  assert.equal(report.passed, false);
  assert(report.failures.includes("maximum_channel_delta"));
  assert.deepEqual(report.pixels.unstable_bbox, { x: 31, y: 42, width: 1, height: 1 });
  assert.equal(report.pixels.unstable, 1);
});

test("dimension mismatch is an evidence failure rather than a cropped comparison", () => {
  const report = compareCanonicalImages(
    solidImage(2, 2, BACKGROUND),
    solidImage(3, 2, BACKGROUND),
    { toleranceProfile: EXACT, backgroundRgba: BACKGROUND },
  );
  assert.equal(report.passed, false);
  assert.deepEqual(report.failures, ["dimension_mismatch"]);
  assert.equal(report.channels, null);
  assert.equal(report.difference_regions, null);
  assert.equal(report.coverage, null);
});

test("feature checks independently fail occupancy and centroid displacement", () => {
  const reference = solidImage(100, 100, BACKGROUND);
  const candidate = cloneImage(reference);
  setPixel(reference, 40, 40, [200, 200, 200, 255]);
  setPixel(candidate, 43, 40, [200, 200, 200, 255]);
  const report = compareCanonicalImages(reference, candidate, {
    toleranceProfile: BOUNDED,
    backgroundRgba: BACKGROUND,
    features: [{
      id: "moving-dot",
      rectangle: { x: 35, y: 35, width: 15, height: 15 },
      minimum_foreground_pixels: 1,
    }],
  });
  assert.equal(report.features[0].centroid_distance_pixels, 3);
  assert(report.features[0].failures.includes("centroid_distance"));

  const missing = solidImage(100, 100, BACKGROUND);
  const missingReport = compareCanonicalImages(reference, missing, {
    toleranceProfile: BOUNDED,
    backgroundRgba: BACKGROUND,
    features: [{
      id: "moving-dot",
      rectangle: { x: 35, y: 35, width: 15, height: 15 },
      minimum_foreground_pixels: 1,
    }],
  });
  assert(missingReport.features[0].failures.includes("candidate_occupancy"));
  assert(missingReport.features[0].failures.includes("centroid_distance"));
});

test("Coverage can be measured over one named rectangle", () => {
  const image = solidImage(6, 4, BACKGROUND);
  setPixel(image, 4, 2, [255, 255, 255, 255]);
  assert.equal(measureCoverage(image, BACKGROUND).foreground_pixels, 1);
  assert.equal(measureCoverage(image, BACKGROUND, 0, { x: 0, y: 0, width: 3, height: 4 }).foreground_pixels, 0);
});

test("temporal comparison selects the worst adjacent pair deterministically", () => {
  const first = solidImage(100, 100, BACKGROUND);
  const second = cloneImage(first);
  const third = cloneImage(second);
  second.data[0] += 1;
  third.data[0] += 9;
  const result = compareTemporalSequence([
    { id: "first", image: first },
    { id: "second", image: second },
    { id: "third", image: third },
  ], {
    toleranceProfile: BOUNDED,
    backgroundRgba: BACKGROUND,
  });
  assert.equal(result.frame_count, 3);
  assert.equal(result.worst_pair_index, 1);
  assert.equal(result.worst_pair.from_id, "second");
  assert.equal(result.worst_pair.to_id, "third");
  assert.equal(result.passed, false);
  assert.deepEqual(
    summarizeTemporalPairs(3, result.pairs),
    result,
  );
});

test("difference image is deterministic, bounded, and supports two-frame storage reuse", () => {
  const reference = solidImage(2, 1, [10, 20, 30, 255]);
  const candidate = cloneImage(reference);
  candidate.data.set([12, 15, 30, 250], 0);
  candidate.data.set([10, 20, 255, 255], 4);

  const expected = Uint8Array.of(5, 5, 5, 255, 225, 225, 225, 255);
  const difference = createDifferenceImage(reference, candidate);
  assert.deepEqual(difference.data, expected);
  assert.deepEqual(reference.data, Uint8Array.of(10, 20, 30, 255, 10, 20, 30, 255));

  const reused = cloneImage(reference);
  assert.equal(writeDifferenceImage(reused, reused, candidate), reused);
  assert.deepEqual(reused.data, expected);
  assert.throws(
    () => createDifferenceImage(reference, solidImage(3, 1, BACKGROUND)),
    /dimensions differ/,
  );
});

test("difference artifact survives deterministic filter-0 PNG encoding", async () => {
  const reference = solidImage(3, 2, [5, 10, 15, 255]);
  const candidate = cloneImage(reference);
  setPixel(candidate, 2, 1, [9, 30, 15, 250]);
  const difference = createDifferenceImage(reference, candidate);
  const encoded = await encodeRgba8Png(difference);
  assert.deepEqual(await decodeRgba8Png(encoded), difference);
});

test("tolerance validation rejects missing, negative, and relaxed hard caps", () => {
  assert.throws(() => validateToleranceProfile({}), /channel_threshold/);
  assert.throws(
    () => validateToleranceProfile({ ...BOUNDED, unstable_pixel_fraction: -1 }),
    /nonnegative/,
  );
  assert.throws(
    () => validateToleranceProfile({ ...BOUNDED, maximum_channel_delta: 5 }),
    /hard cap 4/,
  );
  assert.throws(
    () => validateToleranceProfile({ ...BOUNDED, feature_centroid_distance_pixels: 1.1 }),
    /hard cap 1/,
  );
});

function solidImage(width, height, rgba) {
  const data = new Uint8Array(width * height * 4);
  for (let offset = 0; offset < data.length; offset += 4) data.set(rgba, offset);
  return { width, height, data };
}

function cloneImage(image) {
  return { width: image.width, height: image.height, data: image.data.slice() };
}

function setPixel(image, x, y, rgba) {
  image.data.set(rgba, (y * image.width + x) * 4);
}
