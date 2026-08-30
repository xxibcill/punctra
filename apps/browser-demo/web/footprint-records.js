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
  summarizeFootprintTiming,
  validatePointFootprintBaseline,
} from "./footprint-evidence.js";
import { createVisualValidator } from "./visual-validation.js";

const { requireCondition } = createVisualValidator("Point-footprint runner failed");

export function createPointFootprintEnvironment(options) {
  const {
    browserUserAgent,
    browserPlatform,
    host,
    canonicalTrials,
    focusedTrials,
    fallback,
  } = options;
  const adapter = oneObservedAdapter(canonicalTrials, focusedTrials, fallback);
  return {
    browser_user_agent: browserUserAgent,
    browser_platform: browserPlatform || "unreported browser platform",
    operating_system: operatingSystemName(host),
    adapter_name: adapter.name,
    backend: adapter.backend,
    same_adapter_for_scale_trials: true,
    physical_display_observed: false,
  };
}

export function createPointFootprintBaselineRecord(options) {
  const {
    footprint,
    pins,
    canonicalTrials,
    focusedTrials,
    fallback,
    baselineArtifacts,
    environment,
  } = options;
  requireCondition(canonicalTrials.every(({ passed }) => passed),
    "record baseline requires every canonical trial to pass");
  requireCondition(focusedTrials.every(({ passed }) => passed),
    "record baseline requires every focused trial to pass");
  requireCondition(fallback.passed,
    "record baseline requires the attended resource fallback to pass");

  const candidateImages = footprint.canonical_trials.map((trial) => {
    const record = baselineArtifacts.find(({ kind, trial_id: trialId }) => (
      kind === "canonical" && trialId === trial.id
    ));
    requireCondition(record !== undefined, `record baseline image ${trial.id} is absent`);
    return createPointFootprintImageArtifact(record.artifact, footprint.canonical_profile.id);
  });
  const focusedImages = footprint.focused_trials.flatMap((trial) => (
    [footprint.canonical_profile, ...footprint.scale_profiles].map((profile) => {
      if (profile.id === footprint.canonical_profile.id) {
        return structuredClone(candidateImages.find(({ trial_id: trialId }) => (
          trialId === trial.id
        )));
      }
      const record = baselineArtifacts.find(({
        kind,
        trial_id: trialId,
        profile_id: profileId,
      }) => kind === "focused" && trialId === trial.id && profileId === profile.id);
      requireCondition(record !== undefined,
        `record focused baseline image ${trial.id}/${profile.id} is absent`);
      return createPointFootprintImageArtifact(record.artifact, profile.id);
    })
  ));
  const baseline = {
    schema: FOOTPRINT_BASELINE_SCHEMA,
    release: footprint.release,
    pins: structuredClone(pins),
    environment: structuredClone(environment),
    candidate_images: candidateImages,
    focused_images: focusedImages,
    external_evidence: structuredClone(FOOTPRINT_EXTERNAL_NONCLAIMS),
  };
  return validatePointFootprintBaseline(baseline, footprint);
}

export function recordPointFootprintArchiveEntries(entries, baselineArtifacts, baselinePath) {
  const baselinePaths = new Set([
    baselinePath,
    ...baselineArtifacts.map(({ artifact }) => artifact.path),
  ]);
  return entries.filter(({ path }) => baselinePaths.has(path));
}

export function createPointFootprintEvidenceRecord(options) {
  const {
    startedAt,
    readCompletedAt,
    browserUserAgent,
    browserPlatform,
    backgroundRgba,
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
    environment: providedEnvironment,
  } = options;
  validatePointFootprintBaseline(baseline, footprint);
  requireCondition(localTests.implementation_commit === pins.implementation.commit,
    "local test evidence implementation commit differs from the browser implementation pin");
  requireCondition(localTests.producer_command === FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
    "local test evidence producer command differs from the closed invocation");

  const canonicalEvidence = canonicalTrials.map((trial) => canonicalTrialEvidence(
    trial,
    footprint,
    backgroundRgba,
  ));
  const focusedEvidence = focusedTrials.map((trial) => focusedTrialEvidence(
    trial,
    baseline,
    footprint,
  ));
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
      .map((trial) => createPointFootprintImageArtifact(
        trial.capture.artifact,
        trial.profile_id,
      )),
  ]);
  const environment = providedEnvironment ?? createPointFootprintEnvironment({
    browserUserAgent,
    browserPlatform,
    host,
    canonicalTrials,
    focusedTrials,
    fallback,
  });
  const evidence = {
    schema: FOOTPRINT_EVIDENCE_SCHEMA,
    release: footprint.release,
    mode: "verify",
    started_at: startedAt,
    completed_at: readCompletedAt(),
    baseline: structuredClone(baselineIdentity),
    pins: structuredClone(pins),
    environment: structuredClone(environment),
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
  evidence.summary = derivePointFootprintEvidenceSummary(evidence, {
    baseline,
    corpus: footprint,
  });
  requireCondition(evidence.summary.passed,
    `point-footprint evidence gates failed: ${evidence.summary.failures.join("; ")}`);
  return evidence;
}

export function pointFootprintLocalTestCase(localTests, id) {
  requireCondition(FOOTPRINT_LOCAL_TEST_CASE_IDS.includes(id),
    `local test case ${id} is outside the closed contract`);
  const testCase = localTests.cases?.find((candidate) => candidate.id === id);
  requireCondition(testCase !== undefined && testCase.passed === true,
    `local test case ${id} is absent or failed`);
  return structuredClone(testCase);
}

function canonicalTrialEvidence(trial, footprint, backgroundRgba) {
  requireCondition(trial.passed, `canonical trial ${trial.trial_id} did not pass`);
  const contract = footprint.canonical_trials.find(({ id }) => id === trial.trial_id);
  const predecessorMeasurement = trial.recreations[0].predecessor_topology;
  return {
    trial_id: trial.trial_id,
    predecessor_topology: createTopologyMetricBinding({
      metricId: `canonical/${trial.trial_id}/predecessor`,
      artifactPath: contract.predecessor_baseline.path,
      backgroundRgba,
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
        backgroundRgba,
        measurement: recreation.candidate_topology,
      }),
      component_bridge_check: createComponentBridgeMetricBinding({
        metricId: `canonical/${trial.trial_id}/r${recreation.index}/component-bridges`,
        predecessorArtifactPath: contract.predecessor_baseline.path,
        candidateArtifactPath: recreation.capture.artifact.path,
        backgroundRgba,
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
          backgroundRgba,
          measurement: region.predecessor,
        }),
        candidate: createTopologyMetricBinding({
          metricId: `canonical/${trial.trial_id}/r${recreation.index}/dense/${regionIndex}/candidate`,
          artifactPath: recreation.capture.artifact.path,
          backgroundRgba,
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
  requireCondition(trial.passed,
    `focused trial ${trial.trial_id}/${trial.profile_id} did not pass`);
  const profile = profileById(footprint, trial.profile_id);
  const pinned = baseline.focused_images.find(({
    trial_id: trialId,
    profile_id: profileId,
  }) => trialId === trial.trial_id && profileId === trial.profile_id);
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
  const quality = pointFootprintLocalTestCase(
    localTests,
    "antialiased_footprint_quality_matrix",
  );
  const pick = pointFootprintLocalTestCase(
    localTests,
    "four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking",
  );
  const resources = pointFootprintLocalTestCase(
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
  requireCondition(picks.every(({ matched }) => matched),
    "preferred pick probes did not all match");
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
  requireCondition(profile !== undefined,
    `point-footprint profile ${profileId} is absent`);
  return profile;
}
