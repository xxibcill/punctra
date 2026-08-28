import initializeWasm, { createViewer as createRawViewer } from "./pkg/browser_demo.js";
import { encodeVisualArchive } from "./visual-archive.js";
import {
  decodeTransferV2,
  generateVisualScene,
  loadVisualCorpus,
  materializeVisualTrial,
  projectAuthoredPointAtViewport,
} from "./visual-corpus.js";
import {
  captureCanonicalFrame,
  parseRawJson,
  summarizeSamples,
} from "./visual-capture.js";
import { compareCanonicalImages, measureCoverage } from "./visual-comparison.js";
import {
  createPngArtifactMetadata,
  decodeRgba8Png,
  encodeRgba8Png,
  sha256Hex,
} from "./visual-png.js";
import { loadFootprintCorpus } from "./footprint-corpus.js";
import {
  FOOTPRINT_BASELINE_SCHEMA,
  FOOTPRINT_EVIDENCE_SCHEMA,
  FOOTPRINT_EXTERNAL_NONCLAIMS,
  FOOTPRINT_LOCAL_TEST_CASE_IDS,
  FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
  FOOTPRINT_UNAVAILABLE_MEASUREMENTS,
  createComponentBridgeMetricBinding,
  createFootprintMetricBinding,
  createPointFootprintImageArtifact,
  createPointFootprintResourceEvidence,
  createTopologyMetricBinding,
  derivePointFootprintEvidenceSummary,
  evaluateDenseSolidBlockBudget,
  projectedDensityDisplayDiameter,
  summarizeFootprintTiming,
  validatePointFootprintBaseline,
  validatePointFootprintLocalTestArtifact,
  validatePointFootprintRunInputs,
} from "./footprint-evidence.js";
import {
  FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
  exportFootprintArchiveToLocalServer,
  footprintArchiveTransportFromUrl,
} from "./footprint-export.js";
import {
  evaluateRepresentativeTiming,
  measureIsolatedFootprint,
  measureOccupancyComponentBridges,
  measureOccupancyTopology,
} from "./footprint-runner-core.js";
import { createVisualValidator, errorMessage } from "./visual-validation.js";

const EVIDENCE_SCHEMA = FOOTPRINT_EVIDENCE_SCHEMA;
const BASELINE_SCHEMA = FOOTPRINT_BASELINE_SCHEMA;
const FOOTPRINT_CORPUS_URL = new URL("./fixtures/footprint-v1/corpus.json", import.meta.url);
const BASELINE_URL = new URL("./qualification-footprint-baseline.json", globalThis.location.href);
const LOCAL_TEST_EVIDENCE_URL = new URL(
  "./fixtures/footprint-v1/local-test-evidence.json",
  import.meta.url,
);
const RUNTIME_PATHS = Object.freeze([
  "./package.json",
  "./pkg/browser_demo.js",
  "./pkg/browser_demo_bg.wasm",
]);
const BASELINE_REPOSITORY_PATH = "docs/releases/v0.22-browser-point-footprint-baseline.json";
const EVIDENCE_REPOSITORY_PATH = "docs/releases/v0.22-browser-point-footprint-evidence.json";
const ARTIFACT_ROOT = "docs/releases/v0.22-browser-point-footprint-artifacts";
const BACKGROUND_RGBA = Object.freeze([19, 20, 19, 255]);
const MAX_ARCHIVE_ENTRIES = 128;
const MAX_ARCHIVE_BYTES = 134_217_728;
const TRANSFER_VERTEX_BYTES = 24;
const { requireCondition } = createVisualValidator("Point-footprint runner failed");

const canvas = document.querySelector("#footprint-canvas");
const runButton = document.querySelector("#run-footprint");
const modeSelect = document.querySelector("#footprint-mode");
const sessionInput = document.querySelector("#footprint-session");
const statusOutput = document.querySelector("#footprint-status");
const progressOutput = document.querySelector("#footprint-progress");
const progressCount = document.querySelector("#footprint-progress-count");
const evidenceOutput = document.querySelector("#footprint-evidence");
const artifactOutput = document.querySelector("#footprint-artifacts");
const transportOutput = document.querySelector("#footprint-transport-status");
const downloadArchiveButton = document.querySelector("#download-footprint-archive");
const requestedOutput = document.querySelector("#requested-footprint");
const selectedOutput = document.querySelector("#resolved-footprint");
const displaySizeOutput = document.querySelector("#display-diameter");
const pickSizeOutput = document.querySelector("#pick-diameter");
const transientOutput = document.querySelector("#transient-bytes");

let loadedContext;
let activeRun = false;
let latestArchive;
let latestEvidence;

async function initializePage() {
  try {
    const footprint = await loadFootprintCorpus(FOOTPRINT_CORPUS_URL);
    const visualUrl = new URL(footprint.corpus.predecessor.corpus.path, footprint.url);
    const visual = await loadVisualCorpus(visualUrl);
    loadedContext = { footprint, visual };
    buildProgress(footprint.corpus.canonical_trials);
    updateState("ready", "Ready. Choose record or verify, then start the bounded attended run.");
    runButton.disabled = false;
    configureTransportLabel();
  } catch (error) {
    updateState("failed", errorMessage(error));
    evidenceOutput.textContent = JSON.stringify(errorRecord(error), null, 2);
  }
}

async function startRun(options = {}, activation) {
  requireCondition(!activeRun, "a qualification run is already active");
  requireCondition(loadedContext !== undefined, "closed corpora are not loaded");
  const activationFacts = validateTrustedActivation(activation);
  const mode = validateMode(options.mode ?? modeSelect.value);
  const sessionLabel = validateSessionLabel(options.sessionLabel ?? sessionInput.value);
  activeRun = true;
  latestArchive = undefined;
  latestEvidence = undefined;
  runButton.disabled = true;
  modeSelect.disabled = true;
  sessionInput.disabled = true;
  downloadArchiveButton.disabled = true;
  artifactOutput.replaceChildren();
  resetProgress();
  updateState("running", "Binding implementation, verifier, runtime, and predecessor artifacts…");

  try {
    const result = await runQualification({ mode, sessionLabel, activationFacts });
    latestEvidence = result.evidence;
    latestArchive = result.archive;
    evidenceOutput.textContent = JSON.stringify(result.evidence, null, 2);
    downloadArchiveButton.disabled = false;
    const state = result.evidence.summary.passed ? "passed" : "failed";
    updateState(
      state,
      result.evidence.summary.passed
        ? `PASS — ${result.evidence.summary.canonical_trials}/9 canonical trials, focused DPR checks, and resource fallback are bound.`
        : `FAIL — ${result.evidence.summary.failures.join("; ")}`,
    );
    if (result.transportReceipt !== null) {
      transportOutput.textContent = `Archive persisted: ${result.transportReceipt.path}`;
    }
    return structuredClone(result.evidence);
  } catch (error) {
    const failure = errorRecord(error);
    evidenceOutput.textContent = JSON.stringify(failure, null, 2);
    updateState("failed", failure.message);
    throw error;
  } finally {
    setCanvasProfile(loadedContext.footprint.corpus.canonical_profile);
    resetReadouts();
    activeRun = false;
    runButton.disabled = false;
    modeSelect.disabled = false;
    sessionInput.disabled = false;
  }
}

