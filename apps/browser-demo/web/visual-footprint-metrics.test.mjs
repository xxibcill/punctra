import assert from "node:assert/strict";
import test from "node:test";

import {
  IDEAL_DISK_SAMPLES_PER_AXIS,
  POINT_FOOTPRINT_METRICS_SCHEMA,
  REGION_TOPOLOGY_METRICS_SCHEMA,
  createIdealDiskCoverage,
  measurePointFootprint,
  measureRegionTopology,
} from "./visual-footprint-metrics.js";

const BLACK = Object.freeze([0, 0, 0, 255]);
const WHITE = Object.freeze([255, 255, 255, 255]);
const RECTANGLE = Object.freeze({ x: 0, y: 0, width: 12, height: 12 });
const CENTER = Object.freeze([6, 6]);
const RADIUS = 3.25;

test("16x16 supersampling produces deterministic bounded ideal disk coverage", () => {
  const first = createIdealDiskCoverage({
    rectangle: RECTANGLE,
    center: CENTER,
    radiusPixels: RADIUS,
  });
  const second = createIdealDiskCoverage({
    rectangle: RECTANGLE,
    center: CENTER,
    radiusPixels: RADIUS,
  });

  assert.equal(first.samples_per_axis, IDEAL_DISK_SAMPLES_PER_AXIS);
  assert.equal(first.samples_per_pixel, 256);
  assert.deepEqual(first.rectangle, RECTANGLE);
  assert.deepEqual(first.center, CENTER);
  assert.deepEqual(first.data, second.data);
  assert(first.data.some((coverage) => coverage > 0 && coverage < 1));
  assert(first.data.every((coverage) => Number.isInteger(coverage * 256)));
});

test("ideal antialiasing closely matches its analytic disk and retains partial edges", () => {
  const ideal = idealDisk();
  const report = footprintReport(imageFromCoverage(ideal.data));

  assert.equal(report.schema, POINT_FOOTPRINT_METRICS_SCHEMA);
  assert(report.coverage.mean_absolute_error < 0.002);
  assert(report.coverage.root_mean_square_error < 0.002);
  assert.equal(report.coverage.partial_edge_pixels, report.coverage.ideal_partial_edge_pixels);
  assert(report.coverage.partial_edge_pixels > 0);
  assert(report.centroid.error_pixels < 0.01);
  assert(report.radial.error_pixels < 0.01);
  assert(report.aspect.error < 0.01);
  assert.equal(report.corner_leakage.pixel_count, 0);
  assert.equal(report.corner_leakage.coverage, 0);
  assert.equal(report.corner_leakage.outer_pixel_count, 0);
  assert.equal(report.corner_leakage.outer_coverage, 0);
  assert.deepEqual(report.corner_leakage.exact_distance_outer, {
    definition: "decoded_pixel_center_distance_greater_than_declared_radius_plus_margin",
    sample_location: "decoded_pixel_center",
    margin_physical_pixels: 0.75,
    pixel_count: 0,
    coverage: 0,
  });
});

test("a hard disk loses partial coverage and has worse coverage error", () => {
  const idealReport = footprintReport(imageFromCoverage(idealDisk().data));
  const hardCoverage = coverageGrid(({ x, y }) => (
    Math.hypot(x - CENTER[0], y - CENTER[1]) <= RADIUS ? 1 : 0
  ));
  const hardReport = footprintReport(imageFromCoverage(hardCoverage));

  assert.equal(hardReport.coverage.partial_edge_pixels, 0);
  assert(hardReport.coverage.mean_absolute_error > idealReport.coverage.mean_absolute_error * 10);
  assert(hardReport.coverage.root_mean_square_error > idealReport.coverage.root_mean_square_error * 10);
});

test("a square footprint exposes coverage outside the ideal disk corners", () => {
  const squareCoverage = coverageGrid(({ x, y }) => (
    Math.abs(x - CENTER[0]) <= RADIUS && Math.abs(y - CENTER[1]) <= RADIUS ? 1 : 0
  ));
  const report = footprintReport(imageFromCoverage(squareCoverage));

  assert(report.corner_leakage.pixel_count > 0);
  assert(report.corner_leakage.coverage > 0);
  assert(report.corner_leakage.fraction_of_observed_coverage > 0);
  assert(report.coverage.mean_absolute_error > 0.04);
});

