import { createVisualValidator } from "./visual-validation.js";

export const POINT_FOOTPRINT_METRICS_SCHEMA = "punctra-browser-point-footprint-metrics-v1";
export const REGION_TOPOLOGY_METRICS_SCHEMA = "punctra-browser-region-topology-metrics-v1";
export const COMPONENT_BRIDGE_METRICS_SCHEMA = "punctra-browser-component-bridge-metrics-v1";
export const IDEAL_DISK_SAMPLES_PER_AXIS = 16;

const MAX_IMAGE_AXIS_PIXELS = 4_096;
const MAX_IMAGE_PIXELS = 8_388_608;
const MAX_DISK_REGION_PIXELS = 65_536;
const { requireCondition, requireRecord } = createVisualValidator("Point footprint metrics failed");

/**
 * Produces deterministic pixel-integrated coverage for one analytic disk.
 * Samples are taken at the centers of a fixed 16 by 16 grid in every pixel.
 */
export function createIdealDiskCoverage(options) {
  requireRecord(options, "ideal disk options");
  const rectangle = validateStandaloneRectangle(options.rectangle, MAX_DISK_REGION_PIXELS);
  const center = validateCenter(options.center, rectangle);
  const radiusPixels = validateRadius(options.radiusPixels, rectangle);
  const data = new Float64Array(rectangle.width * rectangle.height);
  const radiusSquared = radiusPixels * radiusPixels;
  const samplesPerPixel = IDEAL_DISK_SAMPLES_PER_AXIS ** 2;

  for (let localY = 0; localY < rectangle.height; localY += 1) {
    for (let localX = 0; localX < rectangle.width; localX += 1) {
      data[localY * rectangle.width + localX] = diskPixelCoverage(
        rectangle.x + localX,
        rectangle.y + localY,
        center,
        radiusSquared,
      ) / samplesPerPixel;
    }
  }

  return {
    rectangle,
    center,
    radius_pixels: radiusPixels,
    samples_per_axis: IDEAL_DISK_SAMPLES_PER_AXIS,
    samples_per_pixel: samplesPerPixel,
    data,
  };
}

/** Measures one bounded rendered footprint against an analytic disk. */
export function measurePointFootprint(image, options) {
  const validatedImage = validateImage(image);
  const config = validateFootprintOptions(validatedImage, options);
  const observed = deriveCoverage(validatedImage, config);
  const ideal = createIdealDiskCoverage({
    rectangle: config.rectangle,
    center: config.center,
    radiusPixels: config.radiusPixels,
  });
  const observedMoments = coverageMoments(observed, config.rectangle, config.center);
  const idealMoments = coverageMoments(ideal.data, config.rectangle, config.center);

  return {
    schema: POINT_FOOTPRINT_METRICS_SCHEMA,
    rectangle: config.rectangle,
    center: config.center,
    radius_pixels: config.radiusPixels,
    normalization: normalizationFacts(config),
    ideal_disk: {
      samples_per_axis: ideal.samples_per_axis,
      samples_per_pixel: ideal.samples_per_pixel,
    },
    coverage: compareCoverage(observed, ideal.data),
    centroid: compareCentroids(observedMoments, idealMoments, config.center),
    radial: compareRadialMoments(observedMoments, idealMoments),
    aspect: compareAspectMoments(observedMoments, idealMoments),
    corner_leakage: measureCornerLeakage(
      observed,
      ideal.data,
      normalizedRgba8QuantizationTolerance(config),
      config.rectangle,
      config.center,
      config.radiusPixels,
    ),
  };
}

/**
 * Measures a thresholded region with 4-connected components. A horizontal
 * bridge touches both left and right edges; a vertical bridge touches both top
 * and bottom edges. Interior background components are therefore holes.
 */
