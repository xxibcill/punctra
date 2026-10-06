import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { validateLodCorpus } from "../apps/browser-demo/web/lod-corpus.js";
import { auditLodRecord, LOD_BASELINE_PATH, LOD_BASELINE_SCHEMA, LOD_EVIDENCE_PATH, LOD_EVIDENCE_SCHEMA,
  LOD_RELEASE, LOD_ROOT } from "../apps/browser-demo/web/lod-records.js";
import { validateVisualCorpus } from "../apps/browser-demo/web/visual-corpus.js";
import { decodeRgba8Png } from "../apps/browser-demo/web/visual-png.js";

const ROOT = fileURLToPath(new URL("../", import.meta.url));
const QUALIFIED = [".gitignore", "Cargo.toml", "Cargo.lock", "fuzz", "crates", "apps", "packages", "scripts", "examples",
  "docs/api/browser-sdk.md", "docs/design/lod-density-transition-continuity-v0.23.md"];
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

function git(...args) {
  const result = spawnSync("git", ["-C", ROOT, ...args], { encoding: "utf8", maxBuffer: 96 * 1024 * 1024 });
  assert.equal(result.status, 0, `git ${args[0]} failed: ${result.stderr}`);
  return result.stdout.trim();
}

function pinned(commit, repositoryPath) {
  canonicalPath(repositoryPath);
  const result = spawnSync("git", ["-C", ROOT, "show", `${commit}:${repositoryPath}`], { maxBuffer: 96 * 1024 * 1024 });
  assert.equal(result.status, 0, `pinned file ${repositoryPath} unavailable`);
  return result.stdout;
}

export function canonicalPath(value) {
  assert.equal(typeof value, "string", "artifact path is absent");
  assert.match(value, /^[A-Za-z0-9._/-]+$/, "artifact path is not portable");
  assert.ok(!path.isAbsolute(value) && !value.includes("//")
    && value.split("/").every((part) => part && part !== "." && part !== ".."), "artifact path escapes repository");
  return value;
}

export function verifyDigest(bytes, record, label) {
  assert.equal(bytes.byteLength, record.byte_length ?? record.encoded_byte_length, `${label} byte length differs`);
  assert.equal(digest(bytes), record.sha256 ?? record.encoded_sha256, `${label} SHA-256 differs`);
}

async function verifyPins(pins) {
  assert.match(pins.implementation.commit, /^[a-f0-9]{40}$/);
  assert.equal(git("rev-parse", `${pins.implementation.commit}^{commit}`), pins.implementation.commit);
  const expectedPaths = git("ls-tree", "-r", "--name-only", pins.implementation.commit, "--", ...QUALIFIED).split("\n");
  assert.deepEqual(pins.implementation.files.map((entry) => entry.path), expectedPaths, "implementation file closure differs");
  for (const record of pins.implementation.files) {
    verifyDigest(pinned(pins.implementation.commit, record.path), record, "pinned implementation");
    verifyDigest(await readFile(path.join(ROOT, record.path)), record, "running implementation");
  }
  assert.equal(git("status", "--porcelain", "--untracked-files=all", "--", ...QUALIFIED), "", "qualified implementation files are dirty");
  assert.equal(pins.verifier.path, "scripts/verify-browser-lod-continuity.mjs");
  verifyDigest(await readFile(path.join(ROOT, pins.verifier.path)), pins.verifier, "running independent verifier");
  assert.equal(pins.runtime.package_name, "@punctra/viewer");
  assert.equal(pins.runtime.package_version, LOD_RELEASE);
  assert.deepEqual(pins.runtime.artifacts.map((entry) => entry.path), ["apps/browser-demo/web/package.json",
    "apps/browser-demo/web/pkg/browser_demo.js", "apps/browser-demo/web/pkg/browser_demo_bg.wasm"]);
  assert.equal(pins.runtime.packed_artifact.path, "target/npm/punctra-viewer-0.23.0-alpha.1.tgz");
  for (const record of [...pins.runtime.artifacts, pins.runtime.packed_artifact, pins.corpus, pins.predecessor, pins.predecessor_evidence]) {
    canonicalPath(record.path);
    verifyDigest(await readFile(path.join(ROOT, record.path)), record, "current bound artifact");
  }
}

