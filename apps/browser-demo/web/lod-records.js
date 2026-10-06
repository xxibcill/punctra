import { measureRasterTransition, rasterDensityPointCount } from "./lod-metrics.js";
import { summarizeSamples } from "./visual-capture.js";
import { canonicalJsonEqual, createVisualValidator } from "./visual-validation.js";
import { QUALIFICATION_LANE } from "./qualification-lane-v0.23.js";
import { pointProjectionInput, validatePointProjectionCapture } from "./footprint-fixture.js";
import { decodeTransferV2, materializeVisualTrial, projectAuthoredPointAtViewport } from "./visual-corpus.js";

const { requireCondition } = createVisualValidator("LOD evidence invalid");
export const LOD_RELEASE = "0.23.0-alpha.1";
export const LOD_BASELINE_SCHEMA = "punctra-browser-lod-baseline-v1";
export const LOD_EVIDENCE_SCHEMA = "punctra-browser-lod-evidence-v1";
export const LOD_BACKGROUND = Object.freeze([19, 20, 19, 255]);
export const LOD_ROOT = "docs/releases/v0.23-browser-lod-artifacts";
export const LOD_BASELINE_PATH = "docs/releases/v0.23-browser-lod-baseline.json";
export const LOD_EVIDENCE_PATH = "docs/releases/v0.23-browser-lod-evidence.json";

export function lodBoundaryCases() {
  return [
    ["side-u8-alias", [0, "1", "1", 9029, 257, 4]], ["step-u8-alias", [0, "1", "1", 9029, 1, 256]],
    ["side-fraction", [0, "1", "1", 9029, 1.5, 4]], ["step-fraction", [0, "1", "1", 9029, 1, 4.5]],
    ["step-nan", [0, "1", "1", 9029, 1, NaN]], ["seed-infinity", [0, "1", "1", Infinity, 1, 4]],
    ["seed-u32-overflow", [0, "1", "1", 4294967296, 1, 4]], ["index-u32-overflow", [4294967296, "1", "1", 9029, 1, 4]],
    ["generation-noncanonical", [0, "01", "1", 9029, 1, 4]], ["version-noncanonical", [0, "1", "01", 9029, 1, 4]],
    ["generation-stale", [0, "0", "1", 9029, 1, 4]], ["version-stale", [0, "1", "0", 9029, 1, 4]],
    ["key-missing", [99, "1", "1", 9029, 1, 4]], ["step-negative", [0, "1", "1", 9029, 1, -1]],
    ["full-invalid", [0, "1", "1", 9029, 0, 4]],
    ["old-generation-after-replacement", [0, "1", "1", 9029, 1, 4]],
  ];
}

export const boundaryArgumentFacts = (args) => args.map((value) => typeof value === "number" && !Number.isFinite(value) ? String(value) : value);

export function validateLodBoundaryMatrix(probes) {
  requireCondition(canonicalJsonEqual(probes.map(({ id, arguments: args }) => [id, args]),
    lodBoundaryCases().map(([id, args]) => [id, boundaryArgumentFacts(args)])), "raw boundary probe matrix differs");
}

export function validateLodArtifact(record, { path, kind, profile }) {
  requireCondition(record.artifact.path === path && record.artifact.kind === kind, "captured artifact role or path differs");
  requireCondition(record.artifact.width === profile.physical_width && record.artifact.height === profile.physical_height
    && record.artifact.decoded_byte_length === profile.physical_width * profile.physical_height * 4
    && record.capture.facts.width === profile.physical_width && record.capture.facts.height === profile.physical_height, "artifact and capture dimensions differ");
}

