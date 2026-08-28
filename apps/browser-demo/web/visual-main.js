import initializeWasm, { createViewer as createRawViewer } from "./pkg/browser_demo.js";
import { encodeVisualArchive } from "./visual-archive.js";
import {
  BASELINE_INPUTS_PATH,
  createBaselineInputsManifest,
  encodeBaselineInputsManifest,
  validateBaselineInputsManifest,
} from "./visual-baseline-inputs.js";
import {
  DISPLAY_MODES,
  RUBRIC_OUTCOMES,
  RUBRIC_PROMPTS,
  VISUAL_VIEWPORT,
  loadVisualCorpus,
  materializeVisualTrial,
} from "./visual-corpus.js";
import {
  captureCanonicalFrame,
  captureVisualEnvironment,
  derivePendingWorkEvidence,
  parseRawJson,
  renderQuietFrames,
  summarizeCaptureTimingSamples,
  visualEnvironmentFingerprint,
} from "./visual-capture.js";
import {
  DIFFERENCE_IMAGE_POLICY,
  compareCanonicalImages,
  summarizeTemporalPairs,
  validateToleranceProfile,
  writeDifferenceImage,
} from "./visual-comparison.js";
import {
  createPngArtifactMetadata,
  decodeRgba8Png,
  encodeRgba8Png,
  sha256Hex,
} from "./visual-png.js";
import {
  exportVisualArchiveToLocalServer,
  visualArchiveTransportFromUrl,
} from "./visual-export.js";
import {
  RUBRIC_PRESENTATION_SCHEMA,
  artifactIdentity,
  buildRubricObservation,
  createRubricReviewPlan,
  createUnobservedRubricObservation,
} from "./visual-rubric.js";
import {
  VISUAL_ATTENDED_LANE,
  VisualTrustedControlGate,
  loadVisualVerifyProvenance,
} from "./visual-provenance.js";
import { VisualRunSession, VisualTrialRunner } from "./visual-run-session.js";
import { verifyNominalPickCoverage } from "./visual-selection.js";
import { cloneJson, createVisualValidator, errorMessage } from "./visual-validation.js";

const EVIDENCE_SCHEMA = "punctra-browser-visual-evidence-v1";
const RUNNER_STATE_SCHEMA = "punctra-browser-visual-runner-state-v1";
const CORPUS_URL = new URL("./fixtures/visual-v1/corpus.json", import.meta.url);
const RUNTIME_ARTIFACT_PATHS = Object.freeze([
  "./package.json",
  "./pkg/browser_demo.js",
  "./pkg/browser_demo_bg.wasm",
]);
const RECREATION_COUNT = 3;
const VISUAL_ARTIFACT_ROOT = "docs/releases/v0.21-browser-visual-artifacts";
const EVIDENCE_FILENAME = "v0.21-browser-visual-evidence.json";
const TRANSFER_VERTEX_BYTES = 24;
const { requireCondition } = createVisualValidator("Visual runner failed");

const canvas = document.querySelector("#visual-canvas");
const runButton = document.querySelector("#run-corpus");
const modeSelect = document.querySelector("#run-mode");
const sessionLabel = document.querySelector("#session-label");
const statusOutput = document.querySelector("#visual-status");
const progressCount = document.querySelector("#progress-count");
const progressList = document.querySelector("#trial-progress");
const evidenceOutput = document.querySelector("#evidence-output");
const artifactList = document.querySelector("#artifact-links");
const downloadEvidenceButton = document.querySelector("#download-evidence");
const downloadBundleButton = document.querySelector("#download-bundle");
const submitRubricButton = document.querySelector("#submit-rubric");
const rubricStatus = document.querySelector("#rubric-status");
const transportStatus = document.querySelector("#transport-status");
const provenanceStatus = document.querySelector("#provenance-status");

const runControlGate = new VisualTrustedControlGate();
const rubricSelectionGate = new VisualTrustedControlGate();
const rubricSubmitGate = new VisualTrustedControlGate();
let provenanceConfigurationSequence = 0;

async function startRun(options, activation) {
  const mode = validateMode(options?.mode ?? modeSelect.value);
  const runInitiation = mode === "verify" ? runControlGate.consume(activation, runButton.id) : null;
  return session.start(() => runVisualCorpus({ ...options, mode }, runInitiation));
}

async function runVisualCorpus(options, runInitiation) {
  const mode = validateMode(options?.mode ?? modeSelect.value);
  modeSelect.value = mode;
  setRunControlsEnabled(false);
  resetRubricReview();
  session.resetForRun();
  evidenceOutput.textContent = "Preparing the immutable corpus and private Wasm runtime…";
  const startedAt = new Date().toISOString();
  const record = {
    schema: EVIDENCE_SCHEMA,
    release: "0.21.0-alpha.1",
    mode,
    started_at: startedAt,
    capture_completed_at: null,
    completed_at: null,
    corpus: null,
    provenance: normalizeProvenance(options?.provenance, startedAt, mode, runInitiation),
    environment: null,
    capture_policy: null,
    presentation_policy: null,
    tolerance_profiles: null,
    baseline_inputs: null,
    trials: [],
    rubric: null,
    artifacts: [],
    artifact_resources: null,
    summary: null,
    external_evidence: externalEvidenceNonclaims(),
    fatal_error: null,
  };
  let corpus;
  let corpusUrl;
  let completedTrials = 0;
  let reviewPending = false;
  try {
    updateRunnerState({
      status: "running",
      mode,
      message: "Loading and validating the closed visual corpus…",
      completed_trials: 0,
      total_trials: 0,
    });
    const loaded = await loadVisualCorpus(CORPUS_URL);
    corpus = loaded.corpus;
    corpusUrl = loaded.corpus_url;
    session.configureTransport(corpus.transport, corpus.resource_limits.total_encoded_artifact_bytes);
    for (const profile of Object.values(corpus.tolerance_profiles)) validateToleranceProfile(profile);
    record.corpus = {
      path: "apps/browser-demo/web/fixtures/visual-v1/corpus.json",
      url: loaded.corpus_url,
      schema: corpus.schema,
      release: corpus.release,
      byte_length: loaded.corpus_byte_length,
      sha256: loaded.corpus_sha256,
    };
    record.capture_policy = cloneJson(corpus.capture);
    record.presentation_policy = cloneJson(corpus.presentation_policy);
    record.tolerance_profiles = cloneJson(corpus.tolerance_profiles);
    const runtimeArtifacts = await captureRuntimeArtifacts();
    bindWasmRuntime(runtimeArtifacts);
    record.provenance.package_artifact = {
      package_name: "@punctra/viewer",
      package_version: corpus.release,
      runtime_artifacts: runtimeArtifacts.records,
    };
    const host = await loadQualificationHost();
    requireCondition(host?.schema === "punctra-qualification-host-v1", "strict qualification host facts are unavailable");
    buildProgressList(corpus.trials);
    updateRunnerState({ total_trials: corpus.trials.length });

    const environmentTracker = createEnvironmentTracker(record, corpus, host);
    const trialRunner = new VisualTrialRunner({
      corpus,
      corpusUrl,
      mode,
      environmentTracker,
      artifacts: session.artifacts,
      materializeTrial: materializeVisualTrial,
      loadBaseline: loadExistingBaseline,
      runRecreation,
      updateRunnerState,
      repositoryBaselinePath,
      observationArtifactPath,
      buildBatchFacts: expectedBatchFacts,
      recreationCount: RECREATION_COUNT,
    });
    for (const trial of corpus.trials) {
      markTrialProgress(trial.id, "running", "running three fresh viewers");
      updateRunnerState({
        trial_id: trial.id,
        recreation_index: 0,
        message: `Running ${trial.id}…`,
      });
      let result;
      try {
        result = await trialRunner.run(trial.id);
      } catch (error) {
        result = failedTrial(trial, error);
      }
      record.trials.push(result);
      completedTrials += 1;
      markTrialProgress(
        trial.id,
        result.passed ? "passed" : "failed",
        result.passed ? "three recreations comparable" : result.failures.join("; "),
      );
      updateRunnerState({
        completed_trials: completedTrials,
        recreation_index: null,
        message: `${trial.id}: ${result.passed ? "passed" : "failed"}`,
      });
      publishPartialRecord(record);
    }

    requireCondition(record.environment !== null, "no successful viewer recreation produced environment facts");
    record.artifacts = session.artifacts.metadata();
    const failedTrials = record.trials.filter((trial) => !trial.passed);
    requireCondition(
      failedTrials.length === 0,
      `visual trials failed before baseline-input publication: ${failedTrials.map((trial) => trial.trial_id).join(", ")}`,
    );
    session.setBaselineInputsEntry(await resolveBaselineInputs({
      mode,
      corpus,
      corpusUrl,
      artifacts: record.artifacts,
      runtimeArtifacts: record.provenance.package_artifact.runtime_artifacts,
    }));
    record.baseline_inputs = {
      path: session.baselineInputsEntry.path,
      schema: session.baselineInputsEntry.manifest.schema,
      byte_length: session.baselineInputsEntry.bytes.byteLength,
      sha256: await sha256Hex(session.baselineInputsEntry.bytes),
    };
    record.artifact_resources = artifactResourceEvidence(corpus);
    record.capture_completed_at = new Date().toISOString();
    session.stageReview(record, corpus);
    await prepareRubricReview(corpus.rubric, record);
    reviewPending = true;
  } catch (error) {
    resetRubricReview();
    record.completed_at = new Date().toISOString();
    record.fatal_error = errorRecord(error);
    record.rubric ??= corpus === undefined ? null : {
      schema: corpus.rubric.schema,
      gating: false,
      review_status: "not_reached",
      observation: createUnobservedRubricObservation(corpus.rubric),
    };
    record.artifacts = session.artifacts.metadata();
    record.artifact_resources = corpus === undefined ? null : artifactResourceEvidence(corpus);
    record.summary = {
      passed: false,
      trial_count: corpus?.trials.length ?? 0,
      completed_trials: completedTrials,
      passed_trials: record.trials.filter((trial) => trial.passed).length,
      failed_trials: record.trials.filter((trial) => !trial.passed).map((trial) => trial.trial_id),
      non_gating_rubric_complete: record.rubric !== null,
      artifact_count: record.artifacts.length,
      total_encoded_artifact_bytes: session.artifacts.totalEncodedBytes(),
      failures: [`fatal:${record.fatal_error.message}`],
    };
  }
  if (reviewPending) {
    updateRunnerState({
      status: "review_pending",
      trial_id: null,
      recreation_index: null,
      message: "Captures complete. Review the exact bound images, record each outcome, then submit the attended rubric.",
    });
    evidenceOutput.textContent = JSON.stringify(record, null, 2);
    rubricStatus.textContent = "All bound images loaded after capture. Record the outcomes below, then submit the review.";
    return {
      schema: "punctra-browser-visual-review-required-v1",
      status: "review_pending",
      capture_completed_at: record.capture_completed_at,
      trial_count: record.trials.length,
      artifact_count: record.artifacts.length,
    };
  }
  session.completeRun(record);
  const passed = record.summary?.passed === true;
  updateRunnerState({
    status: passed ? "passed" : "failed",
    trial_id: null,
    recreation_index: null,
    message: passed
      ? `Passed ${record.summary.passed_trials}/${record.summary.trial_count} trials. Download and verify the evidence files.`
      : `Visual evidence failed: ${(record.summary?.failures ?? [record.fatal_error?.message]).join("; ")}`,
  });
  evidenceOutput.textContent = JSON.stringify(record, null, 2);
  const transportAvailable = session.transportPolicy !== undefined;
  downloadEvidenceButton.disabled = !transportAvailable;
  downloadBundleButton.disabled = !transportAvailable;
  setRunControlsEnabled(true);
  return cloneJson(record);
}