export function measureRegionTopology(image, options) {
  const validatedImage = validateImage(image);
  const config = validateTopologyOptions(validatedImage, options);
  const coverage = deriveCoverage(validatedImage, config);
  const mask = thresholdCoverage(coverage, config.foregroundThreshold);
  const foregroundPixels = countForeground(mask);
  const components = analyzeComponents(mask, config.rectangle.width, config.rectangle.height);

  return {
    schema: REGION_TOPOLOGY_METRICS_SCHEMA,
    rectangle: config.rectangle,
    normalization: normalizationFacts(config),
    foreground_threshold: config.foregroundThreshold,
    foreground_pixels: foregroundPixels,
    partial_edge_pixels: countPartialCoverage(coverage),
    background_pixels: mask.length - foregroundPixels,
    foreground_fraction: foregroundPixels / mask.length,
    solid_2x2_blocks: countSolidBlocks(mask, config.rectangle.width, config.rectangle.height),
    foreground: components.foreground,
    background: components.background,
  };
}

function countPartialCoverage(coverage) {
  let partialPixels = 0;
  for (const value of coverage) {
    if (isPartialCoverage(value)) partialPixels += 1;
  }
  return partialPixels;
}

/** Detects candidate components that coalesce separated predecessor components. */
export function measureForegroundComponentBridges(predecessorImage, candidateImage, options) {
  const predecessor = validateImage(predecessorImage);
  const candidate = validateImage(candidateImage);
  requireCondition(
    candidate.width === predecessor.width && candidate.height === predecessor.height,
    "component-bridge image dimensions differ",
  );
  const config = validateTopologyOptions(predecessor, options);
  validateImageRectangle(config.rectangle, candidate, MAX_IMAGE_PIXELS);
  const minimumClearSeparationPixels = boundedInteger(
    options.minimumClearSeparationPixels,
    "minimum clear component separation",
    1,
    MAX_IMAGE_AXIS_PIXELS,
  );
  const predecessorMask = thresholdCoverage(
    deriveCoverage(predecessor, config),
    config.foregroundThreshold,
  );
  const candidateMask = thresholdCoverage(
    deriveCoverage(candidate, config),
    config.foregroundThreshold,
  );
  const width = config.rectangle.width;
  const height = config.rectangle.height;
  const predecessorComponents = labelForegroundComponents(predecessorMask, width, height);
  const candidateComponents = labelForegroundComponents(candidateMask, width, height);
  let bridgingCandidateComponentCount = 0;
  let firstBridge = null;

  for (let candidateId = 0; candidateId < candidateComponents.components.length; candidateId += 1) {
    const predecessorIds = overlappingComponentIds(
      candidateComponents.components[candidateId],
      predecessorComponents.labels,
    );
    const bridge = firstSeparatedComponentPair(
      predecessorIds,
      predecessorComponents,
      width,
      height,
      minimumClearSeparationPixels,
    );
    if (bridge === null) continue;
    bridgingCandidateComponentCount += 1;
    firstBridge ??= {
      candidate_component: candidateId,
      predecessor_components: bridge,
    };
  }

  return {
    schema: COMPONENT_BRIDGE_METRICS_SCHEMA,
    rectangle: config.rectangle,
    connectivity: 4,
    minimum_clear_separation_pixels: minimumClearSeparationPixels,
    predecessor_component_count: predecessorComponents.components.length,
    candidate_component_count: candidateComponents.components.length,
    bridging_candidate_component_count: bridgingCandidateComponentCount,
    first_bridge: firstBridge,
  };
}

function diskPixelCoverage(pixelX, pixelY, center, radiusSquared) {
  let inside = 0;
  for (let sampleY = 0; sampleY < IDEAL_DISK_SAMPLES_PER_AXIS; sampleY += 1) {
    const y = pixelY + (sampleY + 0.5) / IDEAL_DISK_SAMPLES_PER_AXIS;
    for (let sampleX = 0; sampleX < IDEAL_DISK_SAMPLES_PER_AXIS; sampleX += 1) {
      const x = pixelX + (sampleX + 0.5) / IDEAL_DISK_SAMPLES_PER_AXIS;
      const deltaX = x - center[0];
      const deltaY = y - center[1];
      if (deltaX * deltaX + deltaY * deltaY <= radiusSquared) inside += 1;
    }
  }
  return inside;
}

