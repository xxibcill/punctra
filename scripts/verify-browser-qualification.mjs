import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  QUALIFICATION_LIMITS,
  QUALIFICATION_WORKLOAD,
  evaluateQualification,
  recreationRequiredRecoveryEvidence,
} from "../apps/browser-demo/web/qualification.js";
import {
  QUALIFICATION_LANE,
  QUALIFICATION_RUNTIME_LANE,
} from "../apps/browser-demo/web/qualification-lane.js";

const changelogUrl = new URL("../CHANGELOG.md", import.meta.url);
const matrixUrl = new URL("../docs/releases/v0.19-browser-matrix.json", import.meta.url);
const releaseRecordUrl = new URL("../docs/releases/v0.19.0.md", import.meta.url);
const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const qualificationViewerPackage = path.join(
  repositoryRoot,
  "apps/browser-demo/web/node_modules/@punctra/viewer",
);
const qualificationViewerSource = path.join(repositoryRoot, "apps/browser-demo/web");
const qualificationFixtureRoot = path.join(
  repositoryRoot,
  "apps/browser-demo/web/fixtures/v1",
);
const qualificationManifestPath = path.join(qualificationFixtureRoot, "deployment.json");
const qualificationViewerArtifact = path.join(
  repositoryRoot,
  "target/npm/punctra-viewer-0.19.0-alpha.1.tgz",
);
const verifierSource = await readFile(new URL("./verify-browser-qualification.mjs", import.meta.url), "utf8");
const QUALIFICATION_VERIFIER_SHA256 = createHash("sha256").update(verifierSource).digest("hex");
const EXPECTED_OBSERVATION_DATE = "2026-08-26";
const QUALIFIED_IMPLEMENTATION_PATHS = [
  "Cargo.toml",
  "Cargo.lock",
  "crates",
  "examples",
  "apps/browser-demo/web",
  "packages",
  "scripts/build-browser-demo.sh",
  "scripts/build-browser-sdk.sh",
  "scripts/generate-browser-sdk-reference.mjs",
  "scripts/serve-browser-demo.py",
  "scripts/verify-browser-sdk.mjs",
];
const JAVASCRIPT_HEAP_PHASE_FIELDS = Object.freeze([
  "javascript_heap_before_bytes",
  "javascript_heap_after_cold_bytes",
  "javascript_heap_after_warm_bytes",
  "javascript_heap_after_frames_bytes",
]);
export function verifyBrowserQualificationMatrix(matrix, implementationCommit) {
  assert.equal(matrix.schema, "punctra-browser-qualification-matrix-v1");
  assert.equal(matrix.release, "0.19.0-alpha.1");
  assert.match(matrix.verifier_sha256, /^[0-9a-f]{64}$/);
  assert.equal(
    matrix.verifier_sha256,
    QUALIFICATION_VERIFIER_SHA256,
    "qualification verifier source must match the recorded SHA-256",
  );
  assert.match(matrix.implementation_commit, /^[0-9a-f]{40}$/);
  assert.equal(matrix.implementation_commit, implementationCommit);
  verifyImplementationCommit(matrix.implementation_commit);
  assert.equal(matrix.observed_on, EXPECTED_OBSERVATION_DATE);
  assert.equal(matrix.qualified_entries.length, 1);
  assert.ok(matrix.unqualified_entries.length >= 1);

  const entry = matrix.qualified_entries[0];
  const observations = entry.observations;
  verifyQualifiedLane(entry);

  assertNonnegativeNumbers(observations, "observations");
  assert.equal(observations.acceptance_schema, "punctra-browser-qualification-v1");
  assert.deepEqual(observations.limits, QUALIFICATION_LIMITS);
  verifyEnvironmentObservations(entry);
  verifyWorkloadObservations(entry);
  verifyLoadObservations(entry);
  verifyRenderObservations(entry);
  assert.deepEqual(
    observations.recovery.recreation_required,
    recreationRequiredRecoveryEvidence(),
  );
  for (const outcome of [
    "invalid_resize_preserved_viewport",
    "dpr_change_and_restore",
    "hidden_frame_skipped_and_resumed",
    "prepublication_worker_crash_preserved_viewer",
    "prepublication_worker_recoverable",
    "prepublication_worker_generation_preserved",
    "prepublication_worker_retry_succeeded",
    "prepublication_offline_failure_preserved_viewer",
    "prepublication_offline_recoverable",
    "prepublication_offline_generation_preserved",
    "cancellation_viewer_retained",
    "cancellation_frame_retained",
    "warm_cache_recreation_zero_binary_requests",
    "stale_generation_rejected",
    "generation_replacement_cleared_provisional_pick",
    "generation_replacement_cleared_presentation_highlights",
    "stale_exact_request_rejected",
    "partial_publication_failure_fuses_in_deterministic_tests",
    "device_loss_fuses_in_deterministic_tests",
  ]) {
    assert.equal(observations.recovery[outcome], true, `${outcome} must pass`);
  }
  assert.equal(observations.recovery.physical_device_loss_forced, false);
  assert.equal(observations.recovery.memory_pressure_forced, false);
  verifyTransportObservations(observations);
  verifyFrameObservations(observations.foreground_frames);
  verifyResourceObservations(entry);

  const evaluation = evaluateQualification(evaluationRecord(entry));
  assert.deepEqual(
    evaluation.failures,
    [],
    `recorded observations violate qualification limits: ${evaluation.failures.join("; ")}`,
  );
  assert.equal(observations.passed, evaluation.passed);

  assert.equal(matrix.external_evidence.independent_adopter, false);
  assert.equal(matrix.external_evidence.registry_install, false);
  assert.equal(matrix.external_evidence.support_qualified, false);
  assert.equal(matrix.external_evidence.release_candidate, false);
  return true;
}

