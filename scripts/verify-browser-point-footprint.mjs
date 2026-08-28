#!/usr/bin/env node

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  FOOTPRINT_LOCAL_TEST_CASE_IDS,
  FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
  derivePointFootprintEvidenceSummary,
  validatePointFootprintBaseline,
  validatePointFootprintLocalTestArtifact,
  verifyPointFootprintEvidence,
} from "../apps/browser-demo/web/footprint-evidence.js";
import { validateFootprintCorpus } from "../apps/browser-demo/web/footprint-corpus.js";
import {
  measurePointFootprint,
  measureRegionTopology,
} from "../apps/browser-demo/web/visual-footprint-metrics.js";
import { decodeRgba8Png } from "../apps/browser-demo/web/visual-png.js";

const repositoryRoot = fileURLToPath(new URL("../", import.meta.url));
const MAX_GIT_OBJECT_BYTES = 96 * 1024 * 1024;
const WHITE = Object.freeze([255, 255, 255, 255]);
const BLACK = Object.freeze([0, 0, 0, 255]);

export async function verifyBrowserPointFootprintFiles({ baselinePath, evidencePath }) {
  const baselineLocation = repositoryLocation(baselinePath, "baseline");
  const evidenceLocation = repositoryLocation(evidencePath, "evidence");
  const [baselineBytes, evidenceBytes] = await Promise.all([
    readFile(baselineLocation.absolute),
    readFile(evidenceLocation.absolute),
  ]);
  const baseline = parseJson(baselineBytes, baselineLocation.repository);
  const evidence = parseJson(evidenceBytes, evidenceLocation.repository);
  const implementationCommit = baseline?.pins?.implementation?.commit;
  assert.match(implementationCommit ?? "", /^[0-9a-f]{40}$/, "baseline implementation commit is invalid");
  await requireGitCommit(implementationCommit);

  const corpusBytes = await readPinnedFile(implementationCommit, baseline.pins.corpus.path);
  verifyDigest(corpusBytes, baseline.pins.corpus, "pinned corpus");
  const corpus = validateFootprintCorpus(parseJson(corpusBytes, baseline.pins.corpus.path));
  validatePointFootprintBaseline(baseline, corpus);

  const baselineIdentity = {
    path: baselineLocation.repository,
    byte_length: baselineBytes.byteLength,
    sha256: sha256(baselineBytes),
  };
  assert.deepEqual(evidence.baseline, baselineIdentity, "evidence does not bind the supplied baseline bytes");

  await verifyExactPins(baseline, corpus, implementationCommit);
  await verifyCandidateBaselines(baseline, implementationCommit);
  const artifacts = await loadEvidenceArtifacts(evidence);
  verifyLocalTestClaims(evidence, artifacts.localTests, implementationCommit);
  const predecessorEvidence = await loadPredecessorEvidence(baseline, implementationCommit);
  const predecessorTiming = predecessorTimingMap(predecessorEvidence);
  const imageLoader = createImageLoader({
    evidenceArtifacts: artifacts.png,
    baseline,
    corpus,
    implementationCommit,
  });
  const recomputedMetrics = await recomputeMetricBindings(evidence, imageLoader);
  await verifyDerivedFeatureFacts(evidence, predecessorEvidence, imageLoader);
  await verifyFocusedPixelFacts(evidence, imageLoader);
  verifyPickIdentityReference(evidence.pick_identity_reference, predecessorEvidence);

  const derived = derivePointFootprintEvidenceSummary(evidence, {
    baseline,
    corpus,
    baselineIdentity,
    predecessorTiming,
    recomputedMetrics,
  });
  assert.deepEqual(evidence.summary, derived, "recorded evidence summary was not derived from exact inputs");
  const verified = verifyPointFootprintEvidence(evidence, {
    baseline,
    corpus,
    baselineIdentity,
    predecessorTiming,
    recomputedMetrics,
  });
  return {
    baseline: baselineLocation.repository,
    evidence: evidenceLocation.repository,
    implementation_commit: implementationCommit,
    ...verified.summary,
  };
}