test("a translated disk reports centroid and radial drift from the declared center", () => {
  const shifted = createIdealDiskCoverage({
    rectangle: RECTANGLE,
    center: [CENTER[0] + 1, CENTER[1]],
    radiusPixels: RADIUS,
  });
  const report = footprintReport(imageFromCoverage(shifted.data));

  assert(report.centroid.error_pixels > 0.95);
  assert(report.radial.error_pixels > 0.05);
  assert(report.corner_leakage.pixel_count > 0);
});

test("a stretched footprint reports principal-axis aspect distortion", () => {
  const stretchedCoverage = coverageGrid(({ x, y }) => (
    ((x - CENTER[0]) / 4) ** 2 + ((y - CENTER[1]) / 2) ** 2 <= 1 ? 1 : 0
  ));
  const report = footprintReport(imageFromCoverage(stretchedCoverage));

  assert(report.aspect.observed_ratio > 1.5);
  assert(report.aspect.error > 0.5);
});

test("solid blobs and enclosed holes have exact 2x2 and component facts", () => {
  const blob = binaryImage(5, 5, () => true);
  const blobReport = topologyReport(blob);
  assert.equal(blobReport.schema, REGION_TOPOLOGY_METRICS_SCHEMA);
  assert.equal(blobReport.foreground_fraction, 1);
  assert.equal(blobReport.solid_2x2_blocks, 16);
  assert.equal(blobReport.foreground.component_count, 1);
  assert.equal(blobReport.foreground.left_right_bridge_components, 1);
  assert.equal(blobReport.foreground.top_bottom_bridge_components, 1);
  assert.equal(blobReport.background.component_count, 0);

  const hole = binaryImage(5, 5, (x, y) => x !== 2 || y !== 2);
  const holeReport = topologyReport(hole);
  assert.equal(holeReport.foreground_fraction, 24 / 25);
  assert.equal(holeReport.foreground.component_count, 1);
  assert.equal(holeReport.background.component_count, 1);
  assert.equal(holeReport.background.interior_component_count, 1);
  assert.equal(holeReport.background.largest_interior_component_pixels, 1);
});

test("a one-pixel thin bridge is distinguished from the same feature with a break", () => {
  const continuous = binaryImage(7, 3, (_x, y) => y === 1);
  const continuousReport = topologyReport(continuous);
  assert.equal(continuousReport.solid_2x2_blocks, 0);
  assert.equal(continuousReport.foreground.component_count, 1);
  assert.equal(continuousReport.foreground.left_right_bridge_components, 1);
  assert.equal(continuousReport.background.component_count, 2);
  assert.equal(continuousReport.background.top_bottom_bridge_components, 0);

  const broken = binaryImage(7, 3, (x, y) => y === 1 && x !== 3);
  const brokenReport = topologyReport(broken);
  assert.equal(brokenReport.foreground.component_count, 2);
  assert.equal(brokenReport.foreground.left_right_bridge_components, 0);
  assert.equal(brokenReport.background.component_count, 1);
  assert.equal(brokenReport.background.top_bottom_bridge_components, 1);
});

test("bounded image, region, color, disk, and threshold validation rejects tampering", () => {
  const image = solidImage(4, 4, BLACK);
  const validOptions = {
    rectangle: { x: 0, y: 0, width: 4, height: 4 },
    center: [2, 2],
    radiusPixels: 1,
    foregroundRgba: WHITE,
    backgroundRgba: BLACK,
  };

  assert.throws(
    () => measurePointFootprint({ ...image, data: image.data.slice(1) }, validOptions),
    /RGBA byte length differs/,
  );
  assert.throws(
    () => measurePointFootprint({ width: 4097, height: 1, data: new Uint8Array() }, validOptions),
    /width is invalid/,
  );
  assert.throws(
    () => measurePointFootprint({ width: 4, height: 4, data: [] }, validOptions),
    /Uint8 byte array/,
  );
  assert.throws(
    () => measurePointFootprint(image, { ...validOptions, rectangle: { x: 3, y: 0, width: 2, height: 1 } }),
    /rectangle exceeds the image/,
  );
  assert.throws(
    () => measurePointFootprint(image, { ...validOptions, center: [Number.NaN, 2] }),
    /center/,
  );
  assert.throws(
    () => measurePointFootprint(image, { ...validOptions, radiusPixels: 0 }),
    /radius/,
  );
  assert.throws(
    () => measurePointFootprint(image, { ...validOptions, foregroundRgba: BLACK }),
    /colors must differ/,
  );
  assert.throws(
    () => measureRegionTopology(image, { ...validOptions, foregroundThreshold: 0 }),
    /threshold/,
  );
  assert.throws(
    () => createIdealDiskCoverage({
      rectangle: { x: 0, y: 0, width: 257, height: 256 },
      center: CENTER,
      radiusPixels: RADIUS,
    }),
    /area exceeds/,
  );
});