async function runRecreation(options) {
  const {
    corpus,
    trial,
    materialized,
    recreationIndex,
    baselinePng,
    finalArtifact,
    environmentTracker,
  } = options;
  const recreationStarted = performance.now();
  const artifactStartedIndex = session.artifacts.metadata().length;
  let rawViewer;
  let disposed = false;
  try {
    rawViewer = await createPrivateViewer();
    const initialDiagnostics = parseRawJson(rawViewer.diagnostics(), "initial visual diagnostics");
    const environment = captureVisualEnvironment({
      diagnostics: initialDiagnostics,
      canvas,
      host: environmentTracker.host,
    });
    validateCanonicalEnvironment(environment, corpus, initialDiagnostics);
    const initialEnvironmentMatch = environmentTracker.accept(environment);

    const publicationTiming = publishMaterializedSource(rawViewer, materialized, recreationStarted);
    configureCamera(rawViewer, materialized.camera);
    parseRawJson(rawViewer.setDisplayMode(trial.display_mode), "display-mode diagnostics");
    const configuredDiagnostics = parseRawJson(rawViewer.diagnostics(), "configured visual diagnostics");
    requireCondition(
      configuredDiagnostics.streaming.presentation_version === trial.expected_presentation_version,
      `trial ${trial.id} presentation version differs`,
    );
    parseRawJson(rawViewer.render(), "initial settled-view frame diagnostics");
    const lifecycleTiming = {
      schema: "punctra-browser-visual-lifecycle-timing-v1",
      start: "fresh_private_viewer_creation",
      first_coverage: "first_renderer_accepted_batch_and_sampled_frame_submission",
      settled_view: "complete_stream_camera_display_mode_and_frame_submission",
      first_coverage_milliseconds: publicationTiming.first_coverage_milliseconds,
      settled_view_milliseconds: performance.now() - recreationStarted,
    };

    let transition = null;
    if (trial.temporal_trace.kind === "mixed_lod_parent_child") {
      transition = await captureMixedLodTransition({
        rawViewer,
        corpus,
        trial,
        materialized,
        recreationIndex,
        stableLodRelations: materialized.input_facts.stable_lod_relations ?? [],
      });
    } else {
      for (const batchIndex of materialized.source.expected_view.settled_removed_batch_indices) {
        parseRawJson(rawViewer.removeVisualBatch(batchIndex), "settled batch-removal diagnostics");
      }
    }
    const nominalPick = await verifyNominalPickCoverage(
      rawViewer,
      trial,
      nominalPickExpectations(trial, materialized),
    );
    applySelection(rawViewer, trial, materialized.source_identity);

    const expected = expectedSettledFacts(trial, materialized);
    const quietFrames = [];
    const quietPairs = [];
    const temporalProfile = corpus.tolerance_profiles[trial.temporal_tolerance_profile];
    let previousQuietImage;
    let previousQuietFrame;
    let previousQuietPng;
    let worstPairPngs;
    let finalCaptureRecord;
    let finalArtifactRecord;
    let finalArtifactBytes;
    let finalImage;
    let captureEnvironmentMatch = true;
    let temporalComparisonMilliseconds = 0;
    const settledCaptureTimings = [];
    const settlement = await renderQuietFrames(rawViewer, {
      frameCount: corpus.settling.quiet_frames,
      expected,
    });
    validateRepresentativeSettlement(settlement, corpus.timing_limits);
    const captureWindow = await renderQuietFrames(rawViewer, {
      frameCount: corpus.settling.quiet_frames,
      expected,
      observeFrame: async ({ index }) => {
        let capture = await captureCanonicalFrame(rawViewer, {
          width: corpus.viewport.physical_width,
          height: corpus.viewport.physical_height,
          pollFrameCeiling: corpus.settling.capture_poll_frame_ceiling,
          capturePolicy: corpus.capture,
        });
        validateCaptureAgainstCorpus(capture, corpus, expected);
        captureEnvironmentMatch = environmentTracker.acceptCaptureFacts(capture.facts)
          && captureEnvironmentMatch;
        settledCaptureTimings.push(cloneJson(capture.timing));
        let image = capture.image;
        const isFinalFrame = index === corpus.settling.quiet_frames - 1;
        const artifact = await session.artifacts.createPng(image, {
          ...(isFinalFrame
            ? finalArtifact
            : {
                kind: "settled_quiet_frame_png",
                path: observationArtifactPath(
                  trial.id,
                  recreationIndex,
                  `quiet-${String(index).padStart(2, "0")}`,
                ),
              }),
          trial_id: trial.id,
          recreation_index: recreationIndex,
          frame_index: index,
        });
        const frame = {
          index,
          artifact: artifact.metadata,
          capture: withoutImage(capture),
        };
        quietFrames.push(frame);
        if (previousQuietImage !== undefined) {
          const temporalComparisonStarted = performance.now();
          const comparison = compareCanonicalImages(previousQuietImage, image, {
            toleranceProfile: temporalProfile,
            features: trial.features,
            backgroundRgba: corpus.presentation_policy.canonical_clear_rgba8,
          });
          const pairComparisonMilliseconds = performance.now() - temporalComparisonStarted;
          temporalComparisonMilliseconds += pairComparisonMilliseconds;
          const pair = {
            from_index: index - 1,
            to_index: index,
            from_id: previousQuietFrame.artifact.path,
            to_id: frame.artifact.path,
            from_path: previousQuietFrame.artifact.path,
            to_path: frame.artifact.path,
            comparison,
            comparison_milliseconds: pairComparisonMilliseconds,
          };
          quietPairs.push(pair);
          const partialSummary = summarizeTemporalPairs(index + 1, quietPairs);
          if (partialSummary.worst_pair_index === quietPairs.length - 1) {
            worstPairPngs = {
              from: previousQuietPng,
              to: artifact.bytes,
              pair_index: quietPairs.length - 1,
            };
          }
        }
        previousQuietImage = image;
        previousQuietFrame = frame;
        previousQuietPng = artifact.bytes;
        if (isFinalFrame) {
          finalCaptureRecord = withoutImage(capture);
          finalArtifactRecord = artifact.metadata;
          finalArtifactBytes = artifact.bytes;
          finalImage = image;
        }
        image = undefined;
        capture = undefined;
      },
    });
    requireCondition(quietFrames.length === corpus.settling.quiet_frames, "settled quiet capture count differs");
    requireCondition(quietPairs.length === corpus.settling.quiet_frames - 1, "settled quiet pair count differs");
    requireCondition(finalCaptureRecord !== undefined && finalImage !== undefined, "settled final capture is absent");
    requireCondition(worstPairPngs !== undefined, "settled worst pair is absent");
    const pendingWorkOptions = {
      expected,
      observedBatches: finalCaptureRecord.facts.batches,
      requestPath: "private_direct_transfer_v2",
    };
    settlement.pending_work = derivePendingWorkEvidence(settlement, pendingWorkOptions);
    captureWindow.pending_work = derivePendingWorkEvidence(captureWindow, pendingWorkOptions);
    requireCondition(settlement.pending_work.total === 0, "representative settlement retained pending work");
    requireCondition(captureWindow.pending_work.total === 0, "capture window retained pending work");
    previousQuietImage = undefined;
    previousQuietFrame = undefined;
    previousQuietPng = undefined;
    const temporalSummary = summarizeTemporalPairs(quietFrames.length, quietPairs);
    requireCondition(temporalSummary.worst_pair_index === worstPairPngs.pair_index, "settled worst-pair selection drifted");

    let referenceImage = await decodeRgba8Png(baselinePng ?? finalArtifactBytes);
    const comparisonStarted = performance.now();
    const baselineComparison = compareCanonicalImages(referenceImage, finalImage, {
      toleranceProfile: corpus.tolerance_profiles[trial.tolerance_profile],
      features: trial.features,
      backgroundRgba: corpus.presentation_policy.canonical_clear_rgba8,
    });
    const comparisonMilliseconds = performance.now() - comparisonStarted;
    referenceImage = undefined;
    finalImage = undefined;

    const differenceDerivationStarted = performance.now();
    let differenceReference = await decodeRgba8Png(worstPairPngs.from);
    let differenceCandidate = await decodeRgba8Png(worstPairPngs.to);
    writeDifferenceImage(differenceReference, differenceReference, differenceCandidate);
    differenceCandidate = undefined;
    const differenceDerivationMilliseconds = performance.now() - differenceDerivationStarted;
    const differenceArtifact = await session.artifacts.createPng(differenceReference, {
      kind: "settled_quiet_worst_difference_png",
      path: observationArtifactPath(trial.id, recreationIndex, "quiet-worst-difference"),
      trial_id: trial.id,
      recreation_index: recreationIndex,
      frame_index: temporalSummary.worst_pair_index,
    });
    differenceReference = undefined;
    worstPairPngs = undefined;

    const settledWindow = {
      schema: "punctra-settled-quiet-window-evidence-v1",
      gating: true,
      frame_count: quietFrames.length,
      pair_count: quietPairs.length,
      frames: quietFrames,
      pairs: quietPairs,
      summary: temporalSummary,
      capture_window: captureWindow,
      worst_pair: {
        pair_index: temporalSummary.worst_pair_index,
        ...cloneJson(temporalSummary.worst_pair),
        difference_policy: DIFFERENCE_IMAGE_POLICY,
        difference_artifact: differenceArtifact.metadata,
      },
    };

    const finalDiagnostics = parseRawJson(rawViewer.diagnostics(), "final visual diagnostics");
    requireCondition(finalDiagnostics.streaming.presentation_version === trial.expected_presentation_version, "settled presentation version differs");
    const resources = recreationResourceFacts({
      corpus,
      materialized,
      diagnostics: finalDiagnostics,
      capture: finalCaptureRecord,
      finalArtifact: finalArtifactRecord,
      lifecycleTiming,
      representativeSettlement: settlement,
      settledCaptureTimings,
      transitionTiming: transition?.timing ?? null,
      baselineComparisonMilliseconds: comparisonMilliseconds,
      settledComparisonSamples: quietPairs.map(({ comparison_milliseconds: value }) => value),
      settledComparisonMilliseconds: temporalComparisonMilliseconds,
      differenceDerivationMilliseconds,
      artifactTimingSamples: session.artifacts.metadata().slice(artifactStartedIndex).map((artifact) => ({
        path: artifact.path,
        png_encode_milliseconds: artifact.png_encode_milliseconds,
        artifact_encoding_milliseconds: artifact.artifact_encoding_milliseconds,
      })),
    });
    const failures = [];
    if (!initialEnvironmentMatch || !captureEnvironmentMatch) failures.push("environment_recreation_mismatch");
    if (!baselineComparison.passed) failures.push(...baselineComparison.failures.map((failure) => `baseline:${failure}`));
    if (!temporalSummary.passed) failures.push(...temporalSummary.failures.map((failure) => `settled_temporal:${failure}`));
    if (transition !== null && !transition.complete) failures.push("mixed_lod_transition_incomplete");

    const shutdownDiagnostics = parseRawJson(rawViewer.shutdown(), "visual shutdown diagnostics");
    resources.cleanup.after_shutdown = captureResourceFacts(shutdownDiagnostics, "post-shutdown capture resources");
    validateResourceFacts(resources, corpus, failures);
    rawViewer.free();
    rawViewer = undefined;
    disposed = true;
    const record = {
      index: recreationIndex,
      environment_match: initialEnvironmentMatch && captureEnvironmentMatch,
      settlement,
      capture: {
        ...finalCaptureRecord,
        artifact: finalArtifactRecord,
      },
      comparison: baselineComparison,
      temporal: {
        kind: trial.temporal_trace.kind,
        trace: cloneJson(trial.temporal_trace),
        quiet_frame_count: corpus.settling.quiet_frames,
        settled_window: settledWindow,
        transition,
      },
      batch_facts: expectedBatchFacts(trial, materialized.source.expected_view),
      nominal_pick: nominalPick,
      coverage: {
        declared: trial.coverage,
        expected_points: materialized.point_count,
        published_points: finalDiagnostics.streaming.published_points,
        settled_drawn_points: finalDiagnostics.frame.drawn_points,
        settled_resident_points: materialized.source.expected_view.settled_resident_points,
        declared_authority: "source_or_authored_facts_only",
        settled_draw_authority: "presentation_only",
        query_completion: "not_inferred_from_visual_evidence",
      },
      resources,
      diagnostics: finalDiagnostics,
      cleanup: {
        shutdown_phase: shutdownDiagnostics.phase,
        capture_resources: cloneJson(resources.cleanup),
        raw_viewer_freed: true,
      },
      passed: failures.length === 0,
      failures,
    };
    finalCaptureRecord = undefined;
    return {
      record,
      internal_final_png: finalArtifactBytes,
    };
  } finally {
    if (rawViewer !== undefined) {
      try { rawViewer.shutdown(); } catch { /* Preserve the primary failure. */ }
      try { rawViewer.free(); } catch { /* Preserve the primary failure. */ }
    }
    if (!disposed) await nextAnimationFrame();
  }
}