async function verifyExactPins(baseline, corpus, commit) {
  const implementationRecords = new Map();
  for (const record of baseline.pins.implementation.files) {
    const bytes = await readPinnedFile(commit, record.path);
    verifyDigest(bytes, record, `implementation pin ${record.path}`);
    implementationRecords.set(record.path, record);
  }
  const verifierRecord = implementationRecords.get(baseline.pins.verifier.path);
  assert(verifierRecord, "verifier is absent from the implementation pin");
  assert.deepEqual(verifierRecord, baseline.pins.verifier, "verifier digest differs from its implementation file pin");
  const verifierBytes = await readFile(repositoryPath(baseline.pins.verifier.path));
  verifyDigest(verifierBytes, baseline.pins.verifier, "executing verifier");

  for (const record of baseline.pins.runtime.artifacts) {
    let bytes;
    try {
      bytes = await readPinnedFile(commit, record.path);
    } catch {
      bytes = await readFile(repositoryPath(record.path));
    }
    verifyDigest(bytes, record, `runtime artifact ${record.path}`);
  }

  const corpusDirectory = path.posix.dirname(baseline.pins.corpus.path);
  for (const [name, record] of Object.entries(corpus.predecessor)) {
    if (name === "release") continue;
    const predecessorPath = resolveRelativeRepositoryPath(corpusDirectory, record.path);
    const bytes = await readPinnedFile(commit, predecessorPath);
    verifyDigest(bytes, record, `predecessor ${name}`);
  }
}

async function verifyCandidateBaselines(baseline, commit) {
  const records = new Map(
    [...baseline.candidate_images, ...baseline.focused_images].map((record) => [record.path, record]),
  );
  for (const record of records.values()) {
    const [repositoryBytes, pinnedBytes] = await Promise.all([
      readFile(repositoryPath(record.path)),
      readPinnedFile(commit, record.path),
    ]);
    verifyImageEncodedDigest(repositoryBytes, record, `candidate baseline ${record.trial_id}`);
    assert.deepEqual(repositoryBytes, pinnedBytes, `${record.path} differs from the implementation commit`);
    await verifyDecodedImage(repositoryBytes, record, `candidate baseline ${record.trial_id}`);
  }
}

async function loadEvidenceArtifacts(evidence) {
  const png = new Map();
  for (const record of evidence.artifacts.png) {
    const bytes = await readFile(repositoryPath(record.path));
    verifyImageEncodedDigest(bytes, record, `evidence PNG ${record.path}`);
    const image = await verifyDecodedImage(bytes, record, `evidence PNG ${record.path}`);
    png.set(record.path, { record, bytes, image });
  }
  const localTests = new Map();
  for (const record of evidence.artifacts.local_test_results) {
    const bytes = await readFile(repositoryPath(record.path));
    verifyDigest(bytes, record, `local test result ${record.path}`);
    assert.equal(record.media_type, "application/json", `${record.path} is not JSON test evidence`);
    const json = parseJson(bytes, record.path);
    localTests.set(record.path, { record, bytes, json });
  }
  return { png, localTests };
}

