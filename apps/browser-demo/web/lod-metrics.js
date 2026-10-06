import { createVisualValidator } from "./visual-validation.js";

export const LOD_IMAGE_LIMITS = Object.freeze({
  common_foreground_coverage: 0.25,
  quantization_bytes: 1,
  maximum_changed_common_fraction: 0.25,
  maximum_temporal_rmse: 0.40,
});
const { requireCondition } = createVisualValidator("LOD image metrics invalid");

/** Compares one candidate with endpoint controls at its exact camera and diameter. */
export function measureRasterTransition({ outgoing, incoming, candidate, previous, backgroundRgba, stationary }) {
  validateImages([outgoing, incoming, candidate, ...(previous ? [previous] : [])]);
  requireCondition(Array.isArray(backgroundRgba) && backgroundRgba.length === 4
    && backgroundRgba.every((value) => Number.isInteger(value) && value >= 0 && value <= 255), "clear RGBA differs");
  requireCondition(typeof stationary === "boolean", "stationary disposition is absent");
  let commonForegroundPixels = 0;
  let newClearPixels = 0;
  let changedCommonPixels = 0;
  let squaredDelta = 0;
  for (let offset = 0; offset < candidate.data.byteLength; offset += 4) {
    const common = foreground(outgoing.data, offset, backgroundRgba) >= LOD_IMAGE_LIMITS.common_foreground_coverage
      && foreground(incoming.data, offset, backgroundRgba) >= LOD_IMAGE_LIMITS.common_foreground_coverage;
    if (common) {
      commonForegroundPixels += 1;
      if (foreground(candidate.data, offset, backgroundRgba) <= LOD_IMAGE_LIMITS.quantization_bytes / 255) newClearPixels += 1;
    }
    if (previous) {
      let maximumDelta = 0;
      for (let channel = 0; channel < 4; channel += 1) {
        const delta = candidate.data[offset + channel] - previous.data[offset + channel];
        maximumDelta = Math.max(maximumDelta, Math.abs(delta));
        squaredDelta += delta * delta;
      }
      if (common && maximumDelta > LOD_IMAGE_LIMITS.quantization_bytes) changedCommonPixels += 1;
    }
  }
  const temporalRmse = previous ? Math.sqrt(squaredDelta / (candidate.data.byteLength * 255 * 255)) : null;
  const allowedChangedPixels = Math.ceil(commonForegroundPixels * LOD_IMAGE_LIMITS.maximum_changed_common_fraction);
  const failures = [];
  if (commonForegroundPixels === 0) failures.push("endpoint controls have no common foreground");
  if (newClearPixels !== 0) failures.push("candidate introduces clear common-endpoint pixels");
  if (previous && stationary && changedCommonPixels > allowedChangedPixels) failures.push("stationary common-foreground change exceeds 25%");
  if (previous && temporalRmse > LOD_IMAGE_LIMITS.maximum_temporal_rmse) failures.push("temporal RGBA RMSE exceeds 0.40");
  return {
    normalization: "maximum_absolute_rgba8_delta_from_clear_divided_by_255",
    temporal_normalization: "rgba8_channel_rmse_divided_by_255",
    limits: LOD_IMAGE_LIMITS,
    stationary,
    common_foreground_pixels: commonForegroundPixels,
    new_clear_pixels: newClearPixels,
    changed_common_pixels: previous ? changedCommonPixels : null,
    allowed_changed_common_pixels: allowedChangedPixels,
    temporal_rmse: temporalRmse,
    passed: failures.length === 0,
    failures,
  };
}

/** Recomputes the host's policy input without treating it as observed visibility. */
export function rasterDensityPointCount(batches) {
  let eighthPoints = 0;
  for (const batch of batches) {
    requireCondition(Number.isSafeInteger(batch.point_count) && batch.point_count >= 0, "batch Point count differs");
    requireCondition(Number.isInteger(batch.coverage_eighths) && batch.coverage_eighths >= 0 && batch.coverage_eighths <= 8, "batch coverage fraction differs");
    eighthPoints += batch.point_count * batch.coverage_eighths;
    requireCondition(Number.isSafeInteger(eighthPoints), "density sum exceeds exact integer accounting");
  }
  requireCondition(Number.isSafeInteger(eighthPoints + 4), "density rounding exceeds exact integer accounting");
  return Math.floor((eighthPoints + 4) / 8);
}

function foreground(data, offset, clear) {
  let delta = 0;
  for (let channel = 0; channel < 4; channel += 1) delta = Math.max(delta, Math.abs(data[offset + channel] - clear[channel]));
  return delta / 255;
}

function validateImages(images) {
  const first = images[0];
  for (const image of images) {
    requireCondition(Number.isInteger(image?.width) && image.width > 0 && image.width <= 4_096
      && Number.isInteger(image?.height) && image.height > 0 && image.height <= 4_096, "image dimensions differ");
    requireCondition(image.width === first.width && image.height === first.height, "endpoint/candidate dimensions differ");
    requireCondition(image.data instanceof Uint8Array && image.data.byteLength === image.width * image.height * 4, "tight RGBA8 bytes differ");
  }
}