async function runQualification({ mode, sessionLabel, activationFacts }) {
  const startedAt = new Date().toISOString();
  const { footprint, visual } = loadedContext;
  const [pins, host, localTestBundle, runtime] = await Promise.all([
    loadJson("./qualification-footprint-pins.json", "point-footprint pins"),
    loadJson("./qualification-host.json", "qualification host"),
    loadJsonWithBytes(LOCAL_TEST_EVIDENCE_URL, "local point-footprint test evidence"),
    loadRuntimeArtifacts(),
  ]);
  validateRunPins(mode, pins);
  validatePointFootprintRunInputs({ footprint, visual }, pins.running);
  validatePointFootprintLocalTestArtifact(
    localTestBundle.json,
    pins.running.implementation.commit,
  );
  requireCondition(
    JSON.stringify(runtime.records) === JSON.stringify(pins.running.runtime.artifacts),
    "loaded runtime bytes differ from the running runtime pins",
  );
  await initializeWasm({ module_or_path: runtime.wasmBytes });
  const loadedBaseline = mode === "verify" ? await loadBaseline(pins, footprint.corpus) : null;
  const baseline = loadedBaseline?.manifest ?? null;
  const artifacts = new ArtifactRegistry(artifactOutput);
  const localTestMetadata = await artifacts.addBytes(
    `${ARTIFACT_ROOT}/local-test-evidence.json`,
    localTestBundle.bytes,
    "local_test_results",
  );
  const baselineArtifacts = [];
  const canonicalTrials = [];
  const canonicalObservations = new Map();
  let completed = 0;

  for (const trialContract of footprint.corpus.canonical_trials) {
    markProgress(trialContract.id, "running", "three fresh viewers");
    updateState("running", `Canonical trial ${completed + 1}/9: ${trialContract.id}`);
    try {
      const result = await runCanonicalTrial({
        trialContract,
        footprint: footprint.corpus,
        visual: visual.corpus,
        visualUrl: visual.corpus_url,
        mode,
        baseline,
        artifacts,
        baselineArtifacts,
      });
      canonicalTrials.push(result.record);
      canonicalObservations.set(trialContract.id, result.firstObservation);
      markProgress(trialContract.id, result.record.passed ? "passed" : "failed", result.record.passed ? "bound" : result.record.failures.join("; "));
    } catch (error) {
      canonicalTrials.push(failedCanonicalTrial(trialContract.id, error));
      markProgress(trialContract.id, "failed", errorMessage(error));
    }
    completed += 1;
    progressCount.textContent = `${completed} / ${footprint.corpus.canonical_trials.length}`;
  }

  const focusedTrials = await runFocusedScaleTrials({
    footprint: footprint.corpus,
    visual: visual.corpus,
    visualUrl: visual.corpus_url,
    mode,
    baseline,
    artifacts,
    baselineArtifacts,
    canonicalObservations,
  });
  const fallback = await runResourceFallback({
    footprint: footprint.corpus,
    visual: visual.corpus,
    visualUrl: visual.corpus_url,
    localTests: localTestBundle.json,
  });

  const baselineRecord = mode === "record"
    ? createBaselineRecord({
      footprint,
      pins: pins.running,
      canonicalTrials,
      focusedTrials,
      fallback,
      baselineArtifacts,
    })
    : baseline;
  let baselineIdentityRecord = loadedBaseline?.identity;
  if (mode === "record") {
    const baselineBytes = encodeJson(baselineRecord);
    baselineIdentityRecord = {
      path: BASELINE_REPOSITORY_PATH,
      byte_length: baselineBytes.byteLength,
      sha256: await sha256Hex(baselineBytes),
    };
    await artifacts.addBytes(BASELINE_REPOSITORY_PATH, baselineBytes, "baseline_manifest");
  }
  requireCondition(typeof sessionLabel === "string" && activationFacts.trusted_user_activation,
    "attended session facts disappeared before evidence assembly");
  requireCondition(document.visibilityState === "visible",
    "the attended page became hidden before evidence assembly");
  const evidence = createEvidenceRecord({
    startedAt,
    footprint: footprint.corpus,
    visual: visual.corpus,
    pins: mode === "verify" ? pins.accepted : pins.running,
    host,
    baseline: baselineRecord,
    baselineIdentity: baselineIdentityRecord,
    localTests: localTestBundle.json,
    localTestMetadata,
    canonicalTrials,
    focusedTrials,
    fallback,
  });
  const evidenceBytes = encodeJson(evidence);
  await artifacts.addBytes(EVIDENCE_REPOSITORY_PATH, evidenceBytes, "evidence_json");
  const archive = encodeVisualArchive(artifacts.entries(), {
    maximumEntries: MAX_ARCHIVE_ENTRIES,
    maximumArchiveBytes: MAX_ARCHIVE_BYTES,
  });
  const archiveSha256 = await sha256Hex(archive.bytes);
  const transportReceipt = await publishArchive(archive.bytes, archiveSha256);
  return {
    evidence,
    archive: {
      bytes: archive.bytes,
      sha256: archiveSha256,
      facts: archive.facts,
    },
    transportReceipt,
  };
}

async function runCanonicalTrial(options) {
  const {
    trialContract,
    footprint,
    visual,
    visualUrl,
    mode,
    baseline,
    artifacts,
    baselineArtifacts,
  } = options;
  const trial = visual.trials.find(({ id }) => id === trialContract.id);
  requireCondition(trial !== undefined, `inherited trial ${trialContract.id} is absent`);
  const predecessor = await loadPng(new URL(trialContract.predecessor_baseline.path, FOOTPRINT_CORPUS_URL));
  requireCondition(predecessor.bytes.byteLength === trialContract.predecessor_baseline.byte_length, `predecessor ${trial.id} byte length differs`);
  requireCondition(await sha256Hex(predecessor.bytes) === trialContract.predecessor_baseline.sha256, `predecessor ${trial.id} SHA-256 differs`);
  const materialized = await materializeVisualTrial(visual, trial.id, { corpusUrl: visualUrl });
  const baselineImage = mode === "verify" ? await loadCanonicalBaseline(baseline, trial.id) : null;
  const recreations = [];
  let firstObservation;
  let referenceImage = baselineImage?.image;

  for (let recreationIndex = 0; recreationIndex < footprint.canonical_profile.recreations; recreationIndex += 1) {
    const recreation = await runViewerCapture({
      profile: footprint.canonical_profile,
      visual,
      trial,
      materialized,
      quietFrames: footprint.canonical_profile.quiet_frames,
      predecessorTiming: trialContract.predecessor_timing,
      timingLimits: footprint.timing_limits,
      applyHighlights: true,
      verifyPicks: true,
    });
    updateReadouts(recreation.diagnostics);
    const artifactPath = `${ARTIFACT_ROOT}/canonical/${trial.id}-recreation-${recreationIndex}.png`;
    const artifact = await artifacts.addPng(recreation.image, {
      kind: "canonical_candidate_png",
      path: artifactPath,
      trial_id: trial.id,
      recreation_index: recreationIndex,
      frame_index: null,
    });
    if (firstObservation === undefined) {
      firstObservation = { image: recreation.image, recreation_index: recreationIndex };
    }
    if (referenceImage === undefined) referenceImage = recreation.image;
    const repeatability = compareCanonicalImages(referenceImage, recreation.image, {
      toleranceProfile: visual.tolerance_profiles[trial.tolerance_profile],
      features: trial.features,
      backgroundRgba: visual.presentation_policy.canonical_clear_rgba8,
    });
    const topology = measureOccupancyTopology(recreation.image, {
      rectangle: fullRectangle(recreation.image),
      backgroundRgba: BACKGROUND_RGBA,
    });
    const predecessorTopology = measureOccupancyTopology(predecessor.image, {
      rectangle: fullRectangle(predecessor.image),
      backgroundRgba: BACKGROUND_RGBA,
    });
    const componentBridges = measureOccupancyComponentBridges(
      predecessor.image,
      recreation.image,
      {
        rectangle: fullRectangle(recreation.image),
        backgroundRgba: BACKGROUND_RGBA,
        minimumClearSeparationPixels:
          footprint.metric_limits.minimum_component_clear_separation_pixels,
      },
    );
    const featureComparisons = compareFeatureFacts(predecessor.image, recreation.image, trial.features);
    const densityComparisons = compareDenseRegions(
      predecessor.image,
      recreation.image,
      footprint.focused_trials.find(({ id }) => id === trial.id)?.dense_regions ?? [],
      footprint.metric_limits,
    );
    const quality = evaluateCanonicalQuality({
      topology: topology.metrics,
      predecessorTopology: predecessorTopology.metrics,
      featureComparisons,
      densityComparisons,
      componentBridges: componentBridges.metrics,
      limits: footprint.metric_limits,
    });
    const failures = [];
    if (!repeatability.passed) failures.push(...repeatability.failures.map((failure) => `repeatability:${failure}`));
    if (!recreation.timing_evaluation.passed) failures.push(...recreation.timing_evaluation.failures.map((failure) => `timing:${failure}`));
    if (!quality.passed) failures.push(...quality.failures.map((failure) => `quality:${failure}`));
    if (recreation.nominalPicks.some(({ matched }) => !matched)) failures.push("nominal_pick_identity");
    recreations.push({
      index: recreationIndex,
      point_footprint: structuredClone(recreation.diagnostics.point_footprint),
      adapter: recreation.adapter,
      lifecycle_timing: recreation.lifecycle_timing,
      representative_timing: recreation.representative_timing,
      timing_evaluation: recreation.timing_evaluation,
      resources: recreation.resources,
      nominal_picks: recreation.nominalPicks,
      capture: { facts: recreation.captureFacts, artifact: artifact.metadata },
      repeatability,
      predecessor_topology: predecessorTopology,
      candidate_topology: topology,
      component_bridges: componentBridges,
      feature_comparisons: featureComparisons,
      dense_region_comparisons: densityComparisons,
      quality,
      passed: failures.length === 0,
      failures,
    });
    if (firstObservation?.recreation === undefined && recreationIndex === 0) {
      firstObservation.recreation = recreations.at(-1);
    }
    if (mode === "record" && recreationIndex === 0) {
      const baselinePath = `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}.png`;
      const baselineArtifact = await artifacts.addPng(recreation.image, {
        kind: "canonical_baseline_png",
        path: baselinePath,
        trial_id: trial.id,
        recreation_index: null,
        frame_index: null,
      });
      baselineArtifacts.push({
        kind: "canonical",
        trial_id: trial.id,
        profile_id: footprint.canonical_profile.id,
        web_path: `./fixtures/footprint-v1/baselines/${trial.id}.png`,
        artifact: baselineArtifact.metadata,
      });
    }
  }

  const failures = recreations.flatMap((recreation) => (
    recreation.passed ? [] : recreation.failures.map((failure) => `recreation-${recreation.index}:${failure}`)
  ));
  return {
    firstObservation,
    record: {
      trial_id: trial.id,
      predecessor_artifact: structuredClone(trialContract.predecessor_baseline),
      recreation_count: recreations.length,
      recreations,
      passed: failures.length === 0,
      failures,
    },
  };
}

