import {
  MAX_TRANSFER_BATCHES,
  MAX_TRANSFER_BATCH_POINTS,
  decodeTransferV2,
  encodeTransferV2,
  projectAuthoredPointAtViewport,
} from "./visual-corpus.js";
import { footprintRectangle } from "./footprint-runner-core.js";
import { sha256Hex } from "./visual-png.js";
import { canonicalJsonEqual, createVisualValidator } from "./visual-validation.js";

export const ISOLATED_FOOTPRINT_FIXTURE = "isolated_authored_subset_v1";
const { requireCondition } = createVisualValidator("Isolated footprint fixture invalid");

/** Binds a settled capture to the unchanged projection inputs, excluding decorative size. */
export async function pointProjectionInput(materialized, trial, profile) {
  const bytes = new Uint8Array(materialized.batches.reduce((sum, batch) => sum + batch.byteLength, 0));
  let offset = 0;
  for (const batch of materialized.batches) {
    bytes.set(batch, offset);
    offset += batch.byteLength;
  }
  const view = materialized.source.expected_view;
  return {
    source_identity: materialized.source_identity,
    payload_sha256: await sha256Hex(bytes),
    world_origin: structuredClone(materialized.world_origin),
    camera: structuredClone(materialized.camera),
    display_mode: trial.display_mode,
    highlights: structuredClone(trial.selection.ordinals),
    physical_viewport: [profile.physical_width, profile.physical_height],
    generation: view.generation,
    batches: materialized.batches.flatMap((batch, index) => view.settled_removed_batch_indices.includes(index) ? [] : [{
      batch_index: index, key: view.batch_keys[index], version: trial.expected_settled_batch_versions[index],
      point_count: batch.byteLength / 32, state: "resident", presentation_weight_u8: view.settled_presentation_weights_u8[index],
    }]),
  };
}

export function validatePointProjectionCapture(input, diagnostics, captureFacts) {
  const camera = input.camera;
  for (const field of ["eye", "target", "up", "projection", "vertical_world_height"]) {
    requireCondition(canonicalJsonEqual(diagnostics.camera[field] ?? null, camera[field] ?? null),
      `observed camera ${field} differs from the authored projection`);
  }
  for (const field of ["vertical_field_of_view_radians", "near_distance", "far_distance"]) {
    requireCondition(Math.fround(diagnostics.camera[field] ?? 0) === Math.fround(camera[field] ?? 0),
      `observed camera ${field} differs from the authored projection`);
  }
  requireCondition(diagnostics.streaming.source_identity === input.source_identity,
    "observed Source identity differs");
  requireCondition(canonicalJsonEqual(diagnostics.streaming.world_origin, input.world_origin)
    && diagnostics.streaming.generation === input.generation, "observed world origin or generation differs");
  requireCondition(diagnostics.viewport.physical_width === input.physical_viewport[0]
    && diagnostics.viewport.physical_height === input.physical_viewport[1]
    && captureFacts.width === input.physical_viewport[0]
    && captureFacts.height === input.physical_viewport[1], "observed physical viewport differs");
  requireCondition(diagnostics.display_mode === input.display_mode
    && diagnostics.highlights.point_count === input.highlights.length, "observed display or highlights differ");
  requireCondition(captureFacts.view_generation === input.generation
    && canonicalJsonEqual(captureFacts.batches, input.batches), "observed settled batch inputs differ");
}