export function validateLodCapture(record, { fixture, profile, generation, version, step, seed, coarsen }) {
  const facts = record.capture.facts;
  requireCondition(facts.width === profile.physical_width && facts.height === profile.physical_height, "capture dimensions differ");
  requireCondition(facts.view_generation === generation && facts.resident_bytes === fixture.point_count * 24
    && facts.drawn_points === fixture.point_count && facts.draw_calls === 3, "resident accounting differs");
  requireCondition(record.source.source_identity === fixture.source_identity
    && canonicalJsonEqual(record.source.world_origin, fixture.world_origin)
    && record.source.generation === generation && record.source.phase === "complete", "Source binding differs");
  requireCondition(facts.batches.length === 3 && facts.batches.every((batch, index) => batch.batch_index === index
    && batch.key === index + 1 && batch.version === version && batch.state === "resident"
    && batch.presentation_weight_u8 === 255 && batch.point_count === fixture.batch_point_counts[index]), "resident batch identity differs");
  const expected = facts.batches.map((batch, index) => {
    const incoming = coarsen ? index === 0 : index !== 0;
    return { batch_index: index, key: batch.key, version, generation, seed, step,
      side: incoming ? "incoming" : "outgoing", coverage_eighths: incoming ? step : 8 - step };
  });
  requireCondition(canonicalJsonEqual(facts.raster_transitions, expected) && facts.raster_transition_batches === 3,
    "actual renderer raster controls differ");
  const density = rasterDensityPointCount(expected.map((control, index) => ({ ...control, point_count: fixture.batch_point_counts[index] })));
  requireCondition(density === 81, "paired cut density pulses");
  const diameter = facts.point_footprint.display_size_physical_pixels;
  requireCondition(diameter >= 2 && diameter <= 6 && facts.point_footprint.nominal_pick_size_physical_pixels === 7, "display or pick diameter differs");
  requireCondition(Number.isSafeInteger(facts.renderer_transient_texture_bytes) && facts.renderer_transient_texture_bytes >= 0
    && facts.renderer_transient_texture_bytes <= 67_108_864, "renderer resource ceiling exceeded");
  const expectedStatus = profile.physical_width * profile.physical_height > 1_310_720 ? "resource_fallback" : "multisample4x";
  requireCondition(facts.point_footprint.selected === expectedStatus, "footprint disposition differs");
  return density;
}

export function validateQuietTiming(timing, limits, predecessor = null) {
  requireCondition(timing.frame_count === 30 && timing.capture_free === true, "quiet window differs");
  for (const [samples, summary] of [["frame_interval_samples_milliseconds", "frame_interval"], ["frame_submission_samples_milliseconds", "frame_submission"]]) {
    requireCondition(timing[samples]?.length === 30 && timing[samples].every((value) => Number.isFinite(value) && value >= 0), "quiet samples differ");
    requireCondition(canonicalJsonEqual(timing[summary], summarizeSamples(timing[samples])), "quiet summary differs from samples");
  }
  requireCondition(timing.frame_interval.p95 <= limits.frame_callback_p95_milliseconds
    && timing.frame_submission.p95 <= limits.frame_submission_p95_milliseconds, "quiet timing ceiling exceeded");
  if (predecessor !== null) {
    for (const field of ["frame_interval", "frame_submission"]) {
      if (predecessor[field] > 0) requireCondition(timing[field].p95 <= predecessor[field] * limits.canonical_predecessor_p95_ratio, `canonical ${field} p95 exceeds twice the predecessor`);
    }
  }
}