function deriveCoverage(image, config) {
  const { rectangle, foregroundRgba, backgroundRgba } = config;
  const direction = foregroundRgba.map((value, channel) => value - backgroundRgba[channel]);
  const denominator = direction.reduce((total, value) => total + value * value, 0);
  const coverage = new Float64Array(rectangle.width * rectangle.height);

  for (let localY = 0; localY < rectangle.height; localY += 1) {
    for (let localX = 0; localX < rectangle.width; localX += 1) {
      const imageOffset = ((rectangle.y + localY) * image.width + rectangle.x + localX) * 4;
      let numerator = 0;
      for (let channel = 0; channel < 4; channel += 1) {
        numerator += (image.data[imageOffset + channel] - backgroundRgba[channel]) * direction[channel];
      }
      coverage[localY * rectangle.width + localX] = clampUnit(numerator / denominator);
    }
  }
  return coverage;
}

function compareCoverage(observed, ideal) {
  let absoluteError = 0;
  let squaredError = 0;
  let observedTotal = 0;
  let idealTotal = 0;
  let partialEdgePixels = 0;
  let idealPartialEdgePixels = 0;
  for (let index = 0; index < observed.length; index += 1) {
    const error = Math.abs(observed[index] - ideal[index]);
    absoluteError += error;
    squaredError += error * error;
    observedTotal += observed[index];
    idealTotal += ideal[index];
    if (isPartialCoverage(observed[index])) partialEdgePixels += 1;
    if (isPartialCoverage(ideal[index])) idealPartialEdgePixels += 1;
  }
  return {
    observed_total: observedTotal,
    ideal_total: idealTotal,
    absolute_area_error: Math.abs(observedTotal - idealTotal),
    mean_absolute_error: absoluteError / observed.length,
    root_mean_square_error: Math.sqrt(squaredError / observed.length),
    partial_edge_pixels: partialEdgePixels,
    ideal_partial_edge_pixels: idealPartialEdgePixels,
  };
}

function coverageMoments(coverage, rectangle, declaredCenter) {
  let total = 0;
  let xTotal = 0;
  let yTotal = 0;
  let radialSquareTotal = 0;
  visitCoverage(coverage, rectangle, (value, x, y) => {
    total += value;
    xTotal += value * x;
    yTotal += value * y;
    radialSquareTotal += value * ((x - declaredCenter[0]) ** 2 + (y - declaredCenter[1]) ** 2);
  });
  if (total === 0) return emptyMoments();

  const centroid = { x: xTotal / total, y: yTotal / total };
  let xx = 0;
  let xy = 0;
  let yy = 0;
  visitCoverage(coverage, rectangle, (value, x, y) => {
    const deltaX = x - centroid.x;
    const deltaY = y - centroid.y;
    xx += value * deltaX * deltaX;
    xy += value * deltaX * deltaY;
    yy += value * deltaY * deltaY;
  });
  return {
    total,
    centroid,
    rmsRadiusPixels: Math.sqrt(radialSquareTotal / total),
    aspectRatio: principalAspectRatio(xx / total, xy / total, yy / total),
  };
}

function visitCoverage(coverage, rectangle, visitor) {
  for (let localY = 0; localY < rectangle.height; localY += 1) {
    for (let localX = 0; localX < rectangle.width; localX += 1) {
      const value = coverage[localY * rectangle.width + localX];
      if (value === 0) continue;
      visitor(value, rectangle.x + localX + 0.5, rectangle.y + localY + 0.5);
    }
  }
}

function principalAspectRatio(xx, xy, yy) {
  const trace = xx + yy;
  const discriminant = Math.sqrt(Math.max(0, (xx - yy) ** 2 + 4 * xy * xy));
  const largest = (trace + discriminant) / 2;
  const smallest = (trace - discriminant) / 2;
  if (smallest <= Number.EPSILON || largest <= Number.EPSILON) return null;
  return Math.sqrt(largest / smallest);
}

function compareCentroids(observed, ideal, declaredCenter) {
  return {
    declared: { x: declaredCenter[0], y: declaredCenter[1] },
    observed: observed.centroid,
    ideal: ideal.centroid,
    error_pixels: pointDistance(observed.centroid, ideal.centroid),
    declared_center_error_pixels: pointDistance(observed.centroid, {
      x: declaredCenter[0],
      y: declaredCenter[1],
    }),
  };
}

