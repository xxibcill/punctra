import { createVisualValidator } from "./visual-validation.js";

export const FEATURE_ALIGNMENT_SCHEMA = "punctra-browser-feature-alignment-v1";
export const FEATURE_ALIGNMENT_SEARCH_RADIUS = 4;
export const FEATURE_ALIGNMENT_TILE_SIZE = 32;
const { requireCondition } = createVisualValidator("Feature alignment invalid");

/** Measures translation separately from footprint area and edge opacity. */
export function measureFeatureAlignment(reference, candidate, rectangle, backgroundRgba) {
  validateInputs(reference, candidate, rectangle, backgroundRgba);
  reference = blurImage(reference);
  candidate = blurImage(candidate);
  const energy = contrastEnergyIntegral(candidate, backgroundRgba);
  return alignPrepared(reference, candidate, rectangle, backgroundRgba, energy);
}

/** Fixed predecessor regions prevent unchanged content elsewhere hiding drift. */
export function measureLocalFeatureAlignment(reference, candidate, rectangle, backgroundRgba) {
  const margin = FEATURE_ALIGNMENT_SEARCH_RADIUS;
  const bounds = {
    x: Math.max(margin, rectangle.x),
    y: Math.max(margin, rectangle.y),
    width: Math.min(reference.width - margin, rectangle.x + rectangle.width) - Math.max(margin, rectangle.x),
    height: Math.min(reference.height - margin, rectangle.y + rectangle.height) - Math.max(margin, rectangle.y),
  };
  validateInputs(reference, candidate, bounds, backgroundRgba);
  reference = blurImage(reference);
  candidate = blurImage(candidate);
  const referenceEnergy = contrastEnergyIntegral(reference, backgroundRgba);
  const candidateEnergy = contrastEnergyIntegral(candidate, backgroundRgba);
  const regions = [];
  for (let y = bounds.y; y < bounds.y + bounds.height; y += FEATURE_ALIGNMENT_TILE_SIZE) {
    for (let x = bounds.x; x < bounds.x + bounds.width; x += FEATURE_ALIGNMENT_TILE_SIZE) {
      const tile = { x, y, width: Math.min(FEATURE_ALIGNMENT_TILE_SIZE, bounds.x + bounds.width - x),
        height: Math.min(FEATURE_ALIGNMENT_TILE_SIZE, bounds.y + bounds.height - y) };
      if (rectangleEnergy(referenceEnergy, reference.width, tile, 0, 0) < 64) continue;
      const self = alignPrepared(reference, reference, tile, backgroundRgba, referenceEnergy);
      if (self.ambiguous) continue;
      regions.push(alignPrepared(reference, candidate, tile, backgroundRgba, candidateEnergy));
    }
  }
  return { tile_size_pixels: FEATURE_ALIGNMENT_TILE_SIZE, minimum_reference_contrast_energy: 64,
    regions, passed: regions.length > 0 && regions.every((region) => region.distance_pixels !== null
      && region.distance_pixels <= 1 && !region.ambiguous && region.correlation >= Math.SQRT1_2) };
}

function alignPrepared(reference, candidate, rectangle, backgroundRgba, energy) {
  const samples = contrastSamples(reference, rectangle, backgroundRgba);
  const scores = [];
  for (let y = -FEATURE_ALIGNMENT_SEARCH_RADIUS; y <= FEATURE_ALIGNMENT_SEARCH_RADIUS; y += 1) {
    for (let x = -FEATURE_ALIGNMENT_SEARCH_RADIUS; x <= FEATURE_ALIGNMENT_SEARCH_RADIUS; x += 1) {
      const candidateEnergy = rectangleEnergy(energy, candidate.width, rectangle, x, y);
      let dot = 0;
      for (const sample of samples.points) {
        const offset = ((sample.y + y) * candidate.width + sample.x + x) * 4;
        for (let channel = 0; channel < 3; channel += 1) {
          dot += sample.contrast[channel] * (candidate.data[offset + channel] - backgroundRgba[channel]);
        }
      }
      scores.push({ x, y, correlation: samples.energy === 0 || candidateEnergy === 0
        ? null : dot / Math.sqrt(samples.energy * candidateEnergy) });
    }
  }
  const ranked = scores.filter(({ correlation }) => correlation !== null)
    .sort((left, right) => right.correlation - left.correlation || left.y - right.y || left.x - right.x);
  const best = ranked[0] ?? null;
  return {
    schema: FEATURE_ALIGNMENT_SCHEMA,
    rectangle: structuredClone(rectangle),
    search_radius_pixels: FEATURE_ALIGNMENT_SEARCH_RADIUS,
    normalization: "rgb_contrast_cosine_from_fixed_clear_color_v1",
    best_offset_pixels: best === null ? null : { x: best.x, y: best.y },
    distance_pixels: best === null ? null : Math.hypot(best.x, best.y),
    correlation: best?.correlation ?? null,
    zero_offset_correlation: scores.find(({ x, y }) => x === 0 && y === 0).correlation,
    ambiguous: best === null || ranked.slice(1).some(({ correlation }) => best.correlation - correlation <= 1e-9),
  };
}