async function runViewerCapture(options) {
  const {
    profile,
    visual,
    trial,
    materialized,
    quietFrames,
    predecessorTiming,
    timingLimits,
    applyHighlights,
    verifyPicks,
  } = options;
  const started = performance.now();
  let viewer = await createViewerForProfile(profile);
  let disposed = false;
  try {
    const initial = parseRawJson(viewer.diagnostics(), "initial point-footprint diagnostics");
    validateViewportFacts(initial.viewport, profile);
    validateFootprintFacts(initial.point_footprint, profile.expected_status);
    const firstCoverageMilliseconds = publishMaterializedSource(viewer, materialized, started);
    configureCamera(viewer, materialized.camera);
    parseRawJson(viewer.setDisplayMode(trial.display_mode), "point-footprint display mode");
    settleMaterializedSource(viewer, trial, materialized);
    const nominalPicks = verifyPicks
      ? await verifyTrialPicks(viewer, trial, materialized, profile)
      : [];
    if (applyHighlights) applyTrialHighlights(viewer, trial, materialized.source_identity);
    const settledViewMilliseconds = performance.now() - started;
    requireCondition(firstCoverageMilliseconds <= timingLimits.first_coverage_milliseconds, `trial ${trial.id} first Coverage exceeded its ceiling`);
    requireCondition(settledViewMilliseconds <= timingLimits.settled_view_milliseconds, `trial ${trial.id} settled View exceeded its ceiling`);
    const representativeTiming = await renderRepresentativeFrames(viewer, quietFrames, profile.expected_status);
    const timingEvaluation = predecessorTiming === null
      ? { passed: true, failures: [], predecessor_ratio: null }
      : evaluateRepresentativeTiming(representativeTiming, predecessorTiming, timingLimits);
    const capture = await captureCanonicalFrame(viewer, {
      width: profile.physical_width,
      height: profile.physical_height,
      pollFrameCeiling: visual.settling.capture_poll_frame_ceiling,
      capturePolicy: visual.capture,
    });
    validateFootprintFacts(capture.facts.point_footprint, profile.expected_status, {
      profile,
      residentPoints: materialized.source.expected_view.settled_resident_points,
    });
    validateExactCaptureFacts(capture.facts, { materialized, profile, trial });
    const diagnostics = parseRawJson(viewer.diagnostics(), "final point-footprint diagnostics");
    validateViewportFacts(diagnostics.viewport, profile);
    validateFootprintFacts(diagnostics.point_footprint, profile.expected_status, {
      profile,
      residentPoints: materialized.source.expected_view.settled_resident_points,
    });
    requireCondition(
      JSON.stringify(diagnostics.point_footprint) === JSON.stringify(capture.facts.point_footprint),
      "settled diagnostics and captured point-footprint facts differ",
    );
    requireCondition(diagnostics.frame !== null, "settled frame diagnostics are absent");
    const shutdown = parseRawJson(viewer.shutdown(), "point-footprint shutdown diagnostics");
    requireCondition(shutdown.capture_resources.pending_tickets === 0, "shutdown retained a capture ticket");
    viewer.free();
    viewer = undefined;
    disposed = true;
    return {
      image: capture.image,
      diagnostics,
      captureFacts: withoutImage(capture),
      nominalPicks,
      adapter: adapterFacts(diagnostics),
      lifecycle_timing: {
        first_coverage_milliseconds: firstCoverageMilliseconds,
        settled_view_milliseconds: settledViewMilliseconds,
      },
      representative_timing: representativeTiming,
      timing_evaluation: timingEvaluation,
      resources: {
        resident_points: materialized.source.expected_view.settled_resident_points,
        resident_bytes: diagnostics.frame.resident_bytes,
        point_vertex_bytes: TRANSFER_VERTEX_BYTES,
        transient_texture_bytes: diagnostics.frame.transient_texture_bytes,
        canvas_surface_bytes: diagnostics.viewport.surface_bytes,
        renderer_transient_byte_ceiling: 67_108_864,
      },
    };
  } finally {
    if (!disposed && viewer !== undefined) {
      try { viewer.shutdown(); } catch { /* preserve the primary failure */ }
      viewer.free();
    }
  }
}

async function runFocusedScaleTrials(options) {
  const {
    footprint,
    visual,
    visualUrl,
    mode,
    baseline,
    artifacts,
    baselineArtifacts,
    canonicalObservations,
  } = options;
  const results = [];
  for (const focused of footprint.focused_trials) {
    const trial = visual.trials.find(({ id }) => id === focused.id);
    requireCondition(trial !== undefined, `focused trial ${focused.id} is absent`);
    const materialized = await materializeVisualTrial(visual, trial.id, { corpusUrl: visualUrl });
    for (const profile of [footprint.canonical_profile, ...footprint.scale_profiles]) {
      let image;
      let diagnostics;
      let capture;
      let artifact;
      let run;
      if (profile.id === footprint.canonical_profile.id) {
        const observation = canonicalObservations.get(trial.id);
        requireCondition(observation !== undefined, `canonical observation for focused trial ${trial.id} is absent`);
        image = observation.image;
        run = observation.recreation;
        diagnostics = { point_footprint: run.point_footprint };
        capture = run.capture.facts;
        artifact = { metadata: run.capture.artifact };
      } else {
        run = await runViewerCapture({
          profile,
          visual,
          trial,
          materialized,
          quietFrames: 5,
          predecessorTiming: null,
          timingLimits: footprint.timing_limits,
          applyHighlights: false,
          verifyPicks: true,
        });
        image = run.image;
        diagnostics = run.diagnostics;
        capture = run.captureFacts;
        const artifactPath = `${ARTIFACT_ROOT}/focused/${trial.id}-${profile.id}.png`;
        artifact = await artifacts.addPng(image, {
          kind: "focused_candidate_png",
          path: artifactPath,
          trial_id: trial.id,
          recreation_index: null,
          frame_index: null,
        });
        if (mode === "record") {
          const baselinePath = `apps/browser-demo/web/fixtures/footprint-v1/baselines/${trial.id}-${profile.id}.png`;
          const baselineArtifact = await artifacts.addPng(image, {
            kind: "focused_baseline_png",
            path: baselinePath,
            trial_id: trial.id,
            recreation_index: null,
            frame_index: null,
          });
          baselineArtifacts.push({
            kind: "focused",
            trial_id: trial.id,
            profile_id: profile.id,
            web_path: `./fixtures/footprint-v1/baselines/${trial.id}-${profile.id}.png`,
            artifact: baselineArtifact.metadata,
          });
        }
      }
      const diameter = diagnostics.point_footprint.display_size_physical_pixels;
      const points = decodedPoints(materialized.batches);
      const measurements = focused.isolated_ordinals.map((ordinal) => {
        const point = points.find((candidate) => candidate.ordinal === ordinal);
        requireCondition(point !== undefined, `focused Point ${ordinal} is absent`);
        const projected = projectAuthoredPointAtViewport(point, materialized.world_origin, materialized.camera, profile);
        const report = measureIsolatedFootprint(image, {
          center: [projected.exact_x, projected.exact_y],
          diameterPhysicalPixels: diameter,
          backgroundRgba: BACKGROUND_RGBA,
        });
        return {
          ordinal,
          projected,
          center_foreground: normalizedCenterCoverage(image, report) > 0,
          ...report,
        };
      });
      const baselineComparison = mode === "verify" && profile.id !== footprint.canonical_profile.id
        ? compareCanonicalImages(
          (await loadFocusedBaseline(baseline, trial.id, profile.id)).image,
          image,
          {
            toleranceProfile: visual.tolerance_profiles[trial.tolerance_profile],
            features: scaledFeatures(trial.features, profile.requested_device_pixel_ratio / 2),
            backgroundRgba: visual.presentation_policy.canonical_clear_rgba8,
          },
        )
        : null;
      const failures = [];
      if (diameter !== expectedDisplayDiameter(
        profile,
        materialized.source.expected_view.settled_resident_points,
      )) failures.push("display_diameter_density");
      const nominalPicks = run.nominalPicks ?? run.nominal_picks;
      if (nominalPicks.some(({ matched }) => !matched)) failures.push("nominal_pick_identity");
      for (const measurement of measurements) {
        const metrics = measurement.metrics;
        if (metrics.coverage.root_mean_square_error > footprint.metric_limits.coverage_rmse) failures.push(`ordinal-${measurement.ordinal}:coverage_rmse`);
        if (metrics.corner_leakage.exact_distance_outer.pixel_count > footprint.metric_limits.maximum_outer_leakage_pixels) failures.push(`ordinal-${measurement.ordinal}:outer_leakage`);
        if (!metrics.corner_leakage.all_quad_corners_clear) failures.push(`ordinal-${measurement.ordinal}:quad_corner_leakage`);
        if (metrics.centroid.error_pixels === null || metrics.centroid.error_pixels > footprint.metric_limits.maximum_centroid_distance_pixels) failures.push(`ordinal-${measurement.ordinal}:centroid`);
      }
      if (baselineComparison !== null && !baselineComparison.passed) failures.push(...baselineComparison.failures.map((failure) => `baseline:${failure}`));
      results.push({
        trial_id: trial.id,
        profile_id: profile.id,
        requested_device_pixel_ratio: profile.requested_device_pixel_ratio,
        physical_width: profile.physical_width,
        physical_height: profile.physical_height,
        resident_points: materialized.source.expected_view.settled_resident_points,
        point_footprint: structuredClone(diagnostics.point_footprint),
        adapter: structuredClone(run.adapter),
        resources: structuredClone(run.resources),
        nominal_picks: structuredClone(nominalPicks),
        selected_status: diagnostics.point_footprint.selected,
        display_size_physical_pixels: diameter,
        nominal_pick_size_physical_pixels: diagnostics.point_footprint.nominal_pick_size_physical_pixels,
        measurements,
        capture: profile.id === footprint.canonical_profile.id
          ? { reused_canonical_capture: true, facts: capture, artifact: artifact.metadata }
          : { reused_canonical_capture: false, facts: capture, artifact: artifact.metadata },
        baseline_comparison: baselineComparison,
        passed: failures.length === 0,
        failures,
      });
    }
  }
  return results;
}