async function captureMixedLodTransition(options) {
  const { rawViewer, corpus, trial, materialized, recreationIndex, stableLodRelations } = options;
  const trace = trial.temporal_trace;
  parseRawJson(
    rawViewer.setVisualBatchPresentation(trace.parent_batch_index, 255),
    "initial parent presentation diagnostics",
  );
  parseRawJson(
    rawViewer.setVisualBatchPresentation(trace.child_batch_index, 0),
    "initial child presentation diagnostics",
  );
  const frames = [];
  const pairs = [];
  let previousImage;
  let previousFrame;
  const captureTimings = [];
  let comparisonTotalMilliseconds = 0;
  for (let frameIndex = 0; frameIndex < trace.child_weights_u8.length; frameIndex += 1) {
    const childWeight = trace.child_weights_u8[frameIndex];
    const parentWeight = 255 - childWeight;
    parseRawJson(
      rawViewer.setVisualBatchPresentation(trace.parent_batch_index, parentWeight),
      `mixed-LOD parent frame ${frameIndex} diagnostics`,
    );
    parseRawJson(
      rawViewer.setVisualBatchPresentation(trace.child_batch_index, childWeight),
      `mixed-LOD child frame ${frameIndex} diagnostics`,
    );
    await nextAnimationFrame();
    parseRawJson(rawViewer.render(), `mixed-LOD presented frame ${frameIndex}`);
    let capture = await captureCanonicalFrame(rawViewer, {
      width: corpus.viewport.physical_width,
      height: corpus.viewport.physical_height,
      pollFrameCeiling: corpus.settling.capture_poll_frame_ceiling,
      capturePolicy: corpus.capture,
    });
    validateCaptureFormatAgainstCorpus(capture, corpus);
    const weights = [...materialized.source.expected_view.settled_presentation_weights_u8];
    weights[trace.parent_batch_index] = parentWeight;
    weights[trace.child_batch_index] = childWeight;
    validateCaptureBatchSnapshot(
      capture.facts,
      expectedCaptureBatches(trial, materialized, {
        weightsU8: weights,
        removedBatchIndices: [],
      }),
    );
    captureTimings.push(cloneJson(capture.timing));
    let image = capture.image;
    const artifact = await session.artifacts.createPng(image, {
      kind: "mixed_lod_transition_png",
      path: observationArtifactPath(trial.id, recreationIndex, `transition-${String(frameIndex).padStart(2, "0")}`),
      trial_id: trial.id,
      recreation_index: recreationIndex,
      frame_index: frameIndex,
    });
    const frame = {
      index: frameIndex,
      parent_weight_u8: parentWeight,
      child_weight_u8: childWeight,
      artifact: artifact.metadata,
      capture: withoutImage(capture),
    };
    frames.push(frame);
    if (previousImage !== undefined) {
      const comparisonStarted = performance.now();
      const comparison = compareCanonicalImages(previousImage, image, {
        toleranceProfile: corpus.tolerance_profiles[trial.tolerance_profile],
        features: trial.features,
        backgroundRgba: corpus.presentation_policy.canonical_clear_rgba8,
      });
      const comparisonMilliseconds = performance.now() - comparisonStarted;
      comparisonTotalMilliseconds += comparisonMilliseconds;
      pairs.push({
        from_index: frameIndex - 1,
        to_index: frameIndex,
        from_id: previousFrame.artifact.path,
        to_id: frame.artifact.path,
        comparison,
        comparison_milliseconds: comparisonMilliseconds,
      });
    }
    previousImage = image;
    previousFrame = frame;
    image = undefined;
    capture = undefined;
  }
  previousImage = undefined;
  if (trace.remove_parent_after_transition) {
    parseRawJson(rawViewer.removeVisualBatch(trace.parent_batch_index), "mixed-LOD parent retirement diagnostics");
  }
  const comparisons = summarizeTemporalPairs(frames.length, pairs);
  const changedPairCount = pairs.filter((pair) => pair.comparison.pixels?.unstable > 0).length;
  const stableLodCut = stableLodRelations.find(
    ({ dense_batch_index: denseBatchIndex }) => denseBatchIndex === trace.child_batch_index,
  );
  const captureTimingWindow = summarizeCaptureTimingSamples(captureTimings, {
    expectedCount: trace.child_weights_u8.length,
  });
  return {
    schema: "punctra-mixed-lod-transition-evidence-v1",
    gating: false,
    complete: frames.length === corpus.settling.transition_frame_count
      && frames[0].child_weight_u8 === 0
      && frames.at(-1).child_weight_u8 === 255
      && changedPairCount > 0
      && stableLodCut !== undefined,
    parent_batch_index: trace.parent_batch_index,
    child_batch_index: trace.child_batch_index,
    parent_removed_after_transition: trace.remove_parent_after_transition,
    stable_lod_cut: stableLodCut === undefined ? null : {
      ...cloneJson(stableLodCut),
      dense_weight_u8: 255,
      coarse_weight_u8: 255,
      resident_through_transition: true,
    },
    frames,
    comparisons,
    changed_pair_count: changedPairCount,
    timing: {
      schema: "punctra-browser-visual-transition-timing-v1",
      capture_samples: captureTimings,
      capture_total_milliseconds: captureTimingWindow.totals.total_milliseconds,
      comparison_samples_milliseconds: pairs.map(({ comparison_milliseconds: value }) => value),
      comparison_total_milliseconds: comparisonTotalMilliseconds,
    },
    interpretation: "recorded_dynamic_transition_not_a_static_tolerance_gate",
  };
}

function publishMaterializedSource(rawViewer, materialized, recreationStarted) {
  const [originX, originY, originZ] = materialized.world_origin;
  const [minimumZ, maximumZ] = materialized.source_z_range;
  requireCondition(materialized.batches.length > 0, "materialized visual Source has no batches");
  parseRawJson(rawViewer.beginStreamBatch(
    materialized.source_identity,
    materialized.point_count,
    originX,
    originY,
    originZ,
    minimumZ,
    maximumZ,
    0,
    materialized.batches[0],
  ), "first visual batch diagnostics");
  parseRawJson(rawViewer.render(), "first-coverage sampled frame diagnostics");
  const firstCoverageMilliseconds = performance.now() - recreationStarted;
  for (let batchIndex = 1; batchIndex < materialized.batches.length; batchIndex += 1) {
    parseRawJson(
      rawViewer.publishStreamBatch(batchIndex, materialized.batches[batchIndex]),
      `visual batch ${batchIndex} diagnostics`,
    );
  }
  const completed = parseRawJson(rawViewer.completeStream(), "completed visual stream diagnostics");
  requireCondition(completed.streaming.phase === "complete", "visual stream did not complete");
  requireCondition(completed.streaming.expected_points === materialized.point_count, "visual stream expected Point count differs");
  requireCondition(completed.streaming.published_points === materialized.point_count, "visual stream published Point count differs");
  return { first_coverage_milliseconds: firstCoverageMilliseconds };
}

function configureCamera(rawViewer, camera) {
  const [eyeX, eyeY, eyeZ] = camera.eye;
  const [targetX, targetY, targetZ] = camera.target;
  const [upX, upY, upZ] = camera.up;
  if (camera.projection === "perspective") {
    parseRawJson(rawViewer.setPerspectiveCamera(
      eyeX, eyeY, eyeZ,
      targetX, targetY, targetZ,
      upX, upY, upZ,
      camera.vertical_field_of_view_radians,
      camera.near_distance,
      camera.far_distance,
    ), "perspective camera diagnostics");
  } else {
    parseRawJson(rawViewer.setOrthographicCamera(
      eyeX, eyeY, eyeZ,
      targetX, targetY, targetZ,
      upX, upY, upZ,
      camera.vertical_world_height,
      camera.near_distance,
      camera.far_distance,
    ), "orthographic camera diagnostics");
  }
}

function applySelection(rawViewer, trial, sourceIdentity) {
  const diagnostics = parseRawJson(rawViewer.diagnostics(), "pre-selection diagnostics");
  if (trial.selection.ordinals.length === 0) {
    requireCondition(diagnostics.highlights.point_count === 0, "unselected trial retained highlights");
    return;
  }
  const ordinals = new BigUint64Array(trial.selection.ordinals.map(BigInt));
  const selected = parseRawJson(rawViewer.setHighlights(
    sourceIdentity,
    BigInt(diagnostics.streaming.generation),
    ordinals,
  ), "selection diagnostics");
  requireCondition(selected.highlights.point_count === ordinals.length, "selection highlight count differs");
  requireCondition(selected.highlights.authority === "presentation_only", "selection authority differs");
}

function nominalPickExpectations(trial, materialized) {
  if (trial.selection.ordinals.length === 0) return [];
  requireCondition(materialized.input_facts.kind === "generated", `trial ${trial.id} selected a Source without authored Point batches`);
  const featureById = new Map(trial.features.map((feature) => [feature.id, feature]));
  return trial.selection.nominal_pick_regions.map((region) => {
    const feature = featureById.get(region.feature_id);
    requireCondition(feature !== undefined, `trial ${trial.id} nominal-pick feature is absent`);
    const ordinalIndex = feature.binding.authored_point_ordinals.indexOf(region.ordinal);
    requireCondition(ordinalIndex >= 0, `trial ${trial.id} nominal-pick Point binding is absent`);
    const batchIndex = generatedBatchIndexForOrdinal(materialized.input_facts.batch_roles, region.ordinal);
    requireCondition(
      !materialized.source.expected_view.settled_removed_batch_indices.includes(batchIndex),
      `trial ${trial.id} nominal-pick Point batch was removed before settlement`,
    );
    return {
      ordinal: region.ordinal,
      feature_id: region.feature_id,
      expected_pixel: [...feature.binding.expected_pixels[ordinalIndex]],
      tolerance_pixels: feature.binding.tolerance_pixels,
      nominal_region: { ...feature.rectangle },
      generation: materialized.source.expected_view.generation,
      batch_key: materialized.source.expected_view.batch_keys[batchIndex],
      batch_version: trial.expected_settled_batch_versions[batchIndex],
      source_identity: materialized.source_identity,
    };
  });
}

