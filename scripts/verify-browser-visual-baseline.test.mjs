import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  VISUAL_EVIDENCE_SCHEMA,
  VISUAL_RELEASE,
  compareCanonicalImages,
  verifyBrowserVisualBaseline,
  verifyBrowserVisualEvidence,
  verifyCanonicalImageRecord,
} from "./verify-browser-visual-baseline.mjs";
import {
  encodeTransferV2,
  generateVisualScene,
} from "../apps/browser-demo/web/visual-corpus.js";
import {
  createDifferenceImage,
  summarizeTemporalPairs,
} from "../apps/browser-demo/web/visual-comparison.js";
import { encodeRgba8Png } from "../apps/browser-demo/web/visual-png.js";
import { QUALIFICATION_RUNTIME_LANE } from "../apps/browser-demo/web/qualification-lane.js";
import { VISUAL_ATTENDED_LANE } from "../apps/browser-demo/web/visual-provenance.js";

const implementationCommit = "a".repeat(40);
const baselinePath = new URL("../docs/releases/v0.21-browser-visual-baseline.json", import.meta.url);
const corpusPath = "apps/browser-demo/web/fixtures/visual-v1/corpus.json";
const verifierPath = "scripts/verify-browser-visual-baseline.mjs";
let prePinInputsPromise;
let positiveEvidencePromise;
const verificationCaches = {
  decodedImageCache: new Map(),
  imageDigestByObject: new WeakMap(),
  comparisonCache: new Map(),
};

test("the checked-in visual policy derives from its fixed corpus and repository inputs", async () => {
  const baseline = await pinnedBaselineFixture();
  const verified = await verifyFixture(baseline);
  assert.equal(verified.corpus.trials.length >= 6, true);
});

test("input bytes and executable generated facts cannot be replaced by matching labels", async () => {
  const baseline = await pinnedBaselineFixture();
  await assert.rejects(
    () => verifyWithCorpusTamper(baseline, (corpus) => {
      corpus.sources.find(({ kind }) => kind === "generated").payload_sha256 = "00".repeat(32);
    }),
    /generated visual facts drifted|payload SHA-256 differs|Expected values to be strictly equal/,
  );
});

test("camera and authored feature-region tampering fail derived projection checks", async () => {
  const baseline = await pinnedBaselineFixture();
  await assert.rejects(
    () => verifyWithCorpusTamper(baseline, (corpus) => {
      corpus.trials[0].camera.eye[0] += 2;
    }),
    /projection|expected feature pixel|outside its rectangle/,
  );
  await assert.rejects(
    () => verifyWithCorpusTamper(baseline, (corpus) => {
      corpus.trials[0].features[0].binding.expected_pixels[0][0] += 3;
    }),
    /feature projection differs/,
  );
});

test("condition mappings must retain all seven generated viewing conditions", async () => {
  const baseline = await pinnedBaselineFixture();
  await assert.rejects(
    () => verifyWithCorpusTamper(baseline, (corpus) => {
      const trial = corpus.trials.find(({ conditions }) => conditions.includes("mixed_lod"));
      trial.conditions = trial.conditions.filter((condition) => condition !== "mixed_lod");
    }),
    /missing mixed_lod/,
  );
  await assert.rejects(
    () => verifyWithCorpusTamper(baseline, (corpus) => {
      const generated = corpus.sources.find(({ kind }) => kind === "generated");
      generated.condition_facts.stable_lod_cut.coarse_batch_index = 0;
    }),
    /stable_lod_cut|LOD-cut|Expected values to be strictly equal|stable adjacent mixed-LOD/,
  );
  await assert.rejects(
    () => verifyWithCorpusTamper(baseline, (corpus) => {
      corpus.condition_coverage.generated[0].fact_paths[0] = "condition_facts.dense_batches";
    }),
    /condition coverage|fact mapping|fact path|deep-equal/,
  );
  await assert.rejects(
    () => verifyWithCorpusTamper(baseline, (corpus) => {
      corpus.condition_coverage.derived_modes[0].fact_paths.pop();
    }),
    /condition coverage|fact mapping|fact path|deep-equal/,
  );
});

test("Autzen condition labels are recomputed from the licensed pvis bytes", async () => {
  const baseline = await pinnedBaselineFixture();
  const manifestPath = baseline.corpus.autzen.manifest.path;
  const manifest = JSON.parse((await repositoryBytes(manifestPath)).toString("utf8"));
  manifest.condition_facts.xy_grid.singleton_cells = 0;
  const bytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  baseline.corpus.autzen.manifest.byte_length = bytes.byteLength;
  baseline.corpus.autzen.manifest.sha256 = sha256(bytes);
  await assert.rejects(
    () => verifyFixture(baseline, new Map([[manifestPath, bytes]])),
    /condition facts were not derived/,
  );
});

test("tolerance caps cannot be widened after observation", async () => {
  const baseline = await pinnedBaselineFixture();
  const tampered = structuredClone(baseline);
  tampered.tolerance_policy.profiles["canonical-lane-v1"].channel_threshold = 3;
  await assert.rejects(
    () => verifyWithCorpusTamper(tampered, (corpus) => {
      corpus.tolerance_profiles["canonical-lane-v1"].channel_threshold = 3;
    }),
    /hard cap 2|channel_threshold/,
  );
});

test("predecessor bytes, authority, and external nonclaims are immutable gates", async () => {
  const predecessor = await pinnedBaselineFixture();
  predecessor.predecessor.sha256 = "00".repeat(32);
  await assert.rejects(() => verifyFixture(predecessor), /v0\.20-browser-baseline\.json SHA-256 drifted/);

  const authority = await pinnedBaselineFixture();
  authority.authority.canonical_image = "exact_source_record";
  await assert.rejects(() => verifyFixture(authority));

  const external = await pinnedBaselineFixture();
  external.external_evidence.independent_human = true;
  await assert.rejects(() => verifyFixture(external));
});

test("the implementation and verifier pins reject abbreviated or stale identities", async () => {
  const abbreviated = await pinnedBaselineFixture();
  abbreviated.pins.implementation_commit = "abc123";
  await assert.rejects(() => verifyFixture(abbreviated), /full lowercase Git commit/);

  const staleVerifier = await pinnedBaselineFixture();
  staleVerifier.pins.verifier.sha256 = "00".repeat(32);
  await assert.rejects(() => verifyFixture(staleVerifier), /verify-browser-visual-baseline\.mjs SHA-256 drifted/);
});

test("canonical PNG evidence is decoded before its pixel identity is accepted", async () => {
  const image = solidImage(640, 480, [19, 20, 19, 255]);
  image.data.set([240, 80, 20, 255], (120 * image.width + 320) * 4);
  const encoded = await encodeRgba8Png(image);
  const record = canonicalImageRecord("evidence/canonical.png", encoded, image);
  const readRepositoryFile = async () => Buffer.from(encoded);
  assert.deepEqual(
    await verifyCanonicalImageRecord(record, canonicalViewport(), { readRepositoryFile }),
    image,
  );

  const decodedTamper = structuredClone(record);
  decodedTamper.decoded_sha256 = "00".repeat(32);
  await assert.rejects(
    () => verifyCanonicalImageRecord(decodedTamper, canonicalViewport(), { readRepositoryFile }),
    /decoded_sha256|Expected values to be strictly equal/,
  );

  const corrupt = encoded.slice();
  corrupt[Math.floor(corrupt.length / 2)] ^= 1;
  const corruptRecord = canonicalImageRecord("evidence/canonical.png", corrupt, image);
  await assert.rejects(
    () => verifyCanonicalImageRecord(corruptRecord, canonicalViewport(), {
      readRepositoryFile: async () => Buffer.from(corrupt),
    }),
    /CRC|decompress|length/,
  );
});

