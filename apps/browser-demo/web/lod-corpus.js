import { LOD_FIXTURE_RECIPE, materializeLodFixture } from "./lod-fixture.js";
import { LOD_IMAGE_LIMITS } from "./lod-metrics.js";
import { sha256Hex } from "./visual-png.js";
import { canonicalJsonEqual, createVisualValidator } from "./visual-validation.js";

const { requireCondition } = createVisualValidator("LOD corpus invalid");
export const LOD_CORPUS_SCHEMA = "punctra-browser-lod-continuity-corpus-v1";
const TRIALS = [
  ["opaque-perspective", "perspective-opaque", "opaque_coincident", true],
  ["opaque-orthographic", "orthographic-opaque", "opaque_coincident", true],
  ["refine-perspective", "perspective-colored", "refine", true],
  ["coarsen-orthographic", "orthographic-colored", "coarsen", true],
  ["moving-refine-perspective", "perspective-colored", "moving_refine", false],
  ["interrupted-refine-perspective", "perspective-colored", "interruption", false],
].map(([id, source, kind, stationary]) => ({ id, source, kind, stationary }));

export async function loadLodCorpus(url, fetchImplementation = globalThis.fetch) {
  const response = await fetchImplementation(url, { cache: "no-store", credentials: "same-origin" });
  requireCondition(response.ok, `corpus HTTP ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  const corpus = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  const fixtures = await validateLodCorpus(corpus);
  return { corpus, fixtures, bytes, sha256: await sha256Hex(bytes) };
}

/** Gates are closed before the runner receives any mutable viewer handle. */
export async function validateLodCorpus(corpus) {
  requireCondition(corpus?.schema === LOD_CORPUS_SCHEMA && corpus.release === "0.23.0-alpha.1", "release schema differs");
  requireCondition(corpus.authority === "authored_private_fixtures_not_general_browser_child_lod_streaming", "authority differs");
  requireCondition(corpus.recreations === 3 && corpus.presented_steps === 8 && corpus.seed === 9029 && corpus.quiet_frames === 30, "execution policy differs");
  requireCondition(canonicalJsonEqual(corpus.image_limits, LOD_IMAGE_LIMITS), "image gates differ");
  requireCondition(canonicalJsonEqual(corpus.trials, TRIALS), "closed trial matrix differs");
  requireCondition(canonicalJsonEqual(corpus.profiles, [1, 2, 4].map((dpr) => ({ id: `dpr${dpr}`,
    css_width: 320, css_height: 240, requested_device_pixel_ratio: dpr, physical_width: 320 * dpr, physical_height: 240 * dpr }))), "DPR profiles differ");
  requireCondition(canonicalJsonEqual(corpus.timing_limits, { first_coverage_milliseconds: 10_000,
    settled_view_milliseconds: 15_000, frame_callback_p95_milliseconds: 50,
    frame_submission_p95_milliseconds: 16.7, canonical_predecessor_p95_ratio: 2 }), "timing gates differ");
  requireCondition(canonicalJsonEqual(corpus.resource_limits, { renderer_transient_bytes: 67_108_864,
    live_canonical_bytes: 33_554_432, encoded_archive_bytes: 134_217_728,
    evidence_json_bytes: 16_777_216, archive_entries: 2_048 }), "resource gates differ");
  requireCondition(canonicalJsonEqual(corpus.external_evidence, { physical_display: false,
    independent_interpretation: false, independent_adopter: false, broad_browser_support: false,
    support_qualified: false, release_candidate: false }), "external evidence boundary differs");
  requireCondition(canonicalJsonEqual(corpus.predecessor, { release: "0.22.0-alpha.1",
    implementation_commit: "52ae3afb9312d6968a10c3aeec42b3c8555a86d1",
    baseline_path: "docs/releases/v0.22-browser-point-footprint-baseline.json",
    canonical_trials: 9, decoded_pixels: "exact" }), "predecessor differs");
  requireCondition(Array.isArray(corpus.sources) && corpus.sources.length === 4, "source matrix differs");
  const fixtures = new Map();
  for (const projection of ["perspective", "orthographic"]) {
    for (const opaqueCoincident of [false, true]) {
      const id = `${projection}-${opaqueCoincident ? "opaque" : "colored"}`;
      const source = corpus.sources.find((entry) => entry.id === id);
      requireCondition(source?.recipe === LOD_FIXTURE_RECIPE && source.projection === projection
        && source.opaque_coincident === opaqueCoincident, `source ${id} recipe differs`);
      const fixture = await materializeLodFixture({ projection, opaqueCoincident });
      for (const field of ["source_identity", "payload_sha256", "point_count", "batch_point_counts", "world_origin", "camera"]) {
        requireCondition(canonicalJsonEqual(source[field], fixture[field]), `source ${id} ${field} differs`);
      }
      requireCondition(source.permission === "Maintainer-authored synthetic Points; repository image publication permitted.", `source ${id} permission differs`);
      fixtures.set(id, fixture);
    }
  }
  return fixtures;
}