function verifyLocalTestClaims(evidence, localTests, implementationCommit) {
  assert.equal(localTests.size, 1, "evidence must bind exactly one local test artifact");
  const onlyArtifact = localTests.values().next().value;
  validatePointFootprintLocalTestArtifact(onlyArtifact.json, implementationCommit);
  assert.deepEqual(onlyArtifact.json.cases.map(({ id }) => id), FOOTPRINT_LOCAL_TEST_CASE_IDS,
    "local test artifact cases differ");
  const references = evidence.fallback_trials
    .filter(({ evidence_source }) => evidence_source === "local_renderer_test")
    .map((trial) => ({
      provenance: trial.local_test_evidence,
      facts: {
        selection: trial.selection,
        physical_width: trial.physical_width,
        physical_height: trial.physical_height,
        resources: trial.resources,
        pick_probes: trial.pick_probes,
        hard_circle_mask: trial.hard_circle_mask,
        nominal_pick_identity: trial.nominal_pick_identity,
      },
    }));
  references.push({
    provenance: evidence.local_gpu_fixture.local_test_evidence.quality,
    facts: {
      diameters_physical_pixels: evidence.local_gpu_fixture.diameters_physical_pixels,
      subpixel_center_phases: evidence.local_gpu_fixture.subpixel_center_phases,
      preferred: evidence.local_gpu_fixture.preferred,
      single_sample: evidence.local_gpu_fixture.single_sample,
    },
  }, {
    provenance: evidence.local_gpu_fixture.local_test_evidence.pick_independence,
    facts: {
      pick_independence: evidence.local_gpu_fixture.pick_independence,
    },
  }, {
    provenance: evidence.local_gpu_fixture.local_test_evidence.resource_accounting,
    facts: {
      transient_bounds: evidence.local_gpu_fixture.transient_bounds,
      resource_fallback: evidence.local_gpu_fixture.resource_fallback,
    },
  });
  for (const { provenance, facts } of references) {
    const artifact = localTests.get(provenance.artifact_path);
    assert(artifact, `${provenance.case} local test artifact is absent`);
    assert.equal(artifact.json.schema, "punctra-render-wgpu-point-footprint-test-evidence-v1",
      `${provenance.artifact_path} schema differs`);
    assert.deepEqual(Object.keys(artifact.json).sort(), [
      "cases", "environment", "implementation_commit", "producer_command", "schema",
    ], `${provenance.artifact_path} fields differ`);
    assert.equal(artifact.json.implementation_commit, implementationCommit,
      `${provenance.artifact_path} implementation commit differs`);
    assert.equal(artifact.json.producer_command, FOOTPRINT_LOCAL_TEST_PRODUCER_COMMAND,
      `${provenance.artifact_path} producer command differs`);
    assert.equal(artifact.record.producer_command, artifact.json.producer_command,
      `${provenance.artifact_path} artifact producer command differs`);
    assert.deepEqual(evidence.local_gpu_fixture.environment, artifact.json.environment,
      `${provenance.artifact_path} local GPU environment differs`);
    const testCase = artifact.json.cases?.find(({ id }) => id === provenance.case);
    assert(testCase, `${provenance.artifact_path} omits case ${provenance.case}`);
    assert.deepEqual(testCase, {
      id: provenance.case,
      source_test: provenance.source_test,
      passed: true,
      facts,
    }, `${provenance.case} evidence differs from its pinned local test result`);
  }
}

async function loadPredecessorEvidence(baseline, commit) {
  const corpusDirectory = path.posix.dirname(baseline.pins.corpus.path);
  const record = baseline.pins.predecessor.release_evidence;
  const artifactPath = resolveRelativeRepositoryPath(corpusDirectory, record.path);
  const bytes = await readPinnedFile(commit, artifactPath);
  verifyDigest(bytes, record, "v0.21 release evidence");
  const evidence = parseJson(bytes, artifactPath);
  assert.equal(evidence.schema, "punctra-browser-visual-evidence-v1");
  assert.equal(evidence.release, baseline.pins.predecessor.release);
  return evidence;
}

function predecessorTimingMap(predecessorEvidence) {
  const timing = new Map();
  for (const trial of predecessorEvidence.trials) {
    const interval = Math.max(...trial.recreations.map(
      (recreation) => recreation.settlement.frame_interval_milliseconds.p95,
    ));
    const submission = Math.max(...trial.recreations.map(
      (recreation) => recreation.settlement.frame_submission_milliseconds.p95,
    ));
    for (const recreation of trial.recreations) {
      timing.set(`${trial.trial_id}:${recreation.index}`, {
        frame_interval_p95_milliseconds: interval,
        frame_submission_p95_milliseconds: submission,
      });
    }
  }
  return timing;
}

function createImageLoader({ evidenceArtifacts, baseline, corpus, implementationCommit }) {
  const cache = new Map([...evidenceArtifacts].map(([artifactPath, value]) => [artifactPath, value.image]));
  const predecessorRecords = new Map(corpus.canonical_trials.map((trial) => [
    trial.predecessor_baseline.path,
    trial.predecessor_baseline,
  ]));
  const candidateRecords = new Map(
    [...baseline.candidate_images, ...baseline.focused_images].map((record) => [record.path, record]),
  );
  const corpusDirectory = path.posix.dirname(baseline.pins.corpus.path);
  return async (artifactPath) => {
    if (cache.has(artifactPath)) return cache.get(artifactPath);
    const predecessor = predecessorRecords.get(artifactPath);
    if (predecessor !== undefined) {
      const repositoryArtifactPath = resolveRelativeRepositoryPath(corpusDirectory, artifactPath);
      const bytes = await readPinnedFile(implementationCommit, repositoryArtifactPath);
      verifyDigest(bytes, predecessor, `predecessor PNG ${artifactPath}`);
      const image = await decodeRgba8Png(bytes);
      cache.set(artifactPath, image);
      return image;
    }
    const candidate = candidateRecords.get(artifactPath);
    if (candidate !== undefined) {
      const bytes = await readFile(repositoryPath(artifactPath));
      const image = await verifyDecodedImage(bytes, candidate, `candidate PNG ${artifactPath}`);
      cache.set(artifactPath, image);
      return image;
    }
    throw new Error(`Point-footprint verification failed: image artifact ${artifactPath} is not pinned`);
  };
}

