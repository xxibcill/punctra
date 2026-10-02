import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { loadFootprintCorpus, validateFootprintCorpus } from "./footprint-corpus.js";

const corpusUrl = new URL("./fixtures/footprint-v1/corpus.json", import.meta.url);

test("checked-in point-footprint corpus closes predecessor, profiles, trials, and limits", async () => {
  const corpus = JSON.parse(await readFile(corpusUrl, "utf8"));
  assert.equal(validateFootprintCorpus(corpus), corpus);
  assert.deepEqual(corpus.scale_profiles.map((profile) => profile.requested_device_pixel_ratio), [1, 4]);
  assert.equal(corpus.canonical_trials.length, 9);
  assert.equal(corpus.focused_trials.length, 3);
});

test("point-footprint corpus loader binds exact response bytes", async () => {
  const bytes = await readFile(corpusUrl);
  const loaded = await loadFootprintCorpus("https://example.test/fixtures/footprint-v1/corpus.json", async () => new Response(bytes));
  assert.equal(loaded.corpus.release, "0.22.0-alpha.1");
  assert.equal(loaded.byte_length, bytes.byteLength);
  assert.match(loaded.sha256, /^[0-9a-f]{64}$/);
});

test("point-footprint corpus rejects relaxed policy and unbounded fallback", async () => {
  const original = JSON.parse(await readFile(corpusUrl, "utf8"));
  const relaxed = structuredClone(original);
  relaxed.metric_limits.coverage_rmse = 0.2;
  assert.throws(() => validateFootprintCorpus(relaxed), /coverage RMSE differs/);

  const preferredFallback = structuredClone(original);
  preferredFallback.fallback_profile.physical_height = 1000;
  preferredFallback.fallback_profile.css_height = 500;
  assert.throws(() => validateFootprintCorpus(preferredFallback), /does not exceed/);

  const changedPredecessor = structuredClone(original);
  changedPredecessor.canonical_trials[0].predecessor_baseline.sha256 = "0".repeat(64);
  assert.throws(() => validateFootprintCorpus(changedPredecessor), /SHA-256 is invalid|duplicated|differs/);
});
