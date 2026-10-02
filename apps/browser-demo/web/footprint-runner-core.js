import {
  measureForegroundComponentBridges,
  measurePointFootprint,
  measureRegionTopology,
} from "./visual-footprint-metrics.js";
import { createVisualValidator } from "./visual-validation.js";

export const OCCUPANCY_NORMALIZATION = "maximum_absolute_rgba8_channel_delta_from_clear_color_v1";
const { requireCondition, requireRecord } = createVisualValidator("Point-footprint runner failed");

export function createOccupancyImage(image, backgroundRgba, channelThreshold = 2) {
  validateImage(image);
  validateRgba(backgroundRgba, "background RGBA");
  requireCondition(
    Number.isSafeInteger(channelThreshold) && channelThreshold >= 0 && channelThreshold <= 255,
    "occupancy channel threshold is invalid",
  );
  const data = new Uint8Array(image.data.byteLength);
  for (let offset = 0; offset < image.data.byteLength; offset += 4) {
    let maximumDelta = 0;
    for (let channel = 0; channel < 4; channel += 1) {
      maximumDelta = Math.max(maximumDelta, Math.abs(image.data[offset + channel] - backgroundRgba[channel]));
    }
    const value = maximumDelta > channelThreshold ? 255 : 0;
    data[offset] = value;
    data[offset + 1] = value;
    data[offset + 2] = value;
    data[offset + 3] = 255;
  }
  return { width: image.width, height: image.height, data };
}

export function measureOccupancyTopology(image, options) {
  requireRecord(options, "occupancy topology options");
  const occupancy = createOccupancyImage(
    image,
    options.backgroundRgba,
    options.channelThreshold ?? 2,
  );
  return {
    occupancy_normalization: OCCUPANCY_NORMALIZATION,
    channel_threshold: options.channelThreshold ?? 2,
    metrics: measureRegionTopology(occupancy, {
      rectangle: options.rectangle,
      foregroundRgba: [255, 255, 255, 255],
      backgroundRgba: [0, 0, 0, 255],
      foregroundThreshold: 0.5,
    }),
  };
}

export function measureOccupancyComponentBridges(predecessor, candidate, options) {
  requireRecord(options, "occupancy component-bridge options");
  const channelThreshold = options.channelThreshold ?? 2;
  const predecessorOccupancy = createOccupancyImage(
    predecessor,
    options.backgroundRgba,
    channelThreshold,
  );
  const candidateOccupancy = createOccupancyImage(
    candidate,
    options.backgroundRgba,
    channelThreshold,
  );
  return {
    occupancy_normalization: OCCUPANCY_NORMALIZATION,
    channel_threshold: channelThreshold,
    metrics: measureForegroundComponentBridges(predecessorOccupancy, candidateOccupancy, {
      rectangle: options.rectangle,
      foregroundRgba: [255, 255, 255, 255],
      backgroundRgba: [0, 0, 0, 255],
      foregroundThreshold: 0.5,
      minimumClearSeparationPixels: options.minimumClearSeparationPixels,
    }),
  };
}

export function measureIsolatedFootprint(image, options) {
  requireRecord(options, "isolated footprint options");
  const foregroundRgba = sampleForegroundRgba(
    image,
    options.center,
    options.backgroundRgba,
  );
  const rectangle = footprintRectangle(
    options.center,
    options.diameterPhysicalPixels,
    image.width,
    image.height,
  );
  return {
    foreground_rgba: foregroundRgba,
    metrics: measurePointFootprint(image, {
      rectangle,
      center: options.center,
      radiusPixels: options.diameterPhysicalPixels / 2,
      foregroundRgba,
      backgroundRgba: options.backgroundRgba,
    }),
  };
}

export function footprintRectangle(center, diameterPhysicalPixels, imageWidth, imageHeight) {
  validateCenter(center, imageWidth, imageHeight);
  requireCondition(
    Number.isFinite(diameterPhysicalPixels)
      && diameterPhysicalPixels >= 2
      && diameterPhysicalPixels <= 7,
    "display diameter is outside the closed measurement range",
  );
  const margin = 3;
  const radius = diameterPhysicalPixels / 2;
  const minimumX = Math.max(0, Math.floor(center[0] - radius - margin));
  const minimumY = Math.max(0, Math.floor(center[1] - radius - margin));
  const maximumX = Math.min(imageWidth, Math.ceil(center[0] + radius + margin));
  const maximumY = Math.min(imageHeight, Math.ceil(center[1] + radius + margin));
  requireCondition(maximumX > minimumX && maximumY > minimumY, "footprint rectangle is empty");
  return {
    x: minimumX,
    y: minimumY,
    width: maximumX - minimumX,
    height: maximumY - minimumY,
  };
}