async function recomputeMetricBindings(evidence, loadImage) {
  const bindings = collectMetricBindings(evidence);
  const results = new Map();
  const binaryImageCache = new Map();
  for (const binding of bindings) {
    const image = await loadImage(binding.artifact_path);
    let report;
    if (binding.kind === "known_endpoint_disk_v1") {
      report = measurePointFootprint(image, {
        rectangle: binding.rectangle,
        center: binding.center,
        radiusPixels: binding.radius_pixels,
        foregroundRgba: binding.foreground_rgba,
        backgroundRgba: binding.background_rgba,
      });
    } else {
      const maskKey = `${binding.artifact_path}:${binding.background_rgba.join(",")}:${binding.maximum_background_channel_delta}`;
      let mask = binaryImageCache.get(maskKey);
      if (mask === undefined) {
        mask = backgroundDifferenceImage(
          image,
          binding.background_rgba,
          binding.maximum_background_channel_delta,
        );
        binaryImageCache.set(maskKey, mask);
      }
      report = measureRegionTopology(mask, {
        rectangle: binding.rectangle,
        foregroundRgba: WHITE,
        backgroundRgba: BLACK,
        foregroundThreshold: binding.foreground_threshold,
      });
    }
    results.set(binding.metric_id, report);
  }
  return results;
}

async function verifyDerivedFeatureFacts(evidence, predecessorEvidence, loadImage) {
  const predecessorTrials = new Map(predecessorEvidence.trials.map((trial) => [trial.trial_id, trial]));
  for (const trial of evidence.canonical_trials) {
    const predecessorTrial = predecessorTrials.get(trial.trial_id);
    assert(predecessorTrial, `v0.21 evidence omits ${trial.trial_id}`);
    const expectedFeatures = predecessorTrial.features;
    const predecessorImage = await loadImage(trial.predecessor_topology.artifact_path);
    const predecessorMask = backgroundDifferenceImage(
      predecessorImage,
      trial.predecessor_topology.background_rgba,
      trial.predecessor_topology.maximum_background_channel_delta,
    );
    for (const recreation of trial.recreations) {
      assert.deepEqual(recreation.feature_checks.map(({ id }) => id), expectedFeatures.map(({ id }) => id),
        `${trial.trial_id} feature ids differ from v0.21`);
      const candidateImage = await loadImage(recreation.capture_artifact_path);
      const candidateMask = backgroundDifferenceImage(
        candidateImage,
        recreation.candidate_topology.background_rgba,
        recreation.candidate_topology.maximum_background_channel_delta,
      );
      const derived = expectedFeatures.map((feature) => {
        const predecessor = binaryRegionFacts(predecessorMask, feature.rectangle);
        const candidate = binaryRegionFacts(candidateMask, feature.rectangle);
        return {
          id: feature.id,
          predecessor_foreground_pixels: predecessor.foreground_pixels,
          candidate_foreground_pixels: candidate.foreground_pixels,
          centroid_distance_pixels: pointDistance(predecessor.centroid, candidate.centroid),
        };
      });
      assert.deepEqual(recreation.feature_checks, derived,
        `${trial.trial_id} recreation ${recreation.index} feature facts were not decoded from PNGs`);
    }
  }
}

async function verifyFocusedPixelFacts(evidence, loadImage) {
  for (const trial of evidence.focused_trials) {
    const candidateImage = await loadImage(trial.candidate_artifact_path);
    for (const sample of trial.isolated_footprints) {
      const foreground = normalizedPixelCoverage(candidateImage, sample.candidate);
      assert.equal(sample.center_foreground, foreground > 0,
        `${sample.candidate.metric_id} center-foreground fact differs from its PNG`);
    }
  }
}