function generatedBatchIndexForOrdinal(batchRoles, ordinal) {
  let firstOrdinal = 0;
  for (let index = 0; index < batchRoles.length; index += 1) {
    const batch = batchRoles[index];
    requireCondition(batch.batch_index === index, "generated batch roles are not in Source order");
    const afterLastOrdinal = firstOrdinal + batch.point_count;
    if (ordinal >= firstOrdinal && ordinal < afterLastOrdinal) return batch.batch_index;
    firstOrdinal = afterLastOrdinal;
  }
  requireCondition(false, `selected Point ${ordinal} is absent from generated batches`);
  return -1;
}

function expectedSettledFacts(trial, materialized) {
  const expectedView = materialized.source.expected_view;
  return {
    source_identity: materialized.source_identity,
    point_count: expectedView.published_points,
    published_batches: expectedView.published_batches,
    view_id: expectedView.view_id,
    generation: expectedView.generation,
    display_mode: trial.display_mode,
    projection: materialized.camera.projection,
    highlight_points: trial.selection.ordinals.length,
    physical_width: VISUAL_VIEWPORT.physical_width,
    physical_height: VISUAL_VIEWPORT.physical_height,
    drawn_points: expectedView.settled_drawn_points,
    draw_calls: expectedView.settled_draw_calls,
    resident_bytes: expectedView.settled_resident_points * TRANSFER_VERTEX_BYTES,
    capture_batches: expectedCaptureBatches(trial, materialized),
  };
}

function expectedCaptureBatches(trial, materialized, options = {}) {
  const expectedView = materialized.source.expected_view;
  const weights = options.weightsU8 ?? expectedView.settled_presentation_weights_u8;
  const removed = new Set(options.removedBatchIndices ?? expectedView.settled_removed_batch_indices);
  return materialized.batches.map((batch, batchIndex) => ({
    batch_index: batchIndex,
    key: expectedView.batch_keys[batchIndex],
    version: trial.expected_settled_batch_versions[batchIndex],
    point_count: batch.byteLength / 32,
    state: "resident",
    presentation_weight_u8: weights[batchIndex],
  })).filter(({ batch_index: batchIndex }) => !removed.has(batchIndex));
}

function expectedBatchFacts(trial, expectedView) {
  return {
    schema: "punctra-browser-visual-batch-facts-v1",
    presentation_version: trial.expected_presentation_version,
    entries: expectedView.batch_keys.map((batchKey, batchIndex) => ({
      batch_index: batchIndex,
      batch_key: batchKey,
      initial_version: expectedView.initial_batch_versions[batchIndex],
      settled_version: trial.expected_settled_batch_versions[batchIndex],
      presentation_weight_u8: expectedView.settled_presentation_weights_u8[batchIndex],
      state: expectedView.settled_removed_batch_indices.includes(batchIndex) ? "removed" : "resident",
    })),
  };
}

function validateCanonicalEnvironment(environment, corpus, diagnostics) {
  const required = corpus.required_capabilities;
  requireCondition(environment.document.secure_context === required.secure_context, "secure-context fact differs");
  requireCondition(environment.document.visibility_state === "visible", "visual page must remain visible");
  requireCondition(environment.viewport.requested_css_width === corpus.viewport.css_width, "requested CSS width differs");
  requireCondition(environment.viewport.requested_css_height === corpus.viewport.css_height, "requested CSS height differs");
  requireCondition(environment.viewport.requested_device_pixel_ratio === corpus.viewport.requested_device_pixel_ratio, "requested DPR differs");
  requireCondition(environment.viewport.observed_window_device_pixel_ratio === corpus.viewport.requested_device_pixel_ratio, "observed window DPR differs");
  requireCondition(environment.viewport.observed_css_width === corpus.viewport.css_width, "observed CSS width differs");
  requireCondition(environment.viewport.observed_css_height === corpus.viewport.css_height, "observed CSS height differs");
  requireCondition(environment.viewport.canvas_bitmap_width === corpus.viewport.physical_width, "canvas bitmap width differs");
  requireCondition(environment.viewport.canvas_bitmap_height === corpus.viewport.physical_height, "canvas bitmap height differs");
  requireCondition(environment.viewport.visual_viewport_scale === 1, "browser visual viewport scale differs");
  requireCondition(environment.fallback.used === false && required.fallback_allowed === false, "fallback state differs");
  const capabilities = diagnostics.capabilities;
  requireCondition(capabilities.secure_context === required.secure_context, "raw secure-context capability differs");
  requireCondition(capabilities.webgpu === required.webgpu, "WebGPU capability differs");
  requireCondition(capabilities.surface_format === required.surface_format, "surface format differs");
  requireCondition(capabilities.composite_alpha_mode === required.composite_alpha_mode, "surface alpha mode differs");
  requireCondition(capabilities.present_mode === required.present_mode, "present mode differs");
  requireCondition(capabilities.surface_format_support.render_attachment === required.render_attachment, "render-attachment support differs");
  requireCondition(capabilities.surface_format_support.blendable === required.blendable, "blendability support differs");
}

function validateCaptureAgainstCorpus(capture, corpus, expected) {
  validateCaptureFormatAgainstCorpus(capture, corpus);
  requireCondition(capture.facts.view_generation === expected.generation, "capture View generation differs");
  requireCondition(capture.facts.drawn_points === expected.drawn_points, "capture drawn Point count differs");
  requireCondition(capture.facts.draw_calls === expected.draw_calls, "capture draw-call count differs");
  requireCondition(capture.facts.resident_bytes === expected.resident_bytes, "capture resident bytes differ");
  validateCaptureBatchSnapshot(capture.facts, expected.capture_batches);
}

function validateCaptureFormatAgainstCorpus(capture, corpus) {
  const required = corpus.required_capabilities;
  requireCondition(capture.facts.source_format === required.capture_source_format, "capture source format differs");
  requireCondition(capture.facts.source_channel_order === required.capture_source_channel_order, "capture source channel order differs");
  requireCondition(capture.facts.normalization === required.capture_canonicalization, "capture channel normalization differs");
}

function validateCaptureBatchSnapshot(facts, expectedBatches) {
  requireCondition(facts.batch_state_authority === "renderer_accepted_updates", "capture batch-state authority differs");
  requireCondition(JSON.stringify(facts.batches) === JSON.stringify(expectedBatches), "capture renderer-accepted batch snapshot differs");
}

function recreationResourceFacts(options) {
  const {
    corpus,
    materialized,
    diagnostics,
    capture,
    finalArtifact,
    lifecycleTiming,
    representativeSettlement,
    settledCaptureTimings,
    transitionTiming,
    baselineComparisonMilliseconds,
    settledComparisonSamples,
    settledComparisonMilliseconds,
    differenceDerivationMilliseconds,
    artifactTimingSamples,
  } = options;
  const transitionCaptureSamples = transitionTiming?.capture_samples ?? [];
  const transitionComparisonSamples = transitionTiming?.comparison_samples_milliseconds ?? [];
  const settledCaptureWindow = summarizeCaptureTimingSamples(settledCaptureTimings, {
    expectedCount: corpus.settling.quiet_frames,
  });
  const transitionCaptureWindow = summarizeCaptureTimingSamples(transitionCaptureSamples);
  const settledComparisonTotal = baselineComparisonMilliseconds
    + settledComparisonMilliseconds
    + differenceDerivationMilliseconds;
  const transitionComparisonTotal = transitionComparisonSamples.reduce((total, value) => total + value, 0);
  const pngEncodingMilliseconds = artifactTimingSamples.reduce(
    (total, sample) => total + sample.png_encode_milliseconds,
    0,
  );
  const artifactEncodingMilliseconds = artifactTimingSamples.reduce(
    (total, sample) => total + sample.artifact_encoding_milliseconds,
    0,
  );
  return {
    schema: "punctra-browser-visual-resource-evidence-v1",
    renderer: {
      resident_points: materialized.source.expected_view.settled_resident_points,
      resident_bytes: diagnostics.frame.resident_bytes,
      batches: materialized.source.expected_view.settled_draw_calls,
      highlight_points: diagnostics.highlights.point_count,
      drawn_points: diagnostics.frame.drawn_points,
      draw_calls: diagnostics.frame.draw_calls,
      transient_texture_bytes: diagnostics.frame.transient_texture_bytes,
      canvas_surface_bytes: diagnostics.viewport.surface_bytes,
    },
    transfer: {
      retained_record_bytes: diagnostics.streaming.retained_record_bytes,
      main_thread_batch_bytes_high_water: diagnostics.streaming.main_thread_batch_bytes_high_water,
      worker_staging_bytes: 0,
      queued_range_bytes: 0,
      concurrent_response_bytes: materialized.input_facts.kind === "derived_pvis"
        ? materialized.input_facts.payload_bytes
        : 0,
      memory_cache_bytes: 0,
      persistent_cache_bytes: 0,
      path: "private_direct_transfer_v2",
    },
    capture: {
      capture_texture_bytes: capture.facts.color_texture_bytes,
      staging_buffer_bytes: capture.facts.staging_buffer_bytes,
      row_aligned_readback_bytes: capture.facts.staging_buffer_bytes,
      canonical_pixel_bytes: capture.facts.canonical_pixel_bytes,
      encoded_png_bytes: Math.max(finalArtifact.encoded_byte_length, session.artifacts.maximumEncodedBytes()),
      total_encoded_artifact_bytes: session.artifacts.totalEncodedBytes(),
      png_scanline_bytes: corpus.resource_limits.png_scanline_bytes,
      encoder_working_bytes: corpus.resource_limits.canonical_pixel_bytes
        + corpus.resource_limits.png_scanline_bytes
        + corpus.resource_limits.comparison_workspace_bytes,
      baseline_decoded_bytes: corpus.resource_limits.canonical_pixel_bytes,
      comparison_workspace_bytes: 1_024,
      peak_live_canonical_images: 2,
    },
    cleanup: {
      after_final_capture: captureResourceFacts(diagnostics, "final capture resources"),
      after_shutdown: null,
    },
    timing: {
      schema: "punctra-browser-visual-timing-evidence-v1",
      lifecycle: lifecycleTiming,
      representative_frames: {
        capture_free: true,
        frame_count: representativeSettlement.quiet_frames,
        frame_interval_samples_milliseconds: cloneJson(representativeSettlement.frame_interval_samples_milliseconds),
        frame_submission_samples_milliseconds: cloneJson(representativeSettlement.frame_submission_samples_milliseconds),
        frame_interval_milliseconds: cloneJson(representativeSettlement.frame_interval_milliseconds),
        frame_submission_milliseconds: cloneJson(representativeSettlement.frame_submission_milliseconds),
      },
      capture: {
        settled: {
          ...settledCaptureWindow,
        },
        transition: {
          ...transitionCaptureWindow,
        },
        all_windows_total_milliseconds: settledCaptureWindow.totals.total_milliseconds
          + transitionCaptureWindow.totals.total_milliseconds,
      },
      comparison: {
        baseline_milliseconds: baselineComparisonMilliseconds,
        settled_pair_samples_milliseconds: cloneJson(settledComparisonSamples),
        settled_pair_total_milliseconds: settledComparisonMilliseconds,
        worst_pair_difference_derivation_milliseconds: differenceDerivationMilliseconds,
        settled_total_milliseconds: settledComparisonTotal,
        transition_pair_samples_milliseconds: cloneJson(transitionComparisonSamples),
        transition_total_milliseconds: transitionComparisonTotal,
        all_comparisons_total_milliseconds: settledComparisonTotal + transitionComparisonTotal,
      },
      encoding: {
        artifacts: cloneJson(artifactTimingSamples),
        artifact_count: artifactTimingSamples.length,
        png_encode_total_milliseconds: pngEncodingMilliseconds,
        artifact_encoding_total_milliseconds: artifactEncodingMilliseconds,
      },
    },
    unavailable: {
      gpu_or_driver_allocation_bytes: null,
      process_resident_bytes: null,
      physical_cache_allocation_bytes: null,
    },
  };
}