function compareRadialMoments(observed, ideal) {
  return {
    observed_rms_radius_pixels: observed.rmsRadiusPixels,
    ideal_rms_radius_pixels: ideal.rmsRadiusPixels,
    error_pixels: nullableDifference(observed.rmsRadiusPixels, ideal.rmsRadiusPixels),
  };
}

function compareAspectMoments(observed, ideal) {
  return {
    observed_ratio: observed.aspectRatio,
    ideal_ratio: ideal.aspectRatio,
    error: nullableDifference(observed.aspectRatio, ideal.aspectRatio),
  };
}

function measureCornerLeakage(
  observed,
  ideal,
  quantizationTolerance,
  rectangle,
  center,
  radiusPixels,
) {
  const allQuadCornersClear = quadCornerPixels(center, radiusPixels).every(([x, y]) => (
    observedCoverageAtPixel(observed, rectangle, x, y) < 1 - quantizationTolerance
  ));
  let pixelCount = 0;
  let coverage = 0;
  let outerPixelCount = 0;
  let outerCoverage = 0;
  let exactDistanceOuterPixelCount = 0;
  let exactDistanceOuterCoverage = 0;
  let observedTotal = 0;
  for (let index = 0; index < observed.length; index += 1) {
    observedTotal += observed[index];
    if (
      observed[index] > quantizationTolerance
      && decodedPixelCenterDistance(index, rectangle, center) > radiusPixels + 0.75
    ) {
      exactDistanceOuterPixelCount += 1;
      exactDistanceOuterCoverage += observed[index];
    }
    const excess = observed[index] - ideal[index];
    if (excess <= quantizationTolerance) continue;
    pixelCount += 1;
    coverage += excess;
    if (ideal[index] === 0) {
      outerPixelCount += 1;
      outerCoverage += observed[index];
    }
  }
  return {
    definition: "observed_coverage_above_ideal_disk_after_endpoint_rgba8_quantization_tolerance",
    normalized_rgba8_quantization_tolerance: quantizationTolerance,
    all_quad_corners_clear: allQuadCornersClear,
    pixel_count: pixelCount,
    coverage,
    outer_pixel_count: outerPixelCount,
    outer_coverage: outerCoverage,
    exact_distance_outer: {
      definition: "decoded_pixel_center_distance_greater_than_declared_radius_plus_margin",
      sample_location: "decoded_pixel_center",
      margin_physical_pixels: 0.75,
      pixel_count: exactDistanceOuterPixelCount,
      coverage: exactDistanceOuterCoverage,
    },
    fraction_of_observed_coverage: observedTotal === 0 ? 0 : coverage / observedTotal,
  };
}

function quadCornerPixels(center, radiusPixels) {
  return [
    [-1, -1],
    [1, -1],
    [-1, 1],
    [1, 1],
  ].map(([signX, signY]) => [
    Math.floor(center[0] + signX * radiusPixels),
    Math.floor(center[1] + signY * radiusPixels),
  ]);
}

function observedCoverageAtPixel(observed, rectangle, x, y) {
  const localX = x - rectangle.x;
  const localY = y - rectangle.y;
  if (localX < 0 || localX >= rectangle.width || localY < 0 || localY >= rectangle.height) {
    return 0;
  }
  return observed[localY * rectangle.width + localX];
}

function decodedPixelCenterDistance(index, rectangle, center) {
  const localX = index % rectangle.width;
  const localY = Math.floor(index / rectangle.width);
  return Math.hypot(
    rectangle.x + localX + 0.5 - center[0],
    rectangle.y + localY + 0.5 - center[1],
  );
}

function thresholdCoverage(coverage, threshold) {
  const mask = new Uint8Array(coverage.length);
  for (let index = 0; index < coverage.length; index += 1) {
    mask[index] = coverage[index] >= threshold ? 1 : 0;
  }
  return mask;
}

function countForeground(mask) {
  let count = 0;
  for (const value of mask) count += value;
  return count;
}