export function releaseImplementationCommit(releaseRecord) {
  const match = releaseRecord.match(/^- Implementation commit: `([0-9a-f]{40})`$/m);
  assert.ok(match, "release record must contain one full implementation commit SHA");
  return match[1];
}

export function releaseVerifierSha256(releaseRecord) {
  const match = releaseRecord.match(/^- Qualification verifier SHA-256: `([0-9a-f]{64})`$/m);
  assert.ok(match, "release record must contain one qualification verifier SHA-256");
  return match[1];
}

export function changelogImplementationCommit(changelog) {
  const match = changelog.match(/implementation commit `([0-9a-f]{40})`/);
  assert.ok(match, "changelog must contain one full v0.19 implementation commit SHA");
  return match[1];
}

export function verifyImplementationCommit(commit) {
  const resolution = runGit("rev-parse", "--verify", `${commit}^{commit}`);
  assert.equal(
    resolution.status,
    0,
    `implementation commit ${commit} does not resolve to a repository commit`,
  );
  assert.equal(
    resolution.stdout.trim(),
    commit,
    "implementation commit must resolve to the exact recorded object",
  );

  const ancestry = runGit("merge-base", "--is-ancestor", commit, "HEAD");
  assert.equal(
    ancestry.status,
    0,
    `implementation commit ${commit} is not an ancestor of the verified checkout`,
  );

  const committedChanges = runGit(
    "diff",
    "--name-only",
    `${commit}..HEAD`,
    "--",
    ...QUALIFIED_IMPLEMENTATION_PATHS,
  );
  assert.equal(committedChanges.status, 0, "could not compare qualified implementation files");
  assert.equal(
    committedChanges.stdout.trim(),
    "",
    `qualified implementation files changed after ${commit}:\n${committedChanges.stdout.trim()}`,
  );

  const workingChanges = runGit(
    "diff",
    "--name-only",
    "HEAD",
    "--",
    ...QUALIFIED_IMPLEMENTATION_PATHS,
  );
  assert.equal(workingChanges.status, 0, "could not inspect qualified implementation files");
  assert.equal(
    workingChanges.stdout.trim(),
    "",
    `qualified implementation files have uncommitted changes:\n${workingChanges.stdout.trim()}`,
  );
  verifyQualificationRuntimeTree();
}