test("decoded comparison recomputes independent maximum, unstable, Coverage, and feature gates", () => {
  const reference = solidImage(100, 100, [19, 20, 19, 255]);
  const candidate = structuredClone(reference);
  reference.data.set([240, 80, 20, 255], (50 * 100 + 50) * 4);
  candidate.data.set([240, 80, 20, 255], (50 * 100 + 53) * 4);
  const report = compareCanonicalImages(reference, candidate, {
    toleranceProfile: boundedTolerance(),
    backgroundRgba: [19, 20, 19, 255],
    features: [{
      id: "fixed-dot",
      rectangle: { x: 45, y: 45, width: 15, height: 15 },
      minimum_foreground_pixels: 1,
    }],
  });
  assert.equal(report.passed, false);
  assert(report.failures.includes("maximum_channel_delta"));
  assert(report.features[0].failures.includes("centroid_distance"));
  assert.equal(report.coverage.reference.foreground_pixels, 1);
});

test("complete synthetic attended evidence passes every derived gate end to end", async () => {
  const fixture = await positiveEvidenceFixture();
  const options = await fixtureOptions(fixture.overrides);
  options.evidenceBytes = Buffer.from(JSON.stringify(fixture.evidence));
  const result = await verifyBrowserVisualEvidence(fixture.evidence, fixture.verified, options);
  assert.equal(result.trialResults.length, fixture.verified.corpus.trials.length);
  assert(result.trialResults.every(({ passed }) => passed));
});

test("pixel, settled worst-pair, and canonical-baseline substitutions fail from a valid control", async () => {
  const fixture = await positiveEvidenceFixture();
  await assertPositiveEvidence(fixture);

  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].temporal.settled_window.worst_pair.pair_index = 1;
  }, /worst|Expected values to be strictly deep-equal/);

  const differencePath = fixture.evidence.trials[0].recreations[0]
    .temporal.settled_window.worst_pair.difference_artifact.path;
  const changedTransitionPath = fixture.evidence.trials[0].recreations[0]
    .temporal.transition.frames[1].artifact.path;
  await rejectEvidenceMutation(fixture, (evidence) => {
    const recreation = evidence.trials[0].recreations[0];
    const changed = recreation.temporal.transition.frames[1].artifact;
    const difference = recreation.temporal.settled_window.worst_pair.difference_artifact;
    for (const field of [
      "width", "height", "encoded_byte_length", "encoded_sha256", "decoded_byte_length", "decoded_sha256",
    ]) difference[field] = changed[field];
    const registered = evidence.artifacts.find(({ path }) => path === difference.path);
    Object.assign(registered, difference);
  }, /difference PNG was not pixel-derived/, (overrides) => {
    overrides.set(differencePath, overrides.get(changedTransitionPath));
  });

  const settledPath = fixture.evidence.trials[0].recreations[0]
    .temporal.settled_window.frames[3].artifact.path;
  await rejectEvidenceMutation(fixture, () => {}, /SHA-256 drifted/, (overrides) => {
    overrides.set(settledPath, overrides.get(
      fixture.evidence.trials[0].recreations[0].temporal.transition.frames[1].artifact.path,
    ));
  });

  const baselinePathValue = fixture.evidence.trials[1].baseline.path;
  await rejectEvidenceMutation(fixture, () => {}, /SHA-256 drifted/, (overrides) => {
    overrides.set(
      baselinePathValue,
      overrides.get(fixture.evidence.trials[0].recreations[0].temporal.transition.frames[1].artifact.path),
    );
  });
});

test("coverage authority, capture-bound batches, callback facts, cleanup, and pending work reject tampering", async () => {
  const fixture = await positiveEvidenceFixture();
  await assertPositiveEvidence(fixture);

  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].coverage.declared_authority = "presentation_only";
  }, /Coverage|deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].temporal.settled_window.frames[0]
      .capture.facts.batches[0].version += 1;
  }, /capture-bound renderer batches differ/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].temporal.settled_window.frames[0]
      .capture.timing.callback_ordering = "work_done_before_mapping";
  }, /callback_ordering|Expected values to be strictly equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].resources.cleanup.after_shutdown.owned_textures = 1;
  }, /after shutdown|deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].settlement.pending_work.total = 1;
  }, /pending-work|pending work|deep-equal/);
});

test("resource and timing totals are recomputed from every accepted raw sample", async () => {
  const fixture = await positiveEvidenceFixture();
  await assertPositiveEvidence(fixture);

  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].resources.transfer.retained_record_bytes += 32;
  }, /retained_record_bytes|Expected values to be strictly equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].settlement.frame_interval_samples_milliseconds[0] = 2;
  }, /summary was not derived/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].resources.timing.capture.transition.samples.pop();
  }, /capture|deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].resources.timing.encoding.artifact_count -= 1;
  }, /encoding|deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].diagnostics.streaming.world_origin[0] += 1;
  }, /world_origin|deep-equal|Expected values/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].diagnostics.streaming.source_z_range[1] += 1;
  }, /source_z_range|deep-equal|Expected values/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].diagnostics.frame.view_generation += 1;
  }, /view_generation|Expected values/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].diagnostics.frame.surface_suboptimal = true;
  }, /surface_suboptimal|Expected values/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].resources.transfer.main_thread_batch_bytes_high_water += 32;
  }, /main_thread_batch_bytes_high_water|Expected values/);
});

test("post-capture rubric presentation, paths, ordering, and shown state are immutable", async () => {
  const fixture = await positiveEvidenceFixture();
  await assertPositiveEvidence(fixture);

  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.rubric.observation.answers.depth.artifact_paths[0]
      = evidence.rubric.observation.answers.shape.artifact_paths[0];
  }, /artifact_paths|deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.rubric.observation.answers.depth.selected_at = "2026-08-28T08:30:00.000Z";
  }, /selection predates presentation/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.rubric.observation.answers.depth.shown = false;
  }, /was not shown post-capture/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.rubric.observation.answers.depth.presentation.artifacts[0].path
      = evidence.rubric.observation.answers.shape.artifact_paths[0];
  }, /loaded artifact path differs|Expected values to be strictly equal/);
});

test("environment, nonclaims, runtime pins, baseline inputs, and recorded pass flags cannot substitute derived facts", async () => {
  const fixture = await positiveEvidenceFixture();
  await assertPositiveEvidence(fixture);

  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.environment.screen.width_css_pixels += 1;
  }, /Expected values to be strictly deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.provenance.attended_lane.execution = "programmatic";
  }, /Expected values to be strictly deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.environment.attended_lane.qualification = "self_reported";
  }, /Expected values to be strictly deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.external_evidence.independent_human = true;
  }, /deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.provenance.package_artifact.runtime_artifacts[1].sha256 = "00".repeat(32);
  }, /deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.baseline_inputs.sha256 = "00".repeat(32);
  }, /baseline_inputs|deep-equal/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.trials[0].recreations[0].passed = false;
  }, /recorded pass differs/);
  await rejectEvidenceMutation(fixture, (evidence) => {
    evidence.summary.passed = false;
  }, /Expected values to be strictly equal/);
});

test("the exact evidence-file byte ceiling rejects oversized parse-equivalent JSON", async () => {
  const fixture = await positiveEvidenceFixture();
  await assertPositiveEvidence(fixture);
  const options = await fixtureOptions(fixture.overrides);
  const serialized = Buffer.from(JSON.stringify(fixture.evidence));
  options.evidenceBytes = Buffer.concat([
    serialized,
    Buffer.alloc(fixture.verified.corpus.resource_limits.evidence_json_bytes - serialized.byteLength + 1, 0x20),
  ]);
  await assert.rejects(
    () => verifyBrowserVisualEvidence(fixture.evidence, fixture.verified, options),
    /independent byte ceiling/,
  );
});

test("rubric text remains bounded and not-observed answers are explicit", async () => {
  const baseline = await pinnedBaselineFixture();
  const rubricPath = baseline.rubric.template.path;
  const bytes = await repositoryBytes(rubricPath);
  const rubric = JSON.parse(bytes.toString("utf8"));
  rubric.answers.depth.note = "x".repeat(281);
  const tamperedBytes = Buffer.from(`${JSON.stringify(rubric, null, 2)}\n`);
  const tampered = structuredClone(baseline);
  tampered.rubric.template.byte_length = tamperedBytes.byteLength;
  tampered.rubric.template.sha256 = sha256(tamperedBytes);
  await assert.rejects(
    () => verifyFixture(tampered, new Map([[rubricPath, tamperedBytes]])),
    /rubric note depth is too long/,
  );
});