function validateResourceFacts(resources, corpus, failures) {
  const limits = corpus.resource_limits;
  const checks = [
    [resources.renderer.resident_points, limits.renderer_resident_points, "renderer_resident_points"],
    [resources.renderer.resident_bytes, limits.renderer_resident_bytes, "renderer_resident_bytes"],
    [resources.renderer.batches, limits.renderer_batches, "renderer_batches"],
    [resources.renderer.highlight_points, limits.highlight_points, "highlight_points"],
    [resources.renderer.transient_texture_bytes, limits.renderer_transient_texture_bytes, "renderer_transient_texture_bytes"],
    [resources.renderer.canvas_surface_bytes, limits.canvas_surface_bytes, "canvas_surface_bytes"],
    [resources.transfer.retained_record_bytes, limits.retained_record_bytes, "retained_record_bytes"],
    [resources.transfer.worker_staging_bytes, limits.worker_staging_bytes, "worker_staging_bytes"],
    [resources.transfer.queued_range_bytes, limits.queued_range_bytes, "queued_range_bytes"],
    [resources.transfer.concurrent_response_bytes, limits.concurrent_response_bytes, "concurrent_response_bytes"],
    [resources.transfer.memory_cache_bytes, limits.memory_cache_bytes, "memory_cache_bytes"],
    [resources.transfer.persistent_cache_bytes, limits.persistent_cache_bytes, "persistent_cache_bytes"],
    [resources.capture.capture_texture_bytes, limits.capture_texture_bytes, "capture_texture_bytes"],
    [resources.capture.staging_buffer_bytes, limits.staging_buffer_bytes, "staging_buffer_bytes"],
    [resources.capture.row_aligned_readback_bytes, limits.row_aligned_readback_bytes, "row_aligned_readback_bytes"],
    [resources.capture.canonical_pixel_bytes, limits.canonical_pixel_bytes, "canonical_pixel_bytes"],
    [resources.capture.png_scanline_bytes, limits.png_scanline_bytes, "png_scanline_bytes"],
    [resources.capture.encoder_working_bytes, limits.encoder_working_bytes, "encoder_working_bytes"],
    [resources.capture.encoded_png_bytes, limits.encoded_png_bytes, "encoded_png_bytes"],
    [resources.capture.total_encoded_artifact_bytes, limits.total_encoded_artifact_bytes, "total_encoded_artifact_bytes"],
    [resources.capture.comparison_workspace_bytes, limits.comparison_workspace_bytes, "comparison_workspace_bytes"],
    [resources.capture.peak_live_canonical_images, limits.peak_live_canonical_images, "peak_live_canonical_images"],
  ];
  for (const [actual, allowed, label] of checks) {
    if (!Number.isFinite(actual) || actual < 0 || actual > allowed) failures.push(`resource:${label}`);
  }
  for (const [stage, facts] of Object.entries(resources.cleanup)) {
    if (facts === null) {
      failures.push(`resource:capture_cleanup:${stage}`);
      continue;
    }
    if (facts.pending_tickets !== 0) failures.push(`resource:capture_cleanup:${stage}:pending_tickets`);
    if (facts.owned_textures !== 0) failures.push(`resource:capture_cleanup:${stage}:owned_textures`);
    if (facts.owned_readback_buffers !== 0) failures.push(`resource:capture_cleanup:${stage}:owned_readback_buffers`);
  }
  validateTimingFacts(resources.timing, corpus.timing_limits, failures);
}

function validateTimingFacts(timing, limits, failures) {
  const bounded = (actual, allowed, label) => {
    if (!Number.isFinite(actual) || actual < 0 || actual > allowed) failures.push(`timing:${label}`);
  };
  bounded(timing.lifecycle.first_coverage_milliseconds, limits.first_coverage_milliseconds, "first_coverage_milliseconds");
  bounded(timing.lifecycle.settled_view_milliseconds, limits.settled_view_milliseconds, "settled_view_milliseconds");
  if (timing.lifecycle.settled_view_milliseconds < timing.lifecycle.first_coverage_milliseconds) {
    failures.push("timing:lifecycle_order");
  }
  bounded(
    timing.representative_frames.frame_interval_milliseconds.p95,
    limits.representative_frame_interval_p95_milliseconds,
    "representative_frame_interval_p95_milliseconds",
  );
  bounded(
    timing.representative_frames.frame_submission_milliseconds.p95,
    limits.representative_frame_submission_p95_milliseconds,
    "representative_frame_submission_p95_milliseconds",
  );
  const captureLimits = {
    begin_submission_milliseconds: limits.capture_begin_submission_milliseconds_per_frame,
    poll_wait_milliseconds: limits.capture_poll_wait_milliseconds_per_frame,
    poll_call_milliseconds: limits.capture_poll_call_milliseconds_per_frame,
    canonical_copy_milliseconds: limits.capture_canonical_copy_milliseconds_per_frame,
    submitted_work_done_callback_milliseconds: limits.capture_submitted_work_done_callback_milliseconds_per_frame,
    readback_mapping_callback_milliseconds: limits.capture_readback_mapping_callback_milliseconds_per_frame,
  };
  for (const [windowName, window] of Object.entries(timing.capture).filter(([name]) => name !== "all_windows_total_milliseconds")) {
    for (const sample of window.samples) {
      if (sample.callback_elapsed_origin !== "begin_frame_capture_monotonic_clock"
        || sample.callback_ordering !== "not_inferred"
        || sample.physical_gpu_timing !== "not_observed") {
        failures.push(`timing:${windowName}_capture_callback_authority`);
      }
      for (const [field, allowed] of Object.entries(captureLimits)) {
        bounded(sample[field], allowed, `${windowName}_${field}`);
      }
    }
  }
  bounded(
    timing.capture.settled.totals.total_milliseconds,
    limits.settled_capture_total_milliseconds_per_recreation,
    "settled_capture_total_milliseconds_per_recreation",
  );
  bounded(
    timing.capture.transition.totals.total_milliseconds,
    limits.transition_capture_total_milliseconds_per_recreation,
    "transition_capture_total_milliseconds_per_recreation",
  );
  for (const value of [
    timing.comparison.baseline_milliseconds,
    timing.comparison.worst_pair_difference_derivation_milliseconds,
    ...timing.comparison.settled_pair_samples_milliseconds,
    ...timing.comparison.transition_pair_samples_milliseconds,
  ]) bounded(value, limits.comparison_milliseconds_per_pair, "comparison_milliseconds_per_pair");
  bounded(
    timing.comparison.settled_total_milliseconds,
    limits.settled_comparison_total_milliseconds_per_recreation,
    "settled_comparison_total_milliseconds_per_recreation",
  );
  bounded(
    timing.comparison.transition_total_milliseconds,
    limits.transition_comparison_total_milliseconds_per_recreation,
    "transition_comparison_total_milliseconds_per_recreation",
  );
  for (const artifact of timing.encoding.artifacts) {
    bounded(artifact.png_encode_milliseconds, limits.png_encode_milliseconds_per_artifact, "png_encode_milliseconds_per_artifact");
    bounded(artifact.artifact_encoding_milliseconds, limits.artifact_encoding_milliseconds_per_artifact, "artifact_encoding_milliseconds_per_artifact");
  }
  bounded(
    timing.encoding.artifact_encoding_total_milliseconds,
    limits.artifact_encoding_total_milliseconds_per_recreation,
    "artifact_encoding_total_milliseconds_per_recreation",
  );
}

function validateRepresentativeSettlement(settlement, limits) {
  requireCondition(settlement.observed_frame_captures === 0, "representative settlement includes capture work");
  requireCondition(settlement.frame_interval_samples_milliseconds.length === settlement.quiet_frames, "representative frame-interval samples differ");
  requireCondition(settlement.frame_submission_samples_milliseconds.length === settlement.quiet_frames, "representative frame-submission samples differ");
  requireCondition(
    settlement.frame_interval_milliseconds.p95 <= limits.representative_frame_interval_p95_milliseconds,
    "representative frame interval p95 exceeds its ceiling",
  );
  requireCondition(
    settlement.frame_submission_milliseconds.p95 <= limits.representative_frame_submission_p95_milliseconds,
    "representative frame submission p95 exceeds its ceiling",
  );
  requireCondition(settlement.animation_frame_scheduler.pending === 0, "representative settlement retained a scheduled frame");
}

function captureResourceFacts(diagnostics, label) {
  const facts = diagnostics.capture_resources;
  requireCondition(facts !== null && typeof facts === "object" && !Array.isArray(facts), `${label} are absent`);
  for (const field of ["pending_tickets", "owned_textures", "owned_readback_buffers"]) {
    requireCondition(Number.isSafeInteger(facts[field]) && facts[field] >= 0, `${label} ${field} is invalid`);
  }
  return cloneJson(facts);
}

async function loadExistingBaseline(trial, corpusUrl, repositoryPath) {
  const response = await fetch(new URL(trial.baseline_path, corpusUrl), {
    cache: "no-store",
    credentials: "same-origin",
  });
  requireCondition(response.ok, `baseline ${trial.id} returned HTTP ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  const inspected = await inspectPngArtifact(bytes, {
    kind: "baseline_png",
    path: repositoryPath,
    trial_id: trial.id,
    recreation_index: null,
    frame_index: null,
  });
  return { bytes, metadata: inspected };
}

async function inspectPngArtifact(bytes, descriptor) {
  let decoded = await decodeRgba8Png(bytes);
  requireCondition(decoded.width === VISUAL_VIEWPORT.physical_width && decoded.height === VISUAL_VIEWPORT.physical_height, `PNG ${descriptor.path} dimensions differ`);
  const metadata = await createPngArtifactMetadata({
    descriptor,
    encodedBytes: bytes,
    image: decoded,
  });
  decoded = undefined;
  return metadata;
}

async function captureRuntimeArtifacts() {
  const records = [];
  let wasmBytes;
  for (const relativePath of RUNTIME_ARTIFACT_PATHS) {
    const url = new URL(relativePath, import.meta.url);
    const response = await fetch(url, { cache: "no-store", credentials: "same-origin" });
    requireCondition(response.ok, `runtime artifact ${relativePath} returned HTTP ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const record = {
      path: `apps/browser-demo/web/${relativePath.replace(/^\.\//, "")}`,
      byte_length: bytes.byteLength,
      sha256: await sha256Hex(bytes),
    };
    records.push(record);
    if (relativePath === "./pkg/browser_demo_bg.wasm") wasmBytes = bytes;
  }
  requireCondition(wasmBytes instanceof Uint8Array, "captured runtime omitted the Wasm artifact bytes");
  return { records, wasmBytes };
}

function bindWasmRuntime(runtimeArtifacts) {
  const record = runtimeArtifacts.records.find(
    (candidate) => candidate.path === "apps/browser-demo/web/pkg/browser_demo_bg.wasm",
  );
  requireCondition(record !== undefined, "captured runtime omitted the Wasm artifact identity");
  const identity = `${record.byte_length}:${record.sha256}`;
  session.bindWasmRuntime(identity, runtimeArtifacts.wasmBytes);
}