export function verifyQualificationRuntimeTree() {
  assert.equal(
    existsSync(qualificationViewerArtifact),
    true,
    `packed viewer artifact is missing: ${qualificationViewerArtifact}`,
  );
  assert.equal(
    existsSync(qualificationViewerPackage),
    true,
    `qualification runtime package is missing: ${qualificationViewerPackage}`,
  );

  const archiveFiles = commandOutput("tar", ["-tzf", qualificationViewerArtifact])
    .split(/\r?\n/)
    .filter((entry) => entry.startsWith("package/") && entry !== "package/")
    .map((entry) => entry.slice("package/".length))
    .sort();
  const runtimeFiles = recursiveFiles(qualificationViewerPackage)
    .map((file) => path.relative(qualificationViewerPackage, file).split(path.sep).join("/"))
    .sort();
  assert.deepEqual(
    runtimeFiles,
    archiveFiles,
    "qualification runtime package files must match the packed viewer artifact",
  );

  for (const file of archiveFiles) {
    const packed = commandBinaryOutput("tar", [
      "-xOzf",
      qualificationViewerArtifact,
      `package/${file}`,
    ]);
    const runtime = readFileSync(path.join(qualificationViewerPackage, file));
    assert.deepEqual(runtime, packed, `qualification runtime package differs for ${file}`);
    const source = readFileSync(path.join(qualificationViewerSource, file));
    assert.deepEqual(source, packed, `packed viewer artifact differs from source for ${file}`);
  }
}

function recursiveFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    return entry.isDirectory() ? recursiveFiles(file) : [file];
  });
}