function countSolidBlocks(mask, width, height) {
  let count = 0;
  for (let y = 0; y + 1 < height; y += 1) {
    for (let x = 0; x + 1 < width; x += 1) {
      const topLeft = y * width + x;
      if (mask[topLeft] && mask[topLeft + 1]
        && mask[topLeft + width] && mask[topLeft + width + 1]) count += 1;
    }
  }
  return count;
}

function analyzeComponents(mask, width, height) {
  const summaries = [emptyComponentSummary(), emptyComponentSummary()];
  const visited = new Uint8Array(mask.length);
  const queue = new Uint32Array(mask.length);
  for (let start = 0; start < mask.length; start += 1) {
    if (visited[start]) continue;
    const component = traverseComponent(mask, visited, queue, start, width, height);
    includeComponent(summaries[mask[start]], component);
  }
  return { background: summaries[0], foreground: summaries[1] };
}

function labelForegroundComponents(mask, width, height) {
  const labels = new Int32Array(mask.length);
  labels.fill(-1);
  const components = [];
  const queue = new Uint32Array(mask.length);
  for (let start = 0; start < mask.length; start += 1) {
    if (!mask[start] || labels[start] !== -1) continue;
    const id = components.length;
    const pixels = [];
    let head = 0;
    let tail = 1;
    queue[0] = start;
    labels[start] = id;
    while (head < tail) {
      const index = queue[head];
      head += 1;
      pixels.push(index);
      const x = index % width;
      const y = Math.floor(index / width);
      tail = enqueueLabeledNeighbor(labels, mask, queue, tail, id, index - 1, x > 0);
      tail = enqueueLabeledNeighbor(labels, mask, queue, tail, id, index + 1, x + 1 < width);
      tail = enqueueLabeledNeighbor(labels, mask, queue, tail, id, index - width, y > 0);
      tail = enqueueLabeledNeighbor(labels, mask, queue, tail, id, index + width, y + 1 < height);
    }
    components.push(pixels);
  }
  return { labels, components };
}

function enqueueLabeledNeighbor(labels, mask, queue, tail, id, neighbor, inside) {
  if (!inside || !mask[neighbor] || labels[neighbor] !== -1) return tail;
  labels[neighbor] = id;
  queue[tail] = neighbor;
  return tail + 1;
}

function overlappingComponentIds(candidatePixels, predecessorLabels) {
  const ids = new Set();
  for (const index of candidatePixels) {
    const id = predecessorLabels[index];
    if (id !== -1) ids.add(id);
  }
  return [...ids].sort((left, right) => left - right);
}

function firstSeparatedComponentPair(ids, labeled, width, height, minimumClearPixels) {
  for (let leftIndex = 0; leftIndex < ids.length; leftIndex += 1) {
    for (let rightIndex = leftIndex + 1; rightIndex < ids.length; rightIndex += 1) {
      const left = ids[leftIndex];
      const right = ids[rightIndex];
      if (!componentsWithinManhattanDistance(
        labeled,
        left,
        right,
        width,
        height,
        minimumClearPixels,
      )) return [left, right];
    }
  }
  return null;
}

function componentsWithinManhattanDistance(labeled, left, right, width, height, distance) {
  const [searchPixels, target] = labeled.components[left].length <= labeled.components[right].length
    ? [labeled.components[left], right]
    : [labeled.components[right], left];
  for (const index of searchPixels) {
    const originX = index % width;
    const originY = Math.floor(index / width);
    for (let deltaY = -distance; deltaY <= distance; deltaY += 1) {
      const y = originY + deltaY;
      if (y < 0 || y >= height) continue;
      const remaining = distance - Math.abs(deltaY);
      for (let deltaX = -remaining; deltaX <= remaining; deltaX += 1) {
        const x = originX + deltaX;
        if (x < 0 || x >= width) continue;
        if (labeled.labels[y * width + x] === target) return true;
      }
    }
  }
  return false;
}