async function runResourceFallback({ footprint, visual, visualUrl, localTests }) {
  const profile = footprint.fallback_profile;
  const focused = footprint.focused_trials.find(({ nominal_pick_ordinals }) => nominal_pick_ordinals !== undefined);
  requireCondition(focused !== undefined, "resource fallback nominal-pick fixture is absent");
  const trial = visual.trials.find(({ id }) => id === focused.id);
  const materialized = await materializeVisualTrial(visual, trial.id, { corpusUrl: visualUrl });
  let viewer = await createViewerForProfile(profile);
  let disposed = false;
  try {
    const initial = parseRawJson(viewer.diagnostics(), "resource-fallback initial diagnostics");
    validateViewportFacts(initial.viewport, profile);
    validateFootprintFacts(initial.point_footprint, "resource_fallback");
    publishMaterializedSource(viewer, materialized, performance.now());
    configureCamera(viewer, materialized.camera);
    parseRawJson(viewer.setDisplayMode(trial.display_mode), "resource-fallback display mode");
    settleMaterializedSource(viewer, trial, materialized);
    parseRawJson(viewer.render(), "resource-fallback frame");
    const nominalPicks = await verifyTrialPicks(viewer, trial, materialized, profile);
    const diagnostics = parseRawJson(viewer.diagnostics(), "resource-fallback diagnostics");
    validateViewportFacts(diagnostics.viewport, profile);
    validateFootprintFacts(diagnostics.point_footprint, "resource_fallback", {
      profile,
      residentPoints: materialized.source.expected_view.settled_resident_points,
    });
    const failures = [];
    if (diagnostics.frame.transient_texture_bytes > footprint.policy.renderer_transient_byte_ceiling) failures.push("transient_texture_ceiling");
    const exactFallbackResources = createPointFootprintResourceEvidence({
      pointFootprint: diagnostics.point_footprint,
      profile,
      eyeDomeActive: false,
      pickTargetsRetained: true,
      rendererTransientTextureBytes: diagnostics.frame.transient_texture_bytes,
      ceilingBytes: footprint.policy.renderer_transient_byte_ceiling,
    });
    if (exactFallbackResources.multisample_color_bytes !== 0
      || exactFallbackResources.multisample_depth_bytes !== 0) {
      failures.push("multisample_target_allocated");
    }
    if (nominalPicks.some(({ matched }) => !matched)) failures.push("nominal_pick_identity");
    const browserFacts = {
      profile: structuredClone(profile),
      resident_points: materialized.source.expected_view.settled_resident_points,
      adapter: adapterFacts(diagnostics),
      point_footprint: structuredClone(diagnostics.point_footprint),
      frame: structuredClone(diagnostics.frame),
      nominal_picks: nominalPicks,
      capture_performed: false,
      multisample_target_allocated:
        exactFallbackResources.multisample_color_bytes > 0
        || exactFallbackResources.multisample_depth_bytes > 0,
    };
    const shutdown = parseRawJson(viewer.shutdown(), "resource-fallback shutdown diagnostics");
    requireCondition(shutdown.capture_resources.pending_tickets === 0, "resource fallback retained capture resources");
    viewer.free();
    viewer = undefined;
    disposed = true;
    return {
      browser_resource_fallback: browserFacts,
      single_sample_fixture: localTestCase(
        localTests,
        "single_sample_request_never_becomes_a_fallback",
      ),
      unsupported_fixture: localTestCase(
        localTests,
        "capability_fallback_precedes_the_viewport_resource_check",
      ),
      browser_observation_boundary: {
        single_sample: null,
        unsupported_fallback: null,
        reason: "the fixed browser host requests antialiased footprints on one supported adapter",
      },
      passed: failures.length === 0,
      failures,
    };
  } finally {
    if (!disposed && viewer !== undefined) {
      try { viewer.shutdown(); } catch { /* preserve the primary failure */ }
      viewer.free();
    }
    setCanvasProfile(footprint.canonical_profile);
  }
}

async function createViewerForProfile(profile) {
  setCanvasProfile(profile);
  return createRawViewer(
    canvas,
    profile.css_width,
    profile.css_height,
    profile.requested_device_pixel_ratio,
  );
}

function setCanvasProfile(profile) {
  canvas.style.width = `${profile.css_width}px`;
  canvas.style.height = `${profile.css_height}px`;
  canvas.width = profile.physical_width;
  canvas.height = profile.physical_height;
}

function publishMaterializedSource(viewer, materialized, started) {
  const [originX, originY, originZ] = materialized.world_origin;
  const [minimumZ, maximumZ] = materialized.source_z_range;
  parseRawJson(viewer.beginStreamBatch(
    materialized.source_identity,
    materialized.point_count,
    originX,
    originY,
    originZ,
    minimumZ,
    maximumZ,
    0,
    materialized.batches[0],
  ), "first point-footprint batch");
  parseRawJson(viewer.render(), "first point-footprint Coverage frame");
  const firstCoverageMilliseconds = performance.now() - started;
  for (let index = 1; index < materialized.batches.length; index += 1) {
    parseRawJson(viewer.publishStreamBatch(index, materialized.batches[index]), `point-footprint batch ${index}`);
  }
  const completed = parseRawJson(viewer.completeStream(), "point-footprint completed stream");
  requireCondition(completed.streaming.phase === "complete", "point-footprint stream did not complete");
  return firstCoverageMilliseconds;
}

function settleMaterializedSource(viewer, trial, materialized) {
  if (trial.temporal_trace.kind === "mixed_lod_parent_child") {
    parseRawJson(viewer.setVisualBatchPresentation(trial.temporal_trace.parent_batch_index, 0), "settled parent weight");
    parseRawJson(viewer.setVisualBatchPresentation(trial.temporal_trace.child_batch_index, 255), "settled child weight");
    if (trial.temporal_trace.remove_parent_after_transition) {
      parseRawJson(viewer.removeVisualBatch(trial.temporal_trace.parent_batch_index), "settled parent retirement");
    }
  } else {
    for (const batchIndex of materialized.source.expected_view.settled_removed_batch_indices) {
      parseRawJson(viewer.removeVisualBatch(batchIndex), "settled batch retirement");
    }
  }
  parseRawJson(viewer.render(), "settled point-footprint frame");
}

function configureCamera(viewer, camera) {
  const [eyeX, eyeY, eyeZ] = camera.eye;
  const [targetX, targetY, targetZ] = camera.target;
  const [upX, upY, upZ] = camera.up;
  if (camera.projection === "perspective") {
    parseRawJson(viewer.setPerspectiveCamera(
      eyeX, eyeY, eyeZ,
      targetX, targetY, targetZ,
      upX, upY, upZ,
      camera.vertical_field_of_view_radians,
      camera.near_distance,
      camera.far_distance,
    ), "point-footprint perspective camera");
  } else {
    parseRawJson(viewer.setOrthographicCamera(
      eyeX, eyeY, eyeZ,
      targetX, targetY, targetZ,
      upX, upY, upZ,
      camera.vertical_world_height,
      camera.near_distance,
      camera.far_distance,
    ), "point-footprint orthographic camera");
  }
}

async function renderRepresentativeFrames(viewer, frameCount, expectedStatus) {
  const intervals = [];
  const submissions = [];
  let previous = performance.now();
  for (let index = 0; index < frameCount; index += 1) {
    await nextAnimationFrame();
    const now = performance.now();
    intervals.push(now - previous);
    previous = now;
    const submissionStarted = performance.now();
    const diagnostics = parseRawJson(viewer.render(), `point-footprint quiet frame ${index + 1}`);
    submissions.push(performance.now() - submissionStarted);
    validateFootprintFacts(diagnostics.point_footprint, expectedStatus);
  }
  return {
    frame_count: frameCount,
    capture_free: true,
    frame_interval_samples_milliseconds: intervals,
    frame_submission_samples_milliseconds: submissions,
    frame_interval_milliseconds: summarizeSamples(intervals),
    frame_submission_milliseconds: summarizeSamples(submissions),
  };
}

async function verifyTrialPicks(viewer, trial, materialized, profile) {
  if (trial.selection.ordinals.length === 0) return [];
  const points = decodedPoints(materialized.batches);
  const results = [];
  for (const ordinal of trial.selection.ordinals) {
    const point = points.find((candidate) => candidate.ordinal === ordinal);
    requireCondition(point !== undefined, `nominal-pick Point ${ordinal} is absent`);
    const projected = projectAuthoredPointAtViewport(point, materialized.world_origin, materialized.camera, profile);
    const batchIndex = batchIndexForOrdinal(materialized.batches, ordinal);
    const expected = {
      source_identity: materialized.source_identity,
      generation: materialized.source.expected_view.generation,
      batch_key: materialized.source.expected_view.batch_keys[batchIndex],
      batch_version: trial.expected_settled_batch_versions[batchIndex],
      point_ordinal: String(ordinal),
    };
    results.push(await pickExpectedPoint(viewer, [projected.x, projected.y], expected));
  }
  return results;
}

async function pickExpectedPoint(viewer, center, expected) {
  const candidates = [];
  for (let y = center[1] - 1; y <= center[1] + 1; y += 1) {
    for (let x = center[0] - 1; x <= center[0] + 1; x += 1) candidates.push([x, y]);
  }
  candidates.sort((left, right) => (
    squaredDistance(left, center) - squaredDistance(right, center)
      || left[1] - right[1]
      || left[0] - right[0]
  ));
  const attempts = [];
  for (const pixel of candidates) {
    parseRawJson(viewer.beginPick(pixel[0], pixel[1]), "point-footprint nominal pick request");
    let observed;
    let polls = 0;
    while (polls < 180) {
      await nextAnimationFrame();
      const diagnostics = parseRawJson(viewer.pollPick(), "point-footprint nominal pick poll");
      polls += 1;
      if (diagnostics.pick.status !== "pending") {
        observed = diagnostics.pick;
        break;
      }
    }
    parseRawJson(viewer.cancelPick(), "point-footprint nominal pick cleanup");
    requireCondition(observed !== undefined, "point-footprint nominal pick exceeded its poll ceiling");
    const matched = observed.status === "hit"
      && observed.authority === "provisional_gpu_hint"
      && observed.source_identity === expected.source_identity
      && observed.generation === expected.generation
      && observed.batch_key === expected.batch_key
      && observed.batch_version === expected.batch_version
      && observed.point_ordinal === expected.point_ordinal;
    attempts.push({ pixel, polls, observed: structuredClone(observed), matched });
    if (matched) return { expected, center, attempts, matched: true };
  }
  return { expected, center, attempts, matched: false };
}