async function pinnedBaselineFixture() {
  const baseline = JSON.parse(await readFile(baselinePath, "utf8"));
  const prePin = await prePinInputsFixture();
  await refreshDigestRecords(baseline, prePin.overrides);
  baseline.pins.implementation_commit = implementationCommit;
  baseline.pins.verifier = await digestRecord(verifierPath);
  baseline.baseline_inputs.artifact = {
    path: prePin.path,
    byte_length: prePin.bytes.byteLength,
    sha256: sha256(prePin.bytes),
  };
  return baseline;
}

async function verifyWithCorpusTamper(baseline, mutate) {
  const corpus = JSON.parse((await repositoryBytes(corpusPath)).toString("utf8"));
  mutate(corpus);
  const bytes = Buffer.from(`${JSON.stringify(corpus, null, 2)}\n`);
  const adjusted = structuredClone(baseline);
  adjusted.corpus.artifact.byte_length = bytes.byteLength;
  adjusted.corpus.artifact.sha256 = sha256(bytes);
  return verifyFixture(adjusted, new Map([[corpusPath, bytes]]));
}

async function verifyFixture(baseline, overrides = new Map()) {
  return verifyBrowserVisualBaseline(baseline, await fixtureOptions(overrides));
}

async function fixtureOptions(overrides = new Map()) {
  const prePin = await prePinInputsFixture();
  const effectiveOverrides = new Map([...prePin.overrides, ...overrides]);
  const readRepositoryFile = async (relativePath, encoding) => {
    const bytes = effectiveOverrides.get(relativePath) ?? await repositoryBytes(relativePath);
    return encoding === "utf8" ? bytes.toString("utf8") : bytes;
  };
  return {
    expectedImplementationCommit: implementationCommit,
    runFixtureGenerator: false,
    readRepositoryFile,
    readPinnedFile: async (_commit, relativePath) => readRepositoryFile(relativePath),
    requireCommit: async () => {},
    ...verificationCaches,
  };
}

function prePinInputsFixture() {
  prePinInputsPromise ??= buildPrePinInputsFixture();
  return prePinInputsPromise;
}

async function buildPrePinInputsFixture() {
  const corpus = JSON.parse((await repositoryBytes(corpusPath)).toString("utf8"));
  const image = solidImage(640, 480, [80, 120, 160, 255]);
  const bytes = Buffer.from(await encodeRgba8Png(image));
  const runtimeArtifacts = await Promise.all([
    "apps/browser-demo/web/package.json",
    "apps/browser-demo/web/pkg/browser_demo.js",
    "apps/browser-demo/web/pkg/browser_demo_bg.wasm",
  ].map(digestRecord));
  const canonicalBaselines = corpus.trials.map((trial) => ({
    trial_id: trial.id,
    path: `apps/browser-demo/web/fixtures/visual-v1/baselines/${trial.id}.png`,
    width: image.width,
    height: image.height,
    encoded_byte_length: bytes.byteLength,
    encoded_sha256: sha256(bytes),
    decoded_byte_length: image.data.byteLength,
    decoded_sha256: sha256(image.data),
  }));
  const manifest = {
    schema: "punctra-browser-visual-baseline-inputs-v1",
    release: VISUAL_RELEASE,
    package_artifact: {
      package_name: "@punctra/viewer",
      package_version: VISUAL_RELEASE,
      runtime_artifacts: runtimeArtifacts,
    },
    canonical_baselines: canonicalBaselines,
  };
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
  const path = "apps/browser-demo/web/fixtures/visual-v1/baseline-inputs.json";
  return {
    path,
    bytes: manifestBytes,
    image,
    imageBytes: bytes,
    manifest,
    overrides: new Map([
      [path, manifestBytes],
      ...canonicalBaselines.map((baseline) => [baseline.path, bytes]),
    ]),
  };
}

async function refreshDigestRecords(value, overrides) {
  if (Array.isArray(value)) {
    for (const entry of value) await refreshDigestRecords(entry, overrides);
    return;
  }
  if (value === null || typeof value !== "object") return;
  if (typeof value.path === "string"
    && Number.isSafeInteger(value.byte_length)
    && typeof value.sha256 === "string") {
    const bytes = overrides.get(value.path) ?? await repositoryBytes(value.path);
    value.byte_length = bytes.byteLength;
    value.sha256 = sha256(bytes);
  }
  for (const entry of Object.values(value)) await refreshDigestRecords(entry, overrides);
}

function positiveEvidenceFixture() {
  positiveEvidencePromise ??= buildPositiveEvidenceFixture();
  return positiveEvidencePromise;
}