export function sampleForegroundRgba(image, center, backgroundRgba) {
  validateImage(image);
  validateCenter(center, image.width, image.height);
  validateRgba(backgroundRgba, "background RGBA");
  const anchorX = Math.floor(center[0]);
  const anchorY = Math.floor(center[1]);
  let best;
  let bestDistance = -1;
  for (let y = Math.max(0, anchorY - 1); y <= Math.min(image.height - 1, anchorY + 1); y += 1) {
    for (let x = Math.max(0, anchorX - 1); x <= Math.min(image.width - 1, anchorX + 1); x += 1) {
      const offset = (y * image.width + x) * 4;
      const rgba = Array.from(image.data.subarray(offset, offset + 4));
      const distance = rgba.reduce(
        (total, value, channel) => total + (value - backgroundRgba[channel]) ** 2,
        0,
      );
      if (distance > bestDistance) {
        best = rgba;
        bestDistance = distance;
      }
    }
  }
  requireCondition(bestDistance > 0, "isolated footprint has no foreground endpoint");
  return best;
}

export function evaluateRepresentativeTiming(timing, predecessor, limits) {
  requireRecord(timing, "representative timing");
  requireRecord(predecessor, "predecessor timing");
  requireRecord(limits, "timing limits");
  const interval = timing.frame_interval_milliseconds?.p95;
  const submission = timing.frame_submission_milliseconds?.p95;
  const predecessorInterval = predecessor.maximum_recreation_frame_interval_p95_milliseconds;
  const predecessorSubmission = predecessor.maximum_recreation_frame_submission_p95_milliseconds;
  for (const [label, value] of Object.entries({
    interval,
    submission,
    predecessorInterval,
    predecessorSubmission,
  })) {
    requireCondition(Number.isFinite(value) && value >= 0, `${label} timing is invalid`);
  }
  const failures = [];
  if (interval > limits.frame_interval_p95_milliseconds) failures.push("frame_interval_ceiling");
  if (submission > limits.frame_submission_p95_milliseconds) failures.push("frame_submission_ceiling");
  if (predecessorInterval > 0 && interval > predecessorInterval * limits.maximum_predecessor_ratio) {
    failures.push("frame_interval_predecessor_ratio");
  }
  if (predecessorSubmission > 0 && submission > predecessorSubmission * limits.maximum_predecessor_ratio) {
    failures.push("frame_submission_predecessor_ratio");
  }
  return {
    passed: failures.length === 0,
    failures,
    predecessor_ratio: {
      frame_interval: predecessorInterval === 0 ? null : interval / predecessorInterval,
      frame_submission: predecessorSubmission === 0 ? null : submission / predecessorSubmission,
    },
  };
}

function validateImage(image) {
  requireRecord(image, "RGBA image");
  requireCondition(Number.isSafeInteger(image.width) && image.width > 0, "RGBA image width is invalid");
  requireCondition(Number.isSafeInteger(image.height) && image.height > 0, "RGBA image height is invalid");
  requireCondition(
    (image.data instanceof Uint8Array || image.data instanceof Uint8ClampedArray)
      && image.data.byteLength === image.width * image.height * 4,
    "RGBA image data differs from its dimensions",
  );
}

function validateCenter(center, width, height) {
  requireCondition(
    Array.isArray(center)
      && center.length === 2
      && center.every(Number.isFinite)
      && center[0] >= 0
      && center[0] < width
      && center[1] >= 0
      && center[1] < height,
    "footprint center is outside the image",
  );
}

function validateRgba(rgba, label) {
  requireCondition(
    Array.isArray(rgba)
      && rgba.length === 4
      && rgba.every((value) => Number.isSafeInteger(value) && value >= 0 && value <= 255),
    `${label} is invalid`,
  );
}