function applyTrialHighlights(viewer, trial, sourceIdentity) {
  if (trial.selection.ordinals.length === 0) return;
  const diagnostics = parseRawJson(viewer.diagnostics(), "pre-highlight diagnostics");
  const ordinals = new BigUint64Array(trial.selection.ordinals.map(BigInt));
  const selected = parseRawJson(viewer.setHighlights(
    sourceIdentity,
    BigInt(diagnostics.streaming.generation),
    ordinals,
  ), "point-footprint highlights");
  requireCondition(selected.highlights.point_count === ordinals.length, "highlight count differs");
}

function compareFeatureFacts(predecessor, candidate, features) {
  return features.map((feature) => {
    const before = measureCoverage(predecessor, BACKGROUND_RGBA, 2, feature.rectangle);
    const after = measureCoverage(candidate, BACKGROUND_RGBA, 2, feature.rectangle);
    const centroidDistance = before.centroid === null || after.centroid === null
      ? null
      : Math.hypot(before.centroid.x - after.centroid.x, before.centroid.y - after.centroid.y);
    return {
      feature_id: feature.id,
      minimum_foreground_pixels: feature.minimum_foreground_pixels,
      predecessor: before,
      candidate: after,
      centroid_distance_pixels: centroidDistance,
    };
  });
}

function compareDenseRegions(predecessor, candidate, regions, limits) {
  return regions.map((rectangle) => {
    const before = measureOccupancyTopology(predecessor, { rectangle, backgroundRgba: BACKGROUND_RGBA });
    const after = measureOccupancyTopology(candidate, { rectangle, backgroundRgba: BACKGROUND_RGBA });
    return {
      rectangle: structuredClone(rectangle),
      predecessor: before,
      candidate: after,
      solid_2x2_budget: evaluateDenseSolidBlockBudget(before.metrics, after.metrics, limits),
    };
  });
}

function evaluateCanonicalQuality(options) {
  const {
    topology,
    predecessorTopology,
    featureComparisons,
    densityComparisons,
    componentBridges,
    limits,
  } = options;
  const failures = [];
  const foregroundRatio = predecessorTopology.foreground_fraction === 0
    ? null
    : topology.foreground_fraction / predecessorTopology.foreground_fraction;
  if (foregroundRatio === null
    || foregroundRatio < limits.foreground_fraction_predecessor_ratio.minimum
    || foregroundRatio > limits.foreground_fraction_predecessor_ratio.maximum) {
    failures.push("foreground_fraction_ratio");
  }
  for (const feature of featureComparisons) {
    if (feature.predecessor.foreground_pixels < feature.minimum_foreground_pixels
      || feature.candidate.foreground_pixels < feature.minimum_foreground_pixels) {
      failures.push(`feature-${feature.feature_id}:missing`);
    }
    if (feature.centroid_distance_pixels === null
      || feature.centroid_distance_pixels > limits.maximum_feature_centroid_distance_pixels) {
      failures.push(`feature-${feature.feature_id}:centroid`);
    }
  }
  for (const region of densityComparisons) {
    if (!region.solid_2x2_budget.passed) failures.push("dense_solid_2x2_budget");
  }
  if (topology.foreground.left_right_bridge_components > predecessorTopology.foreground.left_right_bridge_components
    || topology.foreground.top_bottom_bridge_components > predecessorTopology.foreground.top_bottom_bridge_components) {
    failures.push("new_foreground_bridge");
  }
  if (componentBridges.bridging_candidate_component_count > 0) {
    failures.push("separated_predecessor_component_bridge");
  }
  return { passed: failures.length === 0, failures, foreground_fraction_predecessor_ratio: foregroundRatio };
}

function createBaselineRecord(options) {
  const {
    footprint,
    pins,
    canonicalTrials,
    focusedTrials,
    fallback,
    baselineArtifacts,
  } = options;
  requireCondition(canonicalTrials.every(({ passed }) => passed), "record baseline requires every canonical trial to pass");
  requireCondition(focusedTrials.every(({ passed }) => passed), "record baseline requires every focused trial to pass");
  requireCondition(fallback.passed, "record baseline requires the attended resource fallback to pass");
  const candidateImages = footprint.canonical_trials.map((trial) => {
    const record = baselineArtifacts.find(({ kind, trial_id: id }) => (
      kind === "canonical" && id === trial.id
    ));
    requireCondition(record !== undefined, `record baseline image ${trial.id} is absent`);
    return createPointFootprintImageArtifact(record.artifact, footprint.canonical_profile.id);
  });
  const focusedImages = footprint.focused_trials.flatMap((trial) => (
    [footprint.canonical_profile, ...footprint.scale_profiles].map((profile) => {
      if (profile.id === footprint.canonical_profile.id) {
        return structuredClone(candidateImages.find(({ trial_id: id }) => id === trial.id));
      }
      const record = baselineArtifacts.find(({ kind, trial_id: id, profile_id: profileId }) => (
        kind === "focused" && id === trial.id && profileId === profile.id
      ));
      requireCondition(record !== undefined,
        `record focused baseline image ${trial.id}/${profile.id} is absent`);
      return createPointFootprintImageArtifact(record.artifact, profile.id);
    })
  ));
  const baseline = {
    schema: BASELINE_SCHEMA,
    release: footprint.release,
    pins: structuredClone(pins),
    candidate_images: candidateImages,
    focused_images: focusedImages,
    external_evidence: structuredClone(FOOTPRINT_EXTERNAL_NONCLAIMS),
  };
  return validatePointFootprintBaseline(baseline, footprint);
}

function createEvidenceRecord(options) {
  const {
    startedAt,
    footprint,
    pins,
    host,
    baseline,
    baselineIdentity,
    localTests,
    localTestMetadata,
    canonicalTrials,
    focusedTrials,
    fallback,
  } = options;
  validatePointFootprintBaseline(baseline, footprint);
  requireCondition(localTests.implementation_commit === pins.implementation.commit,
    "local test evidence implementation commit differs from the browser implementation pin");
  requireCondition(localTests.producer_command === FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
    "local test evidence producer command differs from the closed invocation");
  const canonicalEvidence = canonicalTrials.map((trial) => canonicalTrialEvidence(trial, footprint));
  const focusedEvidence = focusedTrials.map((trial) => focusedTrialEvidence(trial, baseline, footprint));
  const pickIdentityReference = preferredPickReference(focusedTrials, footprint);
  const fallbackTrials = fallbackTrialEvidence(
    fallback,
    localTests,
    localTestMetadata.path,
    pickIdentityReference,
    footprint,
  );
  const png = uniqueImageArtifacts([
    ...canonicalTrials.flatMap(({ recreations }) => recreations.map(({ capture }) => (
      createPointFootprintImageArtifact(capture.artifact, footprint.canonical_profile.id)
    ))),
    ...focusedTrials
      .filter(({ profile_id: profileId }) => profileId !== footprint.canonical_profile.id)
      .map((trial) => createPointFootprintImageArtifact(trial.capture.artifact, trial.profile_id)),
  ]);
  const adapter = oneObservedAdapter(canonicalTrials, focusedTrials, fallback);
  const evidence = {
    schema: EVIDENCE_SCHEMA,
    release: footprint.release,
    mode: "verify",
    started_at: startedAt,
    completed_at: new Date().toISOString(),
    baseline: structuredClone(baselineIdentity),
    pins: structuredClone(pins),
    environment: {
      browser_user_agent: navigator.userAgent,
      browser_platform: navigator.platform || "unreported browser platform",
      operating_system: operatingSystemName(host),
      adapter_name: adapter.name,
      backend: adapter.backend,
      same_adapter_for_scale_trials: true,
      physical_display_observed: false,
    },
    artifacts: {
      png,
      local_test_results: [{
        path: localTestMetadata.path,
        byte_length: localTestMetadata.encoded_byte_length,
        sha256: localTestMetadata.encoded_sha256,
        media_type: "application/json",
        producer_command: localTests.producer_command,
      }],
    },
    canonical_trials: canonicalEvidence,
    focused_trials: focusedEvidence,
    local_gpu_fixture: localGpuFixtureEvidence(localTests, localTestMetadata.path),
    pick_identity_reference: pickIdentityReference,
    fallback_trials: fallbackTrials,
    summary: {},
    external_evidence: structuredClone(FOOTPRINT_EXTERNAL_NONCLAIMS),
    unavailable_measurements: [...FOOTPRINT_UNAVAILABLE_MEASUREMENTS],
    fatal_error: null,
  };
  evidence.summary = derivePointFootprintEvidenceSummary(evidence, { baseline, corpus: footprint });
  requireCondition(evidence.summary.passed,
    `point-footprint evidence gates failed: ${evidence.summary.failures.join("; ")}`);
  return evidence;
}