async function buildPositiveEvidenceFixture() {
  const baseline = await pinnedBaselineFixture();
  const verified = await verifyFixture(baseline);
  const prePin = await prePinInputsFixture();
  const overrides = new Map(prePin.overrides);
  const foregroundImage = prePin.image;
  const foregroundBytes = prePin.imageBytes;
  const transitionImage = solidImage(640, 480, [160, 80, 120, 255]);
  const transitionBytes = Buffer.from(await encodeRgba8Png(transitionImage));
  const differenceImage = createDifferenceImage(foregroundImage, foregroundImage);
  const differenceBytes = Buffer.from(await encodeRgba8Png(differenceImage));
  const artifacts = [];
  const trials = [];
  const autzen = verified.autzenManifest;

  const addArtifact = (descriptor, encoded, image, timed = true) => {
    const record = {
      kind: descriptor.kind,
      trial_id: descriptor.trial_id ?? null,
      recreation_index: descriptor.recreation_index ?? null,
      frame_index: descriptor.frame_index ?? null,
      path: descriptor.path,
      filename: descriptor.path.split("/").at(-1),
      mime_type: "image/png",
      encoding: "png-rgba8-filter-0",
      width: image.width,
      height: image.height,
      encoded_byte_length: encoded.byteLength,
      encoded_sha256: sha256(encoded),
      decoded_byte_length: image.data.byteLength,
      decoded_sha256: sha256(image.data),
      ...(timed ? {
        encode_milliseconds: 1,
        png_encode_milliseconds: 1,
        artifact_encoding_milliseconds: 2,
      } : {}),
      authority: "presentation_only",
    };
    artifacts.push(record);
    overrides.set(record.path, encoded);
    return record;
  };

  for (const trial of verified.corpus.trials) {
    const source = verified.corpus.sources.find(({ id }) => id === trial.source_id);
    const runtime = evidenceRuntimeFacts(source, autzen);
    const camera = trial.camera === "source" ? autzen.camera : trial.camera;
    const baselineRecord = addArtifact({
      kind: "baseline_png",
      trial_id: trial.id,
      recreation_index: null,
      frame_index: null,
      path: `apps/browser-demo/web/fixtures/visual-v1/baselines/${trial.id}.png`,
    }, foregroundBytes, foregroundImage, false);
    const recreations = [];
    for (let recreationIndex = 0; recreationIndex < 3; recreationIndex += 1) {
      const recreationArtifacts = [];
      let transition = null;
      if (trial.temporal_trace.kind === "mixed_lod_parent_child") {
        transition = buildTransitionEvidence({
          trial,
          source,
          runtime,
          corpus: verified.corpus,
          recreationIndex,
          foregroundImage,
          foregroundBytes,
          transitionImage,
          transitionBytes,
          addArtifact: (...arguments_) => {
            const artifact = addArtifact(...arguments_);
            recreationArtifacts.push(artifact);
            return artifact;
          },
        });
      }
      const settledBatches = evidenceSettledBatches(trial, source, runtime);
      const diagnostics = evidenceDiagnostics(trial, source, runtime, camera);
      const quietFrames = [];
      const quietPairs = [];
      let previousArtifact;
      for (let frameIndex = 0; frameIndex < 30; frameIndex += 1) {
        const final = frameIndex === 29;
        const artifact = addArtifact({
          kind: final ? "recreation_png" : "settled_quiet_frame_png",
          trial_id: trial.id,
          recreation_index: recreationIndex,
          frame_index: frameIndex,
          path: final
            ? evidenceArtifactPath(trial.id, recreationIndex, "settled")
            : evidenceArtifactPath(trial.id, recreationIndex, `quiet-${String(frameIndex).padStart(2, "0")}`),
        }, foregroundBytes, foregroundImage);
        recreationArtifacts.push(artifact);
        const capture = evidenceCapture(trial, source, runtime, verified.corpus, settledBatches);
        quietFrames.push({ index: frameIndex, artifact, capture });
        if (previousArtifact !== undefined) {
          quietPairs.push({
            from_index: frameIndex - 1,
            to_index: frameIndex,
            from_id: previousArtifact.path,
            to_id: artifact.path,
            from_path: previousArtifact.path,
            to_path: artifact.path,
            comparison: compareCanonicalImages(foregroundImage, foregroundImage, evidenceComparisonOptions(
              verified.corpus,
              trial,
              trial.temporal_tolerance_profile,
            )),
            comparison_milliseconds: 1,
          });
        }
        previousArtifact = artifact;
      }
      const temporalSummary = summarizeTemporalPairs(quietFrames.length, quietPairs);
      const differenceArtifact = addArtifact({
        kind: "settled_quiet_worst_difference_png",
        trial_id: trial.id,
        recreation_index: recreationIndex,
        frame_index: temporalSummary.worst_pair_index,
        path: evidenceArtifactPath(trial.id, recreationIndex, "quiet-worst-difference"),
      }, differenceBytes, differenceImage);
      recreationArtifacts.push(differenceArtifact);
      const representativeSettlement = evidenceQuietWindow(
        diagnostics,
        trial,
        source,
        settledBatches,
        0,
        1 + recreationIndex * 100,
      );
      const captureWindow = evidenceQuietWindow(
        diagnostics,
        trial,
        source,
        settledBatches,
        30,
        31 + recreationIndex * 100,
      );
      const finalFrame = quietFrames.at(-1);
      const finalCapture = { ...finalFrame.capture, artifact: finalFrame.artifact };
      const comparison = compareCanonicalImages(foregroundImage, foregroundImage, evidenceComparisonOptions(
        verified.corpus,
        trial,
        trial.tolerance_profile,
      ));
      const temporal = {
        kind: trial.temporal_trace.kind,
        trace: structuredClone(trial.temporal_trace),
        quiet_frame_count: 30,
        settled_window: {
          schema: "punctra-settled-quiet-window-evidence-v1",
          gating: true,
          frame_count: 30,
          pair_count: 29,
          frames: quietFrames,
          pairs: quietPairs,
          summary: temporalSummary,
          capture_window: captureWindow,
          worst_pair: {
            pair_index: temporalSummary.worst_pair_index,
            ...structuredClone(temporalSummary.worst_pair),
            difference_policy: "maximum-absolute-rgba-channel-delta-as-opaque-grayscale-v1",
            difference_artifact: differenceArtifact,
          },
        },
        transition,
      };
      const resources = evidenceResources({
        trial,
        source,
        runtime,
        corpus: verified.corpus,
        diagnostics,
        finalCapture,
        representativeSettlement,
        settledFrames: quietFrames,
        settledPairs: quietPairs,
        transition,
        recreationArtifacts,
        artifactPrefix: artifacts,
      });
      recreations.push({
        index: recreationIndex,
        environment_match: true,
        settlement: representativeSettlement,
        capture: finalCapture,
        comparison,
        temporal,
        batch_facts: evidenceBatchFacts(trial, source.expected_view),
        coverage: evidenceCoverage(trial, source, true),
        resources,
        diagnostics,
        cleanup: {
          shutdown_phase: "shutdown",
          capture_resources: structuredClone(resources.cleanup),
          raw_viewer_freed: true,
        },
        passed: true,
        failures: [],
      });
    }
    trials.push({
      trial_id: trial.id,
      source_id: trial.source_id,
      display_mode: trial.display_mode,
      projection: camera.projection,
      conditions: structuredClone(trial.conditions),
      coverage: evidenceCoverage(trial, source, false),
      input_facts: evidenceInputFacts(source, autzen),
      camera: structuredClone(camera),
      selection: structuredClone(trial.selection),
      features: structuredClone(trial.features),
      expected_view: structuredClone(source.expected_view),
      batch_facts: evidenceBatchFacts(trial, source.expected_view),
      tolerance_profile: trial.tolerance_profile,
      temporal_tolerance_profile: trial.temporal_tolerance_profile,
      baseline: baselineRecord,
      recreations,
      passed: true,
      failures: [],
    });
  }

  const startedAt = "2026-08-28T08:00:00.000Z";
  const captureCompletedAt = "2026-08-28T09:00:00.000Z";
  const completedAt = "2026-08-28T10:00:00.000Z";
  const rubric = evidenceRubric(verified.baseline.rubric, trials, captureCompletedAt, completedAt);
  const totalEncodedBytes = artifacts.reduce((total, artifact) => total + artifact.encoded_byte_length, 0);
  const evidence = {
    schema: VISUAL_EVIDENCE_SCHEMA,
    release: VISUAL_RELEASE,
    mode: "verify",
    started_at: startedAt,
    capture_completed_at: captureCompletedAt,
    completed_at: completedAt,
    corpus: {
      path: corpusPath,
      url: `https://example.invalid/${corpusPath}`,
      schema: verified.corpus.schema,
      release: verified.corpus.release,
      byte_length: verified.baseline.corpus.artifact.byte_length,
      sha256: verified.baseline.corpus.artifact.sha256,
    },
    provenance: evidenceProvenance(verified),
    environment: attendedEnvironment(verified),
    capture_policy: structuredClone(verified.corpus.capture),
    presentation_policy: structuredClone(verified.corpus.presentation_policy),
    tolerance_profiles: structuredClone(verified.corpus.tolerance_profiles),
    baseline_inputs: {
      path: verified.baseline.baseline_inputs.artifact.path,
      schema: verified.baselineInputs.schema,
      byte_length: verified.baseline.baseline_inputs.artifact.byte_length,
      sha256: verified.baseline.baseline_inputs.artifact.sha256,
    },
    trials,
    rubric,
    artifacts,
    artifact_resources: {
      schema: "punctra-browser-visual-artifact-resources-v1",
      artifact_count: artifacts.length,
      total_encoded_artifact_bytes: totalEncodedBytes,
      total_encoded_artifact_bytes_ceiling: verified.corpus.resource_limits.total_encoded_artifact_bytes,
      passed: true,
    },
    summary: {
      passed: true,
      trial_count: trials.length,
      completed_trials: trials.length,
      passed_trials: trials.length,
      failed_trials: [],
      recreations_per_trial: 3,
      non_gating_rubric_complete: true,
      artifact_count: artifacts.length,
      total_encoded_artifact_bytes: totalEncodedBytes,
      failures: [],
    },
    external_evidence: structuredClone(verified.baseline.external_evidence),
    fatal_error: null,
  };
  return { baseline, verified, evidence, overrides };
}

function evidenceRuntimeFacts(source, autzen) {
  if (source.kind === "generated") {
    const scene = generateVisualScene(source.generator);
    const batchPointCounts = scene.batches.map(({ points }) => points.length);
    return {
      sourceIdentity: scene.source_identity,
      worldOrigin: scene.world_origin,
      sourceZRange: scene.source_z_range,
      batchPointCounts,
      maximumBatchPoints: Math.max(...batchPointCounts),
    };
  }
  return {
    sourceIdentity: autzen.source.source_identity,
    worldOrigin: autzen.source.world_origin,
    sourceZRange: [autzen.source.bounds.min[2], autzen.source.bounds.max[2]],
    batchPointCounts: Array.from({ length: source.expected_view.published_batches }, (_, index) => (
      Math.min(1_024, autzen.sample.point_count - index * 1_024)
    )),
    maximumBatchPoints: 1_024,
  };
}

