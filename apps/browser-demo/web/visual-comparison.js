import { createVisualValidator } from "./visual-validation.js";

export const IMAGE_COMPARISON_SCHEMA = "punctra-canonical-image-comparison-v1";
export const TEMPORAL_COMPARISON_SCHEMA = "punctra-temporal-image-comparison-v1";
export const DIFFERENCE_IMAGE_POLICY = "maximum-absolute-rgba-channel-delta-as-opaque-grayscale-v1";
export const HARD_TOLERANCE_CAPS = Object.freeze({
  channel_threshold: 2,
  maximum_channel_delta: 4,
  mean_channel_delta: 2,
  rms_channel_delta: 2,
  p95_channel_delta: 2,
  unstable_pixel_fraction: 0.001,
  coverage_fraction_delta: 0.001,
  feature_occupancy_fraction_delta: 0.005,
  feature_centroid_distance_pixels: 1,
});
const { requireCondition, requireRecord } = createVisualValidator("Visual comparison invalid");

/** Validates a complete tolerance profile against the non-relaxable v0.21 caps. */
export function validateToleranceProfile(profile) {
  requireRecord(profile, "tolerance profile");
  const integerFields = ["channel_threshold", "maximum_channel_delta", "p95_channel_delta"];
  const fractionFields = [
    "unstable_pixel_fraction",
    "coverage_fraction_delta",
    "feature_occupancy_fraction_delta",
  ];
  for (const [field, cap] of Object.entries(HARD_TOLERANCE_CAPS)) {
    const value = profile[field];
    requireCondition(Number.isFinite(value) && value >= 0, `tolerance ${field} must be finite and nonnegative`);
    requireCondition(value <= cap, `tolerance ${field} exceeds the hard cap ${cap}`);
    if (integerFields.includes(field)) requireCondition(Number.isInteger(value), `tolerance ${field} must be an integer`);
    if (fractionFields.includes(field)) requireCondition(value <= 1, `tolerance ${field} must be a fraction`);
  }
  requireCondition(profile.channel_threshold <= profile.maximum_channel_delta, "channel threshold exceeds maximum channel delta");
  requireCondition(profile.mean_channel_delta <= profile.maximum_channel_delta, "mean channel delta exceeds maximum channel delta");
  requireCondition(profile.rms_channel_delta <= profile.maximum_channel_delta, "RMS channel delta exceeds maximum channel delta");
  requireCondition(profile.p95_channel_delta <= profile.maximum_channel_delta, "p95 channel delta exceeds maximum channel delta");
  return Object.freeze({
    channel_threshold: profile.channel_threshold,
    maximum_channel_delta: profile.maximum_channel_delta,
    mean_channel_delta: profile.mean_channel_delta,
    rms_channel_delta: profile.rms_channel_delta,
    p95_channel_delta: profile.p95_channel_delta,
    unstable_pixel_fraction: profile.unstable_pixel_fraction,
    coverage_fraction_delta: profile.coverage_fraction_delta,
    feature_occupancy_fraction_delta: profile.feature_occupancy_fraction_delta,
    feature_centroid_distance_pixels: profile.feature_centroid_distance_pixels,
  });
}

/**
 * Compares two tight top-left RGBA8 images. Every aggregate remains subordinate
 * to the independent maximum, unstable-pixel, Coverage, and feature gates.
 */
