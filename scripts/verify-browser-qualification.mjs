import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

import {
  QUALIFICATION_LIMITS,
  QUALIFICATION_RUNTIME_LANE,
  evaluateQualification,
  recreationRequiredRecoveryEvidence,
} from "../apps/browser-demo/web/qualification.js";

const changelogUrl = new URL("../CHANGELOG.md", import.meta.url);
const matrixUrl = new URL("../docs/releases/v0.19-browser-matrix.json", import.meta.url);
const releaseRecordUrl = new URL("../docs/releases/v0.19.0.md", import.meta.url);
const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
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
const EXPECTED_QUALIFIED_LANE = {
  id: "codex-iab-chromium-151-macos-26-apple-m5-pro",
  status: "repository_qualified_exact_lane",
  browser: {
    surface: "Codex in-app browser",
    engine: "Chromium",
    user_agent_version: "151.0.0.0",
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36",
    language: "en-US",
    logical_processors: 15,
  },
  operating_system: {
    name: "macOS",
    version: "26.5.2",
    build: "25F84",
    architecture: "arm64",
    user_agent_platform: "MacIntel",
    note: "The browser's reduced user-agent OS token is not the operating-system version authority.",
  },
  device: {
    class: "Apple silicon laptop",
    gpu: "Apple M5 Pro",
    gpu_cores: 16,
    gpu_class: "integrated",
    metal_support: "Metal 4",
    mapping_note: "The browser exposed only a generic WebGPU adapter name; the physical GPU mapping is a local-system inference from the sole installed GPU.",
  },
  webgpu: {
    adapter_name: "browser WebGPU adapter",
    backend: "BrowserWebGpu",
    device_type: "Other",
    surface_format: "Bgra8Unorm",
    composite_alpha_mode: "Opaque",
    present_mode: "fifo",
    render_attachment: true,
    blendable: true,
    required_feature_count: 0,
    max_buffer_size: 4_294_967_292,
    max_texture_dimension_2d: 16_384,
    max_bind_groups: 4,
    max_vertex_buffers: 8,
    max_color_attachments: 8,
  },
  display: {
    physical_viewport: [1_749, 1_093],
    css_viewport: [874.28125, 546.421875],
    device_pixel_ratio: 2,
    screen_css_pixels: [1_512, 982],
    color_depth: 30,
    pixel_depth: 30,
    canvas_bytes: 7_646_628,
    display_path: "built-in Retina display",
  },
  workload: {
    deployment_id: "repository-las-v1",
    source_identity: "c459ff39717b7d6994aaebf344641f5a3add7faf65e249b85933ebd066d1c26e",
    source_points: 70_000,
    coverage: "sampled",
    displayed_points: 4_096,
    displayed_batches: 4,
  },
};

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
        publishedPoints: entry.workload.displayed_points,
        publishedBatches: entry.workload.displayed_batches,
        retainedRecordBytes: observations.resources.retained_record_bytes,
      },
      render: {
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
  assert.equal(entry.id, EXPECTED_QUALIFIED_LANE.id);
  assert.equal(entry.status, EXPECTED_QUALIFIED_LANE.status);
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
      EXPECTED_QUALIFIED_LANE[section],
      `qualified ${section} facts must match the exact recorded lane`,
    );
  }
  assert.deepEqual(
    {
      id: entry.id,
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

function loadRecord(load) {
  return {
    timings: {
      firstCoverageMilliseconds: load.first_coverage_milliseconds,
      settledViewMilliseconds: load.settled_view_milliseconds,
    },
    metrics: {
      requestCount: load.binary_requests,
      concurrentResponseBytesHighWater: load.concurrent_response_bytes_high_water,
      decodedStagingBytesHighWater: load.worker_staging_bytes_high_water,
      cacheBytes: load.verified_cache_bytes,
    },
  };
}

function verifyTransportObservations(observations) {
  const { cold, warm } = observations;
  assert.equal(cold.binary_requests, 3);
  assert.equal(cold.requested_bytes, cold.received_bytes);
  assert.equal(cold.verified_cache_bytes, 0);
  assert.equal(warm.binary_requests, 0);
  assert.equal(warm.requested_bytes, 0);
  assert.equal(warm.received_bytes, 0);
  assert.equal(warm.concurrent_response_bytes_high_water, 0);
  assert.equal(warm.verified_cache_hits, cold.binary_requests);
  assert.equal(warm.verified_cache_bytes, cold.received_bytes);
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
  assert.equal(resources.javascript_heap_status, "non_standard_observation");
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