/** Samples unchanged authored Points into a separate, non-overlapping GPU trial. */
export async function materializeFootprintFixture(materialized, focused) {
  requireCondition(focused.fixture === ISOLATED_FOOTPRINT_FIXTURE, "fixture recipe differs");
  const ordinals = new Set(focused.isolated_ordinals);
  const authored = materialized.batches.flatMap((batch) => decodeTransferV2(batch));
  const points = authored.filter(({ ordinal }) => ordinals.has(ordinal));
  requireCondition(points.length === ordinals.size, "an authored Point is absent");
  const partition = partitionAuthoredPoints(authored, ordinals);
  requireCondition(partition.length <= MAX_TRANSFER_BATCHES, "partition exceeds the batch ceiling");
  const batches = partition.map((batch) => encodeTransferV2(batch));
  const removed = partition.flatMap((batch, index) => ordinals.has(batch[0].ordinal) ? [] : [index]);
  const count = points.length;
  const batchCount = batches.length;
  return {
    ...materialized,
    batches,
    trial: {
      ...materialized.trial,
      temporal_trace: { kind: "static" },
      expected_settled_batch_versions: Array(batchCount).fill(2),
    },
    source: {
      ...materialized.source,
      batch_count: batchCount,
      expected_view: {
        ...materialized.source.expected_view,
        published_points: authored.length,
        published_batches: batchCount,
        transferred_bytes: authored.length * 32,
        batch_keys: batches.map((_, index) => index + 1),
        initial_batch_versions: Array(batchCount).fill(1),
        settled_removed_batch_indices: removed,
        settled_resident_points: count,
        settled_drawn_points: count,
        settled_draw_calls: batchCount - removed.length,
        settled_presentation_weights_u8: Array(batchCount).fill(255),
      },
    },
    input_facts: {
      kind: ISOLATED_FOOTPRINT_FIXTURE,
      source_identity: materialized.source_identity,
      authored_ordinals: points.map(({ ordinal }) => ordinal),
      transfer_bytes: authored.length * 32,
      resident_transfer_bytes: count * 32,
      payload_sha256: await sha256Hex(encodeTransferV2(points)),
      published_batches: batchCount,
      retired_batch_indices: removed,
    },
  };
}

function partitionAuthoredPoints(points, isolatedOrdinals) {
  const batches = [];
  let batch = [];
  for (const point of points) {
    const isolated = isolatedOrdinals.has(point.ordinal);
    if (batch.length === MAX_TRANSFER_BATCH_POINTS
      || (batch.length > 0 && isolated !== isolatedOrdinals.has(batch[0].ordinal))) {
      batches.push(batch);
      batch = [];
    }
    batch.push(point);
  }
  if (batch.length > 0) batches.push(batch);
  return batches;
}

export function residentFootprintPoints(materialized) {
  const removed = new Set(materialized.source.expected_view.settled_removed_batch_indices);
  return materialized.batches.flatMap((batch, index) => removed.has(index) ? [] : decodeTransferV2(batch));

}

/** Rejects contaminated or clipped measurement regions before GPU execution. */
export function validateIsolatedFootprintFixture(materialized, profile, diameter = 6) {
  const points = residentFootprintPoints(materialized);
  const projected = points.map((point) => ({
    ordinal: point.ordinal,
    ...projectAuthoredPointAtViewport(point, materialized.world_origin, materialized.camera, profile),
  }));
  const samples = projected.map((point) => {
    const center = [point.exact_x, point.exact_y];
    const margin = diameter / 2 + 3;
    requireCondition(center[0] - margin >= 0 && center[1] - margin >= 0
      && center[0] + margin <= profile.physical_width
      && center[1] + margin <= profile.physical_height,
    `Point ${point.ordinal} measurement region is clipped`);
    const rectangle = footprintRectangle(center, diameter, profile.physical_width, profile.physical_height);
    let nearestSeparation = Infinity;
    for (const other of projected) {
      if (other.ordinal === point.ordinal) continue;
      const separation = Math.hypot(other.exact_x - point.exact_x, other.exact_y - point.exact_y);
      nearestSeparation = Math.min(nearestSeparation, separation);
      const dx = Math.max(rectangle.x - other.exact_x, 0, other.exact_x - rectangle.x - rectangle.width);
      const dy = Math.max(rectangle.y - other.exact_y, 0, other.exact_y - rectangle.y - rectangle.height);
      requireCondition(Math.hypot(dx, dy) > diameter / 2 + 0.75,
        `Point ${point.ordinal} measurement overlaps Point ${other.ordinal}`);
      requireCondition(separation > 7,
        `Point ${point.ordinal} nominal-pick target overlaps Point ${other.ordinal}`);
    }
    return { ordinal: point.ordinal, projected: point, rectangle, nearest_separation_pixels: nearestSeparation };
  });
  requireCondition(samples.length >= 2, "at least two isolated Points are required");
  return samples;
}

/** Binds the measured mask to the complete regenerated kernel window. */
export function validateFootprintSampleBinding(sample, isolated, diameter) {
  requireCondition(sample.ordinal === isolated.ordinal, "measurement ordinal differs");
  requireCondition(canonicalJsonEqual(sample.candidate.center,
    [isolated.projected.exact_x, isolated.projected.exact_y]), "measurement center differs");
  requireCondition(canonicalJsonEqual(sample.candidate.rectangle, isolated.rectangle),
    "measurement rectangle differs");
  requireCondition(sample.candidate.radius_pixels === diameter / 2, "measurement radius differs");
}