function evidenceInputFacts(source, autzen) {
  if (source.kind === "generated") {
    const scene = generateVisualScene(source.generator);
    const parts = scene.batches.map(({ points }) => Buffer.from(encodeTransferV2(points)));
    const payload = Buffer.concat(parts);
    return {
      kind: "generated",
      generator: scene.generator,
      conditions: scene.conditions,
      batch_roles: scene.batches.map(({ index, role, points }) => ({
        batch_index: index,
        role,
        point_count: points.length,
      })),
      lod_relations: scene.lod_relations,
      stable_lod_relations: scene.stable_lod_relations,
      transfer_bytes: payload.byteLength,
      payload_sha256: sha256(payload),
    };
  }
  return {
    kind: "derived_pvis",
    fixture_id: autzen.fixture_id,
    manifest_url: "https://example.invalid/apps/browser-demo/web/fixtures/visual-v1/autzen-classified-sample.json",
    payload_url: "https://example.invalid/apps/browser-demo/web/fixtures/visual-v1/autzen-classified-sample.pvis",
    payload_bytes: autzen.sample.byte_length,
    payload_sha256: autzen.sample.sha256,
    upstream_source_sha256: autzen.source.sha256,
    permission: structuredClone(autzen.permission),
    conditions: structuredClone(autzen.conditions),
  };
}

function evidenceSettledBatches(trial, source, runtime) {
  const removed = new Set(source.expected_view.settled_removed_batch_indices);
  return runtime.batchPointCounts.map((pointCount, batchIndex) => ({
    batch_index: batchIndex,
    key: source.expected_view.batch_keys[batchIndex],
    version: trial.expected_settled_batch_versions[batchIndex],
    point_count: pointCount,
    state: "resident",
    presentation_weight_u8: source.expected_view.settled_presentation_weights_u8[batchIndex],
  })).filter(({ batch_index: batchIndex }) => !removed.has(batchIndex));
}