async function resolveBaselineInputs(options) {
  const { mode, corpus, corpusUrl, artifacts, runtimeArtifacts } = options;
  let manifest;
  let bytes;
  if (mode === "record") {
    manifest = createBaselineInputsManifest({
      release: corpus.release,
      trials: corpus.trials,
      artifacts,
      runtimeArtifacts,
    });
    bytes = encodeBaselineInputsManifest(manifest);
  } else {
    const response = await fetch(new URL("./baseline-inputs.json", corpusUrl), {
      cache: "no-store",
      credentials: "same-origin",
    });
    requireCondition(response.ok, `baseline-input manifest returned HTTP ${response.status}`);
    bytes = new Uint8Array(await response.arrayBuffer());
    try {
      manifest = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
    } catch (error) {
      throw new Error(`baseline-input manifest could not be decoded: ${error?.message ?? error}`);
    }
    validateBaselineInputsManifest(manifest, {
      trials: corpus.trials,
      runtimeArtifacts,
      artifacts,
    });
    requireCondition(
      JSON.stringify(manifest, null, 2) + "\n" === new TextDecoder().decode(bytes),
      "baseline-input manifest encoding is not canonical",
    );
  }
  requireCondition(bytes.byteLength <= corpus.resource_limits.baseline_inputs_json_bytes, "baseline-input manifest exceeds its resource ceiling");
  return { path: BASELINE_INPUTS_PATH, manifest, bytes };
}

async function createPrivateViewer() {
  await session.initializeWasm((wasmRuntimeBytes) => initializeWasm({ module_or_path: wasmRuntimeBytes }));
  return createRawViewer(
    canvas,
    VISUAL_VIEWPORT.css_width,
    VISUAL_VIEWPORT.css_height,
    VISUAL_VIEWPORT.requested_device_pixel_ratio,
  );
}

async function loadQualificationHost() {
  try {
    const response = await fetch("./qualification-host.json", {
      cache: "no-store",
      credentials: "same-origin",
    });
    return response.ok ? await response.json() : null;
  } catch {
    return null;
  }
}

function createEnvironmentTracker(record, corpus, host) {
  let fingerprint;
  let captureFingerprint;
  return {
    host,
    accept(environment) {
      const claimedEnvironment = {
        ...cloneJson(environment),
        attended_lane: cloneJson(record.provenance.attended_lane),
        canonical_requirements: cloneJson(corpus.required_capabilities),
      };
      const next = visualEnvironmentFingerprint(claimedEnvironment);
      if (fingerprint === undefined) {
        fingerprint = next;
        record.environment = claimedEnvironment;
        return true;
      }
      return fingerprint === next;
    },
    acceptCaptureFacts(facts) {
      const next = JSON.stringify({
        source_format: facts.source_format,
        source_channel_order: facts.source_channel_order,
        source_encoding: facts.source_encoding,
        canonical_format: facts.canonical_format,
        canonical_channel_order: facts.canonical_channel_order,
        canonical_encoding: facts.canonical_encoding,
        configured_surface_color_space: facts.configured_surface_color_space,
        origin: facts.origin,
        normalization: facts.normalization,
      });
      if (captureFingerprint === undefined) {
        captureFingerprint = next;
        record.environment.capture = JSON.parse(next);
        return true;
      }
      return captureFingerprint === next;
    },
  };
}

function normalizeProvenance(input, startedAt, mode, runInitiation) {
  const value = input ?? {};
  requireCondition(value !== null && typeof value === "object" && !Array.isArray(value), "provenance must be an object");
  const implementationCommit = value.implementation_commit ?? null;
  requireCondition(implementationCommit === null || /^[0-9a-f]{40}$/.test(implementationCommit), "implementation commit must be null or a full lowercase Git SHA");
  let verifier = value.verifier ?? null;
  if (verifier !== null) {
    requireCondition(typeof verifier.path === "string" && verifier.path.length > 0, "verifier path is invalid");
    requireCondition(Number.isSafeInteger(verifier.byte_length) && verifier.byte_length > 0, "verifier byte length is invalid");
    requireCondition(/^[0-9a-f]{64}$/.test(verifier.sha256), "verifier SHA-256 is invalid");
    verifier = cloneJson(verifier);
  }
  const attendedLane = mode === "verify" ? VISUAL_ATTENDED_LANE : {
    id: "local-attended-private-webgpu-v1",
    execution: "visible_user_gesture",
    qualification: "exact_observed_lane_only",
  };
  requireCondition(typeof attendedLane.id === "string" && attendedLane.id.length > 0, "attended lane identity is invalid");
  return {
    implementation_commit: implementationCommit,
    verifier,
    observation_date: startedAt.slice(0, 10),
    package_artifact: null,
    attended_lane: cloneJson(attendedLane),
    run_initiation: runInitiation === null ? null : cloneJson(runInitiation),
    final_pin_required: implementationCommit === null || verifier === null,
  };
}

function summarizeEvidence(record, corpus) {
  const passedTrials = record.trials.filter((trial) => trial.passed).length;
  const failedTrials = record.trials.filter((trial) => !trial.passed).map((trial) => trial.trial_id);
  const failures = record.trials.flatMap((trial) => trial.failures.map((failure) => `${trial.trial_id}:${failure}`));
  if (record.trials.length !== corpus.trials.length) failures.push("trial_count");
  if (record.environment === null) failures.push("environment_missing");
  if (record.artifact_resources?.passed !== true) failures.push("total_encoded_artifact_bytes");
  if (record.mode === "verify" && record.provenance.final_pin_required) failures.push("provenance_final_pin_required");
  const rubricComplete = record.rubric?.review_status === "submitted"
    && RUBRIC_PROMPTS.every((prompt) => record.rubric.observation.answers[prompt].shown === true);
  if (!rubricComplete) failures.push("attended_rubric_incomplete");
  return {
    passed: failures.length === 0,
    trial_count: corpus.trials.length,
    completed_trials: record.trials.length,
    passed_trials: passedTrials,
    failed_trials: failedTrials,
    recreations_per_trial: RECREATION_COUNT,
    non_gating_rubric_complete: rubricComplete,
    artifact_count: session.artifacts.metadata().length,
    total_encoded_artifact_bytes: session.artifacts.totalEncodedBytes(),
    failures,
  };
}

function artifactResourceEvidence(corpus) {
  const totalEncodedBytes = session.artifacts.totalEncodedBytes();
  const ceiling = corpus.resource_limits.total_encoded_artifact_bytes;
  return {
    schema: "punctra-browser-visual-artifact-resources-v1",
    artifact_count: session.artifacts.metadata().length,
    total_encoded_artifact_bytes: totalEncodedBytes,
    total_encoded_artifact_bytes_ceiling: ceiling,
    passed: totalEncodedBytes <= ceiling,
  };
}

function failedTrial(trial, error) {
  const failure = errorRecord(error);
  return {
    trial_id: trial.id,
    source_id: trial.source_id,
    display_mode: trial.display_mode,
    projection: trial.camera === "source" ? "source" : trial.camera.projection,
    conditions: [...trial.conditions],
    coverage: {
      declared: trial.coverage,
      declared_authority: "source_or_authored_facts_only",
      settled_draw_authority: "presentation_only",
      query_completion: "not_inferred_from_visual_evidence",
    },
    input_facts: null,
    camera: trial.camera,
    selection: trial.selection,
    features: trial.features,
    expected_view: null,
    tolerance_profile: trial.tolerance_profile,
    temporal_tolerance_profile: trial.temporal_tolerance_profile,
    baseline: null,
    recreations: [],
    passed: false,
    failures: [`fatal:${failure.message}`],
    fatal_error: failure,
  };
}

function externalEvidenceNonclaims() {
  return {
    cross_browser: false,
    cross_operating_system: false,
    cross_adapter: false,
    cross_device: false,
    physical_display_presentation: false,
    independent_human: false,
    independent_adopter: false,
    professional_usability: false,
    registry_or_cdn_publication: false,
    final_visual_quality: false,
    support_qualified: false,
    api_stable: false,
    beta: false,
    release_candidate: false,
    v1: false,
  };
}

async function prepareRubricReview(policy, record) {
  requireCondition(document.visibilityState === "visible", "rubric images require a visible attended document");
  const plan = createRubricReviewPlan(policy, record.trials, session.artifacts.metadata());
  const review = {
    policy,
    plan,
    captureCompletedAt: record.capture_completed_at,
    presentations: {},
    selections: {},
    loadSequence: 0,
    presentationSequence: 0,
    selectionSequence: 0,
    ready: false,
  };
  session.rubricReview = review;
  rubricStatus.textContent = "Loading the fixed post-capture artifact bindings…";
  for (const prompt of RUBRIC_PROMPTS) {
    const row = rubricRow(prompt);
    row.dataset.reviewState = "loading";
    const gallery = row.querySelector("[data-rubric-gallery]");
    const status = row.querySelector("[data-rubric-presentation-status]");
    status.textContent = "Loading bound captures…";
    const planned = plan.prompts[prompt];
    const loadedArtifacts = await Promise.all(planned.artifact_identities.map(
      (artifact, index) => loadRubricPresentationImage({
        gallery,
        artifact,
        trialId: planned.trial_ids[index],
        review,
      }),
    ));
    await nextAnimationFrame();
    await nextAnimationFrame();
    requireCondition(document.visibilityState === "visible", `rubric ${prompt} was not presented in a visible document`);
    review.presentations[prompt] = {
      schema: RUBRIC_PRESENTATION_SCHEMA,
      presented_at: new Date().toISOString(),
      presentation_order: ++review.presentationSequence,
      document_visibility_state: document.visibilityState,
      artifacts: loadedArtifacts,
    };
    row.dataset.reviewState = "presented";
    status.textContent = `${loadedArtifacts.length} exact bound image${loadedArtifacts.length === 1 ? "" : "s"} presented.`;
  }
  review.ready = true;
  sessionLabel.disabled = false;
  for (const prompt of RUBRIC_PROMPTS) {
    const row = rubricRow(prompt);
    row.querySelector("select").disabled = false;
    row.querySelector("input").disabled = false;
  }
  updateRubricSubmitState();
}

async function submitRubricReview(programmaticAnswers, activation) {
  requireCondition(session.rubricReview?.ready === true, "post-capture rubric review is not ready");
  requireCondition(session.draft !== undefined && session.corpus !== undefined, "post-capture evidence draft is unavailable");
  const review = session.rubricReview;
  const draft = session.draft;
  const corpus = session.corpus;
  if (programmaticAnswers !== undefined) {
    requireCondition(draft.mode !== "verify", "verify rubric answers must be selected in the attended controls");
    applyProgrammaticRubricAnswers(programmaticAnswers);
  }
  requireCondition(
    RUBRIC_PROMPTS.every((prompt) => review.selections[prompt] !== undefined),
    "every rubric outcome must be explicitly selected before submission",
  );
  const submission = draft.mode === "verify"
    ? rubricSubmitGate.consume(activation, submitRubricButton.id)
    : null;
  const submittedAnswers = {};
  for (const prompt of RUBRIC_PROMPTS) {
    const row = rubricRow(prompt);
    const selection = review.selections[prompt];
    requireCondition(selection.outcome === row.querySelector("select").value, `rubric ${prompt} selection changed without an attended event`);
    submittedAnswers[prompt] = {
      outcome: selection.outcome,
      note: row.querySelector("input").value,
      presentation: review.presentations[prompt],
      selected_at: selection.selected_at,
      selection_order: selection.selection_order,
      selection_activation: selection.selection_activation,
    };
  }
  const submittedAt = new Date().toISOString();
  const observation = buildRubricObservation({
    policy: review.policy,
    plan: review.plan,
    captureCompletedAt: review.captureCompletedAt,
    submittedAt,
    submission,
    sessionLabel: sessionLabel.value,
    answers: submittedAnswers,
    requireTrustedControls: draft.mode === "verify",
  });
  draft.rubric = {
    schema: corpus.rubric.schema,
    gating: false,
    review_status: "submitted",
    observation,
  };
  draft.artifacts = session.artifacts.metadata();
  draft.artifact_resources = artifactResourceEvidence(corpus);
  draft.completed_at = submittedAt;
  draft.summary = summarizeEvidence(draft, corpus);
  const report = session.completeReview();
  setRubricControlsEnabled(false);
  const passed = report.summary.passed;
  updateRunnerState({
    status: passed ? "passed" : "failed",
    trial_id: null,
    recreation_index: null,
    message: passed
      ? `Passed ${report.summary.passed_trials}/${report.summary.trial_count} trials. Download and verify the evidence files.`
      : `Visual evidence failed: ${report.summary.failures.join("; ")}`,
  });
  rubricStatus.textContent = "Post-capture attended review submitted and frozen into the evidence record.";
  evidenceOutput.textContent = JSON.stringify(report, null, 2);
  downloadEvidenceButton.disabled = false;
  downloadBundleButton.disabled = false;
  setRunControlsEnabled(true);
  return cloneJson(report);
}

