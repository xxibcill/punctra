import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { validateLodCorpus } from "./lod-corpus.js";

const corpus = JSON.parse(await readFile(new URL("./fixtures/lod-v1/corpus.json", import.meta.url)));

test("the accepted closed corpus regenerates all four Source payloads", async () => {
  assert.equal((await validateLodCorpus(corpus)).size, 4);
});

test("tampered gates, source bytes, camera inputs and external claims fail before viewer creation", async () => {
  for (const mutate of [
    (value) => { value.image_limits.maximum_temporal_rmse = 1; },
    (value) => { value.timing_limits.frame_callback_p95_milliseconds = 1_000; },
    (value) => { value.resource_limits.renderer_transient_bytes *= 2; },
    (value) => { value.sources[0].payload_sha256 = "a".repeat(64); },
    (value) => { value.sources[0].camera.eye[0] += 1; },
    (value) => { value.external_evidence.support_qualified = true; },
    (value) => { value.trials.pop(); },
  ]) {
    const tampered = structuredClone(corpus);
    mutate(tampered);
    await assert.rejects(validateLodCorpus(tampered), /differ/);
  }
});
