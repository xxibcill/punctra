import {
  decodeTransferV2,
  encodeTransferV2,
  projectAuthoredPointAtViewport,
} from "./visual-corpus.js";
import { footprintRectangle } from "./footprint-runner-core.js";
import { sha256Hex } from "./visual-png.js";
import { createVisualValidator } from "./visual-validation.js";

export const ISOLATED_FOOTPRINT_FIXTURE = "isolated_authored_subset_v1";
const { requireCondition } = createVisualValidator("Isolated footprint fixture invalid");

/** Samples unchanged authored Points into a separate, non-overlapping GPU trial. */
export async function materializeFootprintFixture(materialized, focused) {
  requireCondition(focused.fixture === ISOLATED_FOOTPRINT_FIXTURE, "fixture recipe differs");
  const ordinals = new Set(focused.isolated_ordinals);
  const points = materialized.batches.flatMap((batch) => decodeTransferV2(batch))
    .filter(({ ordinal }) => ordinals.has(ordinal));
  requireCondition(points.length === ordinals.size, "an authored Point is absent");
  const payload = encodeTransferV2(points);
  const count = points.length;
  return {
    ...materialized,
    batches: [payload],
    trial: {
      ...materialized.trial,
      temporal_trace: { kind: "static" },
      expected_settled_batch_versions: [2],
    },
    source: {
      ...materialized.source,
      expected_view: {
        ...materialized.source.expected_view,
        published_points: count,
        published_batches: 1,
        transferred_bytes: payload.byteLength,
        batch_keys: [1],
        initial_batch_versions: [1],
        settled_removed_batch_indices: [],
        settled_resident_points: count,
        settled_drawn_points: count,
        settled_draw_calls: 1,
        settled_presentation_weights_u8: [255],
      },
    },
    input_facts: {
      kind: ISOLATED_FOOTPRINT_FIXTURE,
      source_identity: materialized.source_identity,
      authored_ordinals: points.map(({ ordinal }) => ordinal),
      transfer_bytes: payload.byteLength,
      payload_sha256: await sha256Hex(payload),
    },
  };
}

/** Rejects contaminated or clipped measurement regions before GPU execution. */
export function validateIsolatedFootprintFixture(materialized, profile, diameter = 6) {
  const points = materialized.batches.flatMap((batch) => decodeTransferV2(batch));
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