function commandOutput(command, arguments_) {
  const result = spawnSync(command, arguments_, {
    cwd: repositoryRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  assert.equal(result.status, 0, `${command} ${arguments_.join(" ")} failed: ${result.stderr}`);
  return result.stdout;
}

function commandBinaryOutput(command, arguments_) {
  const result = spawnSync(command, arguments_, {
    cwd: repositoryRoot,
    encoding: null,
    stdio: ["ignore", "pipe", "pipe"],
  });
  assert.equal(result.status, 0, `${command} ${arguments_.join(" ")} failed`);
  return result.stdout;
}

function evaluationRecord(entry) {
  const observations = entry.observations;
  const [physicalWidth, physicalHeight] = entry.display.physical_viewport;
  return {
    cold: loadRecord(observations.cold),
    warm: loadRecord(observations.warm),
    frames: {
      callbackIntervalMilliseconds: {
        p95: observations.foreground_frames.callback_interval_p95_milliseconds,
      },
      submissionMilliseconds: {
        p95: observations.foreground_frames.submission_p95_milliseconds,
      },
    },
    cancellation: {
      acknowledgementMilliseconds:
        observations.recovery.cancellation_acknowledgement_milliseconds,
    },
    viewport: {
      physicalWidth,
      physicalHeight,
      surfaceBytes: entry.display.canvas_bytes,
    },
    state: {
      source: {
        coverage: observations.render.coverage,
        publishedPoints: observations.workload.displayed_points,
        publishedBatches: observations.workload.displayed_batches,
        retainedRecordBytes: observations.resources.retained_record_bytes,
      },
      render: {
        drawnPoints: observations.render.drawn_points,
        residentBytes: observations.resources.renderer_resident_bytes,
        transientTextureBytes: observations.resources.transient_texture_bytes,
      },
    },
    recovery: {
      lifecycle: {
        prior_viewport_preserved: observations.recovery.invalid_resize_preserved_viewport,
        resumed: observations.recovery.hidden_frame_skipped_and_resumed,
      },
      worker: {
        recoverable: observations.recovery.prepublication_worker_recoverable,
        viewer_retained: observations.recovery.prepublication_worker_crash_preserved_viewer,
        generation_preserved: observations.recovery.prepublication_worker_generation_preserved,
        retry_succeeded: observations.recovery.prepublication_worker_retry_succeeded,
      },
      network: {
        recoverable: observations.recovery.prepublication_offline_recoverable,
        viewer_retained: observations.recovery.prepublication_offline_failure_preserved_viewer,
        generation_preserved: observations.recovery.prepublication_offline_generation_preserved,
      },
    },
  };
}

function verifyQualifiedLane(entry) {
  assert.equal(entry.id, QUALIFICATION_LANE.id);
  assert.equal(entry.status, QUALIFICATION_LANE.status);
  for (const section of [
    "browser",
    "operating_system",
    "device",
    "webgpu",
    "display",
    "workload",
  ]) {
    assert.deepEqual(
      entry[section],
      QUALIFICATION_LANE[section],
      `qualified ${section} facts must match the exact recorded lane`,
    );
  }
  assert.deepEqual(
    {
      id: entry.id,
      host: {
        schema: QUALIFICATION_RUNTIME_LANE.host.schema,
        operatingSystem: {
          name: entry.operating_system.name,
          version: entry.operating_system.version,
          build: entry.operating_system.build,
          architecture: entry.operating_system.architecture,
        },
        device: {
          class: entry.device.class,
          gpu: entry.device.gpu,
          gpuCores: entry.device.gpu_cores,
          gpuClass: entry.device.gpu_class,
          metalSupport: entry.device.metal_support,
        },
        displayPath: entry.display.display_path,
        package: QUALIFICATION_RUNTIME_LANE.host.package,
      },
      browser: {
        userAgent: entry.browser.user_agent,
        platform: entry.operating_system.user_agent_platform,
        language: entry.browser.language,
        logicalProcessors: entry.browser.logical_processors,
      },
      screen: {
        width: entry.display.screen_css_pixels[0],
        height: entry.display.screen_css_pixels[1],
        colorDepth: entry.display.color_depth,
        pixelDepth: entry.display.pixel_depth,
      },
      display: {
        physicalWidth: entry.display.physical_viewport[0],
        physicalHeight: entry.display.physical_viewport[1],
        cssWidth: entry.display.css_viewport[0],
        cssHeight: entry.display.css_viewport[1],
        devicePixelRatio: entry.display.device_pixel_ratio,
        surfaceBytes: entry.display.canvas_bytes,
      },
      capabilities: {
        secure_context: true,
        webgpu: true,
        browser_user_agent: entry.browser.user_agent,
        browser_platform: entry.operating_system.user_agent_platform,
        adapter_name: entry.webgpu.adapter_name,
        backend: entry.webgpu.backend,
        device_type: entry.webgpu.device_type,
        surface_format: entry.webgpu.surface_format,
        composite_alpha_mode: entry.webgpu.composite_alpha_mode,
        present_mode: entry.webgpu.present_mode,
        surface_format_support: {
          render_attachment: entry.webgpu.render_attachment,
          blendable: entry.webgpu.blendable,
        },
        required_feature_count: entry.webgpu.required_feature_count,
        adapter_max_buffer_size: entry.webgpu.max_buffer_size,
        adapter_max_texture_dimension_2d: entry.webgpu.max_texture_dimension_2d,
        adapter_max_bind_groups: entry.webgpu.max_bind_groups,
        adapter_max_vertex_buffers: entry.webgpu.max_vertex_buffers,
        adapter_max_color_attachments: entry.webgpu.max_color_attachments,
      },
    },
    QUALIFICATION_RUNTIME_LANE,
    "checked-in exact lane must match the runtime qualification gate",
  );
}

export function verifyWorkloadObservations(entry) {
  const { workload } = entry;
  assert.deepEqual(
    entry.observations.workload,
    {
      deployment_id: workload.deployment_id,
      source_identity: workload.source_identity,
      source_points: workload.source_points,
      coverage: workload.coverage,
      displayed_points: workload.displayed_points,
      displayed_batches: workload.displayed_batches,
    },
    "observed workload identity must match the qualified deployment",
  );
  const deployment = JSON.parse(readFileSync(qualificationManifestPath, "utf8"));
  assert.equal(
    deployment.deployment_id,
    workload.deployment_id,
    "recorded workload deployment must match the checked-in deployment",
  );
  assert.equal(
    deployment.source.source_identity,
    workload.source_identity,
    "recorded Source identity must match the checked-in deployment",
  );
  assert.equal(
    deployment.source.point_count,
    workload.source_points,
    "recorded Source point count must match the checked-in deployment",
  );
  assert.equal(
    deployment.index.root.coverage,
    workload.coverage,
    "recorded Coverage must match the checked-in deployment",
  );
  assert.equal(
    deployment.index.root.display_point_count,
    workload.displayed_points,
    "recorded displayed Point count must match the checked-in deployment",
  );
  assert.equal(deployment.source.url, "./representative.las");
  assert.equal(deployment.index.url, "./representative.pidx");
  const sourcePath = path.join(qualificationFixtureRoot, "representative.las");
  const indexPath = path.join(qualificationFixtureRoot, "representative.pidx");
  const sourceBytes = readFileSync(sourcePath);
  const indexBytes = readFileSync(indexPath);
  assert.equal(sourceBytes.byteLength, deployment.source.byte_length);
  assert.equal(indexBytes.byteLength, deployment.index.byte_length);
  assert.equal(
    createHash("sha256").update(sourceBytes).digest("hex"),
    deployment.source.sha256,
    "checked-in LAS bytes must match the deployment digest",
  );
  assert.equal(
    createHash("sha256").update(indexBytes).digest("hex"),
    deployment.index.sha256,
    "checked-in index bytes must match the deployment digest",
  );
}

function verifyRenderObservations(entry) {
  assert.deepEqual(
    entry.observations.render,
    {
      coverage: entry.workload.coverage,
      drawn_points: entry.workload.displayed_points,
    },
    "observed render output must match the qualified workload",
  );
}

function verifyEnvironmentObservations(entry) {
  assert.deepEqual(
    entry.observations.environment,
    {
      user_agent: entry.browser.user_agent,
      platform: entry.operating_system.user_agent_platform,
      language: entry.browser.language,
      logical_processors: entry.browser.logical_processors,
      screen: {
        width: entry.display.screen_css_pixels[0],
        height: entry.display.screen_css_pixels[1],
        color_depth: entry.display.color_depth,
        pixel_depth: entry.display.pixel_depth,
      },
      visibility_state: "visible",
      secure_context: true,
    },
    "recorded browser environment must include the declared runtime facts",
  );
}

function loadRecord(load) {
  return {
    workload: load.workload,
    timings: {
      firstCoverageMilliseconds: load.first_coverage_milliseconds,
      settledViewMilliseconds: load.settled_view_milliseconds,
      mainThreadBatchMillisecondsHighWater: load.main_thread_batch_milliseconds_high_water,
    },
    metrics: {
      requestCount: load.binary_requests,
      concurrentResponseBytesHighWater: load.concurrent_response_bytes_high_water,
      decodedStagingBytesHighWater: load.worker_staging_bytes_high_water,
      cacheBytes: load.verified_cache_bytes,
    },
  };
}

function verifyLoadObservations(entry) {
  const expectedWorkload = {
    deployment_id: entry.workload.deployment_id,
    source_identity: entry.workload.source_identity,
    source_points: entry.workload.source_points,
    coverage: entry.workload.coverage,
    displayed_points: entry.workload.displayed_points,
    displayed_batches: entry.workload.displayed_batches,
    ordinal_count: entry.workload.displayed_points,
    transferred_bytes: QUALIFICATION_WORKLOAD.transferRecordBytes,
  };
  for (const label of ["cold", "warm"]) {
    const load = entry.observations[label];
    assert.deepEqual(
      load.workload,
      expectedWorkload,
      `${label} load workload facts must match the qualified deployment`,
    );
    for (const field of [
      "first_coverage_milliseconds",
      "settled_view_milliseconds",
      "main_thread_batch_milliseconds_high_water",
    ]) {
      assert.equal(
        Object.hasOwn(load, field),
        true,
        `${label} load must preserve ${field}`,
      );
    }
  }
}

function verifyTransportObservations(observations) {
  const { cold, warm } = observations;
  const deployment = JSON.parse(readFileSync(qualificationManifestPath, "utf8"));
  const expectedColdRequestedBytes = deployment.source.probe.length
    + deployment.index.header_and_root.length
    + deployment.index.root.sample_range.length;
  assert.equal(cold.binary_requests, 3);
  assert.equal(cold.requested_bytes, expectedColdRequestedBytes);
  assert.equal(cold.received_bytes, expectedColdRequestedBytes);
  assert.equal(cold.requested_bytes, cold.received_bytes);
  assert.equal(cold.verified_cache_bytes, 0);
  assert.equal(warm.binary_requests, 0);
  assert.equal(warm.requested_bytes, 0);
  assert.equal(warm.received_bytes, 0);
  assert.equal(warm.concurrent_response_bytes_high_water, 0);
  assert.equal(warm.verified_cache_hits, cold.binary_requests);
  assert.equal(warm.verified_cache_bytes, expectedColdRequestedBytes);
}

function verifyFrameObservations(frames) {
  assert.equal(frames.sample_count, 30);
  assertOrderedSummary(
    frames.callback_interval_p50_milliseconds,
    frames.callback_interval_p95_milliseconds,
    frames.callback_interval_max_milliseconds,
    "callback interval",
  );
  assertOrderedSummary(
    frames.submission_p50_milliseconds,
    frames.submission_p95_milliseconds,
    frames.submission_max_milliseconds,
    "submission",
  );
}

function assertOrderedSummary(p50, p95, maximum, label) {
  assert.ok(p50 <= p95, `${label} p50 must not exceed p95`);
  assert.ok(p95 <= maximum, `${label} p95 must not exceed max`);
}

function verifyResourceObservations(entry) {
  const resources = entry.observations.resources;
  const [physicalWidth, physicalHeight] = entry.display.physical_viewport;
  assert.equal(entry.display.canvas_bytes, physicalWidth * physicalHeight * 4);
  assert.equal(resources.javascript_heap_api, "performance.memory.usedJSHeapSize");
  assert.ok(
    ["unavailable", "non_standard_observation"].includes(resources.javascript_heap_status),
    "javascript heap status must be unavailable or non_standard_observation",
  );
  for (const field of JAVASCRIPT_HEAP_PHASE_FIELDS) {
    assert.equal(
      Object.hasOwn(resources, field),
      true,
      `resources must preserve ${field}`,
    );
    const value = resources[field];
    assert.equal(
      value === null || (Number.isSafeInteger(value) && value >= 0),
      true,
      `${field} must be a nullable nonnegative integer`,
    );
  }
  assert.equal(
    Object.hasOwn(resources, "javascript_heap_high_water_bytes"),
    true,
    "resources must preserve javascript_heap_high_water_bytes",
  );
  const phaseValues = JAVASCRIPT_HEAP_PHASE_FIELDS.map((field) => resources[field]);
  const highWater = resources.javascript_heap_high_water_bytes;
  assert.equal(
    highWater === null || (Number.isSafeInteger(highWater) && highWater >= 0),
    true,
    "javascript_heap_high_water_bytes must be a nullable nonnegative integer",
  );
  if (resources.javascript_heap_status === "unavailable") {
    assert.deepEqual(
      phaseValues,
      JAVASCRIPT_HEAP_PHASE_FIELDS.map(() => null),
      "unavailable JavaScript heap observations must be explicit nulls",
    );
    assert.equal(
      highWater,
      null,
      "unavailable JavaScript heap observations must have a null high-water",
    );
  } else {
    assert.equal(
      phaseValues.every((value) => Number.isSafeInteger(value) && value >= 0),
      true,
      "non-standard JavaScript heap observations must include every numeric phase",
    );
    assert.equal(
      highWater,
      Math.max(...phaseValues),
      "JavaScript heap high-water must match the phase observations",
    );
  }
  assert.equal(resources.process_resident_bytes, null);
  assert.equal(resources.physical_cache_allocation_bytes, null);
  assert.equal(resources.physical_gpu_allocation_bytes, null);
}

function assertNonnegativeNumbers(value, path) {
  if (typeof value === "number") {
    assert.ok(Number.isFinite(value) && value >= 0, `${path} must be finite and nonnegative`);
    return;
  }
  if (value === null || typeof value !== "object") return;
  for (const [key, nested] of Object.entries(value)) {
    assertNonnegativeNumbers(nested, `${path}.${key}`);
  }
}

function runGit(...arguments_) {
  return spawnSync("git", arguments_, {
    cwd: repositoryRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

if (process.argv[1] && pathToFileURL(process.argv[1]).href === import.meta.url) {
  const [changelog, matrixSource, releaseRecord] = await Promise.all([
    readFile(changelogUrl, "utf8"),
    readFile(matrixUrl, "utf8"),
    readFile(releaseRecordUrl, "utf8"),
  ]);
  const matrix = JSON.parse(matrixSource);
  const implementationCommit = releaseImplementationCommit(releaseRecord);
  assert.equal(
    matrix.verifier_sha256,
    releaseVerifierSha256(releaseRecord),
    "matrix and release records must pin the same qualification verifier hash",
  );
  assert.equal(
    changelogImplementationCommit(changelog),
    implementationCommit,
    "changelog and evidence records must pin the same implementation commit",
  );
  verifyBrowserQualificationMatrix(matrix, implementationCommit);
  console.log("browser qualification matrix passed");
}