function canonicalTrialEvidence(trial, footprint) {
  requireCondition(trial.passed, `canonical trial ${trial.trial_id} did not pass`);
  const contract = footprint.canonical_trials.find(({ id }) => id === trial.trial_id);
  const predecessorMeasurement = trial.recreations[0].predecessor_topology;
  return {
    trial_id: trial.trial_id,
    predecessor_topology: createTopologyMetricBinding({
      metricId: `canonical/${trial.trial_id}/predecessor`,
      artifactPath: contract.predecessor_baseline.path,
      backgroundRgba: BACKGROUND_RGBA,
      measurement: predecessorMeasurement,
    }),
    recreations: trial.recreations.map((recreation) => ({
      index: recreation.index,
      adapter: structuredClone(recreation.adapter),
      resident_points: recreation.resources.resident_points,
      point_footprint: structuredClone(recreation.point_footprint),
      timing: timingEvidence(recreation, contract),
      resources: resourceEvidence(
        recreation,
        footprint.canonical_profile,
        recreation.nominal_picks.length > 0,
        footprint,
      ),
      capture_artifact_path: recreation.capture.artifact.path,
      candidate_topology: createTopologyMetricBinding({
        metricId: `canonical/${trial.trial_id}/r${recreation.index}`,
        artifactPath: recreation.capture.artifact.path,
        backgroundRgba: BACKGROUND_RGBA,
        measurement: recreation.candidate_topology,
      }),
      component_bridge_check: createComponentBridgeMetricBinding({
        metricId: `canonical/${trial.trial_id}/r${recreation.index}/component-bridges`,
        predecessorArtifactPath: contract.predecessor_baseline.path,
        candidateArtifactPath: recreation.capture.artifact.path,
        backgroundRgba: BACKGROUND_RGBA,
        measurement: recreation.component_bridges,
      }),
      feature_checks: recreation.feature_comparisons.map((feature) => ({
        id: feature.feature_id,
        predecessor_foreground_pixels: feature.predecessor.foreground_pixels,
        candidate_foreground_pixels: feature.candidate.foreground_pixels,
        centroid_distance_pixels: feature.centroid_distance_pixels,
      })),
      dense_region_checks: recreation.dense_region_comparisons.map((region, regionIndex) => ({
        rectangle: structuredClone(region.rectangle),
        predecessor: createTopologyMetricBinding({
          metricId: `canonical/${trial.trial_id}/r${recreation.index}/dense/${regionIndex}/predecessor`,
          artifactPath: contract.predecessor_baseline.path,
          backgroundRgba: BACKGROUND_RGBA,
          measurement: region.predecessor,
        }),
        candidate: createTopologyMetricBinding({
          metricId: `canonical/${trial.trial_id}/r${recreation.index}/dense/${regionIndex}/candidate`,
          artifactPath: recreation.capture.artifact.path,
          backgroundRgba: BACKGROUND_RGBA,
          measurement: region.candidate,
        }),
      })),
    })),
  };
}

function timingEvidence(recreation, contract) {
  const intervals = recreation.representative_timing.frame_interval_samples_milliseconds;
  const submissions = recreation.representative_timing.frame_submission_samples_milliseconds;
  return {
    frame_interval_samples_milliseconds: [...intervals],
    frame_submission_samples_milliseconds: [...submissions],
    frame_interval: summarizeFootprintTiming(intervals),
    frame_submission: summarizeFootprintTiming(submissions),
    predecessor_frame_interval_p95_milliseconds:
      contract.predecessor_timing.maximum_recreation_frame_interval_p95_milliseconds,
    predecessor_frame_submission_p95_milliseconds:
      contract.predecessor_timing.maximum_recreation_frame_submission_p95_milliseconds,
    first_coverage_milliseconds: recreation.lifecycle_timing.first_coverage_milliseconds,
    settled_view_milliseconds: recreation.lifecycle_timing.settled_view_milliseconds,
  };
}

function focusedTrialEvidence(trial, baseline, footprint) {
  requireCondition(trial.passed, `focused trial ${trial.trial_id}/${trial.profile_id} did not pass`);
  const profile = profileById(footprint, trial.profile_id);
  const pinned = baseline.focused_images.find(({ trial_id: trialId, profile_id: profileId }) => (
    trialId === trial.trial_id && profileId === trial.profile_id
  ));
  requireCondition(pinned !== undefined,
    `focused baseline ${trial.trial_id}/${trial.profile_id} is absent`);
  return {
    trial_id: trial.trial_id,
    profile_id: trial.profile_id,
    adapter: structuredClone(trial.adapter),
    resident_points: trial.resident_points,
    point_footprint: structuredClone(trial.point_footprint),
    resources: resourceEvidence(trial, profile, trial.nominal_picks.length > 0, footprint),
    candidate_artifact_path: trial.capture.artifact.path,
    baseline_artifact_path: pinned.path,
    isolated_footprints: trial.measurements.map((measurement) => ({
      ordinal: measurement.ordinal,
      center_foreground: measurement.center_foreground,
      candidate: createFootprintMetricBinding({
        metricId: `focused/${trial.trial_id}/${trial.profile_id}/point/${measurement.ordinal}`,
        artifactPath: trial.capture.artifact.path,
        measurement,
      }),
    })),
  };
}

function resourceEvidence(observation, profile, pickTargetsRetained, footprint) {
  return createPointFootprintResourceEvidence({
    pointFootprint: observation.point_footprint,
    profile,
    eyeDomeActive: false,
    pickTargetsRetained,
    rendererTransientTextureBytes: observation.resources.transient_texture_bytes,
    ceilingBytes: footprint.policy.renderer_transient_byte_ceiling,
  });
}

function preferredPickReference(focusedTrials, footprint) {
  const contract = footprint.focused_trials.find(({ nominal_pick_ordinals: ordinals }) => (
    Array.isArray(ordinals)
  ));
  const observation = focusedTrials.find(({ trial_id: trialId, profile_id: profileId }) => (
    trialId === contract.id && profileId === footprint.canonical_profile.id
  ));
  requireCondition(observation !== undefined, "preferred pick observation is absent");
  return {
    profile_id: footprint.canonical_profile.id,
    resident_points: observation.resident_points,
    point_footprint: structuredClone(observation.point_footprint),
    pick_probes: pickProbeEvidence(observation.nominal_picks),
    pick_mask_artifact_path: observation.capture.artifact.path,
  };
}

function fallbackTrialEvidence(fallback, localTests, artifactPath, reference, footprint) {
  requireCondition(fallback.passed, "attended resource fallback did not pass");
  const local = [fallback.single_sample_fixture, fallback.unsupported_fixture].map((testCase) => ({
    id: testCase.facts.selection.selected,
    evidence_source: "local_renderer_test",
    ...structuredClone(testCase.facts),
    browser_observation: null,
    local_test_evidence: localTestProvenance(testCase, artifactPath),
  }));
  const browser = fallback.browser_resource_fallback;
  const resources = createPointFootprintResourceEvidence({
    pointFootprint: browser.point_footprint,
    profile: footprint.fallback_profile,
    eyeDomeActive: false,
    pickTargetsRetained: true,
    rendererTransientTextureBytes: browser.frame.transient_texture_bytes,
    ceilingBytes: footprint.policy.renderer_transient_byte_ceiling,
  });
  const picks = pickProbeEvidence(browser.nominal_picks);
  requireCondition(JSON.stringify(picks) === JSON.stringify(reference.pick_probes),
    "resource-fallback picks differ from preferred picks");
  local.push({
    id: "resource_fallback",
    evidence_source: "attended_browser",
    physical_width: footprint.fallback_profile.physical_width,
    physical_height: footprint.fallback_profile.physical_height,
    selection: {
      requested: browser.point_footprint.requested,
      selected: browser.point_footprint.selected,
      sample_count: 1,
      multisample_target_allocated: browser.multisample_target_allocated,
    },
    resources,
    pick_probes: picks,
    hard_circle_mask: null,
    nominal_pick_identity: null,
    browser_observation: {
      profile_id: footprint.fallback_profile.id,
      capture_performed: browser.capture_performed,
      adapter: structuredClone(browser.adapter),
      resident_points: browser.resident_points,
      point_footprint: structuredClone(browser.point_footprint),
      resources: structuredClone(resources),
    },
    local_test_evidence: null,
  });
  return local;
}

function localGpuFixtureEvidence(localTests, artifactPath) {
  const quality = localTestCase(localTests, "antialiased_footprint_quality_matrix");
  const pick = localTestCase(
    localTests,
    "four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking",
  );
  const resources = localTestCase(
    localTests,
    "exact_high_water_accounts_for_pick_and_eye_dome_targets",
  );
  return {
    evidence_source: "local_renderer_gpu_test",
    browser_observation: null,
    environment: structuredClone(localTests.environment),
    local_test_evidence: {
      quality: localTestProvenance(quality, artifactPath),
      pick_independence: localTestProvenance(pick, artifactPath),
      resource_accounting: localTestProvenance(resources, artifactPath),
    },
    ...structuredClone(quality.facts),
    ...structuredClone(pick.facts),
    ...structuredClone(resources.facts),
  };
}

function localTestProvenance(testCase, artifactPath) {
  return {
    artifact_path: artifactPath,
    case: testCase.id,
    source_test: testCase.source_test,
    result: "passed",
  };
}

function pickProbeEvidence(picks) {
  requireCondition(picks.every(({ matched }) => matched), "preferred pick probes did not all match");
  return picks.map(({ expected }) => ({
    ordinal: Number(expected.point_ordinal),
    generation: expected.generation,
    source_identity: expected.source_identity,
    batch_key: expected.batch_key,
    batch_version: expected.batch_version,
    point_ordinal: expected.point_ordinal,
  }));
}

function uniqueImageArtifacts(artifacts) {
  const unique = new Map();
  for (const artifact of artifacts) {
    const previous = unique.get(artifact.path);
    requireCondition(previous === undefined
      || JSON.stringify(previous) === JSON.stringify(artifact),
    `PNG artifact ${artifact.path} has inconsistent metadata`);
    unique.set(artifact.path, artifact);
  }
  return [...unique.values()];
}

