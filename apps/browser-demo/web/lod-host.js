import { createViewer as createRawViewer } from "./pkg/browser_demo.js";
import { captureCanonicalFrame, parseRawJson, summarizeSamples } from "./visual-capture.js";
import { createVisualValidator } from "./visual-validation.js";

const { requireCondition } = createVisualValidator("LOD host failed");
export const animationFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));
export const raw = (value) => parseRawJson(value, "LOD raw viewer response");

export async function createLodViewer(canvas, profile) {
  canvas.style.width = `${profile.css_width}px`;
  canvas.style.height = `${profile.css_height}px`;
  canvas.width = profile.physical_width;
  canvas.height = profile.physical_height;
  const viewer = await createRawViewer(canvas, profile.css_width, profile.css_height, profile.requested_device_pixel_ratio);
  try {
    const diagnostics = raw(viewer.diagnostics());
    requireCondition(diagnostics.package_version === "0.23.0-alpha.1", "loaded runtime version differs");
    requireCondition(diagnostics.viewport.physical_width === profile.physical_width
      && diagnostics.viewport.physical_height === profile.physical_height, "physical viewport differs");
    return viewer;
  } catch (error) {
    try { viewer.shutdown(); } catch { /* keep the validation failure */ }
    viewer.free();
    throw error;
  }
}

/** Publishes all authored replacement batches before their first presentation. */
export function publishLodSource(viewer, fixture) {
  raw(viewer.beginStreamBatch(fixture.source_identity, fixture.point_count, ...fixture.world_origin,
    ...fixture.source_z_range, 0, fixture.batches[0]));
  for (let index = 1; index < fixture.batches.length; index += 1) raw(viewer.publishStreamBatch(index, fixture.batches[index]));
  const diagnostics = raw(viewer.completeStream());
  requireCondition(diagnostics.streaming.phase === "complete", "authored Source publication did not complete");
  return diagnostics.streaming.generation;
}

export function configureLodCamera(viewer, camera) {
  const args = [...camera.eye, ...camera.target, ...camera.up,
    camera.projection === "perspective" ? camera.vertical_field_of_view_radians : camera.vertical_world_height,
    camera.near_distance, camera.far_distance];
  const diagnostics = raw(camera.projection === "perspective" ? viewer.setPerspectiveCamera(...args) : viewer.setOrthographicCamera(...args));
  return diagnostics.camera;
}

export function setLodCut(viewer, { generation, version, seed, step, coarsen = false }) {
  for (let index = 0; index < 3; index += 1) {
    const incoming = coarsen ? index === 0 : index !== 0;
    raw(viewer.setVisualBatchRasterTransition(index, String(generation), String(version), seed, incoming ? 1 : 2, step));
  }
}

export function restoreLodBatch(viewer, index, generation, version) {
  raw(viewer.setVisualBatchRasterTransition(index, String(generation), String(version), 0, 0, 8));
}

export async function captureLodFrame(viewer, profile) {
  const diagnostics = raw(viewer.diagnostics());
  const captured = await captureCanonicalFrame(viewer, { width: profile.physical_width, height: profile.physical_height });
  const { image, ...capture } = captured;
  requireCondition(capture.facts.renderer_transient_texture_bytes <= 67_108_864, "renderer transient ceiling exceeded");
  requireCondition(capture.facts.point_footprint.nominal_pick_size_physical_pixels === 7, "nominal pick diameter changed");
  return { image, record: { capture, camera: diagnostics.camera, viewport: diagnostics.viewport,
    source: diagnostics.streaming, highlights: diagnostics.highlights, adapter: diagnostics.capabilities,
    limits: diagnostics.limits } };
}

export async function quietLodFrames(viewer, count) {
  const intervals = [];
  const submissions = [];
  let previous = performance.now();
  for (let index = 0; index < count; index += 1) {
    await animationFrame();
    const now = performance.now();
    intervals.push(now - previous);
    previous = now;
    const started = performance.now();
    raw(viewer.render());
    submissions.push(performance.now() - started);
  }
  return { frame_count: count, capture_free: true,
    frame_interval_samples_milliseconds: intervals, frame_submission_samples_milliseconds: submissions,
    frame_interval: summarizeSamples(intervals), frame_submission: summarizeSamples(submissions) };
}

export async function pickLodPoint(viewer, center, expected, { searchNeighbors = false } = {}) {
  const pixels = searchNeighbors
    ? [-1, 0, 1].flatMap((y) => [-1, 0, 1].map((x) => [center[0] + x, center[1] + y]))
    : [center];
  pixels.sort((first, second) => (first[0] - center[0]) ** 2 + (first[1] - center[1]) ** 2
    - (second[0] - center[0]) ** 2 - (second[1] - center[1]) ** 2 || first[1] - second[1] || first[0] - second[0]);
  const attempts = [];
  for (const pixel of pixels) {
    raw(viewer.beginPick(...pixel));
    let observed;
    let polls = 0;
    while (polls < 180) {
      await animationFrame();
      polls += 1;
      observed = raw(viewer.pollPick()).pick;
      if (observed.status !== "pending") break;
    }
    raw(viewer.cancelPick());
    const matched = observed.status === "hit" && observed.authority === "provisional_gpu_hint"
      && Object.entries(expected).every(([field, value]) => observed[field] === value);
    attempts.push({ pixel, polls, observed, matched });
    if (matched) return { center, pixel, polls, expected, observed, attempts };
  }
  throw new Error(`LOD host failed: bounded nominal pick differs: ${JSON.stringify({ expected, center, attempts })}`);
}

export function disposeLodViewer(viewer) {
  let diagnostics;
  try { diagnostics = raw(viewer.shutdown()); } finally { viewer.free(); }
  requireCondition(diagnostics.capture_resources.pending_tickets === 0, "shutdown retained a capture ticket");
  return { phase: diagnostics.phase, pending_capture_tickets: diagnostics.capture_resources.pending_tickets, freed: true };
}
