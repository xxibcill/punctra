import { measureRasterTransition, rasterDensityPointCount } from "./lod-metrics.js";
import { summarizeSamples } from "./visual-capture.js";
import { canonicalJsonEqual, createVisualValidator } from "./visual-validation.js";

const { requireCondition } = createVisualValidator("LOD evidence invalid");
export const LOD_RELEASE = "0.23.0-alpha.1";
export const LOD_BASELINE_SCHEMA = "punctra-browser-lod-baseline-v1";
export const LOD_EVIDENCE_SCHEMA = "punctra-browser-lod-evidence-v1";
export const LOD_BACKGROUND = Object.freeze([19, 20, 19, 255]);
export const LOD_ROOT = "docs/releases/v0.23-browser-lod-artifacts";
export const LOD_BASELINE_PATH = "docs/releases/v0.23-browser-lod-baseline.json";
export const LOD_EVIDENCE_PATH = "docs/releases/v0.23-browser-lod-evidence.json";

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
  requireCondition(facts.renderer_transient_texture_bytes <= 67_108_864, "renderer resource ceiling exceeded");
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
export async function auditLodRecord(record, { corpus, fixtures, visual, predecessor, predecessorEvidence, loadImage, baseline = null }) {
  requireCondition(record.release === LOD_RELEASE && [LOD_BASELINE_SCHEMA, LOD_EVIDENCE_SCHEMA].includes(record.schema), "release schema differs");
  requireCondition(canonicalJsonEqual(record.external_evidence, corpus.external_evidence), "external evidence boundary differs");
  requireCondition(record.activation?.trusted_user_activation === true && record.activation.page_visibility === "visible", "attended activation is absent");
  requireCondition(record.environment?.physical_display_observed === false && typeof record.environment.browser_user_agent === "string", "environment boundary differs");
  const expectedRuns = corpus.trials.flatMap((trial) => corpus.profiles.flatMap((profile) => [0, 1, 2].map((index) => `${trial.id}/${profile.id}/${index}`)));
  requireCondition(canonicalJsonEqual(record.transitions.map((run) => `${run.trial_id}/${run.profile.id}/${run.recreation_index}`), expectedRuns), "transition matrix differs");
  let frames = 0;
  let maximumTemporalRmse = 0;
  let maximumChangedCommonFraction = 0;
  let maximumRendererBytes = 0;
  for (const run of record.transitions) {
    const trial = corpus.trials.find(({ id }) => id === run.trial_id);
    const fixture = fixtures.get(trial.source);
    const profile = corpus.profiles.find(({ id }) => id === run.profile.id);
    requireCondition(canonicalJsonEqual(profile, run.profile), "profile differs");
    const coarsen = trial.kind === "coarsen";
    const finalStep = trial.kind === "interruption" ? 4 : 8;
    requireCondition(run.frames.length === finalStep + 1, "presented step count differs");
    let previous;
    for (let step = 0; step <= finalStep; step += 1) {
      const frame = run.frames[step];
      requireCondition(frame.step === step, "presented progress differs");
      validateLodCapture(frame.candidate, { fixture, profile, generation: 1, version: 2, seed: corpus.seed, step, coarsen });
      validateLodCapture(frame.outgoing, { fixture, profile, generation: 1, version: 2, seed: corpus.seed, step: 0, coarsen });
      validateLodCapture(frame.incoming, { fixture, profile, generation: 1, version: 2, seed: corpus.seed, step: 8, coarsen });
      requireCondition(canonicalJsonEqual(frame.candidate.camera, frame.outgoing.camera)
        && canonicalJsonEqual(frame.candidate.camera, frame.incoming.camera), "endpoint cameras differ");
      requireCondition(frame.candidate.capture.facts.point_footprint.display_size_physical_pixels === frame.outgoing.capture.facts.point_footprint.display_size_physical_pixels
        && frame.candidate.capture.facts.point_footprint.display_size_physical_pixels === frame.incoming.capture.facts.point_footprint.display_size_physical_pixels, "endpoint diameters differ");
      const [candidate, outgoing, incoming] = await Promise.all([loadImage(frame.candidate.artifact), loadImage(frame.outgoing.artifact), loadImage(frame.incoming.artifact)]);
      const metrics = measureRasterTransition({ candidate, outgoing, incoming, previous, backgroundRgba: LOD_BACKGROUND, stationary: trial.stationary });
      requireCondition(canonicalJsonEqual(frame.metrics, metrics) && metrics.passed, `recomputed ${run.trial_id}/${profile.id}/${step} image gates fail: ${metrics.failures.join(", ")}`);
      if (trial.kind === "opaque_coincident") {
        let delta = 0;
        for (let offset = 0; offset < candidate.data.length; offset += 1) delta = Math.max(delta, Math.abs(candidate.data[offset] - outgoing.data[offset]));
        requireCondition(delta <= 1 && frame.maximum_opaque_delta_bytes === delta, "opaque coincident image leaks background");
      } else requireCondition(frame.maximum_opaque_delta_bytes === null, "unmeasured opaque claim differs");
      requireCondition(frame.nominal_pick.observed.point_ordinal === "40" && frame.nominal_pick.observed.batch_key === 1
        && frame.nominal_pick.observed.batch_version === 2 && frame.nominal_pick.observed.generation === 1
        && frame.nominal_pick.observed.source_identity === fixture.source_identity
        && frame.nominal_pick.observed.authority === "provisional_gpu_hint" && frame.nominal_pick.observed.status === "hit", "mask changed nominal pick identity");
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
    requireCondition(run.first_coverage_milliseconds <= corpus.timing_limits.first_coverage_milliseconds
      && run.settled_view_milliseconds <= corpus.timing_limits.settled_view_milliseconds, "lifecycle timing ceiling exceeded");
    const cleanup = run.cleanup.capture.facts;
    requireCondition((cleanup.raster_transition_batches ?? 0) === 0 && (cleanup.raster_transitions ?? []).length === 0
      && cleanup.resident_bytes === 81 * 24 && cleanup.drawn_points === 81
      && canonicalJsonEqual(cleanup.batches.map((batch) => batch.batch_index), coarsen ? [0] : [1, 2]), "cleanup leaves a ghost or controlled batch");
    requireCondition(run.disposal.freed === true && run.disposal.pending_capture_tickets === 0, "viewer cleanup differs");
  }
  requireCondition(record.canonical.length === 27, "canonical recreation count differs");
  for (const trial of visual.trials) {
    const runs = record.canonical.filter((run) => run.trial_id === trial.id);
    requireCondition(canonicalJsonEqual(runs.map((run) => run.recreation_index), [0, 1, 2]), "canonical matrix differs");
    const expectedImage = predecessor.candidate_images.find((image) => image.trial_id === trial.id);
    const oldTiming = predecessorEvidence.canonical_trials.find((entry) => entry.trial_id === trial.id).recreations;
    const referenceTiming = Object.fromEntries(["frame_interval", "frame_submission"].map((field) => [field, Math.max(...oldTiming.map((entry) => entry.timing[field].p95))]));
    for (const run of runs) {
      await loadImage(run.capture.artifact);
      requireCondition(run.capture.artifact.decoded_sha256 === expectedImage.decoded_sha256, "canonical v0.22 decoded pixels differ");
      validateQuietTiming(run.quiet_timing, corpus.timing_limits, referenceTiming);
      requireCondition(run.disposal.freed && run.disposal.pending_capture_tickets === 0, "canonical disposal differs");
    }
  }
  requireCondition(record.fallback.length === 3, "fallback recreation count differs");
  for (const run of record.fallback) {
    const fixture = fixtures.get("orthographic-colored");
    for (const [role, step] of [["candidate", 4], ["outgoing", 0], ["incoming", 8]]) {
      validateLodCapture(run[role], { fixture, profile: run.profile, generation: 1, version: 2, seed: corpus.seed, step, coarsen: false });
    }
    const [candidate, outgoing, incoming] = await Promise.all([loadImage(run.candidate.artifact), loadImage(run.outgoing.artifact), loadImage(run.incoming.artifact)]);
    const metrics = measureRasterTransition({ candidate, outgoing, incoming, backgroundRgba: LOD_BACKGROUND, stationary: true });
    requireCondition(metrics.passed && canonicalJsonEqual(metrics, run.metrics), "fallback coverage gate fails");
    requireCondition(run.disposal.freed && run.disposal.pending_capture_tickets === 0, "fallback disposal differs");
    maximumRendererBytes = Math.max(maximumRendererBytes, run.candidate.capture.facts.renderer_transient_texture_bytes);
  }
  requireCondition(record.boundary_probes.length >= 12 && record.boundary_probes.every((probe) => probe.rejected
    && probe.before_decoded_sha256 === probe.after_decoded_sha256 && canonicalJsonEqual(probe.before_controls, probe.after_controls)), "invalid raw ABI controls mutated renderer state");
  return { passed: true, transition_recreations: expectedRuns.length, transition_frames: frames,
    canonical_recreations: record.canonical.length, fallback_recreations: record.fallback.length,
    raw_boundary_probes: record.boundary_probes.length, maximum_temporal_rmse: maximumTemporalRmse,
    maximum_changed_common_fraction: maximumChangedCommonFraction, maximum_renderer_transient_bytes: maximumRendererBytes };
}
