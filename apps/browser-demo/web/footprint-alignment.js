import { createVisualValidator } from "./visual-validation.js";

export const FEATURE_ALIGNMENT_SCHEMA = "punctra-browser-feature-alignment-v1";
export const FEATURE_ALIGNMENT_SEARCH_RADIUS = 4;
const { requireCondition } = createVisualValidator("Feature alignment invalid");

/** Measures translation separately from footprint area and edge opacity. */
export function measureFeatureAlignment(reference, candidate, rectangle, backgroundRgba) {
  validateInputs(reference, candidate, rectangle, backgroundRgba);
  reference = blurImage(reference);
  candidate = blurImage(candidate);
  const energy = contrastEnergyIntegral(candidate, backgroundRgba);
  return alignPrepared(reference, candidate, rectangle, backgroundRgba, energy);
}

/** Registers every predecessor component without weighting it by its color. */
export function measureFeatureComponentAlignment(reference, candidate, rectangle, backgroundRgba) {
  validateImagesAndRectangle(reference, candidate, rectangle, backgroundRgba);
  const referenceMask = occupancyMask(reference, backgroundRgba);
  const candidateMask = occupancyMask(candidate, backgroundRgba);
  const components = foregroundComponents(referenceMask, reference.width, reference.height);
  const referenceLabels = componentLabels(components, referenceMask.length);
  const candidateLabels = assignCandidateLabels(candidateMask, referenceLabels, reference.width, reference.height);
  const regions = components.flatMap((component, index) => {
    if (!component.pixels.some((pixel) => insideRectangle(pixel, reference.width, rectangle))) return [];
    const crop = { x: component.bounds.x - 8, y: component.bounds.y - 8,
      width: component.bounds.width + 16, height: component.bounds.height + 16 };
    requireCondition(crop.x >= 0 && crop.y >= 0 && crop.x + crop.width <= reference.width
      && crop.y + crop.height <= reference.height, "component lacks complete search and blur margins");
    const before = binaryCrop(crop);
    const after = binaryCrop(crop);
    for (const pixel of component.pixels) {
      const local = (Math.floor(pixel / reference.width) - crop.y) * crop.width + pixel % reference.width - crop.x;
      before.data.set([255, 255, 255, 255], local * 4);
    }
    for (let y = 0; y < crop.height; y += 1) {
      for (let x = 0; x < crop.width; x += 1) {
        if (candidateLabels[(crop.y + y) * candidate.width + crop.x + x] === index) {
          after.data.set([255, 255, 255, 255], (y * crop.width + x) * 4);
        }
      }
    }
    const alignment = measureFeatureAlignment(before, after, {
      x: 6, y: 6, width: component.bounds.width + 4, height: component.bounds.height + 4,
    }, [0, 0, 0, 255]);
    alignment.rectangle.x += crop.x;
    alignment.rectangle.y += crop.y;
    return [{ component_index: index, predecessor_pixel_count: component.pixels.length,
      predecessor_bounds: component.bounds, alignment }];
  });
  return { normalization: "predecessor_four_connected_binary_components_before_blur_v1",
    maximum_background_channel_delta: 2,
    regions, passed: regions.length > 0 && regions.every(({ alignment }) => alignment.distance_pixels !== null
      && alignment.distance_pixels <= 1 && !alignment.ambiguous && alignment.correlation >= Math.SQRT1_2) };
}

function occupancyMask(image, background) {
  const mask = new Uint8Array(image.width * image.height);
  for (let pixel = 0; pixel < mask.length; pixel += 1) {
    mask[pixel] = [0, 1, 2, 3].some((channel) => Math.abs(image.data[pixel * 4 + channel] - background[channel]) > 2) ? 1 : 0;
  }
  return mask;
}