export function compareCanonicalImages(reference, candidate, options) {
  const referenceImage = validateImage(reference, "reference image");
  const candidateImage = validateImage(candidate, "candidate image");
  const tolerance = validateToleranceProfile(options?.toleranceProfile);
  const features = validateFeatures(options?.features ?? [], referenceImage.width, referenceImage.height);
  const background = validateRgba(options?.backgroundRgba, "background RGBA");
  const dimensions = {
    match: referenceImage.width === candidateImage.width && referenceImage.height === candidateImage.height,
    reference: { width: referenceImage.width, height: referenceImage.height },
    candidate: { width: candidateImage.width, height: candidateImage.height },
    pixel_count: null,
    channel_count: null,
  };
  if (!dimensions.match) {
    return {
      schema: IMAGE_COMPARISON_SCHEMA,
      passed: false,
      dimensions,
      tolerance_profile: tolerance,
      background_rgba: background,
      pixels: null,
      channels: null,
      difference_regions: null,
      coverage: null,
      features: [],
      failures: ["dimension_mismatch"],
    };
  }

  const pixelCount = referenceImage.width * referenceImage.height;
  const channelCount = pixelCount * 4;
  dimensions.pixel_count = pixelCount;
  dimensions.channel_count = channelCount;
  const histogram = new Uint32Array(256);
  let exactPixels = 0;
  let unstablePixels = 0;
  let channelSum = 0;
  let channelSquareSum = 0;
  let maximumChannelDelta = 0;
  const unstableBounds = emptyBounds();
  const backgroundDifference = createDifferenceAccumulator();
  const foregroundDifference = createDifferenceAccumulator();

  for (let pixel = 0; pixel < pixelCount; pixel += 1) {
    const offset = pixel * 4;
    const regionDifference = pixelMatchesRgba(referenceImage.data, offset, background)
      ? backgroundDifference
      : foregroundDifference;
    regionDifference.pixel_count += 1;
    let pixelMaximum = 0;
    let exact = true;
    for (let channel = 0; channel < 4; channel += 1) {
      const delta = Math.abs(referenceImage.data[offset + channel] - candidateImage.data[offset + channel]);
      histogram[delta] += 1;
      channelSum += delta;
      channelSquareSum += delta * delta;
      maximumChannelDelta = Math.max(maximumChannelDelta, delta);
      accumulateDifferenceChannel(regionDifference, delta);
      pixelMaximum = Math.max(pixelMaximum, delta);
      exact &&= delta === 0;
    }
    if (exact) exactPixels += 1;
    if (exact) regionDifference.exact_pixels += 1;
    if (pixelMaximum > tolerance.channel_threshold) {
      unstablePixels += 1;
      regionDifference.unstable_pixels += 1;
      includePixel(unstableBounds, pixel % referenceImage.width, Math.floor(pixel / referenceImage.width));
    }
  }

  const unstableFraction = unstablePixels / pixelCount;
  const meanChannelDelta = channelSum / channelCount;
  const rmsChannelDelta = Math.sqrt(channelSquareSum / channelCount);
  const p95ChannelDelta = histogramPercentile(histogram, channelCount, 95);
  const referenceCoverage = measureCoverage(referenceImage, background, tolerance.channel_threshold);
  const candidateCoverage = measureCoverage(candidateImage, background, tolerance.channel_threshold);
  const coverageFractionDelta = Math.abs(referenceCoverage.fraction - candidateCoverage.fraction);
  const featureReports = features.map((feature) => compareFeature(
    feature,
    referenceImage,
    candidateImage,
    background,
    tolerance,
  ));
  const failures = [];
  if (maximumChannelDelta > tolerance.maximum_channel_delta) failures.push("maximum_channel_delta");
  if (meanChannelDelta > tolerance.mean_channel_delta) failures.push("mean_channel_delta");
  if (rmsChannelDelta > tolerance.rms_channel_delta) failures.push("rms_channel_delta");
  if (p95ChannelDelta > tolerance.p95_channel_delta) failures.push("p95_channel_delta");
  if (unstableFraction > tolerance.unstable_pixel_fraction) failures.push("unstable_pixel_fraction");
  if (coverageFractionDelta > tolerance.coverage_fraction_delta) failures.push("coverage_fraction_delta");
  for (const feature of featureReports) {
    for (const failure of feature.failures) failures.push(`feature:${feature.id}:${failure}`);
  }
  return {
    schema: IMAGE_COMPARISON_SCHEMA,
    passed: failures.length === 0,
    dimensions,
    tolerance_profile: tolerance,
    background_rgba: background,
    pixels: {
      exact: exactPixels,
      unstable: unstablePixels,
      unstable_fraction: unstableFraction,
      unstable_bbox: finishBounds(unstableBounds),
    },
    channels: {
      maximum_absolute_delta: maximumChannelDelta,
      mean_absolute_delta: meanChannelDelta,
      rms_absolute_delta: rmsChannelDelta,
      p95_absolute_delta: p95ChannelDelta,
    },
    difference_regions: {
      mask: {
        kind: "reference_exact_background_rgba8-v1",
        background_rgba: background,
      },
      background: finishDifferenceAccumulator(backgroundDifference),
      foreground: finishDifferenceAccumulator(foregroundDifference),
    },
    coverage: {
      reference: referenceCoverage,
      candidate: candidateCoverage,
      foreground_fraction_delta: coverageFractionDelta,
    },
    features: featureReports,
    failures,
  };
}