function verifyPickIdentityReference(reference, predecessorEvidence) {
  const predecessor = predecessorEvidence.trials.find(
    ({ trial_id: trialId }) => trialId === "generated-classification-selection-perspective",
  );
  assert(predecessor, "v0.21 nominal-pick trial is absent");
  const expected = predecessor.recreations[0].nominal_pick.checks.map((check) => ({
    ordinal: check.ordinal,
    generation: check.expected.generation,
    source_identity: check.expected.source_identity,
    batch_key: check.expected.batch_key,
    batch_version: check.expected.batch_version,
    point_ordinal: check.expected.point_ordinal,
  }));
  for (const recreation of predecessor.recreations) {
    assert.deepEqual(recreation.nominal_pick.checks.map((check) => ({
      ordinal: check.ordinal,
      generation: check.expected.generation,
      source_identity: check.expected.source_identity,
      batch_key: check.expected.batch_key,
      batch_version: check.expected.batch_version,
      point_ordinal: check.expected.point_ordinal,
    })), expected, "v0.21 nominal-pick identities differ across recreations");
  }
  assert.deepEqual(reference.pick_probes, expected,
    "preferred nominal-pick identities differ from pinned v0.21 evidence");
}

function collectMetricBindings(evidence) {
  const bindings = [];
  for (const trial of evidence.canonical_trials) {
    bindings.push(trial.predecessor_topology);
    for (const recreation of trial.recreations) {
      bindings.push(recreation.candidate_topology);
      for (const region of recreation.dense_region_checks) {
        bindings.push(region.predecessor, region.candidate);
      }
    }
  }
  for (const trial of evidence.focused_trials) {
    for (const footprint of trial.isolated_footprints) bindings.push(footprint.candidate);
  }
  return bindings;
}

function backgroundDifferenceImage(image, backgroundRgba, maximumDelta) {
  const data = new Uint8Array(image.data.length);
  for (let offset = 0; offset < image.data.length; offset += 4) {
    let foreground = false;
    for (let channel = 0; channel < 4; channel += 1) {
      if (Math.abs(image.data[offset + channel] - backgroundRgba[channel]) > maximumDelta) {
        foreground = true;
        break;
      }
    }
    data.set(foreground ? WHITE : BLACK, offset);
  }
  return { width: image.width, height: image.height, data };
}

function binaryRegionFacts(image, rectangle) {
  let foregroundPixels = 0;
  let xTotal = 0;
  let yTotal = 0;
  for (let y = rectangle.y; y < rectangle.y + rectangle.height; y += 1) {
    for (let x = rectangle.x; x < rectangle.x + rectangle.width; x += 1) {
      if (binaryPixel(image, x, y) === 0) continue;
      foregroundPixels += 1;
      xTotal += x + 0.5;
      yTotal += y + 0.5;
    }
  }
  return {
    foreground_pixels: foregroundPixels,
    centroid: foregroundPixels === 0 ? null : {
      x: xTotal / foregroundPixels,
      y: yTotal / foregroundPixels,
    },
  };
}

function binaryPixel(image, x, y) {
  assert(Number.isSafeInteger(x) && Number.isSafeInteger(y)
    && x >= 0 && y >= 0 && x < image.width && y < image.height, "pixel coordinate exceeds image");
  return image.data[(y * image.width + x) * 4] === 255 ? 1 : 0;
}

function normalizedPixelCoverage(image, binding) {
  const x = Math.min(image.width - 1, Math.max(0, Math.floor(binding.center[0])));
  const y = Math.min(image.height - 1, Math.max(0, Math.floor(binding.center[1])));
  const offset = (y * image.width + x) * 4;
  const direction = binding.foreground_rgba.map((value, channel) => value - binding.background_rgba[channel]);
  const denominator = direction.reduce((sum, value) => sum + value * value, 0);
  let numerator = 0;
  for (let channel = 0; channel < 4; channel += 1) {
    numerator += (image.data[offset + channel] - binding.background_rgba[channel]) * direction[channel];
  }
  return Math.min(1, Math.max(0, numerator / denominator));
}

function pointDistance(left, right) {
  if (left === null || right === null) return Number.POSITIVE_INFINITY;
  return Math.hypot(left.x - right.x, left.y - right.y);
}

async function verifyDecodedImage(bytes, record, label) {
  const image = await decodeRgba8Png(bytes);
  assert.equal(image.width, record.width, `${label} width differs`);
  assert.equal(image.height, record.height, `${label} height differs`);
  assert.equal(image.data.byteLength, record.decoded_byte_length, `${label} decoded byte length differs`);
  assert.equal(sha256(image.data), record.decoded_sha256, `${label} decoded SHA-256 differs`);
  return image;
}