function traverseComponent(mask, visited, queue, start, width, height) {
  const target = mask[start];
  let head = 0;
  let tail = 1;
  let pixels = 0;
  let edgeMask = 0;
  queue[0] = start;
  visited[start] = 1;
  while (head < tail) {
    const index = queue[head];
    head += 1;
    pixels += 1;
    const x = index % width;
    const y = Math.floor(index / width);
    edgeMask |= touchedEdges(x, y, width, height);
    tail = enqueueNeighbor(mask, visited, queue, tail, target, index - 1, x > 0);
    tail = enqueueNeighbor(mask, visited, queue, tail, target, index + 1, x + 1 < width);
    tail = enqueueNeighbor(mask, visited, queue, tail, target, index - width, y > 0);
    tail = enqueueNeighbor(mask, visited, queue, tail, target, index + width, y + 1 < height);
  }
  return { pixels, edgeMask };
}

function enqueueNeighbor(mask, visited, queue, tail, target, neighbor, inside) {
  if (!inside || visited[neighbor] || mask[neighbor] !== target) return tail;
  visited[neighbor] = 1;
  queue[tail] = neighbor;
  return tail + 1;
}

function touchedEdges(x, y, width, height) {
  let mask = 0;
  if (x === 0) mask |= 1;
  if (x + 1 === width) mask |= 2;
  if (y === 0) mask |= 4;
  if (y + 1 === height) mask |= 8;
  return mask;
}

function includeComponent(summary, component) {
  summary.component_count += 1;
  summary.largest_component_pixels = Math.max(summary.largest_component_pixels, component.pixels);
  if (component.pixels === 1) summary.isolated_pixel_components += 1;
  if (component.edgeMask === 0) {
    summary.interior_component_count += 1;
    summary.largest_interior_component_pixels = Math.max(
      summary.largest_interior_component_pixels,
      component.pixels,
    );
  } else {
    summary.boundary_component_count += 1;
  }
  if ((component.edgeMask & 3) === 3) summary.left_right_bridge_components += 1;
  if ((component.edgeMask & 12) === 12) summary.top_bottom_bridge_components += 1;
}

function emptyComponentSummary() {
  return {
    connectivity: 4,
    component_count: 0,
    largest_component_pixels: 0,
    isolated_pixel_components: 0,
    interior_component_count: 0,
    largest_interior_component_pixels: 0,
    boundary_component_count: 0,
    left_right_bridge_components: 0,
    top_bottom_bridge_components: 0,
  };
}

function validateFootprintOptions(image, options) {
  const common = validateCommonOptions(image, options, MAX_DISK_REGION_PIXELS);
  return {
    ...common,
    center: validateCenter(options.center, common.rectangle),
    radiusPixels: validateRadius(options.radiusPixels, common.rectangle),
  };
}

function validateTopologyOptions(image, options) {
  const common = validateCommonOptions(image, options, MAX_IMAGE_PIXELS);
  requireCondition(
    Number.isFinite(options.foregroundThreshold)
      && options.foregroundThreshold > 0
      && options.foregroundThreshold <= 1,
    "foreground threshold must be finite and inside (0, 1]",
  );
  return { ...common, foregroundThreshold: options.foregroundThreshold };
}

function validateCommonOptions(image, options, maximumArea) {
  requireRecord(options, "metric options");
  const rectangle = validateImageRectangle(options.rectangle, image, maximumArea);
  const foregroundRgba = validateRgba(options.foregroundRgba, "foreground RGBA");
  const backgroundRgba = validateRgba(options.backgroundRgba, "background RGBA");
  requireCondition(
    foregroundRgba.some((value, channel) => value !== backgroundRgba[channel]),
    "foreground and background colors must differ",
  );
  return { rectangle, foregroundRgba, backgroundRgba };
}

function validateImage(image) {
  requireRecord(image, "RGBA image");
  const width = boundedInteger(image.width, "RGBA image width", 1, MAX_IMAGE_AXIS_PIXELS);
  const height = boundedInteger(image.height, "RGBA image height", 1, MAX_IMAGE_AXIS_PIXELS);
  const pixels = width * height;
  requireCondition(pixels <= MAX_IMAGE_PIXELS, "RGBA image area exceeds its pixel ceiling");
  requireCondition(isRgbaByteArray(image.data), "RGBA image data must be a Uint8 byte array");
  requireCondition(image.data.byteLength === pixels * 4, "RGBA byte length differs from image dimensions");
  return { width, height, data: image.data };
}