/** Measures foreground pixels, bounds, and centroid against one explicit color. */
export function measureCoverage(image, backgroundRgba, channelThreshold = 0, rectangle) {
  const validated = validateImage(image, "coverage image");
  const background = validateRgba(backgroundRgba, "background RGBA");
  requireCondition(Number.isInteger(channelThreshold) && channelThreshold >= 0 && channelThreshold <= 255, "Coverage channel threshold is invalid");
  const region = rectangle === undefined
    ? { x: 0, y: 0, width: validated.width, height: validated.height }
    : validateRectangle(rectangle, validated.width, validated.height, "Coverage rectangle");
  let foregroundPixels = 0;
  let xSum = 0;
  let ySum = 0;
  const bounds = emptyBounds();
  for (let y = region.y; y < region.y + region.height; y += 1) {
    for (let x = region.x; x < region.x + region.width; x += 1) {
      const offset = (y * validated.width + x) * 4;
      let foreground = false;
      for (let channel = 0; channel < 4; channel += 1) {
        foreground ||= Math.abs(validated.data[offset + channel] - background[channel]) > channelThreshold;
      }
      if (foreground) {
        foregroundPixels += 1;
        xSum += x;
        ySum += y;
        includePixel(bounds, x, y);
      }
    }
  }
  const area = region.width * region.height;
  return {
    rectangle: region,
    foreground_pixels: foregroundPixels,
    fraction: foregroundPixels / area,
    bbox: finishBounds(bounds),
    centroid: foregroundPixels === 0 ? null : { x: xSum / foregroundPixels, y: ySum / foregroundPixels },
  };
}

/** Derives a deterministic opaque grayscale visualization of per-pixel change. */
export function createDifferenceImage(reference, candidate) {
  const referenceImage = validateMatchingImages(reference, candidate);
  const output = {
    width: referenceImage.width,
    height: referenceImage.height,
    data: new Uint8Array(referenceImage.data.byteLength),
  };
  return writeDifferenceImage(output, reference, candidate);
}

/** Writes the same deterministic difference into caller-owned bounded storage. */
export function writeDifferenceImage(output, reference, candidate) {
  const referenceImage = validateMatchingImages(reference, candidate);
  const outputImage = validateImage(output, "difference output image");
  requireCondition(
    outputImage.width === referenceImage.width && outputImage.height === referenceImage.height,
    "difference output dimensions differ",
  );
  const candidateImage = validateImage(candidate, "difference candidate image");
  for (let offset = 0; offset < referenceImage.data.byteLength; offset += 4) {
    const maximumDelta = Math.max(
      Math.abs(referenceImage.data[offset] - candidateImage.data[offset]),
      Math.abs(referenceImage.data[offset + 1] - candidateImage.data[offset + 1]),
      Math.abs(referenceImage.data[offset + 2] - candidateImage.data[offset + 2]),
      Math.abs(referenceImage.data[offset + 3] - candidateImage.data[offset + 3]),
    );
    outputImage.data[offset] = maximumDelta;
    outputImage.data[offset + 1] = maximumDelta;
    outputImage.data[offset + 2] = maximumDelta;
    outputImage.data[offset + 3] = 255;
  }
  return outputImage;
}

/** Compares adjacent temporal frames and selects the independently worst pair. */
export function compareTemporalSequence(frames, options) {
  requireCondition(Array.isArray(frames) && frames.length >= 2, "temporal sequence requires at least two frames");
  const pairs = [];
  for (let index = 1; index < frames.length; index += 1) {
    const previous = normalizeTemporalFrame(frames[index - 1], index - 1);
    const current = normalizeTemporalFrame(frames[index], index);
    pairs.push({
      from_index: index - 1,
      to_index: index,
      from_id: previous.id,
      to_id: current.id,
      comparison: compareCanonicalImages(previous.image, current.image, options),
    });
  }
  return summarizeTemporalPairs(frames.length, pairs);
}