function applyProgrammaticRubricAnswers(value) {
  requireCondition(value !== null && typeof value === "object" && !Array.isArray(value), "programmatic rubric answers must be an object");
  requireCondition(
    RUBRIC_PROMPTS.every((prompt) => Object.hasOwn(value, prompt))
      && Object.keys(value).length === RUBRIC_PROMPTS.length,
    "programmatic rubric answers are incomplete",
  );
  for (const prompt of RUBRIC_PROMPTS) {
    const answer = value[prompt];
    requireCondition(answer !== null && typeof answer === "object" && !Array.isArray(answer), `programmatic rubric answer ${prompt} is invalid`);
    requireCondition(RUBRIC_OUTCOMES.includes(answer.outcome), `programmatic rubric outcome ${prompt} is invalid`);
    const row = rubricRow(prompt);
    row.querySelector("select").value = answer.outcome;
    row.querySelector("input").value = answer.note ?? "";
    recordRubricSelection(prompt);
  }
}

function recordRubricSelection(prompt, event) {
  const review = session.rubricReview;
  if (review?.ready !== true) return;
  const select = rubricRow(prompt).querySelector("select");
  requireCondition(RUBRIC_OUTCOMES.includes(select.value), `rubric ${prompt} requires an explicit outcome`);
  let selectionActivation = null;
  if (session.draft?.mode === "verify") {
    const issued = rubricSelectionGate.issue(event, {
      control: select,
      controlId: select.name,
      eventType: "change",
      visibilityState: document.visibilityState,
      userActivationIsActive: navigator.userActivation?.isActive === true,
    });
    selectionActivation = rubricSelectionGate.consume(issued, select.name);
  }
  const selectedAt = selectionActivation?.recorded_at ?? new Date().toISOString();
  review.selections[prompt] = {
    outcome: select.value,
    selected_at: selectedAt,
    selection_order: ++review.selectionSequence,
    selection_activation: selectionActivation,
  };
  updateRubricSubmitState();
}

function updateRubricSubmitState() {
  const review = session.rubricReview;
  submitRubricButton.disabled = review?.ready !== true
    || !RUBRIC_PROMPTS.every((prompt) => review.selections[prompt] !== undefined);
}

function loadRubricPresentationImage({ gallery, artifact, trialId, review }) {
  const source = session.artifacts.presentationSource(artifact.path);
  requireCondition(JSON.stringify(artifactIdentity(source.metadata)) === JSON.stringify(artifact), `rubric artifact ${artifact.path} identity changed before presentation`);
  const figure = document.createElement("figure");
  const image = document.createElement("img");
  image.alt = `${trialId} final recreation capture`;
  image.dataset.artifactPath = artifact.path;
  const caption = document.createElement("figcaption");
  caption.textContent = `${trialId} · ${artifact.path}`;
  figure.append(image, caption);
  gallery.append(figure);
  return new Promise((resolve, reject) => {
    image.addEventListener("load", () => {
      try {
        requireCondition(image.complete, `rubric artifact ${artifact.path} did not finish loading`);
        requireCondition(image.naturalWidth === artifact.width && image.naturalHeight === artifact.height, `rubric artifact ${artifact.path} decoded dimensions differ`);
        resolve({
          trial_id: trialId,
          path: artifact.path,
          loaded_at: new Date().toISOString(),
          load_order: ++review.loadSequence,
          natural_width: image.naturalWidth,
          natural_height: image.naturalHeight,
          complete: image.complete,
        });
      } catch (error) {
        reject(error);
      }
    }, { once: true });
    image.addEventListener("error", () => reject(new Error(`rubric artifact ${artifact.path} did not load`)), { once: true });
    image.src = source.url;
  });
}

function rubricRow(prompt) {
  const row = document.querySelector(`[data-rubric-prompt="${prompt}"]`);
  requireCondition(row !== null, `rubric row ${prompt} is absent`);
  return row;
}

function configureRubricFields() {
  const labels = {
    depth: "Depth",
    shape: "Shape",
    density_transition: "Density transition",
    color_meaning: "Color meaning",
    selection: "Selection",
    false_feature: "False feature",
  };
  for (const prompt of RUBRIC_PROMPTS) {
    const row = document.querySelector(`[data-rubric-prompt="${prompt}"]`);
    const outcomeLabel = document.createElement("label");
    outcomeLabel.textContent = labels[prompt];
    const select = document.createElement("select");
    select.name = `rubric-${prompt}`;
    select.disabled = true;
    const placeholder = document.createElement("option");
    placeholder.value = "";
    placeholder.textContent = "Select an outcome";
    placeholder.disabled = true;
    placeholder.selected = true;
    select.append(placeholder);
    for (const outcome of RUBRIC_OUTCOMES) {
      const option = document.createElement("option");
      option.value = outcome;
      option.textContent = outcome.replaceAll("_", " ");
      select.append(option);
    }
    select.addEventListener("change", (event) => recordRubricSelection(prompt, event));
    outcomeLabel.append(select);
    const noteLabel = document.createElement("label");
    noteLabel.textContent = `${labels[prompt]} note`;
    const note = document.createElement("input");
    note.maxLength = 280;
    note.autocomplete = "off";
    note.placeholder = "Optional, max 280 characters";
    note.disabled = true;
    noteLabel.append(note);
    const presentationStatus = document.createElement("p");
    presentationStatus.dataset.rubricPresentationStatus = "";
    presentationStatus.textContent = "Waiting for post-capture images.";
    const gallery = document.createElement("div");
    gallery.dataset.rubricGallery = "";
    gallery.className = "rubric-gallery";
    row.append(presentationStatus, gallery, outcomeLabel, noteLabel);
  }
}

function configureModeFromUrl() {
  const requested = new URL(location.href).searchParams.get("mode");
  if (requested === "record" || requested === "verify") modeSelect.value = requested;
}

async function configureVerifyProvenance() {
  const sequence = ++provenanceConfigurationSequence;
  session.verifyProvenance = null;
  runButton.disabled = true;
  if (modeSelect.value !== "verify") {
    provenanceStatus.textContent = "Record mode creates commit-free baseline inputs; final pin provenance is intentionally absent.";
    if (!modeSelect.disabled) runButton.disabled = false;
    return;
  }
  provenanceStatus.textContent = "Validating final verify pins against the running checkout…";
  try {
    const provenance = await loadVisualVerifyProvenance(window.location.href);
    if (sequence !== provenanceConfigurationSequence || modeSelect.value !== "verify") return;
    session.verifyProvenance = provenance;
    provenanceStatus.textContent = session.verifyProvenance === null
      ? "Final verify requires the documented implementation commit and verifier identity in this page URL."
      : `Final verify pins match running implementation ${session.verifyProvenance.implementation_commit}.`;
  } catch (error) {
    if (sequence !== provenanceConfigurationSequence || modeSelect.value !== "verify") return;
    provenanceStatus.textContent = `Final verify provenance rejected: ${errorMessage(error)}`;
  }
  if (!modeSelect.disabled) runButton.disabled = verifyProvenanceMissing();
}

function verifyProvenanceMissing() {
  return modeSelect.value === "verify" && session.verifyProvenance === null;
}

function buildProgressList(trials) {
  progressList.replaceChildren(...trials.map((trial) => {
    const item = document.createElement("li");
    item.dataset.trialId = trial.id;
    item.dataset.state = "pending";
    item.textContent = `${trial.id} — pending`;
    return item;
  }));
  progressCount.textContent = `0 / ${trials.length}`;
}

function markTrialProgress(trialId, state, detail) {
  const item = [...progressList.children].find((entry) => entry.dataset.trialId === trialId);
  if (item === undefined) return;
  item.dataset.state = state;
  item.textContent = `${trialId} — ${detail}`;
}

function updateRunnerState(patch) {
  publishRunnerState(session.updateRunnerState(patch));
}

function publishRunnerState(state) {
  document.body.dataset.visualBaseline = state.status;
  statusOutput.textContent = state.message;
  progressCount.textContent = `${state.completed_trials} / ${state.total_trials}`;
}

function publishPartialRecord(record) {
  evidenceOutput.textContent = JSON.stringify({
    schema: record.schema,
    mode: record.mode,
    environment: record.environment,
    trials: record.trials,
  }, null, 2);
}

function setRunControlsEnabled(enabled) {
  runButton.disabled = !enabled || verifyProvenanceMissing();
  modeSelect.disabled = !enabled;
  sessionLabel.disabled = !enabled;
}

function setRubricControlsEnabled(enabled) {
  document.querySelectorAll("#rubric-fields select, #rubric-fields input").forEach((control) => {
    control.disabled = !enabled;
  });
  submitRubricButton.disabled = !enabled;
}

function resetRubricReview() {
  session.rubricReview = undefined;
  setRubricControlsEnabled(false);
  rubricStatus.textContent = "Run the corpus first. Exact captured images will be bound and presented here afterward.";
  for (const prompt of RUBRIC_PROMPTS) {
    const row = rubricRow(prompt);
    row.dataset.reviewState = "waiting";
    row.querySelector("select").value = "";
    row.querySelector("input").value = "";
    row.querySelector("[data-rubric-gallery]").replaceChildren();
    row.querySelector("[data-rubric-presentation-status]").textContent = "Waiting for post-capture images.";
  }
}

function downloadEvidence() {
  requireCondition(session.report !== undefined, "no visual evidence record is available");
  const bytes = evidenceJsonBytes();
  requireCondition(session.transportPolicy !== undefined, "visual transport policy is unavailable");
  requireCondition(bytes.byteLength <= session.transportPolicy.maximum_evidence_json_bytes, "evidence JSON exceeds its byte ceiling");
  triggerBlobDownload(bytes, "application/json", EVIDENCE_FILENAME);
  return {
    filename: EVIDENCE_FILENAME,
    mime_type: "application/json",
    byte_length: bytes.byteLength,
    byte_length_ceiling: session.transportPolicy.maximum_evidence_json_bytes,
  };
}