function validateImageRectangle(rectangle, image, maximumArea) {
  const validated = validateStandaloneRectangle(rectangle, maximumArea);
  requireCondition(
    validated.x + validated.width <= image.width && validated.y + validated.height <= image.height,
    "metric rectangle exceeds the image",
  );
  return validated;
}

function validateStandaloneRectangle(rectangle, maximumArea) {
  requireRecord(rectangle, "metric rectangle");
  const x = boundedInteger(rectangle.x, "metric rectangle x", 0, MAX_IMAGE_AXIS_PIXELS - 1);
  const y = boundedInteger(rectangle.y, "metric rectangle y", 0, MAX_IMAGE_AXIS_PIXELS - 1);
  const width = boundedInteger(rectangle.width, "metric rectangle width", 1, MAX_IMAGE_AXIS_PIXELS);
  const height = boundedInteger(rectangle.height, "metric rectangle height", 1, MAX_IMAGE_AXIS_PIXELS);
  requireCondition(x + width <= MAX_IMAGE_AXIS_PIXELS, "metric rectangle x extent exceeds its axis ceiling");
  requireCondition(y + height <= MAX_IMAGE_AXIS_PIXELS, "metric rectangle y extent exceeds its axis ceiling");
  requireCondition(width * height <= maximumArea, "metric rectangle area exceeds its pixel ceiling");
  return { x, y, width, height };
}

function validateCenter(center, rectangle) {
  requireCondition(
    Array.isArray(center) && center.length === 2 && center.every(Number.isFinite),
    "disk center must contain two finite coordinates",
  );
  requireCondition(
    center[0] >= rectangle.x && center[0] <= rectangle.x + rectangle.width
      && center[1] >= rectangle.y && center[1] <= rectangle.y + rectangle.height,
    "disk center must lie inside the metric rectangle",
  );
  return [center[0], center[1]];
}

function validateRadius(radiusPixels, rectangle) {
  requireCondition(
    Number.isFinite(radiusPixels)
      && radiusPixels > 0
      && radiusPixels <= Math.max(rectangle.width, rectangle.height),
    "disk radius must be positive, finite, and bounded by the metric rectangle",
  );
  return radiusPixels;
}

function validateRgba(value, label) {
  requireCondition(
    Array.isArray(value)
      && value.length === 4
      && value.every((channel) => Number.isInteger(channel) && channel >= 0 && channel <= 255),
    `${label} must contain four RGBA8 channels`,
  );
  return [...value];
}

function normalizationFacts(config) {
  return {
    rule: "least_squares_projection_between_known_rgba8_endpoints_clamped_to_unit_interval",
    foreground_rgba: config.foregroundRgba,
    background_rgba: config.backgroundRgba,
  };
}

function normalizedRgba8QuantizationTolerance(config) {
  const direction = config.foregroundRgba.map(
    (value, channel) => value - config.backgroundRgba[channel],
  );
  const denominator = direction.reduce((total, value) => total + value * value, 0);
  const maximumProjectedRounding = direction.reduce(
    (total, value) => total + Math.abs(value) * 0.5,
    0,
  );
  return maximumProjectedRounding / denominator + Number.EPSILON;
}

function emptyMoments() {
  return { total: 0, centroid: null, rmsRadiusPixels: null, aspectRatio: null };
}

function pointDistance(left, right) {
  if (left === null || right === null) return null;
  return Math.hypot(left.x - right.x, left.y - right.y);
}

function nullableDifference(left, right) {
  return left === null || right === null ? null : Math.abs(left - right);
}

function isPartialCoverage(value) {
  return value > 0 && value < 1;
}

function clampUnit(value) {
  return Math.min(1, Math.max(0, value));
}

function isRgbaByteArray(value) {
  return value instanceof Uint8Array || value instanceof Uint8ClampedArray;
}

function boundedInteger(value, label, minimum, maximum) {
  requireCondition(Number.isSafeInteger(value) && value >= minimum && value <= maximum, `${label} is invalid`);
  return value;
}