/** Recomputes metrics from decoded artifacts; recorded pass flags are never inputs. */
export async function auditLodRecord(record, { corpus, fixtures, visual, predecessor, predecessorEvidence, loadImage, baseline = null, visualOptions = {} }) {
  requireCondition(record.release === LOD_RELEASE && [LOD_BASELINE_SCHEMA, LOD_EVIDENCE_SCHEMA].includes(record.schema), "release schema differs");
  requireCondition(canonicalJsonEqual(record.external_evidence, corpus.external_evidence), "external evidence boundary differs");
  requireCondition(record.activation?.trusted_user_activation === true && record.activation.page_visibility === "visible", "attended activation is absent");
  validateLodEnvironment(record.environment);
  const expectedRuns = corpus.trials.flatMap((trial) => corpus.profiles.flatMap((profile) => [0, 1, 2].map((index) => `${trial.id}/${profile.id}/${index}`)));
  requireCondition(canonicalJsonEqual(record.transitions.map((run) => `${run.trial_id}/${run.profile.id}/${run.recreation_index}`), expectedRuns), "transition matrix differs");
  let frames = 0;
  let maximumTemporalRmse = 0;
  let maximumChangedCommonFraction = 0;
  let maximumRendererBytes = 0;
  for (const run of record.transitions) {
    const trial = corpus.trials.find(({ id }) => id === run.trial_id);
    const fixture = fixtures.get(trial.source);
    requireCondition(canonicalJsonEqual(Object.keys(run.fixture_input).sort(), ["recipe", "fixture_authority", "source_identity", "payload_sha256", "world_origin", "source_z_range", "point_count", "batch_point_counts", "camera"].sort()), "fixture input closure differs");
    for (const field of Object.keys(run.fixture_input)) requireCondition(canonicalJsonEqual(run.fixture_input[field], fixture[field]), `authored fixture ${field} differs`);
    const profile = corpus.profiles.find(({ id }) => id === run.profile.id);
    requireCondition(canonicalJsonEqual(profile, run.profile), "profile differs");
    const coarsen = trial.kind === "coarsen";
    const finalStep = trial.kind === "interruption" ? 4 : 8;
    requireCondition(run.frames.length === finalStep + 1, "presented step count differs");
    let previous;
    for (let step = 0; step <= finalStep; step += 1) {
      const frame = run.frames[step];
      requireCondition(frame.step === step, "presented progress differs");
      for (const role of ["candidate", "outgoing", "incoming"]) validateLodArtifact(frame[role], {
        path: `${LOD_ROOT}/${record.mode}/${trial.id}/${profile.id}/r${run.recreation_index}/step${step}-${role}.png`,
        kind: role === "candidate" ? "lod_candidate_png" : "lod_endpoint_png", profile });
      validateLodCapture(frame.candidate, { fixture, profile, generation: 1, version: 1, seed: corpus.seed, step, coarsen });
      validateLodCapture(frame.outgoing, { fixture, profile, generation: 1, version: 1, seed: corpus.seed, step: 0, coarsen });
      validateLodCapture(frame.incoming, { fixture, profile, generation: 1, version: 1, seed: corpus.seed, step: 8, coarsen });
      requireCondition(canonicalJsonEqual(frame.candidate.camera, frame.outgoing.camera)
        && canonicalJsonEqual(frame.candidate.camera, frame.incoming.camera), "endpoint cameras differ");
      const expectedCamera = structuredClone(fixture.camera);
      if (trial.kind === "moving_refine" || trial.kind === "interruption") {
        expectedCamera.eye[0] += step * 0.015;
        expectedCamera.target[0] += step * 0.015;
      }
      validateCameraBinding(frame.candidate.camera, expectedCamera);
      requireCondition(frame.candidate.adapter.adapter_name === QUALIFICATION_LANE.webgpu.adapter_name
        && frame.candidate.adapter.backend === QUALIFICATION_LANE.webgpu.backend, "captured adapter differs");
      requireCondition(frame.candidate.capture.facts.point_footprint.display_size_physical_pixels === frame.outgoing.capture.facts.point_footprint.display_size_physical_pixels
        && frame.candidate.capture.facts.point_footprint.display_size_physical_pixels === frame.incoming.capture.facts.point_footprint.display_size_physical_pixels, "endpoint diameters differ");
      const [candidate, outgoing, incoming] = await Promise.all([loadImage(frame.candidate.artifact), loadImage(frame.outgoing.artifact), loadImage(frame.incoming.artifact)]);
      for (const image of [candidate, outgoing, incoming]) requireCondition(image.width === profile.physical_width && image.height === profile.physical_height, "decoded transition PNG dimensions differ");
      const metrics = measureRasterTransition({ candidate, outgoing, incoming, previous, backgroundRgba: LOD_BACKGROUND, stationary: trial.stationary });
      requireCondition(canonicalJsonEqual(frame.metrics, metrics) && metrics.passed, `recomputed ${run.trial_id}/${profile.id}/${step} image gates fail: ${metrics.failures.join(", ")}`);
      if (trial.kind === "opaque_coincident") {
        let delta = 0;
        for (let offset = 0; offset < candidate.data.length; offset += 1) delta = Math.max(delta, Math.abs(candidate.data[offset] - outgoing.data[offset]));
        requireCondition(delta <= 1 && frame.maximum_opaque_delta_bytes === delta, "opaque coincident image leaks background");
      } else requireCondition(frame.maximum_opaque_delta_bytes === null, "unmeasured opaque claim differs");
      requireCondition(frame.nominal_pick.observed.point_ordinal === "40" && frame.nominal_pick.observed.batch_key === 1
        && frame.nominal_pick.observed.batch_version === 1 && frame.nominal_pick.observed.generation === 1
        && frame.nominal_pick.observed.source_identity === fixture.source_identity
        && frame.nominal_pick.observed.authority === "provisional_gpu_hint" && frame.nominal_pick.observed.status === "hit", "mask changed nominal pick identity");
      const projected = projectAuthoredPointAtViewport(decodeTransferV2(fixture.batches[0]).find((point) => point.ordinal === 40), fixture.world_origin, expectedCamera, profile);
      requireCondition(canonicalJsonEqual(frame.nominal_pick.pixel, [projected.x, projected.y]), "nominal pick pixel differs from current camera projection");
      if (baseline) {
        const reference = baseline.transitions.find((entry) => entry.trial_id === run.trial_id && entry.profile.id === profile.id && entry.recreation_index === run.recreation_index).frames[step];
        requireCondition(frame.candidate.artifact.decoded_sha256 === reference.candidate.artifact.decoded_sha256, "separate verify candidate pixels differ from recorded baseline");
      }
      previous = candidate;
      frames += 1;
      maximumTemporalRmse = Math.max(maximumTemporalRmse, metrics.temporal_rmse ?? 0);
      maximumChangedCommonFraction = Math.max(maximumChangedCommonFraction,
        trial.stationary ? (metrics.changed_common_pixels ?? 0) / metrics.common_foreground_pixels : 0);
      maximumRendererBytes = Math.max(maximumRendererBytes, frame.candidate.capture.facts.renderer_transient_texture_bytes);
    }
    validateQuietTiming(run.quiet_timing, corpus.timing_limits);
    validateLifecycleTiming(run, corpus.timing_limits);
    const cleanup = run.cleanup.capture.facts;
    validateLodArtifact(run.cleanup, { path: `${LOD_ROOT}/${record.mode}/${trial.id}/${profile.id}/r${run.recreation_index}/cleanup.png`,
      kind: "lod_cleanup_png", profile });
    requireCondition((cleanup.raster_transition_batches ?? 0) === 0 && (cleanup.raster_transitions ?? []).length === 0
      && cleanup.resident_bytes === 81 * 24 && cleanup.drawn_points === 81
      && canonicalJsonEqual(cleanup.batches.map((batch) => batch.batch_index), coarsen ? [0] : [1, 2]), "cleanup leaves a ghost or controlled batch");
    const cleanupImage = await loadImage(run.cleanup.artifact);
    const endpointImage = await loadImage(run.frames.at(-1).incoming.artifact);
    requireCondition(cleanupImage.data.every((value, offset) => value === endpointImage.data[offset]), "settlement differs from the retained endpoint");
    requireCondition(run.disposal.freed === true && run.disposal.pending_capture_tickets === 0, "viewer cleanup differs");
  }
  requireCondition(record.canonical.length === 27, "canonical recreation count differs");
  for (const trial of visual.trials) {
    const materialized = await materializeVisualTrial(visual, trial.id, {
      corpusUrl: globalThis.location ? new URL("./fixtures/visual-v1/corpus.json", location.href).href : "http://localhost/fixtures/visual-v1/corpus.json",
      ...visualOptions,
    });
    const runs = record.canonical.filter((run) => run.trial_id === trial.id);
    requireCondition(canonicalJsonEqual(runs.map((run) => run.recreation_index), [0, 1, 2]), "canonical matrix differs");
    const expectedImage = predecessor.candidate_images.find((image) => image.trial_id === trial.id);
    const oldTiming = predecessorEvidence.canonical_trials.find((entry) => entry.trial_id === trial.id).recreations;
    const referenceTiming = Object.fromEntries(["frame_interval", "frame_submission"].map((field) => [field, Math.max(...oldTiming.map((entry) => entry.timing[field].p95))]));
    for (const run of runs) {
      validateLodArtifact(run.capture, { path: `${LOD_ROOT}/${record.mode}/canonical/${trial.id}-r${run.recreation_index}.png`,
        kind: "lod_canonical_png", profile: { physical_width: 640, physical_height: 480 } });
      const image = await loadImage(run.capture.artifact);
      requireCondition(image.width === 640 && image.height === 480 && run.capture.capture.facts.width === 640 && run.capture.capture.facts.height === 480, "canonical PNG dimensions differ");
      requireCondition(run.capture.artifact.decoded_sha256 === expectedImage.decoded_sha256, "canonical v0.22 decoded pixels differ");
      const input = await pointProjectionInput(materialized, trial, run.profile);
      requireCondition(canonicalJsonEqual(input, run.projection_input), "canonical Source or projection input differs");
      validatePointProjectionCapture(input, { camera: run.capture.camera, viewport: run.capture.viewport,
        streaming: run.capture.source, point_footprint: run.capture.capture.facts.point_footprint,
        display_mode: run.capture.source.display_mode, highlights: run.capture.highlights }, run.capture.capture.facts);
      requireCondition(run.nominal_picks.length === trial.selection.ordinals.length, "canonical nominal pick count differs");
      for (const [index, pick] of run.nominal_picks.entries()) {
        requireCondition(pick.observed.point_ordinal === String(trial.selection.ordinals[index])
          && pick.observed.source_identity === materialized.source_identity && pick.observed.generation === 1
          && pick.observed.status === "hit" && pick.observed.authority === "provisional_gpu_hint", "canonical nominal pick differs");
      }
      validateLifecycleTiming(run, corpus.timing_limits);
      validateQuietTiming(run.quiet_timing, corpus.timing_limits, referenceTiming);
      requireCondition(run.disposal.freed && run.disposal.pending_capture_tickets === 0, "canonical disposal differs");
    }
  }
  requireCondition(record.fallback.length === 3, "fallback recreation count differs");
  requireCondition(canonicalJsonEqual(record.fallback.map((run) => run.recreation_index), [0, 1, 2]), "fallback recreations differ");
  for (const run of record.fallback) {
    const fixture = fixtures.get("orthographic-colored");
    for (const [role, step] of [["candidate", 4], ["outgoing", 0], ["incoming", 8]]) {
      validateLodArtifact(run[role], { path: `${LOD_ROOT}/${record.mode}/fallback/r${run.recreation_index}-${role}.png`,
        kind: role === "candidate" ? "lod_candidate_png" : "lod_endpoint_png", profile: run.profile });
      validateLodCapture(run[role], { fixture, profile: run.profile, generation: 1, version: 1, seed: corpus.seed, step, coarsen: false });
    }
    const [candidate, outgoing, incoming] = await Promise.all([loadImage(run.candidate.artifact), loadImage(run.outgoing.artifact), loadImage(run.incoming.artifact)]);
    requireCondition(canonicalJsonEqual(run.profile, { id: "resource-fallback", css_width: 1281, css_height: 1024,
      requested_device_pixel_ratio: 1, physical_width: 1281, physical_height: 1024 }), "fallback profile differs");
    for (const image of [candidate, outgoing, incoming]) requireCondition(image.width === 1281 && image.height === 1024, "fallback PNG dimensions differ");
    const metrics = measureRasterTransition({ candidate, outgoing, incoming, backgroundRgba: LOD_BACKGROUND, stationary: true });
    requireCondition(metrics.passed && canonicalJsonEqual(metrics, run.metrics), "fallback coverage gate fails");
    requireCondition(run.disposal.freed && run.disposal.pending_capture_tickets === 0, "fallback disposal differs");
    requireCondition(run.nominal_pick.observed.status === "hit" && run.nominal_pick.observed.point_ordinal === "40"
      && run.nominal_pick.observed.batch_key === 1 && run.nominal_pick.observed.batch_version === 1
      && run.nominal_pick.observed.generation === 1 && run.nominal_pick.observed.source_identity === fixture.source_identity
      && run.nominal_pick.observed.authority === "provisional_gpu_hint"
      && canonicalJsonEqual(run.nominal_pick.pixel, [640, 512]), "fallback nominal pick differs");
    maximumRendererBytes = Math.max(maximumRendererBytes, run.candidate.capture.facts.renderer_transient_texture_bytes);
  }
  validateLodBoundaryMatrix(record.boundary_probes);
  requireCondition(record.boundary_probes.every((probe) => probe.rejected
    && probe.before_decoded_sha256 === probe.after_decoded_sha256 && canonicalJsonEqual(probe.before_controls, probe.after_controls)), "invalid raw ABI controls mutated renderer state");
  const replacement = record.boundary_probes.at(-1);
  requireCondition(replacement.before.capture.facts.view_generation === 2 && replacement.after.capture.facts.view_generation === 2
    && replacement.before.source.source_identity === fixtures.get("orthographic-opaque").source_identity
    && replacement.after.source.source_identity === fixtures.get("orthographic-opaque").source_identity
    && replacement.before_controls.length === 0 && replacement.after_controls.length === 0, "old callback rejection did not exercise a new Source epoch");
  for (const probe of record.boundary_probes) {
    const beforeName = probe.id === "old-generation-after-replacement" ? "replaced" : "before";
    for (const [role, name] of [["before", beforeName], ["after", probe.id]]) validateLodArtifact(probe[role], {
      path: `${LOD_ROOT}/${record.mode}/boundary/${name}.png`, kind: "lod_boundary_png",
      profile: { physical_width: 640, physical_height: 480 } });
  }
  if (baseline) {
    const oldArtifacts = new Map(baseline.artifacts.map((entry) => [entry.path.replace("/record/", "/verify/"), entry]));
    requireCondition(record.artifacts.length === baseline.artifacts.length, "separate verify artifact closure differs");
    for (const artifact of record.artifacts) requireCondition(artifact.decoded_sha256 === oldArtifacts.get(artifact.path)?.decoded_sha256,
      `separate verify ${artifact.path} decoded pixels differ`);
  }
  return { passed: true, transition_recreations: expectedRuns.length, transition_frames: frames,
    canonical_recreations: record.canonical.length, fallback_recreations: record.fallback.length,
    raw_boundary_probes: record.boundary_probes.length, maximum_temporal_rmse: maximumTemporalRmse,
    maximum_changed_common_fraction: maximumChangedCommonFraction, maximum_renderer_transient_bytes: maximumRendererBytes };
}