async function downloadBundle() {
  requireCondition(session.report !== undefined, "no visual evidence record is available");
  requireCondition(session.transportPolicy !== undefined && session.artifactByteCeiling !== undefined, "visual transport policy is unavailable");
  const evidenceBytes = evidenceJsonBytes();
  requireCondition(evidenceBytes.byteLength <= session.transportPolicy.maximum_evidence_json_bytes, "evidence JSON exceeds its byte ceiling");
  const artifactEntries = session.artifacts.bundleEntries();
  const encodedArtifactBytes = artifactEntries.reduce((total, entry) => total + entry.bytes.byteLength, 0);
  requireCondition(encodedArtifactBytes === session.artifacts.totalEncodedBytes(), "transport artifact byte accounting differs");
  requireCondition(encodedArtifactBytes <= session.artifactByteCeiling, "transport artifacts exceed their byte ceiling");
  requireCondition(
    session.baselineInputsEntry === undefined
      || session.baselineInputsEntry.bytes.byteLength <= session.transportPolicy.maximum_baseline_inputs_json_bytes,
    "baseline-input manifest exceeds its byte ceiling",
  );
  const archive = encodeVisualArchive([
    ...artifactEntries,
    ...(session.baselineInputsEntry === undefined ? [] : [{
      path: session.baselineInputsEntry.path,
      bytes: session.baselineInputsEntry.bytes,
    }]),
    { path: session.transportPolicy.evidence_repository_path, bytes: evidenceBytes },
  ], {
    maximumEntries: session.transportPolicy.maximum_entries,
    maximumArchiveBytes: session.transportPolicy.maximum_archive_bytes,
  });
  requireCondition(
    archive.facts.archive_structure_bytes <= session.transportPolicy.maximum_archive_structure_bytes,
    "transport archive structure exceeds its byte ceiling",
  );
  const archiveOverheadBytes = archive.bytes.byteLength - encodedArtifactBytes;
  requireCondition(
    archiveOverheadBytes <= session.transportPolicy.maximum_archive_overhead_bytes,
    "transport archive overhead exceeds its byte ceiling",
  );
  const sha256 = await sha256Hex(archive.bytes);
  const transport = visualArchiveTransportFromUrl(window.location.href);
  let localExportReceipt = null;
  if (transport === "same-origin-local-server") {
    transportStatus.textContent = "Writing the bounded TAR once through the opt-in same-origin local server…";
    localExportReceipt = await exportVisualArchiveToLocalServer({
      archiveBytes: archive.bytes,
      filename: session.transportPolicy.archive_filename,
      sha256,
      pageUrl: window.location.href,
    });
    downloadBundleButton.disabled = true;
    transportStatus.textContent = `Local TAR export verified: ${localExportReceipt.byte_length} bytes, SHA-256 ${localExportReceipt.sha256}.`;
  } else {
    triggerBlobDownload(archive.bytes, "application/x-tar", session.transportPolicy.archive_filename);
    transportStatus.textContent = "Standard browser TAR download prepared. Extract it only into a fresh directory.";
  }
  return {
    ...archive.facts,
    filename: session.transportPolicy.archive_filename,
    mime_type: "application/x-tar",
    sha256,
    encoded_artifact_bytes: encodedArtifactBytes,
    evidence_json_bytes: evidenceBytes.byteLength,
    baseline_inputs_bytes: session.baselineInputsEntry?.bytes.byteLength ?? 0,
    allocation_bytes: archive.bytes.byteLength,
    allocation_ceiling_bytes: session.transportPolicy.maximum_archive_bytes,
    allocation_overhead_bytes: archiveOverheadBytes,
    allocation_overhead_ceiling_bytes: session.transportPolicy.maximum_archive_overhead_bytes,
    evidence_artifact: false,
    private_transport_only: true,
    transport,
    local_export_receipt: localExportReceipt,
  };
}

function evidenceJsonBytes() {
  return new TextEncoder().encode(`${JSON.stringify(session.report, null, 2)}\n`);
}

function triggerBlobDownload(bytes, mimeType, filename) {
  const url = URL.createObjectURL(new Blob([bytes], { type: mimeType }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.textContent = `Download prepared ${filename}`;
  const item = document.createElement("li");
  item.dataset.privateTransportDownload = filename;
  item.append(link);
  artifactList.prepend(item);
  link.click();
  setTimeout(() => {
    item.remove();
    URL.revokeObjectURL(url);
  }, 300_000);
}

function configureArchiveTransport() {
  const transport = visualArchiveTransportFromUrl(window.location.href);
  document.body.dataset.visualArchiveTransport = transport;
  transportStatus.textContent = transport === "same-origin-local-server"
    ? "Opt-in same-origin local TAR export is active. The server will accept one bounded archive without overwrite."
    : "Standard browser TAR download is active.";
}

function repositoryBaselinePath(trial) {
  return `apps/browser-demo/web/fixtures/visual-v1/${trial.baseline_path.replace(/^\.\//, "")}`;
}

function observationArtifactPath(trialId, recreationIndex, label) {
  return `${VISUAL_ARTIFACT_ROOT}/${trialId}-recreation-${recreationIndex}-${label}.png`;
}

function validateMode(value) {
  requireCondition(value === "record" || value === "verify", "visual mode must be record or verify");
  return value;
}

function withoutImage(capture) {
  const { image: _image, ...facts } = capture;
  return facts;
}

function errorRecord(error) {
  return {
    name: error?.name ?? "Error",
    message: error?.message ?? String(error),
  };
}


function nextAnimationFrame() {
  return new Promise((resolve) => requestAnimationFrame(resolve));
}

class ArtifactRegistry {
  #artifacts = [];
  #entries = [];
  #list;

  constructor(list) {
    this.#list = list;
  }

  async createPng(image, descriptor) {
    const artifactStarted = performance.now();
    const pngStarted = performance.now();
    const bytes = await encodeRgba8Png(image);
    const pngEncodeMilliseconds = performance.now() - pngStarted;
    const encodedSha256 = await sha256Hex(bytes);
    const decodedSha256 = await sha256Hex(image.data);
    const artifactEncodingMilliseconds = performance.now() - artifactStarted;
    const metadata = await createPngArtifactMetadata({
      descriptor,
      encodedBytes: bytes,
      image,
      identities: {
        encoded_sha256: encodedSha256,
        decoded_sha256: decodedSha256,
      },
      timing: {
        encode_milliseconds: pngEncodeMilliseconds,
        png_encode_milliseconds: pngEncodeMilliseconds,
        artifact_encoding_milliseconds: artifactEncodingMilliseconds,
      },
    });
    this.#register(metadata, bytes);
    return { metadata, bytes };
  }

  recordMetadata(metadata, bytes) {
    if (this.#artifacts.some((artifact) => artifact.path === metadata.path)) return;
    requireCondition(bytes instanceof Uint8Array, `artifact ${metadata.path} transport bytes are unavailable`);
    this.#register(metadata, bytes);
  }

  metadata() {
    return cloneJson(this.#artifacts);
  }

  totalEncodedBytes() {
    return this.#artifacts.reduce((total, artifact) => total + artifact.encoded_byte_length, 0);
  }

  maximumEncodedBytes() {
    return this.#artifacts.reduce(
      (maximum, artifact) => Math.max(maximum, artifact.encoded_byte_length),
      0,
    );
  }

  totalEncodeMilliseconds() {
    return this.#artifacts.reduce(
      (total, artifact) => total + (artifact.encode_milliseconds ?? 0),
      0,
    );
  }

  totalArtifactEncodingMilliseconds() {
    return this.#artifacts.reduce(
      (total, artifact) => total + (artifact.artifact_encoding_milliseconds ?? 0),
      0,
    );
  }

  bundleEntries() {
    requireCondition(this.#entries.length === this.#artifacts.length, "artifact transport registry is incomplete");
    return this.#entries.map(({ path, bytes }) => ({ path, bytes }));
  }

  presentationSource(path) {
    const artifact = this.#artifacts.find((candidate) => candidate.path === path);
    const entry = this.#entries.find((candidate) => candidate.path === path);
    requireCondition(artifact !== undefined && entry !== undefined, `artifact ${path} is unavailable for attended presentation`);
    return { metadata: cloneJson(artifact), url: entry.url };
  }

  clear() {
    for (const entry of this.#entries) URL.revokeObjectURL(entry.url);
    this.#entries = [];
    this.#artifacts = [];
    this.#list.replaceChildren();
    downloadEvidenceButton.disabled = true;
    downloadBundleButton.disabled = true;
  }

  #register(metadata, bytes) {
    requireCondition(!this.#artifacts.some((artifact) => artifact.path === metadata.path), `duplicate artifact path ${metadata.path}`);
    this.#artifacts.push(cloneJson(metadata));
    const url = URL.createObjectURL(new Blob([bytes], { type: metadata.mime_type }));
    const item = document.createElement("li");
    const link = document.createElement("a");
    link.href = url;
    link.download = metadata.path.replaceAll("/", "--");
    link.textContent = `${metadata.kind}: ${metadata.path} (${metadata.encoded_byte_length} bytes)`;
    item.append(link);
    this.#list.append(item);
    this.#entries.push({ url, item, path: metadata.path, bytes });
  }
}

const session = new VisualRunSession({
  artifactRegistry: new ArtifactRegistry(artifactList),
  runnerStateSchema: RUNNER_STATE_SCHEMA,
});

configureModeFromUrl();
void configureVerifyProvenance();
configureArchiveTransport();
configureRubricFields();
publishRunnerState(session.runnerState());

window.__PUNCTRA_BROWSER_VISUAL__ = Object.freeze({
  schema: RUNNER_STATE_SCHEMA,
  run: (options = {}) => startRun(options),
  submitReview: (answers) => submitRubricReview(answers),
  state: () => session.runnerState(),
  report: () => session.report === undefined ? null : cloneJson(session.report),
  draft: () => session.draft === undefined ? null : cloneJson(session.draft),
  baselineInputs: () => session.baselineInputsEntry === undefined
    ? null
    : cloneJson(session.baselineInputsEntry.manifest),
  artifacts: () => session.artifacts.metadata(),
  downloadEvidence: () => downloadEvidence(),
  downloadBundle: () => downloadBundle(),
});

runButton.addEventListener("click", (event) => {
  try {
    const activation = modeSelect.value === "verify"
      ? runControlGate.issue(event, {
        control: runButton,
        controlId: runButton.id,
        visibilityState: document.visibilityState,
        userActivationIsActive: navigator.userActivation?.isActive === true,
      })
      : undefined;
    void startRun({
      mode: modeSelect.value,
      provenance: modeSelect.value === "verify" ? session.verifyProvenance : undefined,
    }, activation).catch((error) => {
      statusOutput.textContent = `Visual run failed: ${errorMessage(error)}`;
    });
  } catch (error) {
    statusOutput.textContent = `Visual run failed: ${errorMessage(error)}`;
  }
});
modeSelect.addEventListener("change", () => {
  void configureVerifyProvenance();
});
downloadEvidenceButton.addEventListener("click", () => downloadEvidence());
downloadBundleButton.addEventListener("click", () => {
  void downloadBundle().catch((error) => {
    transportStatus.textContent = `Bundle export failed: ${errorMessage(error)}`;
  });
});
submitRubricButton.addEventListener("click", (event) => {
  try {
    const activation = session.draft?.mode === "verify"
      ? rubricSubmitGate.issue(event, {
        control: submitRubricButton,
        controlId: submitRubricButton.id,
        visibilityState: document.visibilityState,
        userActivationIsActive: navigator.userActivation?.isActive === true,
      })
      : undefined;
    void submitRubricReview(undefined, activation).catch((error) => {
      rubricStatus.textContent = `Post-capture review failed: ${errorMessage(error)}`;
    });
  } catch (error) {
    rubricStatus.textContent = `Post-capture review failed: ${errorMessage(error)}`;
  }
});

requireCondition(DISPLAY_MODES.length === 5, "display-mode contract differs");