/** Selects a worst pair from already-streamed pair results without retaining frames. */
export function summarizeTemporalPairs(frameCount, pairs) {
  requireCondition(Number.isInteger(frameCount) && frameCount >= 2, "temporal frame count is invalid");
  requireCondition(Array.isArray(pairs) && pairs.length === frameCount - 1, "temporal pair count differs");
  let worstPairIndex = 0;
  for (let index = 0; index < pairs.length; index += 1) {
    validateTemporalPair(pairs[index], index);
    if (temporalRank(pairs[index].comparison) > temporalRank(pairs[worstPairIndex].comparison)) {
      worstPairIndex = index;
    }
  }
  return {
    schema: TEMPORAL_COMPARISON_SCHEMA,
    passed: pairs.every((pair) => pair.comparison.passed),
    frame_count: frameCount,
    pair_count: pairs.length,
    worst_pair_index: worstPairIndex,
    worst_pair: pairs[worstPairIndex],
    pairs,
    failures: pairs.flatMap((pair, index) => pair.comparison.failures.map((failure) => `pair:${index}:${failure}`)),
  };
}

function compareFeature(feature, reference, candidate, background, tolerance) {
  const referenceCoverage = measureCoverage(reference, background, tolerance.channel_threshold, feature.rectangle);
  const candidateCoverage = measureCoverage(candidate, background, tolerance.channel_threshold, feature.rectangle);
  const occupancyFractionDelta = Math.abs(referenceCoverage.fraction - candidateCoverage.fraction);
  const centroidDistance = centroidDistancePixels(referenceCoverage.centroid, candidateCoverage.centroid);
  const failures = [];
  if (referenceCoverage.foreground_pixels < feature.minimum_foreground_pixels) failures.push("reference_occupancy");
  if (candidateCoverage.foreground_pixels < feature.minimum_foreground_pixels) failures.push("candidate_occupancy");
  if (occupancyFractionDelta > tolerance.feature_occupancy_fraction_delta) failures.push("occupancy_fraction_delta");
  if (centroidDistance === null || centroidDistance > tolerance.feature_centroid_distance_pixels) failures.push("centroid_distance");
  return {
    id: feature.id,
    rectangle: feature.rectangle,
    minimum_foreground_pixels: feature.minimum_foreground_pixels,
    reference: referenceCoverage,
    candidate: candidateCoverage,
    occupancy_fraction_delta: occupancyFractionDelta,
    centroid_distance_pixels: centroidDistance,
    passed: failures.length === 0,
    failures,
  };
}

function validateFeatures(features, width, height) {
  requireCondition(Array.isArray(features), "features must be an array");
  const ids = new Set();
  return features.map((feature) => {
    requireRecord(feature, "feature");
    requireCondition(typeof feature.id === "string" && feature.id.length > 0 && !ids.has(feature.id), "feature identity is invalid or duplicated");
    ids.add(feature.id);
    requireCondition(Number.isInteger(feature.minimum_foreground_pixels) && feature.minimum_foreground_pixels > 0, `feature ${feature.id} minimum foreground differs`);
    const rectangle = validateRectangle(feature.rectangle, width, height, `feature ${feature.id}`);
    requireCondition(feature.minimum_foreground_pixels <= rectangle.width * rectangle.height, `feature ${feature.id} minimum foreground exceeds its area`);
    return { id: feature.id, rectangle, minimum_foreground_pixels: feature.minimum_foreground_pixels };
  });
}

function validateImage(image, label) {
  requireRecord(image, label);
  requireCondition(Number.isInteger(image.width) && image.width > 0 && image.width <= 4_096, `${label} width is invalid`);
  requireCondition(Number.isInteger(image.height) && image.height > 0 && image.height <= 4_096, `${label} height is invalid`);
  requireCondition(image.width * image.height <= 8_388_608, `${label} area exceeds the bound`);
  requireCondition(image.data instanceof Uint8Array, `${label} data must be Uint8Array`);
  requireCondition(image.data.byteLength === image.width * image.height * 4, `${label} RGBA8 length differs`);
  return image;
}

function validateMatchingImages(reference, candidate) {
  const referenceImage = validateImage(reference, "difference reference image");
  const candidateImage = validateImage(candidate, "difference candidate image");
  requireCondition(
    referenceImage.width === candidateImage.width && referenceImage.height === candidateImage.height,
    "difference image dimensions differ",
  );
  return referenceImage;
}

function validateRgba(value, label) {
  requireCondition(Array.isArray(value) && value.length === 4 && value.every((channel) => Number.isInteger(channel) && channel >= 0 && channel <= 255), `${label} must contain four U8 channels`);
  return [...value];
}