export async function verifyBrowserLodFiles({ baselinePath = LOD_BASELINE_PATH, evidencePath = LOD_EVIDENCE_PATH, recordOnly = false } = {}) {
  canonicalPath(baselinePath);
  canonicalPath(evidencePath);
  const baselineBytes = await readFile(path.join(ROOT, baselinePath));
  assert.ok(baselineBytes.length <= 16_777_216, "baseline JSON ceiling exceeded");
  const baseline = JSON.parse(baselineBytes);
  assert.equal(baseline.schema, LOD_BASELINE_SCHEMA);
  assert.equal(baseline.mode, "record");
  await verifyPins(baseline.pins);
  const corpusBytes = pinned(baseline.pins.implementation.commit, baseline.pins.corpus.path);
  verifyDigest(corpusBytes, baseline.pins.corpus, "corpus");
  const corpus = JSON.parse(corpusBytes);
  const fixtures = await validateLodCorpus(corpus);
  const visual = validateVisualCorpus(JSON.parse(pinned(baseline.pins.implementation.commit, "apps/browser-demo/web/fixtures/visual-v1/corpus.json")));
  const predecessor = JSON.parse(await readFile(path.join(ROOT, baseline.pins.predecessor.path)));
  const predecessorEvidence = JSON.parse(await readFile(path.join(ROOT, baseline.pins.predecessor_evidence.path)));
  assert.equal(predecessor.pins.implementation.commit, corpus.predecessor.implementation_commit);
  let verifiedArtifacts = 0;
  const imageLoader = (record) => {
    const metadata = new Map(record.artifacts.map((entry) => [entry.path, entry]));
    assert.equal(metadata.size, record.artifacts.length, "artifact paths duplicated");
    assert.ok(metadata.size <= corpus.resource_limits.archive_entries);
    const expectedPrefix = `${LOD_ROOT}/${record.mode}/`;
    let totalBytes = 0;
    for (const entry of metadata.values()) {
      canonicalPath(entry.path);
      assert.ok(entry.path.startsWith(expectedPrefix), "artifact is outside its release/mode root");
      assert.equal(entry.mime_type, "image/png");
      totalBytes += entry.encoded_byte_length;
    }
    assert.ok(totalBytes <= corpus.resource_limits.encoded_archive_bytes);
    assert.equal(record.resources.encoded_artifact_bytes, totalBytes);
    assert.equal(record.resources.live_canonical_images, 4);
    assert.equal(record.resources.live_canonical_bytes_high_water, 4 * 1281 * 1024 * 4);
    assert.ok(record.resources.live_canonical_bytes_high_water <= corpus.resource_limits.live_canonical_bytes);
    assert.equal(record.resources.observed_heap_bytes, null);
    assert.equal(record.resources.observed_driver_memory_bytes, null);
    const used = new Set();
    const load = async (entry) => {
      assert.deepEqual(entry, metadata.get(entry.path), "capture metadata is not an exact registered artifact");
      const bytes = await readFile(path.join(ROOT, entry.path));
      verifyDigest(bytes, entry, "encoded PNG");
      const image = await decodeRgba8Png(bytes);
      assert.equal(image.width, entry.width);
      assert.equal(image.height, entry.height);
      assert.equal(image.data.length, entry.decoded_byte_length);
      assert.equal(digest(image.data), entry.decoded_sha256, "decoded PNG differs");
      if (!used.has(entry.path)) verifiedArtifacts += 1;
      used.add(entry.path);
      return image;
    };
    return { load, metadata, used };
  };
  async function audit(record, reference = null) {
    const images = imageLoader(record);
    const summary = await auditLodRecord(record, { corpus, fixtures, visual, predecessor, predecessorEvidence,
      loadImage: images.load, baseline: reference,
      visualOptions: { corpusUrl: "https://punctra.invalid/apps/browser-demo/web/fixtures/visual-v1/corpus.json",
        fetchImplementation: async (url) => {
          const location = new URL(url);
          assert.equal(location.origin, "https://punctra.invalid");
          const bytes = pinned(baseline.pins.implementation.commit, location.pathname.slice(1));
          return { ok: true, json: async () => JSON.parse(bytes),
            arrayBuffer: async () => bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) };
        } },
    });
    for (const run of record.transitions) await images.load(run.cleanup.artifact);
    for (const probe of record.boundary_probes) {
      await images.load(probe.before.artifact);
      await images.load(probe.after.artifact);
      assert.equal(probe.before_decoded_sha256, probe.before.artifact.decoded_sha256);
      assert.equal(probe.after_decoded_sha256, probe.after.artifact.decoded_sha256);
      assert.deepEqual(probe.before_controls, probe.before.capture.facts.raster_transitions ?? []);
      assert.deepEqual(probe.after_controls, probe.after.capture.facts.raster_transitions ?? []);
      assert.equal(probe.before.capture.facts.resident_bytes, probe.after.capture.facts.resident_bytes);
      assert.deepEqual(probe.before.capture.facts.batches, probe.after.capture.facts.batches);
    }
    assert.equal(images.used.size, images.metadata.size, "registered artifact is not consumed by any evidence claim");
    assert.deepEqual(summary, record.summary, "recorded summary differs from independently recomputed result");
    return summary;
  }
  await audit(baseline);
  if (recordOnly) return { implementation_commit: baseline.pins.implementation.commit, mode: "record-audit", verified_artifacts: verifiedArtifacts, ...baseline.summary };
  const evidenceBytes = await readFile(path.join(ROOT, evidencePath));
  assert.ok(evidenceBytes.length <= corpus.resource_limits.evidence_json_bytes);
  const evidence = JSON.parse(evidenceBytes);
  assert.equal(evidence.schema, LOD_EVIDENCE_SCHEMA);
  assert.equal(evidence.mode, "verify");
  assert.deepEqual(evidence.pins, baseline.pins);
  assert.deepEqual(evidence.baseline, { path: baselinePath, byte_length: baselineBytes.byteLength, sha256: digest(baselineBytes) });
  const summary = await audit(evidence, baseline);
  return { implementation_commit: baseline.pins.implementation.commit, mode: "separate-verify", verified_artifacts: verifiedArtifacts, ...summary };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const args = process.argv.slice(2);
  const option = (name, fallback) => args.includes(name) ? args[args.indexOf(name) + 1] : fallback;
  console.log(JSON.stringify(await verifyBrowserLodFiles({ baselinePath: option("--baseline", LOD_BASELINE_PATH),
    evidencePath: option("--evidence", LOD_EVIDENCE_PATH), recordOnly: args.includes("--record-only") }), null, 2));
}