function blurImage(image) {
  const kernel = [1, 4, 6, 4, 1];
  const horizontal = new Float64Array(image.data.length);
  const data = new Float64Array(image.data.length);
  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      for (let channel = 0; channel < 3; channel += 1) {
        for (let tap = 0; tap < kernel.length; tap += 1) {
          const sampledX = Math.max(0, Math.min(image.width - 1, x + tap - 2));
          horizontal[(y * image.width + x) * 4 + channel]
            += image.data[(y * image.width + sampledX) * 4 + channel] * kernel[tap] / 16;
        }
      }
    }
  }
  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      for (let channel = 0; channel < 3; channel += 1) {
        for (let tap = 0; tap < kernel.length; tap += 1) {
          const sampledY = Math.max(0, Math.min(image.height - 1, y + tap - 2));
          data[(y * image.width + x) * 4 + channel]
            += horizontal[(sampledY * image.width + x) * 4 + channel] * kernel[tap] / 16;
        }
      }
    }
  }
  return { width: image.width, height: image.height, data };
}

function contrastSamples(image, rectangle, background) {
  const points = [];
  let energy = 0;
  for (let y = rectangle.y; y < rectangle.y + rectangle.height; y += 1) {
    for (let x = rectangle.x; x < rectangle.x + rectangle.width; x += 1) {
      const offset = (y * image.width + x) * 4;
      const contrast = [0, 1, 2].map((channel) => image.data[offset + channel] - background[channel]);
      const square = contrast.reduce((sum, value) => sum + value * value, 0);
      if (square === 0) continue;
      energy += square;
      points.push({ x, y, contrast });
    }
  }
  return { points, energy };
}

function contrastEnergyIntegral(image, background) {
  const stride = image.width + 1;
  const integral = new Float64Array(stride * (image.height + 1));
  for (let y = 0; y < image.height; y += 1) {
    let rowEnergy = 0;
    for (let x = 0; x < image.width; x += 1) {
      const offset = (y * image.width + x) * 4;
      for (let channel = 0; channel < 3; channel += 1) {
        rowEnergy += (image.data[offset + channel] - background[channel]) ** 2;
      }
      integral[(y + 1) * stride + x + 1] = integral[y * stride + x + 1] + rowEnergy;
    }
  }
  return integral;
}

function rectangleEnergy(integral, width, rectangle, dx, dy) {
  const stride = width + 1;
  const left = rectangle.x + dx;
  const top = rectangle.y + dy;
  const right = left + rectangle.width;
  const bottom = top + rectangle.height;
  return integral[bottom * stride + right] - integral[top * stride + right]
    - integral[bottom * stride + left] + integral[top * stride + left];
}

function validateInputs(reference, candidate, rectangle, background) {
  for (const image of [reference, candidate]) {
    requireCondition(Number.isSafeInteger(image?.width) && Number.isSafeInteger(image?.height)
      && image.width > 0 && image.height > 0 && image.width <= 4096 && image.height <= 4096
      && image.width * image.height <= 8_388_608
      && image.data instanceof Uint8Array && image.data.length === image.width * image.height * 4,
    "image must be bounded tight RGBA8");
  }
  requireCondition(reference.width === candidate.width && reference.height === candidate.height,
    "image dimensions differ");
  requireCondition(background?.length === 4 && background.every((channel) => Number.isInteger(channel)
    && channel >= 0 && channel <= 255), "clear color is invalid");
  requireCondition([rectangle?.x, rectangle?.y, rectangle?.width, rectangle?.height].every(Number.isSafeInteger)
    && rectangle.width > 0 && rectangle.height > 0
    && rectangle.x >= FEATURE_ALIGNMENT_SEARCH_RADIUS && rectangle.y >= FEATURE_ALIGNMENT_SEARCH_RADIUS
    && rectangle.x + rectangle.width + FEATURE_ALIGNMENT_SEARCH_RADIUS <= reference.width
    && rectangle.y + rectangle.height + FEATURE_ALIGNMENT_SEARCH_RADIUS <= reference.height,
  "feature rectangle requires the complete search margin");
}