function validateCameraBinding(observed, expected) {
  for (const field of ["eye", "target", "up", "projection", "vertical_world_height"]) {
    requireCondition(canonicalJsonEqual(observed[field], expected[field] ?? null), `observed camera ${field} differs`);
  }
  for (const field of ["vertical_field_of_view_radians", "near_distance", "far_distance"]) {
    if (expected[field] === null || expected[field] === undefined) requireCondition(observed[field] === null, `camera ${field} differs`);
    else requireCondition(Math.fround(observed[field]) === Math.fround(expected[field]), `camera ${field} differs`);
  }
}

function validateLifecycleTiming(run, limits) {
  requireCondition(Number.isFinite(run.first_coverage_milliseconds) && run.first_coverage_milliseconds >= 0
    && Number.isFinite(run.settled_view_milliseconds) && run.settled_view_milliseconds >= run.first_coverage_milliseconds
    && run.first_coverage_milliseconds <= limits.first_coverage_milliseconds
    && run.settled_view_milliseconds <= limits.settled_view_milliseconds, "lifecycle timing ceiling exceeded");
}

export function validateLodEnvironment(environment) {
  requireCondition(environment?.physical_display_observed === false && typeof environment.browser_user_agent === "string", "environment boundary differs");
  requireCondition(environment.browser_user_agent === QUALIFICATION_LANE.browser.user_agent
    && environment.browser_platform === QUALIFICATION_LANE.operating_system.user_agent_platform
    && environment.screen.width === QUALIFICATION_LANE.display.screen_css_pixels[0]
    && environment.screen.height === QUALIFICATION_LANE.display.screen_css_pixels[1]
    && environment.screen.color_depth_bits === QUALIFICATION_LANE.display.color_depth,
  `observed browser lane differs: ${JSON.stringify({ observed: environment, expected: {
    browser_user_agent: QUALIFICATION_LANE.browser.user_agent,
    browser_platform: QUALIFICATION_LANE.operating_system.user_agent_platform,
    screen_css_pixels: QUALIFICATION_LANE.display.screen_css_pixels,
    color_depth_bits: QUALIFICATION_LANE.display.color_depth } })}`);
  for (const field of ["name", "version", "build", "architecture"]) requireCondition(environment.host.operating_system[field] === QUALIFICATION_LANE.operating_system[field], `host OS ${field} differs`);
  for (const field of ["class", "gpu", "gpu_cores", "gpu_class", "metal_support"]) requireCondition(environment.host.device[field] === QUALIFICATION_LANE.device[field], `host device ${field} differs`);
  requireCondition(environment.host.package.version === LOD_RELEASE && environment.host.package.name === "@punctra/viewer", "host package differs");
}