function verifyImageEncodedDigest(bytes, record, label) {
  verifyDigest(bytes, {
    path: record.path,
    byte_length: record.encoded_byte_length,
    sha256: record.encoded_sha256,
  }, label);
}

function verifyDigest(bytes, record, label) {
  assert.equal(bytes.byteLength, record.byte_length, `${label} byte length drifted`);
  assert.equal(sha256(bytes), record.sha256, `${label} SHA-256 drifted`);
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function parseJson(bytes, label) {
  try {
    return JSON.parse(bytes.toString("utf8"));
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error instanceof Error ? error.message : error}`);
  }
}

function repositoryLocation(value, label) {
  assert.equal(typeof value, "string", `${label} path is required`);
  const absolute = path.resolve(value);
  const relative = path.relative(repositoryRoot, absolute).split(path.sep).join("/");
  assert(relative !== "" && !relative.startsWith("../") && relative !== "..", `${label} must be inside the repository`);
  return { absolute, repository: relative };
}

function repositoryPath(repositoryRelativePath) {
  assert.equal(typeof repositoryRelativePath, "string");
  const absolute = path.resolve(repositoryRoot, repositoryRelativePath);
  assert(absolute.startsWith(`${repositoryRoot}${path.sep}`), `repository path escapes root: ${repositoryRelativePath}`);
  return absolute;
}

function resolveRelativeRepositoryPath(directory, relativePath) {
  const resolved = path.posix.normalize(path.posix.join(directory, relativePath));
  assert(!resolved.startsWith("../") && resolved !== ".." && !resolved.startsWith("/"),
    `relative artifact path escapes repository: ${relativePath}`);
  return resolved;
}

async function requireGitCommit(commit) {
  await git(["cat-file", "-e", `${commit}^{commit}`]);
}

async function readPinnedFile(commit, repositoryRelativePath) {
  assert(!repositoryRelativePath.includes("\0") && !repositoryRelativePath.startsWith("-"), "Git object path is invalid");
  return git(["cat-file", "blob", `${commit}:${repositoryRelativePath}`]);
}

function git(arguments_) {
  return new Promise((resolve, reject) => {
    execFile("git", arguments_, {
      cwd: repositoryRoot,
      encoding: null,
      maxBuffer: MAX_GIT_OBJECT_BYTES,
    }, (error, stdout, stderr) => {
      if (error !== null) {
        reject(new Error(`git ${arguments_.join(" ")} failed: ${stderr.toString("utf8").trim()}`));
        return;
      }
      resolve(stdout);
    });
  });
}

function parseArguments(argv) {
  if (argv.includes("--help") || argv.includes("-h")) return { help: true };
  const options = {};
  const positional = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--baseline" || argument === "--evidence") {
      const value = argv[index + 1];
      assert(value !== undefined && !value.startsWith("--"), `${argument} requires a path`);
      options[argument.slice(2)] = value;
      index += 1;
    } else {
      assert(!argument.startsWith("-"), `unknown option ${argument}`);
      positional.push(argument);
    }
  }
  if (positional.length > 0) {
    assert.equal(positional.length, 2, "expected BASELINE EVIDENCE positional paths");
    assert(options.baseline === undefined && options.evidence === undefined,
      "do not mix positional paths with --baseline/--evidence");
    [options.baseline, options.evidence] = positional;
  }
  assert(options.baseline !== undefined && options.evidence !== undefined,
    "both --baseline and --evidence are required");
  return { baselinePath: options.baseline, evidencePath: options.evidence, help: false };
}

function usage() {
  return [
    "Usage:",
    "  node scripts/verify-browser-point-footprint.mjs --baseline <path> --evidence <path>",
    "  node scripts/verify-browser-point-footprint.mjs <baseline-path> <evidence-path>",
  ].join("\n");
}

const isMain = process.argv[1] !== undefined
  && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const options = parseArguments(process.argv.slice(2));
    if (options.help) {
      process.stdout.write(`${usage()}\n`);
    } else {
      const result = await verifyBrowserPointFootprintFiles(options);
      process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    }
  } catch (error) {
    process.stderr.write(`Point-footprint verification failed: ${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  }
}