function validateRectangle(rectangle, width, height, label) {
  requireRecord(rectangle, label);
  const fields = [rectangle.x, rectangle.y, rectangle.width, rectangle.height];
  requireCondition(fields.every(Number.isInteger), `${label} rectangle must contain integers`);
  requireCondition(rectangle.x >= 0 && rectangle.y >= 0 && rectangle.width > 0 && rectangle.height > 0, `${label} rectangle must be positive`);
  requireCondition(rectangle.x + rectangle.width <= width && rectangle.y + rectangle.height <= height, `${label} rectangle exceeds the image`);
  return { x: rectangle.x, y: rectangle.y, width: rectangle.width, height: rectangle.height };
}

function normalizeTemporalFrame(value, index) {
  if (value?.image !== undefined) return { id: value.id ?? `frame-${index}`, image: value.image };
  return { id: `frame-${index}`, image: value };
}

function validateTemporalPair(pair, index) {
  requireRecord(pair, `temporal pair ${index}`);
  requireCondition(pair.comparison?.schema === IMAGE_COMPARISON_SCHEMA, `temporal pair ${index} comparison differs`);
}

function temporalRank(comparison) {
  if (comparison.channels === null) return Number.POSITIVE_INFINITY;
  return comparison.channels.maximum_absolute_delta * 1e12
    + comparison.pixels.unstable_fraction * 1e9
    + comparison.channels.rms_absolute_delta * 1e6
    + comparison.channels.mean_absolute_delta;
}

function histogramPercentile(histogram, count, percentile) {
  const target = Math.ceil(count * percentile / 100);
  let cumulative = 0;
  for (let value = 0; value < histogram.length; value += 1) {
    cumulative += histogram[value];
    if (cumulative >= target) return value;
  }
  return histogram.length - 1;
}

function createDifferenceAccumulator() {
  return {
    pixel_count: 0,
    exact_pixels: 0,
    unstable_pixels: 0,
    channel_sum: 0,
    channel_square_sum: 0,
    maximum_channel_delta: 0,
    histogram: new Uint32Array(256),
  };
}

function accumulateDifferenceChannel(accumulator, delta) {
  accumulator.channel_sum += delta;
  accumulator.channel_square_sum += delta * delta;
  accumulator.maximum_channel_delta = Math.max(accumulator.maximum_channel_delta, delta);
  accumulator.histogram[delta] += 1;
}

function finishDifferenceAccumulator(accumulator) {
  const channelCount = accumulator.pixel_count * 4;
  return {
    pixel_count: accumulator.pixel_count,
    channel_count: channelCount,
    exact_pixels: accumulator.exact_pixels,
    unstable_pixels: accumulator.unstable_pixels,
    unstable_pixel_fraction: accumulator.pixel_count === 0
      ? 0
      : accumulator.unstable_pixels / accumulator.pixel_count,
    channels: {
      maximum_absolute_delta: accumulator.maximum_channel_delta,
      mean_absolute_delta: channelCount === 0 ? 0 : accumulator.channel_sum / channelCount,
      rms_absolute_delta: channelCount === 0 ? 0 : Math.sqrt(accumulator.channel_square_sum / channelCount),
      p95_absolute_delta: channelCount === 0 ? 0 : histogramPercentile(accumulator.histogram, channelCount, 95),
    },
  };
}

function pixelMatchesRgba(data, offset, rgba) {
  return data[offset] === rgba[0]
    && data[offset + 1] === rgba[1]
    && data[offset + 2] === rgba[2]
    && data[offset + 3] === rgba[3];
}

function centroidDistancePixels(left, right) {
  if (left === null || right === null) return null;
  return Math.hypot(left.x - right.x, left.y - right.y);
}

function emptyBounds() {
  return { minimumX: Number.POSITIVE_INFINITY, minimumY: Number.POSITIVE_INFINITY, maximumX: -1, maximumY: -1 };
}

function includePixel(bounds, x, y) {
  bounds.minimumX = Math.min(bounds.minimumX, x);
  bounds.minimumY = Math.min(bounds.minimumY, y);
  bounds.maximumX = Math.max(bounds.maximumX, x);
  bounds.maximumY = Math.max(bounds.maximumY, y);
}

function finishBounds(bounds) {
  if (bounds.maximumX < 0) return null;
  return {
    x: bounds.minimumX,
    y: bounds.minimumY,
    width: bounds.maximumX - bounds.minimumX + 1,
    height: bounds.maximumY - bounds.minimumY + 1,
  };
}