function oneObservedAdapter(canonicalTrials, focusedTrials, fallback) {
  const observations = [
    ...canonicalTrials.flatMap(({ recreations }) => recreations.map(({ adapter }) => adapter)),
    ...focusedTrials.map(({ adapter }) => adapter),
    fallback.browser_resource_fallback.adapter,
  ];
  requireCondition(observations.length > 0, "no browser adapter observation was recorded");
  const expected = JSON.stringify(observations[0]);
  requireCondition(observations.every((adapter) => JSON.stringify(adapter) === expected),
    "fresh viewers did not use one consistent adapter and backend");
  return structuredClone(observations[0]);
}

function operatingSystemName(host) {
  const system = host?.operating_system;
  requireCondition(system !== null && typeof system === "object",
    "qualification host operating-system facts are absent");
  return [system.name, system.version, system.build, system.architecture]
    .filter((value) => typeof value === "string" && value.length > 0)
    .join(" ");
}

function profileById(footprint, profileId) {
  const profile = [
    footprint.canonical_profile,
    ...footprint.scale_profiles,
    footprint.fallback_profile,
  ].find(({ id }) => id === profileId);
  requireCondition(profile !== undefined, `point-footprint profile ${profileId} is absent`);
  return profile;
}

async function loadBaseline(pins, corpus) {
  const response = await fetch(BASELINE_URL, { cache: "no-store", credentials: "same-origin" });
  requireCondition(response.ok, `v0.22 baseline returned HTTP ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  const baseline = JSON.parse(new TextDecoder().decode(bytes));
  requireCondition(baseline.schema === BASELINE_SCHEMA, "v0.22 baseline schema differs");
  requireCondition(baseline.release === "0.22.0-alpha.1", "v0.22 baseline release differs");
  requireCondition(JSON.stringify(baseline.pins) === JSON.stringify(pins.accepted), "baseline and server accepted pins differ");
  validatePointFootprintBaseline(baseline, corpus);
  return {
    manifest: baseline,
    identity: {
      path: BASELINE_REPOSITORY_PATH,
      byte_length: bytes.byteLength,
      sha256: await sha256Hex(bytes),
    },
  };
}

async function loadCanonicalBaseline(baseline, trialId) {
  const record = baseline.candidate_images.find(({ trial_id: candidate }) => candidate === trialId);
  requireCondition(record !== undefined, `canonical baseline ${trialId} is absent`);
  return loadBoundBaseline(record);
}

async function loadFocusedBaseline(baseline, trialId, profileId) {
  const record = baseline.focused_images.find(({ trial_id: candidate, profile_id: profile }) => (
    candidate === trialId && profile === profileId
  ));
  requireCondition(record !== undefined, `focused baseline ${trialId}/${profileId} is absent`);
  return loadBoundBaseline(record);
}

async function loadBoundBaseline(record) {
  const webPath = `./${record.path.replace("apps/browser-demo/web/", "")}`;
  const loaded = await loadPng(new URL(webPath, import.meta.url));
  requireCondition(loaded.bytes.byteLength === record.encoded_byte_length, `baseline ${record.path} byte length differs`);
  requireCondition(await sha256Hex(loaded.bytes) === record.encoded_sha256, `baseline ${record.path} SHA-256 differs`);
  return loaded;
}

async function loadPng(url) {
  const response = await fetch(url, { cache: "no-store", credentials: "same-origin" });
  requireCondition(response.ok, `PNG ${url.pathname} returned HTTP ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  return { bytes, image: await decodeRgba8Png(bytes) };
}

async function loadRuntimeArtifacts() {
  const records = [];
  let wasmBytes;
  for (const relativePath of RUNTIME_PATHS) {
    const response = await fetch(new URL(relativePath, import.meta.url), {
      cache: "no-store",
      credentials: "same-origin",
    });
    requireCondition(response.ok, `runtime ${relativePath} returned HTTP ${response.status}`);
    const bytes = new Uint8Array(await response.arrayBuffer());
    records.push({
      path: `apps/browser-demo/web/${relativePath.replace(/^\.\//, "")}`,
      byte_length: bytes.byteLength,
      sha256: await sha256Hex(bytes),
    });
    if (relativePath.endsWith(".wasm")) wasmBytes = bytes;
  }
  requireCondition(wasmBytes instanceof Uint8Array, "Wasm runtime bytes are absent");
  return { records, wasmBytes };
}

function validateRunPins(mode, pins) {
  requireCondition(pins?.schema === "punctra-browser-point-footprint-verify-pins-v1", "point-footprint pin schema differs");
  validatePins(pins.running, "running pins");
  if (mode === "record") return;
  validatePins(pins.accepted, "accepted pins");
  requireCondition(JSON.stringify(pins.running) === JSON.stringify(pins.accepted), "running implementation or verifier differs from the accepted baseline pin");
}

function validatePins(pins, label) {
  requireCondition(pins !== null && typeof pins === "object", `${label} are absent`);
  requireCondition(/^[0-9a-f]{40}$/.test(pins.implementation?.commit),
    `${label} implementation commit is invalid`);
  requireCondition(Array.isArray(pins.implementation.files)
    && pins.implementation.files.length > 0,
  `${label} implementation file pins are absent`);
  for (const record of pins.implementation.files) validateDigest(record, `${label} implementation file`);
  requireCondition(pins.verifier?.path === "scripts/verify-browser-point-footprint.mjs", `${label} verifier path differs`);
  validateDigest(pins.verifier, `${label} verifier`);
  requireCondition(pins.runtime?.package_name === "@punctra/viewer",
    `${label} runtime package name differs`);
  requireCondition(Array.isArray(pins.runtime?.artifacts), `${label} runtime artifacts are absent`);
  for (const record of pins.runtime.artifacts) validateDigest(record, `${label} runtime artifact`);
  validateDigest(pins.corpus, `${label} corpus`);
  requireCondition(pins.predecessor !== null && typeof pins.predecessor === "object",
    `${label} predecessor pins are absent`);
}

function validateDigest(record, label) {
  requireCondition(typeof record?.path === "string" && record.path.length > 0,
    `${label} path is invalid`);
  requireCondition(Number.isSafeInteger(record.byte_length) && record.byte_length > 0,
    `${label} byte length is invalid`);
  requireCondition(/^[0-9a-f]{64}$/.test(record.sha256), `${label} SHA-256 is invalid`);
}

function validateFootprintFacts(facts, expectedStatus, density = null) {
  requireCondition(facts?.requested === "antialiased", "renderer did not retain the antialiased request");
  requireCondition(facts.selected === expectedStatus, `renderer selected ${facts.selected} instead of ${expectedStatus}`);
  requireCondition(facts.nominal_pick_size_physical_pixels === 7, "nominal pick diameter differs");
  requireCondition(
    Number.isFinite(facts.display_size_physical_pixels)
      && facts.display_size_physical_pixels >= 2
      && facts.display_size_physical_pixels <= 6,
    "display diameter is outside 2 through 6 physical pixels",
  );
  if (density !== null) {
    requireCondition(
      Object.is(
        Math.fround(facts.display_size_physical_pixels),
        expectedDisplayDiameter(density.profile, density.residentPoints),
      ),
      "display diameter differs from the settled resident-point density",
    );
  }
}

function expectedDisplayDiameter(profile, residentPoints) {
  return projectedDensityDisplayDiameter(
    profile,
    residentPoints,
    loadedContext.footprint.corpus.policy,
  );
}

function adapterFacts(diagnostics) {
  const name = diagnostics.capabilities?.adapter_name;
  const backend = diagnostics.capabilities?.backend;
  requireCondition(typeof name === "string" && name.length > 0,
    "browser adapter name is absent");
  requireCondition(typeof backend === "string" && backend.length > 0,
    "browser adapter backend is absent");
  return { name, backend };
}

function validateViewportFacts(viewport, profile) {
  requireCondition(viewport?.css_width === profile.css_width
    && viewport.css_height === profile.css_height,
  "browser CSS viewport differs from the trial profile");
  requireCondition(viewport.device_pixel_ratio === profile.requested_device_pixel_ratio,
    "browser device-pixel ratio differs from the trial profile");
  requireCondition(viewport.physical_width === profile.physical_width
    && viewport.physical_height === profile.physical_height,
  "browser physical viewport differs from the trial profile");
  requireCondition(canvas.width === profile.physical_width
    && canvas.height === profile.physical_height,
  "canvas bitmap dimensions differ from the trial profile");
}

function validateExactCaptureFacts(facts, { materialized, profile, trial }) {
  const view = materialized.source.expected_view;
  requireCondition(facts.width === profile.physical_width
    && facts.height === profile.physical_height,
  "captured physical dimensions differ from the requested profile");
  requireCondition(facts.view_generation === view.generation,
    "captured view generation differs from the materialized source");
  requireCondition(facts.drawn_points === view.settled_drawn_points,
    "captured drawn-point count differs from the settled source");
  requireCondition(facts.draw_calls === view.settled_draw_calls,
    "captured draw-call count differs from the settled source");
  requireCondition(
    facts.resident_bytes === view.settled_resident_points * TRANSFER_VERTEX_BYTES,
    "captured resident bytes differ from the settled source",
  );
  const removed = new Set(view.settled_removed_batch_indices);
  const expectedBatches = materialized.batches.flatMap((batch, batchIndex) => (
    removed.has(batchIndex) ? [] : [{
      batch_index: batchIndex,
      key: view.batch_keys[batchIndex],
      version: trial.expected_settled_batch_versions[batchIndex],
      point_count: batch.byteLength / 32,
      state: "resident",
      presentation_weight_u8: view.settled_presentation_weights_u8[batchIndex],
    }]
  ));
  requireCondition(JSON.stringify(facts.batches) === JSON.stringify(expectedBatches),
    "captured batch identities differ from the exact settled source");
}

function normalizedCenterCoverage(image, measurement) {
  const binding = measurement.metrics;
  const x = Math.min(image.width - 1, Math.max(0, Math.floor(binding.center[0])));
  const y = Math.min(image.height - 1, Math.max(0, Math.floor(binding.center[1])));
  const offset = (y * image.width + x) * 4;
  const background = binding.normalization.background_rgba;
  const direction = measurement.foreground_rgba.map((value, channel) => (
    value - background[channel]
  ));
  const denominator = direction.reduce((sum, value) => sum + value * value, 0);
  requireCondition(denominator > 0, "isolated footprint foreground endpoint is degenerate");
  let numerator = 0;
  for (let channel = 0; channel < 4; channel += 1) {
    numerator += (image.data[offset + channel] - background[channel]) * direction[channel];
  }
  return Math.min(1, Math.max(0, numerator / denominator));
}

function decodedPoints(batches) {
  const points = [];
  let previous = -1;
  for (const batch of batches) {
    const decoded = decodeTransferV2(batch, previous);
    points.push(...decoded);
    previous = decoded.at(-1).ordinal;
  }
  return points;
}

function batchIndexForOrdinal(batches, ordinal) {
  let first = 0;
  for (let index = 0; index < batches.length; index += 1) {
    const count = batches[index].byteLength / 32;
    if (ordinal >= first && ordinal < first + count) return index;
    first += count;
  }
  throw new Error(`Point-footprint runner failed: Point ${ordinal} has no batch`);
}

function scaledFeatures(features, scale) {
  return features.map((feature) => ({
    ...structuredClone(feature),
    rectangle: scaleRectangle(feature.rectangle, scale),
    binding: feature.binding === undefined ? undefined : {
      ...structuredClone(feature.binding),
      expected_pixels: feature.binding.expected_pixels.map(([x, y]) => [Math.round(x * scale), Math.round(y * scale)]),
      tolerance_pixels: Math.max(1, Math.round(feature.binding.tolerance_pixels * scale)),
    },
  }));
}

function scaleRectangle(rectangle, scale) {
  return {
    x: Math.round(rectangle.x * scale),
    y: Math.round(rectangle.y * scale),
    width: Math.max(1, Math.round(rectangle.width * scale)),
    height: Math.max(1, Math.round(rectangle.height * scale)),
  };
}

function localTestCase(localTests, id) {
  requireCondition(FOOTPRINT_LOCAL_TEST_CASE_IDS.includes(id),
    `local test case ${id} is outside the closed contract`);
  const testCase = localTests.cases?.find((candidate) => candidate.id === id);
  requireCondition(testCase !== undefined && testCase.passed === true, `local test case ${id} is absent or failed`);
  return structuredClone(testCase);
}

async function loadJson(url, label) {
  const response = await fetch(url, { cache: "no-store", credentials: "same-origin" });
  requireCondition(response.ok, `${label} returned HTTP ${response.status}`);
  return response.json();
}

async function loadJsonWithBytes(url, label) {
  const response = await fetch(url, { cache: "no-store", credentials: "same-origin" });
  requireCondition(response.ok, `${label} returned HTTP ${response.status}`);
  const bytes = new Uint8Array(await response.arrayBuffer());
  return {
    bytes,
    json: JSON.parse(new TextDecoder().decode(bytes)),
  };
}

async function publishArchive(bytes, sha256) {
  const transport = footprintArchiveTransportFromUrl(window.location.href);
  if (transport === "same-origin-local-server") {
    return exportFootprintArchiveToLocalServer({
      archiveBytes: bytes,
      filename: FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
      sha256,
      pageUrl: window.location.href,
    });
  }
  downloadBytes(bytes, FOOTPRINT_EXPORT_ARCHIVE_FILENAME, "application/x-tar");
  return null;
}

function downloadLatestArchive() {
  requireCondition(latestArchive !== undefined, "no point-footprint archive is staged");
  downloadBytes(latestArchive.bytes, FOOTPRINT_EXPORT_ARCHIVE_FILENAME, "application/x-tar");
}

function downloadBytes(bytes, filename, type) {
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 300_000);
}

