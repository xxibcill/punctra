import { encodeTransferV2 } from "./visual-corpus.js";
import { sha256Hex } from "./visual-png.js";
import { createVisualValidator } from "./visual-validation.js";

const { requireCondition } = createVisualValidator("LOD fixture invalid");
export const LOD_FIXTURE_RECIPE = "paired_opaque_9_by_9_cut_v1";

/** Creates one ancestor and two child batches with coincident projected samples. */
export async function materializeLodFixture({ projection, opaqueCoincident = false }) {
  requireCondition(projection === "perspective" || projection === "orthographic", "camera family differs");
  const origin = [3_000_000_000, 4_600_000_000, 6_500_000_000];
  const parent = [];
  const children = [[], []];
  for (let row = 0; row < 9; row += 1) {
    for (let column = 0; column < 9; column += 1) {
      const index = row * 9 + column;
      const x = (column - 4) * 0.35;
      const z = (row - 4) * 0.35;
      // Perspective coordinates scale with eye distance so both endpoints
      // project to the same centers while retaining distinct Source depths.
      const incomingScale = projection === "perspective" ? 5.2 / 4.8 : 1;
      parent.push(sample(index, [x, -0.2, z], [65_535, 0, 0]));
      children[column < 5 ? 0 : 1].push(sample(index + 81,
        [x * incomingScale, 0.2, z * incomingScale],
        opaqueCoincident ? [65_535, 0, 0] : [0, 65_535, 0]));
    }
  }
  // Each authored child batch is sorted so the strict transfer's global
  // ordinal invariant holds; ordinals stay fixed across the two camera recipes.
  let nextOrdinal = 81;
  for (const child of children) for (const point of child) point.ordinal = nextOrdinal++;
  const batches = [parent, ...children].map(encodeTransferV2);
  const joined = new Uint8Array(batches.reduce((sum, bytes) => sum + bytes.byteLength, 0));
  let offset = 0;
  for (const bytes of batches) { joined.set(bytes, offset); offset += bytes.byteLength; }
  const digest = await sha256Hex(joined);
  return {
    recipe: LOD_FIXTURE_RECIPE,
    fixture_authority: "authored_private_cut_not_browser_child_lod_streaming",
    source_identity: digest,
    payload_sha256: digest,
    world_origin: origin,
    source_z_range: [origin[2] - 2, origin[2] + 2],
    point_count: 162,
    batches,
    batch_point_counts: batches.map((bytes) => bytes.byteLength / 32),
    outgoing_batch_indices: [0],
    incoming_batch_indices: [1, 2],
    opaque_coincident: opaqueCoincident,
    camera: {
      eye: [origin[0], origin[1] - 5, origin[2]], target: [...origin], up: [0, 0, 1],
      projection,
      vertical_world_height: projection === "orthographic" ? 4 : null,
      vertical_field_of_view_radians: projection === "perspective" ? 0.76101273 : null,
      near_distance: 0.1, far_distance: 100,
    },
  };
}

function sample(ordinal, position, rgb) {
  return { ordinal, relative_position: position, intensity: 1_024, classification: 2, rgb };
}