test("one adversarial edge-pixel mutation changes derived metrics", () => {
  const ideal = idealDisk();
  const image = imageFromCoverage(ideal.data);
  const control = footprintReport(image);
  const mutated = cloneImage(image);
  setPixel(mutated, 2, 2, WHITE);
  const changed = footprintReport(mutated);

  assert(changed.coverage.mean_absolute_error > control.coverage.mean_absolute_error);
  assert(changed.corner_leakage.pixel_count > control.corner_leakage.pixel_count);
  assert(changed.corner_leakage.coverage > control.corner_leakage.coverage);
  assert(changed.corner_leakage.outer_pixel_count > control.corner_leakage.outer_pixel_count);
  assert(changed.corner_leakage.outer_coverage > control.corner_leakage.outer_coverage);
  assert.equal(changed.corner_leakage.exact_distance_outer.pixel_count, 1);
  assert.equal(changed.corner_leakage.exact_distance_outer.coverage, 1);
});

test("exact-distance leakage uses decoded pixel centers and the 0.75-pixel margin", () => {
  const image = imageFromCoverage(idealDisk().data);
  const insideMargin = cloneImage(image);
  const beyondMargin = cloneImage(image);
  setPixel(insideMargin, 2, 4, WHITE);
  setPixel(beyondMargin, 1, 4, WHITE);

  const insideReport = footprintReport(insideMargin);
  const beyondReport = footprintReport(beyondMargin);
  assert.equal(insideReport.corner_leakage.exact_distance_outer.pixel_count, 0);
  assert.equal(beyondReport.corner_leakage.exact_distance_outer.pixel_count, 1);
  assert.equal(beyondReport.corner_leakage.exact_distance_outer.coverage, 1);
});

function idealDisk() {
  return createIdealDiskCoverage({
    rectangle: RECTANGLE,
    center: CENTER,
    radiusPixels: RADIUS,
  });
}

function footprintReport(image) {
  return measurePointFootprint(image, {
    rectangle: RECTANGLE,
    center: CENTER,
    radiusPixels: RADIUS,
    foregroundRgba: WHITE,
    backgroundRgba: BLACK,
  });
}

function topologyReport(image) {
  return measureRegionTopology(image, {
    rectangle: { x: 0, y: 0, width: image.width, height: image.height },
    foregroundRgba: WHITE,
    backgroundRgba: BLACK,
    foregroundThreshold: 0.5,
  });
}

function coverageGrid(coverageAt) {
  const coverage = new Float64Array(RECTANGLE.width * RECTANGLE.height);
  for (let y = 0; y < RECTANGLE.height; y += 1) {
    for (let x = 0; x < RECTANGLE.width; x += 1) {
      coverage[y * RECTANGLE.width + x] = coverageAt({ x: x + 0.5, y: y + 0.5 });
    }
  }
  return coverage;
}

function imageFromCoverage(coverage) {
  const image = solidImage(RECTANGLE.width, RECTANGLE.height, BLACK);
  for (let index = 0; index < coverage.length; index += 1) {
    const value = Math.round(coverage[index] * 255);
    const offset = index * 4;
    image.data[offset] = value;
    image.data[offset + 1] = value;
    image.data[offset + 2] = value;
  }
  return image;
}

function binaryImage(width, height, isForeground) {
  const image = solidImage(width, height, BLACK);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      if (isForeground(x, y)) setPixel(image, x, y, WHITE);
    }
  }
  return image;
}

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