function evidenceBatchFacts(trial, expectedView) {
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

function evidenceCoverage(trial, source, recreation) {
  return recreation ? {
    declared: trial.coverage,
    expected_points: source.expected_view.published_points,
    published_points: source.expected_view.published_points,
    settled_drawn_points: source.expected_view.settled_drawn_points,
    settled_resident_points: source.expected_view.settled_resident_points,
    declared_authority: "source_or_authored_facts_only",
    settled_draw_authority: "presentation_only",
    query_completion: "not_inferred_from_visual_evidence",
  } : {
    declared: trial.coverage,
    raw_stream: source.expected_view.stream_coverage,
    expected_points: source.expected_view.published_points,
    settled_drawn_points: source.expected_view.settled_drawn_points,
    declared_authority: "source_or_authored_facts_only",
    settled_draw_authority: "presentation_only",
    query_completion: "not_inferred_from_visual_evidence",
  };
}

function evidenceObservedCamera(camera) {
  return {
    eye: structuredClone(camera.eye),
    target: structuredClone(camera.target),
    up: structuredClone(camera.up),
    projection: camera.projection,
    near_distance: Math.fround(camera.near_distance),
    far_distance: Math.fround(camera.far_distance),
    vertical_field_of_view_radians: camera.projection === "perspective"
      ? Math.fround(camera.vertical_field_of_view_radians)
      : null,
    vertical_world_height: camera.projection === "orthographic" ? camera.vertical_world_height : null,
  };
}

function evidenceDiagnostics(trial, source, runtime, camera) {
  return {
    phase: "ready",
    capabilities: structuredClone(QUALIFICATION_RUNTIME_LANE.capabilities),
    viewport: {
      css_width: 320,
      css_height: 240,
      device_pixel_ratio: 2,
      physical_width: 640,
      physical_height: 480,
      surface_bytes: 1_228_800,
    },
    streaming: {
      phase: "complete",
      source_identity: runtime.sourceIdentity,
      expected_points: source.expected_view.published_points,
      published_points: source.expected_view.published_points,
      published_batches: source.expected_view.published_batches,
      transferred_bytes: source.expected_view.transferred_bytes,
      coverage: source.expected_view.stream_coverage,
      view_id: source.expected_view.view_id,
      generation: source.expected_view.generation,
      presentation_version: trial.expected_presentation_version,
      retained_record_bytes: source.expected_view.transferred_bytes,
      main_thread_batch_points_high_water: runtime.maximumBatchPoints,
      main_thread_batch_bytes_high_water: runtime.maximumBatchPoints * 32,
      world_origin: structuredClone(runtime.worldOrigin),
      source_z_range: structuredClone(runtime.sourceZRange),
      display_mode: trial.display_mode,
    },
    frame: {
      view_generation: source.expected_view.generation,
      drawn_points: source.expected_view.settled_drawn_points,
      draw_calls: source.expected_view.settled_draw_calls,
      resident_bytes: source.expected_view.settled_resident_points * 24,
      transient_texture_bytes: 0,
      surface_suboptimal: false,
    },
    capture_resources: zeroCaptureResources(),
    pick: {
      status: "not_requested",
      authority: "provisional_gpu_hint",
      generation: null,
      batch_key: null,
      batch_version: null,
      source_identity: null,
      point_ordinal: null,
    },
    highlights: {
      point_count: trial.selection.ordinals.length,
      authority: "presentation_only",
      source_identity: trial.selection.ordinals.length === 0 ? null : runtime.sourceIdentity,
      generation: trial.selection.ordinals.length === 0 ? null : source.expected_view.generation,
    },
    camera: evidenceObservedCamera(camera),
    display_mode: trial.display_mode,
    display_authority: "presentation_only",
  };
}

function evidenceCapture(trial, source, runtime, corpus, batches) {
  const callbackFacts = {
    schema: "punctra-browser-frame-capture-completion-v1",
    origin: "begin_frame_capture_monotonic_clock",
    submitted_work_done_callback_milliseconds: 2,
    readback_mapping_callback_milliseconds: 3,
  };
  const facts = {
    schema: corpus.capture.schema,
    status: "ready",
    completion: "map_callback_completed_and_copied",
    presentation: corpus.capture.presentation_claim,
    width: corpus.viewport.physical_width,
    height: corpus.viewport.physical_height,
    configured_surface_color_space: "srgb",
    canonical_format: corpus.capture.canonical_format,
    canonical_channel_order: corpus.capture.canonical_channel_order,
    origin: corpus.capture.origin,
    bytes_per_pixel: 4,
    tight_bytes_per_row: corpus.viewport.physical_width * 4,
    output_bytes: corpus.resource_limits.canonical_pixel_bytes,
    color_texture_bytes: corpus.resource_limits.capture_texture_bytes,
    source_format: corpus.required_capabilities.capture_source_format,
    source_channel_order: corpus.required_capabilities.capture_source_channel_order,
    source_encoding: "linear",
    canonical_encoding: corpus.capture.canonical_encoding,
    normalization: corpus.required_capabilities.capture_canonicalization,
    canonical_pixel_bytes: corpus.resource_limits.canonical_pixel_bytes,
    physical_presentation_observed: false,
    row_alignment_bytes: 256,
    padded_bytes_per_row: corpus.viewport.physical_width * 4,
    staging_buffer_bytes: corpus.resource_limits.staging_buffer_bytes,
    view_generation: source.expected_view.generation,
    drawn_points: batches.reduce((total, batch) => total + batch.point_count, 0),
    draw_calls: batches.length,
    resident_bytes: batches.reduce((total, batch) => total + batch.point_count * 24, 0),
    renderer_transient_texture_bytes: 0,
    batch_state_authority: "renderer_accepted_updates",
    batches: structuredClone(batches),
    completion_callbacks: callbackFacts,
  };
  return {
    schema: "punctra-browser-canonical-capture-v1",
    facts,
    timing: evidenceCaptureTiming(),
    resource_facts: {
      capture_texture_bytes: facts.color_texture_bytes,
      row_aligned_readback_bytes: facts.staging_buffer_bytes,
      canonical_pixel_bytes: facts.canonical_pixel_bytes,
      peak_live_canonical_images_during_capture: 1,
    },
  };
}

function evidenceCaptureTiming() {
  return {
    begin_submission_milliseconds: 1,
    poll_wait_milliseconds: 1,
    poll_call_milliseconds: 1,
    canonical_copy_milliseconds: 1,
    submitted_work_done_callback_milliseconds: 2,
    readback_mapping_callback_milliseconds: 3,
    callback_elapsed_origin: "begin_frame_capture_monotonic_clock",
    callback_ordering: "not_inferred",
    physical_gpu_timing: "not_observed",
    total_milliseconds: 4,
    poll_count: 1,
    animation_frames: 1,
  };
}

function evidenceQuietWindow(diagnostics, trial, source, batches, captureCount, firstFrame) {
  const animationScheduler = {
    authority: "runner_owned_request_animation_frame_tracker",
    scheduled: 30,
    resolved: 30,
    pending: 0,
  };
  const samples = Array(30).fill(1);
  const summary = { count: 30, p50: 1, p95: 1, maximum: 1 };
  const pendingWork = {
    schema: "punctra-browser-visual-pending-work-v1",
    categories: {
      load: 0,
      request: 0,
      publication: 0,
      replacement: 0,
      retirement: 0,
      recolor: 0,
      highlight: 0,
      scheduled_render: 0,
    },
    total: 0,
    sources: {
      load: { viewer_phase: diagnostics.phase },
      request: { transfer_path: "private_direct_transfer_v2" },
      publication: {
        stream_phase: diagnostics.streaming.phase,
        expected_points: diagnostics.streaming.expected_points,
        published_points: diagnostics.streaming.published_points,
      },
      replacement_and_retirement: {
        authority: "renderer_accepted_capture_batch_snapshot",
        expected_batches: structuredClone(batches),
        observed_batches: structuredClone(batches),
      },
      recolor: {
        expected_display_mode: trial.display_mode,
        observed_display_mode: diagnostics.display_mode,
        expected_batches: structuredClone(batches),
        observed_batches: structuredClone(batches),
      },
      highlight: {
        expected_points: trial.selection.ordinals.length,
        observed_points: diagnostics.highlights.point_count,
      },
      scheduled_render: animationScheduler,
    },
  };
  return {
    schema: "punctra-browser-quiet-window-v1",
    complete: true,
    quiet_frames: 30,
    first_settled_frame: firstFrame,
    quiet_window_complete_frame: firstFrame + 29,
    animation_frame_scheduler: animationScheduler,
    generation: source.expected_view.generation,
    coverage: source.expected_view.stream_coverage,
    required_frames: 30,
    observed_frames: 30,
    first_rendered_frame: firstFrame,
    last_rendered_frame: firstFrame + 29,
    stable_facts: structuredClone(diagnostics),
    observed_frame_captures: captureCount,
    frame_interval_milliseconds: summary,
    frame_submission_milliseconds: summary,
    frame_interval_samples_milliseconds: samples,
    frame_submission_samples_milliseconds: samples,
    pending_work: pendingWork,
  };
}

function buildTransitionEvidence(options) {
  const {
    trial,
    source,
    runtime,
    corpus,
    recreationIndex,
    foregroundImage,
    foregroundBytes,
    transitionImage,
    transitionBytes,
    addArtifact,
  } = options;
  const trace = trial.temporal_trace;
  const frames = [];
  const pairs = [];
  let previousImage;
  let previousArtifact;
  for (let frameIndex = 0; frameIndex < trace.child_weights_u8.length; frameIndex += 1) {
    const childWeight = trace.child_weights_u8[frameIndex];
    const parentWeight = 255 - childWeight;
    const image = frameIndex === 0 ? foregroundImage : transitionImage;
    const bytes = frameIndex === 0 ? foregroundBytes : transitionBytes;
    const weights = [...source.expected_view.settled_presentation_weights_u8];
    weights[trace.parent_batch_index] = parentWeight;
    weights[trace.child_batch_index] = childWeight;
    const batches = runtime.batchPointCounts.map((pointCount, batchIndex) => ({
      batch_index: batchIndex,
      key: source.expected_view.batch_keys[batchIndex],
      version: trial.expected_settled_batch_versions[batchIndex],
      point_count: pointCount,
      state: "resident",
      presentation_weight_u8: weights[batchIndex],
    }));
    const artifact = addArtifact({
      kind: "mixed_lod_transition_png",
      trial_id: trial.id,
      recreation_index: recreationIndex,
      frame_index: frameIndex,
      path: evidenceArtifactPath(trial.id, recreationIndex, `transition-${String(frameIndex).padStart(2, "0")}`),
    }, bytes, image);
    frames.push({
      index: frameIndex,
      parent_weight_u8: parentWeight,
      child_weight_u8: childWeight,
      artifact,
      capture: evidenceCapture(trial, source, runtime, corpus, batches),
    });
    if (previousImage !== undefined) {
      pairs.push({
        from_index: frameIndex - 1,
        to_index: frameIndex,
        from_id: previousArtifact.path,
        to_id: artifact.path,
        comparison: compareCanonicalImages(previousImage, image, evidenceComparisonOptions(
          corpus,
          trial,
          trial.tolerance_profile,
        )),
        comparison_milliseconds: 1,
      });
    }
    previousImage = image;
    previousArtifact = artifact;
  }
  const comparisons = summarizeTemporalPairs(frames.length, pairs);
  const stableRelation = generateVisualScene(source.generator).stable_lod_relations.find(
    ({ dense_batch_index: denseBatchIndex }) => denseBatchIndex === trace.child_batch_index,
  );
  return {
    schema: "punctra-mixed-lod-transition-evidence-v1",
    gating: false,
    complete: true,
    parent_batch_index: trace.parent_batch_index,
    child_batch_index: trace.child_batch_index,
    parent_removed_after_transition: true,
    stable_lod_cut: {
      ...structuredClone(stableRelation),
      dense_weight_u8: 255,
      coarse_weight_u8: 255,
      resident_through_transition: true,
    },
    frames,
    comparisons,
    changed_pair_count: pairs.filter(({ comparison }) => comparison.pixels.unstable > 0).length,
    timing: {
      schema: "punctra-browser-visual-transition-timing-v1",
      capture_samples: frames.map(({ capture }) => structuredClone(capture.timing)),
      capture_total_milliseconds: frames.length * 4,
      comparison_samples_milliseconds: pairs.map(() => 1),
      comparison_total_milliseconds: pairs.length,
    },
    interpretation: "recorded_dynamic_transition_not_a_static_tolerance_gate",
  };
}

function evidenceResources(options) {
  const {
    trial,
    source,
    runtime,
    corpus,
    diagnostics,
    finalCapture,
    representativeSettlement,
    settledFrames,
    settledPairs,
    transition,
    recreationArtifacts,
    artifactPrefix,
  } = options;
  const settledCaptureSamples = settledFrames.map(({ capture }) => structuredClone(capture.timing));
  const transitionCaptureSamples = transition?.timing.capture_samples ?? [];
  const settledWindow = evidenceTimingWindow(settledCaptureSamples);
  const transitionWindow = evidenceTimingWindow(transitionCaptureSamples);
  const settledPairSamples = settledPairs.map(({ comparison_milliseconds: value }) => value);
  const transitionPairSamples = transition?.timing.comparison_samples_milliseconds ?? [];
  const artifactTimings = recreationArtifacts.map((artifact) => ({
    path: artifact.path,
    png_encode_milliseconds: artifact.png_encode_milliseconds,
    artifact_encoding_milliseconds: artifact.artifact_encoding_milliseconds,
  }));
  const baselineMilliseconds = 1;
  const differenceMilliseconds = 1;
  const settledPairTotal = settledPairSamples.reduce((total, value) => total + value, 0);
  const transitionPairTotal = transitionPairSamples.reduce((total, value) => total + value, 0);
  const settledTotal = baselineMilliseconds + settledPairTotal + differenceMilliseconds;
  const pngTotal = artifactTimings.reduce((total, artifact) => total + artifact.png_encode_milliseconds, 0);
  const artifactTotal = artifactTimings.reduce((total, artifact) => total + artifact.artifact_encoding_milliseconds, 0);
  return {
    schema: "punctra-browser-visual-resource-evidence-v1",
    renderer: {
      resident_points: source.expected_view.settled_resident_points,
      resident_bytes: source.expected_view.settled_resident_points * 24,
      batches: source.expected_view.settled_draw_calls,
      highlight_points: trial.selection.ordinals.length,
      drawn_points: source.expected_view.settled_drawn_points,
      draw_calls: source.expected_view.settled_draw_calls,
      transient_texture_bytes: diagnostics.frame.transient_texture_bytes,
      canvas_surface_bytes: diagnostics.viewport.surface_bytes,
    },
    transfer: {
      retained_record_bytes: source.expected_view.transferred_bytes,
      main_thread_batch_bytes_high_water: runtime.maximumBatchPoints * 32,
      worker_staging_bytes: 0,
      queued_range_bytes: 0,
      concurrent_response_bytes: source.kind === "derived_pvis" ? source.expected_view.transferred_bytes : 0,
      memory_cache_bytes: 0,
      persistent_cache_bytes: 0,
      path: "private_direct_transfer_v2",
    },
    capture: {
      capture_texture_bytes: finalCapture.facts.color_texture_bytes,
      staging_buffer_bytes: finalCapture.facts.staging_buffer_bytes,
      row_aligned_readback_bytes: finalCapture.facts.staging_buffer_bytes,
      canonical_pixel_bytes: finalCapture.facts.canonical_pixel_bytes,
      encoded_png_bytes: Math.max(...artifactPrefix.map(({ encoded_byte_length: value }) => value)),
      total_encoded_artifact_bytes: artifactPrefix.reduce((total, artifact) => total + artifact.encoded_byte_length, 0),
      png_scanline_bytes: corpus.resource_limits.png_scanline_bytes,
      encoder_working_bytes: corpus.resource_limits.canonical_pixel_bytes
        + corpus.resource_limits.png_scanline_bytes
        + corpus.resource_limits.comparison_workspace_bytes,
      baseline_decoded_bytes: corpus.resource_limits.canonical_pixel_bytes,
      comparison_workspace_bytes: 1_024,
      peak_live_canonical_images: 2,
    },
    cleanup: {
      after_final_capture: zeroCaptureResources(),
      after_shutdown: zeroCaptureResources(),
    },
    timing: {
      schema: "punctra-browser-visual-timing-evidence-v1",
      lifecycle: {
        schema: "punctra-browser-visual-lifecycle-timing-v1",
        start: "fresh_private_viewer_creation",
        first_coverage: "first_renderer_accepted_batch_and_sampled_frame_submission",
        settled_view: "complete_stream_camera_display_mode_and_frame_submission",
        first_coverage_milliseconds: 1,
        settled_view_milliseconds: 2,
      },
      representative_frames: {
        capture_free: true,
        frame_count: 30,
        frame_interval_samples_milliseconds: representativeSettlement.frame_interval_samples_milliseconds,
        frame_submission_samples_milliseconds: representativeSettlement.frame_submission_samples_milliseconds,
        frame_interval_milliseconds: representativeSettlement.frame_interval_milliseconds,
        frame_submission_milliseconds: representativeSettlement.frame_submission_milliseconds,
      },
      capture: {
        settled: settledWindow,
        transition: transitionWindow,
        all_windows_total_milliseconds:
          settledWindow.totals.total_milliseconds + transitionWindow.totals.total_milliseconds,
      },
      comparison: {
        baseline_milliseconds: baselineMilliseconds,
        settled_pair_samples_milliseconds: settledPairSamples,
        settled_pair_total_milliseconds: settledPairTotal,
        worst_pair_difference_derivation_milliseconds: differenceMilliseconds,
        settled_total_milliseconds: settledTotal,
        transition_pair_samples_milliseconds: transitionPairSamples,
        transition_total_milliseconds: transitionPairTotal,
        all_comparisons_total_milliseconds: settledTotal + transitionPairTotal,
      },
      encoding: {
        artifacts: artifactTimings,
        artifact_count: artifactTimings.length,
        png_encode_total_milliseconds: pngTotal,
        artifact_encoding_total_milliseconds: artifactTotal,
      },
    },
    unavailable: {
      gpu_or_driver_allocation_bytes: null,
      process_resident_bytes: null,
      physical_cache_allocation_bytes: null,
    },
  };
}

function evidenceTimingWindow(samples) {
  const fields = [
    "begin_submission_milliseconds",
    "poll_wait_milliseconds",
    "poll_call_milliseconds",
    "canonical_copy_milliseconds",
    "submitted_work_done_callback_milliseconds",
    "readback_mapping_callback_milliseconds",
    "total_milliseconds",
    "poll_count",
    "animation_frames",
  ];
  return {
    sample_count: samples.length,
    samples: structuredClone(samples),
    totals: Object.fromEntries(fields.map((field) => [
      field,
      samples.reduce((total, sample) => total + sample[field], 0),
    ])),
  };
}

function evidenceComparisonOptions(corpus, trial, profileName) {
  return {
    toleranceProfile: corpus.tolerance_profiles[profileName],
    features: trial.features,
    backgroundRgba: corpus.presentation_policy.canonical_clear_rgba8,
  };
}

function evidenceArtifactPath(trialId, recreationIndex, label) {
  return `docs/releases/v0.21-browser-visual-artifacts/${trialId}-recreation-${recreationIndex}-${label}.png`;
}

function evidenceRubric(policy, trials, captureCompletedAt, completedAt) {
  const results = new Map(trials.map((trial) => [trial.trial_id, trial]));
  let loadOrder = 0;
  const answers = {};
  for (let promptIndex = 0; promptIndex < policy.prompts.length; promptIndex += 1) {
    const prompt = policy.prompts[promptIndex];
    const trialIds = policy.trial_bindings[prompt];
    const identities = trialIds.map((trialId) => rubricIdentity(
      results.get(trialId).recreations[0].capture.artifact,
    ));
    const loadedAt = `2026-08-28T09:${String(promptIndex + 1).padStart(2, "0")}:00.000Z`;
    const presentedAt = `2026-08-28T09:${String(promptIndex + 1).padStart(2, "0")}:10.000Z`;
    const selectedAt = `2026-08-28T09:${String(promptIndex + 1).padStart(2, "0")}:20.000Z`;
    answers[prompt] = {
      outcome: "not_observed",
      note: "Synthetic verifier fixture; not release evidence.",
      shown: true,
      trial_ids: structuredClone(trialIds),
      artifact_paths: identities.map(({ path }) => path),
      artifact_identities: identities,
      presentation: {
        schema: "punctra-browser-visual-rubric-presentation-v1",
        presented_at: presentedAt,
        presentation_order: promptIndex + 1,
        document_visibility_state: "visible",
        artifacts: identities.map((identity) => ({
          trial_id: identity.trial_id,
          path: identity.path,
          loaded_at: loadedAt,
          load_order: ++loadOrder,
          natural_width: identity.width,
          natural_height: identity.height,
          complete: true,
        })),
      },
      selected_at: selectedAt,
      selection_order: promptIndex + 1,
    };
  }
  return {
    schema: "punctra-browser-interpretation-rubric-v1",
    gating: false,
    review_status: "submitted",
    observation: {
      session_label: "synthetic-verifier-fixture",
      capture_completed_at: captureCompletedAt,
      submitted_at: completedAt,
      answers,
    },
  };
}

function rubricIdentity(artifact) {
  return Object.fromEntries([
    "kind",
    "trial_id",
    "recreation_index",
    "frame_index",
    "path",
    "width",
    "height",
    "encoded_byte_length",
    "encoded_sha256",
    "decoded_byte_length",
    "decoded_sha256",
  ].map((field) => [field, artifact[field]]));
}

function evidenceProvenance(verified) {
  return {
    implementation_commit: implementationCommit,
    verifier: structuredClone(verified.baseline.pins.verifier),
    observation_date: "2026-08-28",
    package_artifact: {
      package_name: "@punctra/viewer",
      package_version: VISUAL_RELEASE,
      runtime_artifacts: structuredClone(verified.baseline.package_runtime.built_runtime_artifacts),
    },
    attended_lane: structuredClone(VISUAL_ATTENDED_LANE),
    final_pin_required: false,
  };
}

function zeroCaptureResources() {
  return {
    pending_tickets: 0,
    owned_textures: 0,
    owned_readback_buffers: 0,
  };
}

async function assertPositiveEvidence(fixture) {
  const options = await fixtureOptions(fixture.overrides);
  options.evidenceBytes = Buffer.from(JSON.stringify(fixture.evidence));
  await verifyBrowserVisualEvidence(fixture.evidence, fixture.verified, options);
}

async function rejectEvidenceMutation(fixture, mutateEvidence, expected, mutateOverrides = () => {}) {
  const undoMutation = recordMutation(fixture.evidence, mutateEvidence);
  const overrides = new Map(fixture.overrides);
  mutateOverrides(overrides);
  try {
    const options = await fixtureOptions(overrides);
    options.evidenceBytes = Buffer.from(JSON.stringify(fixture.evidence));
    await assert.rejects(
      () => verifyBrowserVisualEvidence(fixture.evidence, fixture.verified, options),
      expected,
    );
  } finally {
    undoMutation();
  }
}

function recordMutation(target, mutate) {
  const undoOperations = [];
  const proxies = new WeakMap();
  const wrap = (value) => {
    if (value === null || typeof value !== "object") return value;
    if (proxies.has(value)) return proxies.get(value);
    const proxy = new Proxy(value, {
      get(object, property, receiver) {
        return wrap(Reflect.get(object, property, receiver));
      },
      set(object, property, nextValue, receiver) {
        const existed = Object.hasOwn(object, property);
        const previous = object[property];
        const changed = Reflect.set(object, property, nextValue, receiver);
        if (changed) {
          undoOperations.push(() => {
            if (existed) Reflect.set(object, property, previous);
            else Reflect.deleteProperty(object, property);
          });
        }
        return changed;
      },
      deleteProperty(object, property) {
        const existed = Object.hasOwn(object, property);
        const previous = object[property];
        const changed = Reflect.deleteProperty(object, property);
        if (changed && existed) undoOperations.push(() => Reflect.set(object, property, previous));
        return changed;
      },
    });
    proxies.set(value, proxy);
    return proxy;
  };
  mutate(wrap(target));
  return () => {
    for (let index = undoOperations.length - 1; index >= 0; index -= 1) undoOperations[index]();
  };
}

async function emptyEvidenceFixture(verified) {
  const runtimeArtifacts = await Promise.all([
    "apps/browser-demo/web/package.json",
    "apps/browser-demo/web/pkg/browser_demo.js",
    "apps/browser-demo/web/pkg/browser_demo_bg.wasm",
  ].map(digestRecord));
  return {
    schema: VISUAL_EVIDENCE_SCHEMA,
    release: VISUAL_RELEASE,
    provenance: {
      implementation_commit: implementationCommit,
      verifier: verified.baseline.pins.verifier,
      observation_date: "2026-08-28",
      attended_lane: structuredClone(VISUAL_ATTENDED_LANE),
      final_pin_required: false,
      package_artifact: {
        package_name: "@punctra/viewer",
        package_version: VISUAL_RELEASE,
        runtime_artifacts: runtimeArtifacts,
      },
    },
    environment: attendedEnvironment(verified),
    capture_policy: verified.corpus.capture,
    tolerance_profiles: verified.corpus.tolerance_profiles,
    external_evidence: verified.baseline.external_evidence,
    trials: [],
    rubric: {},
    summary: {},
  };
}

function attendedEnvironment(verified) {
  const requirements = verified.corpus.required_capabilities;
  return {
    schema: "punctra-browser-visual-environment-v1",
    browser: {
      user_agent: QUALIFICATION_RUNTIME_LANE.browser.userAgent,
      platform: QUALIFICATION_RUNTIME_LANE.browser.platform,
      language: QUALIFICATION_RUNTIME_LANE.browser.language,
      logical_processors: QUALIFICATION_RUNTIME_LANE.browser.logicalProcessors,
    },
    document: { secure_context: true, visibility_state: "visible", cross_origin_isolated: false },
    screen: {
      width_css_pixels: QUALIFICATION_RUNTIME_LANE.screen.width,
      height_css_pixels: QUALIFICATION_RUNTIME_LANE.screen.height,
      color_depth_bits: QUALIFICATION_RUNTIME_LANE.screen.colorDepth,
      pixel_depth_bits: QUALIFICATION_RUNTIME_LANE.screen.pixelDepth,
    },
    viewport: {
      requested_css_width: 320,
      requested_css_height: 240,
      requested_device_pixel_ratio: 2,
      observed_window_device_pixel_ratio: 2,
      observed_css_width: 320,
      observed_css_height: 240,
      canvas_bitmap_width: 640,
      canvas_bitmap_height: 480,
      visual_viewport_scale: 1,
      visual_viewport_width: QUALIFICATION_RUNTIME_LANE.display.cssWidth,
      visual_viewport_height: QUALIFICATION_RUNTIME_LANE.display.cssHeight,
    },
    color_capabilities: {
      gamut_srgb: true,
      gamut_p3: true,
      gamut_rec2020: false,
      dynamic_range_high: false,
      video_dynamic_range_high: false,
      configured_surface_color_space: "srgb",
      display_icc_profile: null,
      physical_panel_hdr_state: null,
    },
    webgpu: structuredClone(QUALIFICATION_RUNTIME_LANE.capabilities),
    fallback: { allowed: false, requested: false, used: false },
    host: structuredClone(QUALIFICATION_RUNTIME_LANE.host),
    unavailable_measurements: {
      driver_gpu_memory_bytes: null,
      energy: null,
      gpu_completion_time: null,
      physical_cache_allocation_bytes: null,
      physical_display_panel_presentation: null,
      process_resident_memory_bytes: null,
      thermal_state: null,
    },
    attended_lane: structuredClone(VISUAL_ATTENDED_LANE),
    canonical_requirements: structuredClone(requirements),
    capture: {
      source_format: requirements.capture_source_format,
      source_channel_order: requirements.capture_source_channel_order,
      source_encoding: "linear",
      canonical_format: "rgba8",
      canonical_channel_order: "rgba",
      canonical_encoding: "linear",
      configured_surface_color_space: "srgb",
      origin: "top_left",
      normalization: requirements.capture_canonicalization,
    },
  };
}

function canonicalImageRecord(relativePath, encoded, image) {
  return {
    path: relativePath,
    encoded_byte_length: encoded.byteLength,
    encoded_sha256: sha256(encoded),
    decoded_byte_length: image.data.byteLength,
    decoded_sha256: sha256(image.data),
    width: image.width,
    height: image.height,
  };
}

function solidImage(width, height, rgba) {
  const data = new Uint8Array(width * height * 4);
  for (let offset = 0; offset < data.length; offset += 4) data.set(rgba, offset);
  return { width, height, data };
}

function canonicalViewport() {
  return {
    css_width: 320,
    css_height: 240,
    requested_device_pixel_ratio: 2,
    physical_width: 640,
    physical_height: 480,
  };
}

function boundedTolerance() {
  return {
    channel_threshold: 2,
    maximum_channel_delta: 4,
    mean_channel_delta: 0.25,
    rms_channel_delta: 0.75,
    p95_channel_delta: 2,
    unstable_pixel_fraction: 0.001,
    coverage_fraction_delta: 0.001,
    feature_occupancy_fraction_delta: 0.005,
    feature_centroid_distance_pixels: 1,
  };
}

async function digestRecord(relativePath) {
  const bytes = await repositoryBytes(relativePath);
  return { path: relativePath, byte_length: bytes.byteLength, sha256: sha256(bytes) };
}

function repositoryBytes(relativePath) {
  return readFile(new URL(`../${relativePath}`, import.meta.url));
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}