function foregroundComponents(mask, width, height) {
  const visited = new Uint8Array(mask.length);
  const queue = new Uint32Array(mask.length);
  const components = [];
  for (let start = 0; start < mask.length; start += 1) {
    if (!mask[start] || visited[start]) continue;
    requireCondition(components.length < 4096, "feature component count exceeds its ceiling");
    const pixels = [];
    let head = 0;
    let tail = 1;
    queue[0] = start;
    visited[start] = 1;
    let left = width;
    let top = height;
    let right = 0;
    let bottom = 0;
    while (head < tail) {
      const pixel = queue[head++];
      const x = pixel % width;
      const y = Math.floor(pixel / width);
      pixels.push(pixel);
      left = Math.min(left, x);
      top = Math.min(top, y);
      right = Math.max(right, x);
      bottom = Math.max(bottom, y);
      for (const neighbor of [x > 0 ? pixel - 1 : -1, x + 1 < width ? pixel + 1 : -1,
        y > 0 ? pixel - width : -1, y + 1 < height ? pixel + width : -1]) {
        if (neighbor < 0 || !mask[neighbor] || visited[neighbor]) continue;
        visited[neighbor] = 1;
        queue[tail++] = neighbor;
      }
    }
    components.push({ pixels, bounds: { x: left, y: top, width: right - left + 1, height: bottom - top + 1 } });
  }
  return components;
}

function componentLabels(components, pixelCount) {
  const labels = new Int32Array(pixelCount).fill(-1);
  for (let index = 0; index < components.length; index += 1) {
    for (const pixel of components[index].pixels) labels[pixel] = index;
  }
  return labels;
}

function assignCandidateLabels(mask, reference, width, height) {
  const labels = new Int32Array(mask.length).fill(-1);
  for (let pixel = 0; pixel < mask.length; pixel += 1) {
    if (!mask[pixel]) continue;
    if (reference[pixel] >= 0) {
      labels[pixel] = reference[pixel];
      continue;
    }
    const x = pixel % width;
    const y = Math.floor(pixel / width);
    let nearestDistance = Number.POSITIVE_INFINITY;
    let label = -1;
    for (let dy = -FEATURE_ALIGNMENT_SEARCH_RADIUS; dy <= FEATURE_ALIGNMENT_SEARCH_RADIUS; dy += 1) {
      for (let dx = -FEATURE_ALIGNMENT_SEARCH_RADIUS; dx <= FEATURE_ALIGNMENT_SEARCH_RADIUS; dx += 1) {
        if (x + dx < 0 || y + dy < 0 || x + dx >= width || y + dy >= height) continue;
        const neighborLabel = reference[(y + dy) * width + x + dx];
        if (neighborLabel < 0) continue;
        const distance = dx * dx + dy * dy;
        if (distance < nearestDistance) {
          nearestDistance = distance;
          label = neighborLabel;
        } else if (distance === nearestDistance && label !== neighborLabel) {
          label = -1;
        }
      }
    }
    labels[pixel] = label;
  }
  return labels;
}

function insideRectangle(pixel, width, rectangle) {
  const x = pixel % width;
  const y = Math.floor(pixel / width);
  return x >= rectangle.x && x < rectangle.x + rectangle.width
    && y >= rectangle.y && y < rectangle.y + rectangle.height;
}

function binaryCrop(rectangle) {
  const data = new Uint8Array(rectangle.width * rectangle.height * 4);
  for (let offset = 3; offset < data.length; offset += 4) data[offset] = 255;
  return { width: rectangle.width, height: rectangle.height, data };
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

function validateImagesAndRectangle(reference, candidate, rectangle, background) {
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
    && rectangle.x >= 0 && rectangle.y >= 0
    && rectangle.x + rectangle.width <= reference.width
    && rectangle.y + rectangle.height <= reference.height, "feature rectangle is invalid");
}

function validateInputs(reference, candidate, rectangle, background) {
  validateImagesAndRectangle(reference, candidate, rectangle, background);
  requireCondition(rectangle.x >= FEATURE_ALIGNMENT_SEARCH_RADIUS && rectangle.y >= FEATURE_ALIGNMENT_SEARCH_RADIUS
    && rectangle.x + rectangle.width + FEATURE_ALIGNMENT_SEARCH_RADIUS <= reference.width
    && rectangle.y + rectangle.height + FEATURE_ALIGNMENT_SEARCH_RADIUS <= reference.height,
  "feature rectangle requires the complete search margin");
}