function updateReadouts(diagnostics) {
  const facts = diagnostics.point_footprint;
  requestedOutput.textContent = facts.requested;
  selectedOutput.textContent = facts.selected;
  displaySizeOutput.textContent = `${facts.display_size_physical_pixels.toFixed(3)} px`;
  pickSizeOutput.textContent = `${facts.nominal_pick_size_physical_pixels.toFixed(0)} px`;
  transientOutput.textContent = `${diagnostics.frame?.transient_texture_bytes?.toLocaleString() ?? "—"} B`;
}

function resetReadouts() {
  requestedOutput.textContent = "—";
  selectedOutput.textContent = "—";
  displaySizeOutput.textContent = "—";
  pickSizeOutput.textContent = "—";
  transientOutput.textContent = "—";
}

function buildProgress(trials) {
  progressOutput.replaceChildren();
  for (const trial of trials) {
    const item = document.createElement("li");
    item.dataset.trialId = trial.id;
    item.dataset.state = "pending";
    item.textContent = trial.id;
    progressOutput.append(item);
  }
  progressCount.textContent = `0 / ${trials.length}`;
}

function resetProgress() {
  for (const item of progressOutput.children) {
    item.dataset.state = "pending";
    item.textContent = item.dataset.trialId;
  }
  progressCount.textContent = `0 / ${progressOutput.children.length}`;
}

function markProgress(trialId, state, detail) {
  const item = [...progressOutput.children].find((candidate) => candidate.dataset.trialId === trialId);
  requireCondition(item !== undefined, `progress item ${trialId} is absent`);
  item.dataset.state = state;
  item.textContent = `${trialId} — ${detail}`;
}

function updateState(state, message) {
  document.body.dataset.footprintRunner = state;
  statusOutput.textContent = message;
}

function configureTransportLabel() {
  const transport = footprintArchiveTransportFromUrl(window.location.href);
  transportOutput.textContent = transport === "same-origin-local-server"
    ? "Opt-in same-origin local TAR export is active."
    : "Standard attended browser TAR download is active.";
}

function validateMode(mode) {
  requireCondition(mode === "record" || mode === "verify", "mode must be record or verify");
  return mode;
}

function validateSessionLabel(value) {
  requireCondition(typeof value === "string" && /^[A-Za-z0-9._-]{1,64}$/.test(value), "session label is invalid");
  return value;
}

function validateTrustedActivation(event) {
  requireCondition(event?.isTrusted === true, "attended run requires a trusted click");
  requireCondition(
    navigator.userActivation?.isActive === true,
    "attended run requires active browser user activation",
  );
  requireCondition(document.visibilityState === "visible",
    "attended run requires a visible qualification page");
  return {
    trusted_user_activation: true,
    browser_user_activation_active: true,
    control_id: runButton.id,
    event_type: event.type,
  };
}

function fullRectangle(image) {
  return { x: 0, y: 0, width: image.width, height: image.height };
}

function withoutImage(capture) {
  const { image: _image, ...record } = capture;
  return record;
}

function encodeJson(value) {
  return new TextEncoder().encode(`${JSON.stringify(value, null, 2)}\n`);
}

function squaredDistance(left, right) {
  return (left[0] - right[0]) ** 2 + (left[1] - right[1]) ** 2;
}

function nextAnimationFrame() {
  return new Promise((resolve) => requestAnimationFrame(resolve));
}

function failedCanonicalTrial(trialId, error) {
  return {
    trial_id: trialId,
    recreation_count: 0,
    recreations: [],
    passed: false,
    failures: [`fatal:${errorMessage(error)}`],
  };
}

function errorRecord(error) {
  return {
    schema: "punctra-browser-point-footprint-runner-error-v1",
    name: error?.name ?? "Error",
    message: errorMessage(error),
  };
}

class ArtifactRegistry {
  #entries = [];
  #metadata = [];
  #paths = new Set();
  #list;

  constructor(list) {
    this.#list = list;
  }

  async addPng(image, descriptor) {
    const bytes = await encodeRgba8Png(image);
    const metadata = await createPngArtifactMetadata({ descriptor, encodedBytes: bytes, image });
    this.#add(descriptor.path, bytes, metadata);
    return { bytes, metadata };
  }

  async addBytes(path, bytes, kind) {
    const metadata = {
      kind,
      path,
      mime_type: path.endsWith(".json") ? "application/json" : "application/octet-stream",
      encoded_byte_length: bytes.byteLength,
      encoded_sha256: await sha256Hex(bytes),
      authority: "release_evidence",
    };
    this.#add(path, bytes, metadata);
    return metadata;
  }

  #add(path, bytes, metadata) {
    requireCondition(!this.#paths.has(path), `artifact path ${path} is duplicated`);
    this.#paths.add(path);
    this.#entries.push({ path, bytes });
    this.#metadata.push(metadata);
    const item = document.createElement("li");
    const link = document.createElement("a");
    const url = URL.createObjectURL(new Blob([bytes], { type: metadata.mime_type }));
    link.href = url;
    link.download = path.split("/").at(-1);
    link.textContent = path;
    item.append(link);
    this.#list.append(item);
  }

  entries() {
    return [...this.#entries];
  }

  metadata() {
    return structuredClone(this.#metadata);
  }
}

runButton.addEventListener("click", (event) => {
  startRun({}, event).catch(() => {});
});
downloadArchiveButton.addEventListener("click", downloadLatestArchive);

window.__punctraFootprintRunner = Object.freeze({
  getState: () => document.body.dataset.footprintRunner,
  getEvidence: () => latestEvidence === undefined ? null : structuredClone(latestEvidence),
});

initializePage();
