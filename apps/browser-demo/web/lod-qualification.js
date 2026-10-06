import initializeWasm from "./pkg/browser_demo.js";
import { ArtifactRegistry } from "./footprint-artifacts.js";
import { pointProjectionInput, validatePointProjectionCapture } from "./footprint-fixture.js";
import { encodeVisualArchive } from "./visual-archive.js";
import { decodeTransferV2, materializeVisualTrial, projectAuthoredPointAtViewport } from "./visual-corpus.js";
import { sha256Hex } from "./visual-png.js";
import { canonicalJsonEqual, createVisualValidator } from "./visual-validation.js";
import { measureRasterTransition } from "./lod-metrics.js";
import { animationFrame, captureLodFrame, configureLodCamera, createLodViewer, disposeLodViewer,
  pickLodPoint, publishLodSource, quietLodFrames, raw, restoreLodBatch, setLodCut } from "./lod-host.js";
import { auditLodRecord, LOD_BACKGROUND, LOD_BASELINE_PATH, LOD_BASELINE_SCHEMA,
  LOD_EVIDENCE_PATH, LOD_EVIDENCE_SCHEMA, LOD_RELEASE, LOD_ROOT, lodBoundaryCases, boundaryArgumentFacts, validateLodEnvironment } from "./lod-records.js";

const { requireCondition } = createVisualValidator("LOD qualification failed");
const jsonBytes = (value) => new TextEncoder().encode(`${JSON.stringify(value, null, 2)}\n`);
const RUNTIME_PATHS = ["package.json", "pkg/browser_demo.js", "pkg/browser_demo_bg.wasm"];

export async function runLodQualification({ mode, sessionLabel, activation, inputs, canvas, state, publishArchive }) {
  const startedAt = new Date().toISOString();
  const { corpus, fixtures } = inputs.lod;
  const [pinBundle, host, predecessorBundle, baseline] = await Promise.all([
    loadJson("./qualification-lod-pins.json"), loadJson("./qualification-host.json"),
    loadJson("./qualification-lod-predecessor.json"), mode === "verify" ? loadJson("./qualification-lod-baseline.json") : null,
  ]);
  requireCondition(pinBundle.schema === "punctra-browser-lod-pins-v1" && pinBundle.implementation_clean
    && pinBundle.implementation_dirty_paths.length === 0, "qualified implementation files are dirty");
  const pins = pinBundle.running;
  requireCondition(pins.runtime.package_version === LOD_RELEASE && pins.corpus.sha256 === inputs.lod.sha256, "runtime or corpus pin differs");
  if (mode === "verify") requireCondition(canonicalJsonEqual(pins, pinBundle.accepted)
    && canonicalJsonEqual(pins, baseline.pins), "verify runtime differs from recorded pin");
  let wasmBytes;
  for (const relativePath of RUNTIME_PATHS) {
    const response = await fetch(`./${relativePath}`, { cache: "no-store" });
    requireCondition(response.ok, `runtime ${relativePath} unavailable`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const bound = pins.runtime.artifacts.find((entry) => entry.path === `apps/browser-demo/web/${relativePath}`);
    requireCondition(bound.byte_length === bytes.byteLength && bound.sha256 === await sha256Hex(bytes), `runtime ${relativePath} digest differs`);
    if (relativePath.endsWith(".wasm")) wasmBytes = bytes;
  }
  await initializeWasm({ module_or_path: wasmBytes });
  const artifactImages = new Map();
  let encodedBytes = 0;
  const artifacts = new ArtifactRegistry(({ path, bytes, metadata }) => {
    encodedBytes += bytes.byteLength;
    requireCondition(encodedBytes <= corpus.resource_limits.encoded_archive_bytes, "encoded artifact ceiling exceeded");
    if (metadata.mime_type === "image/png") artifactImages.set(path, bytes);
  });
  const record = { schema: mode === "record" ? LOD_BASELINE_SCHEMA : LOD_EVIDENCE_SCHEMA,
    release: LOD_RELEASE, mode, pins, started_at: startedAt, session_label: sessionLabel, activation,
    environment: { browser_user_agent: navigator.userAgent, browser_platform: navigator.platform,
      host, screen: { width: screen.width, height: screen.height, color_depth_bits: screen.colorDepth },
      physical_display_observed: false },
    transitions: [], canonical: [], fallback: [], boundary_probes: [],
    external_evidence: corpus.external_evidence,
    resources: { live_canonical_images: 4, live_canonical_bytes_high_water: 4 * 1281 * 1024 * 4,
      observed_heap_bytes: null, observed_driver_memory_bytes: null, encoded_artifact_bytes: 0 },
  };
  validateLodEnvironment(record.environment);
  requireCondition(record.resources.live_canonical_bytes_high_water <= corpus.resource_limits.live_canonical_bytes, "live canonical bound exceeded");
  for (const trial of corpus.trials) for (const profile of corpus.profiles) for (let index = 0; index < 3; index += 1) {
    state(`Transition ${record.transitions.length + 1}/54 · ${trial.id} · ${profile.id} · recreation ${index + 1}`);
    record.transitions.push(await runTransition({ fixture: fixtures.get(trial.source), trial, profile, index,
      corpus, canvas, artifacts, mode }));
  }
  for (const trial of inputs.visual.corpus.trials) for (let index = 0; index < 3; index += 1) {
    state(`Canonical ${record.canonical.length + 1}/27 · ${trial.id} · recreation ${index + 1}`);
    record.canonical.push(await runCanonical({ trial, visual: inputs.visual, index, canvas, artifacts, mode }));
  }
  for (let index = 0; index < 3; index += 1) {
    state(`Resource fallback · recreation ${index + 1}/3`);
    record.fallback.push(await runFallback({ fixture: fixtures.get("orthographic-colored"), index, corpus, canvas, artifacts, mode }));
  }
  state("Probing invalid raw Wasm controls and Source generation replacement…");
  record.boundary_probes = await runBoundaryProbes({ fixture: fixtures.get("perspective-colored"), replacement: fixtures.get("orthographic-opaque"),
    profile: corpus.profiles[1], corpus, canvas, artifacts, mode });
  record.artifacts = artifacts.metadata();
  record.resources.encoded_artifact_bytes = encodedBytes;
  state("Recomputing image metrics and checking every recorded requirement…");
  record.summary = await auditLodRecord(record, { corpus, fixtures, visual: inputs.visual.corpus,
    predecessor: predecessorBundle.baseline, predecessorEvidence: predecessorBundle.evidence, baseline,
    loadImage: async (metadata) => {
      const { decodeRgba8Png } = await import("./visual-png.js");
      return decodeRgba8Png(artifactImages.get(metadata.path));
    } });
  requireCondition(document.visibilityState === "visible", "attended page became hidden");
  record.completed_at = new Date().toISOString();
  if (mode === "verify") {
    const response = await fetch("./qualification-lod-baseline.json", { cache: "no-store" });
    const bytes = new Uint8Array(await response.arrayBuffer());
    record.baseline = { path: LOD_BASELINE_PATH, byte_length: bytes.byteLength, sha256: await sha256Hex(bytes) };
  }
  const bytes = jsonBytes(record);
  requireCondition(bytes.byteLength <= corpus.resource_limits.evidence_json_bytes, "evidence JSON ceiling exceeded");
  await artifacts.addBytes(mode === "record" ? LOD_BASELINE_PATH : LOD_EVIDENCE_PATH, bytes, "lod_record");
  const archive = encodeVisualArchive(artifacts.entries(), { maximumEntries: corpus.resource_limits.archive_entries,
    maximumArchiveBytes: corpus.resource_limits.encoded_archive_bytes });
  const receipt = await publishArchive(archive.bytes, await sha256Hex(archive.bytes));
  return { record, archive, receipt };
}

async function runTransition({ fixture, trial, profile, index, corpus, canvas, artifacts, mode }) {
  const started = performance.now();
  let viewer = await createLodViewer(canvas, profile);
  try {
    const generation = publishLodSource(viewer, fixture);
    raw(viewer.setDisplayMode("rgb"));
    const control = { generation, version: 1, seed: corpus.seed, step: 0, coarsen: trial.kind === "coarsen" };
    setLodCut(viewer, control);
    const frames = [];
    let previous;
    let firstCoverage;
    const finalStep = trial.kind === "interruption" ? 4 : 8;
    for (let step = 0; step <= finalStep; step += 1) {
      const camera = cameraAtStep(fixture.camera, trial.kind, step);
      configureLodCamera(viewer, camera);
      control.step = step;
      setLodCut(viewer, control);
      await animationFrame();
      raw(viewer.render());
      if (step === 0) firstCoverage = performance.now() - started;
      const projected = projectAuthoredPointAtViewport(decodeTransferV2(fixture.batches[0]).find((point) => point.ordinal === 40),
        fixture.world_origin, camera, profile);
      const nominalPick = await pickLodPoint(viewer, [projected.x, projected.y], { source_identity: fixture.source_identity,
        generation, batch_key: 1, batch_version: 1, point_ordinal: "40" });
      const prefix = `${LOD_ROOT}/${mode}/${trial.id}/${profile.id}/r${index}/step${step}`;
      const result = await capturePairedEndpoints(viewer, profile, control, artifacts, prefix);
      const metrics = measureRasterTransition({ ...result.images, previous,
        backgroundRgba: LOD_BACKGROUND, stationary: trial.stationary });
      requireCondition(metrics.passed, `${trial.id}/${profile.id}/${step}: ${metrics.failures.join(", ")}`);
      let maximumOpaqueDelta = null;
      if (trial.kind === "opaque_coincident") {
        maximumOpaqueDelta = 0;
        for (let offset = 0; offset < result.images.candidate.data.length; offset += 1) maximumOpaqueDelta = Math.max(maximumOpaqueDelta,
          Math.abs(result.images.candidate.data[offset] - result.images.outgoing.data[offset]));
        requireCondition(maximumOpaqueDelta <= 1, "opaque midpoint leaked background");
      }
      frames.push({ step, ...result.records, metrics, maximum_opaque_delta_bytes: maximumOpaqueDelta, nominal_pick: nominalPick });
      previous = result.images.candidate;
    }
    const retiring = control.coarsen ? [1, 2] : [0];
    const retaining = control.coarsen ? [0] : [1, 2];
    for (const batch of retiring) raw(viewer.removeVisualBatch(batch));
    for (const batch of retaining) restoreLodBatch(viewer, batch, generation, 1);
    raw(viewer.render());
    const settled = performance.now() - started;
    const quietTiming = await quietLodFrames(viewer, corpus.quiet_frames);
    const cleanup = await captureAndBind(viewer, profile, artifacts, `${LOD_ROOT}/${mode}/${trial.id}/${profile.id}/r${index}/cleanup.png`, "lod_cleanup_png");
    const disposal = disposeLodViewer(viewer);
    viewer = null;
    return { trial_id: trial.id, profile, recreation_index: index, fixture_input: fixtureInput(fixture),
      first_coverage_milliseconds: firstCoverage, settled_view_milliseconds: settled,
      presented_transition_frames: finalStep, interrupted_at_step: trial.kind === "interruption" ? 4 : null,
      frames, quiet_timing: quietTiming, cleanup: cleanup.record, disposal };
  } finally { if (viewer) disposeLodViewer(viewer); }
}

async function capturePairedEndpoints(viewer, profile, control, artifacts, prefix) {
  const candidate = await captureAndBind(viewer, profile, artifacts, `${prefix}-candidate.png`, "lod_candidate_png");
  const diameter = candidate.record.capture.facts.point_footprint.display_size_physical_pixels;
  raw(viewer.setVisualCaptureDisplayDiameter(diameter));
  try {
    setLodCut(viewer, { ...control, step: 0 });
    const outgoing = await captureAndBind(viewer, profile, artifacts, `${prefix}-outgoing.png`, "lod_endpoint_png");
    setLodCut(viewer, { ...control, step: 8 });
    const incoming = await captureAndBind(viewer, profile, artifacts, `${prefix}-incoming.png`, "lod_endpoint_png");
    return { images: { candidate: candidate.image, outgoing: outgoing.image, incoming: incoming.image },
      records: { candidate: candidate.record, outgoing: outgoing.record, incoming: incoming.record } };
  } finally {
    setLodCut(viewer, control);
    raw(viewer.clearVisualCaptureDisplayDiameter());
  }
}

async function runCanonical({ trial, visual, index, canvas, artifacts, mode }) {
  const profile = { id: "canonical-dpr2", css_width: 320, css_height: 240,
    requested_device_pixel_ratio: 2, physical_width: 640, physical_height: 480 };
  const fixture = await materializeVisualTrial(visual.corpus, trial.id, { corpusUrl: visual.corpus_url });
  const started = performance.now();
  let viewer = await createLodViewer(canvas, profile);
  try {
    raw(viewer.beginStreamBatch(fixture.source_identity, fixture.point_count, ...fixture.world_origin,
      ...fixture.source_z_range, 0, fixture.batches[0]));
    raw(viewer.render());
    const firstCoverage = performance.now() - started;
    for (let batch = 1; batch < fixture.batches.length; batch += 1) raw(viewer.publishStreamBatch(batch, fixture.batches[batch]));
    raw(viewer.completeStream());
    configureLodCamera(viewer, fixture.camera);
    raw(viewer.setDisplayMode(trial.display_mode));
    if (trial.temporal_trace.kind === "mixed_lod_parent_child") {
      raw(viewer.setVisualBatchPresentation(trial.temporal_trace.parent_batch_index, 0));
      raw(viewer.setVisualBatchPresentation(trial.temporal_trace.child_batch_index, 255));
      if (trial.temporal_trace.remove_parent_after_transition) raw(viewer.removeVisualBatch(trial.temporal_trace.parent_batch_index));
    } else for (const batch of fixture.source.expected_view.settled_removed_batch_indices) raw(viewer.removeVisualBatch(batch));
    raw(viewer.render());
    const nominalPicks = [];
    const points = fixture.batches.flatMap((bytes) => decodeTransferV2(bytes));
    for (const ordinal of trial.selection.ordinals) {
      const point = points.find((entry) => entry.ordinal === ordinal);
      const projection = projectAuthoredPointAtViewport(point, fixture.world_origin, fixture.camera, profile);
      const batchIndex = fixture.batches.findIndex((bytes) => decodeTransferV2(bytes).some((entry) => entry.ordinal === ordinal));
      nominalPicks.push(await pickLodPoint(viewer, [projection.x, projection.y], { source_identity: fixture.source_identity,
        generation: 1, batch_key: fixture.source.expected_view.batch_keys[batchIndex],
        batch_version: trial.expected_settled_batch_versions[batchIndex], point_ordinal: String(ordinal) }, { searchNeighbors: true }));
    }
    if (trial.selection.ordinals.length) raw(viewer.setHighlights(fixture.source_identity, 1n, new BigUint64Array(trial.selection.ordinals.map(BigInt))));
    const settled = performance.now() - started;
    const quietTiming = await quietLodFrames(viewer, 30);
    const capture = await captureAndBind(viewer, profile, artifacts, `${LOD_ROOT}/${mode}/canonical/${trial.id}-r${index}.png`, "lod_canonical_png");
    const projectionInput = await pointProjectionInput(fixture, trial, profile);
    validatePointProjectionCapture(projectionInput, raw(viewer.diagnostics()), capture.record.capture.facts);
    const disposal = disposeLodViewer(viewer);
    viewer = null;
    return { trial_id: trial.id, recreation_index: index, profile, projection_input: projectionInput, nominal_picks: nominalPicks,
      first_coverage_milliseconds: firstCoverage, settled_view_milliseconds: settled, quiet_timing: quietTiming,
      capture: capture.record, disposal };
  } finally { if (viewer) disposeLodViewer(viewer); }
}

async function runFallback({ fixture, index, corpus, canvas, artifacts, mode }) {
  const profile = { id: "resource-fallback", css_width: 1281, css_height: 1024, requested_device_pixel_ratio: 1,
    physical_width: 1281, physical_height: 1024 };
  let viewer = await createLodViewer(canvas, profile);
  try {
    const generation = publishLodSource(viewer, fixture);
    raw(viewer.setDisplayMode("rgb"));
    configureLodCamera(viewer, fixture.camera);
    const control = { generation, version: 1, seed: corpus.seed, step: 4 };
    setLodCut(viewer, control);
    raw(viewer.render());
    const nominalPick = await pickLodPoint(viewer, [640, 512], { source_identity: fixture.source_identity,
      generation, batch_key: 1, batch_version: 1, point_ordinal: "40" });
    const result = await capturePairedEndpoints(viewer, profile, control, artifacts, `${LOD_ROOT}/${mode}/fallback/r${index}`);
    const metrics = measureRasterTransition({ ...result.images, backgroundRgba: LOD_BACKGROUND, stationary: true });
    const disposal = disposeLodViewer(viewer);
    viewer = null;
    return { recreation_index: index, profile, ...result.records, nominal_pick: nominalPick, metrics, disposal };
  } finally { if (viewer) disposeLodViewer(viewer); }
}

async function runBoundaryProbes({ fixture, replacement, profile, corpus, canvas, artifacts, mode }) {
  let viewer = await createLodViewer(canvas, profile);
  try {
    const generation = publishLodSource(viewer, fixture);
    raw(viewer.setDisplayMode("rgb"));
    configureLodCamera(viewer, fixture.camera);
    setLodCut(viewer, { generation, version: 1, seed: corpus.seed, step: 4 });
    raw(viewer.render());
    const before = await captureAndBind(viewer, profile, artifacts, `${LOD_ROOT}/${mode}/boundary/before.png`, "lod_boundary_png");
    const probes = lodBoundaryCases().slice(0, -1);
    const records = [];
    for (const [id, args] of probes) records.push(await boundaryProbe(viewer, profile, artifacts, mode, id, args, before));
    const nextGeneration = publishLodSource(viewer, replacement);
    raw(viewer.setDisplayMode("rgb"));
    configureLodCamera(viewer, replacement.camera);
    raw(viewer.render());
    requireCondition(nextGeneration === 2, "Source replacement did not advance generation");
    const replaced = await captureAndBind(viewer, profile, artifacts, `${LOD_ROOT}/${mode}/boundary/replaced.png`, "lod_boundary_png");
    records.push(await boundaryProbe(viewer, profile, artifacts, mode, "old-generation-after-replacement", [0, "1", "1", 9029, 1, 4], replaced));
    disposeLodViewer(viewer);
    viewer = null;
    return records;
  } finally { if (viewer) disposeLodViewer(viewer); }
}

async function boundaryProbe(viewer, profile, artifacts, mode, id, args, before) {
  let rejection = null;
  try { viewer.setVisualBatchRasterTransition(...args); } catch (error) { rejection = String(error); }
  requireCondition(rejection !== null, `invalid raw ABI ${id} was accepted`);
  const after = await captureAndBind(viewer, profile, artifacts, `${LOD_ROOT}/${mode}/boundary/${id}.png`, "lod_boundary_png");
  requireCondition(before.record.artifact.decoded_sha256 === after.record.artifact.decoded_sha256, `invalid ${id} changed pixels`);
  const controls = (frame) => frame.record.capture.facts.raster_transitions ?? [];
  requireCondition(canonicalJsonEqual(controls(before), controls(after)), `invalid ${id} changed controls`);
  return { id, arguments: boundaryArgumentFacts(args),
    rejected: true, rejection, before: before.record, after: after.record,
    before_decoded_sha256: before.record.artifact.decoded_sha256, after_decoded_sha256: after.record.artifact.decoded_sha256,
    before_controls: controls(before), after_controls: controls(after) };
}

async function captureAndBind(viewer, profile, artifacts, path, kind) {
  const result = await captureLodFrame(viewer, profile);
  const artifact = await artifacts.addPng(result.image, { path, kind, trial_id: null, recreation_index: null, frame_index: null });
  return { image: result.image, record: { ...result.record, artifact: artifact.metadata } };
}

function cameraAtStep(camera, kind, step) {
  const result = structuredClone(camera);
  if (kind === "moving_refine" || kind === "interruption") {
    result.eye[0] += step * 0.015;
    result.target[0] += step * 0.015;
  }
  return result;
}

function fixtureInput(fixture) {
  return Object.fromEntries(["recipe", "fixture_authority", "source_identity", "payload_sha256", "world_origin", "source_z_range",
    "point_count", "batch_point_counts", "camera"].map((field) => [field, fixture[field]]));
}

async function loadJson(url) {
  const response = await fetch(url, { cache: "no-store", credentials: "same-origin" });
  requireCondition(response.ok, `${url} returned HTTP ${response.status}`);
  return response.json();
}
