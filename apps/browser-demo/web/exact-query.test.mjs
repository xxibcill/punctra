import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  ExactQueryError,
  createLasExactQueryBridge,
  decodeLasLayout,
  decodeLasPointRecord,
} from "./exact-query.js";
import { validateManifest } from "./streaming-protocol.js";

const fixtureDirectory = new URL("./fixtures/v1/", import.meta.url);
const manifest = JSON.parse(await readFile(new URL("deployment.json", fixtureDirectory), "utf8"));
const source = new Uint8Array(await readFile(new URL("representative.las", fixtureDirectory)));
const manifestUrl = "https://fixtures.test/v1/deployment.json";

test("exact bridge reads one immutable LAS record without using display samples", async () => {
  const requests = [];
  const bridge = createLasExactQueryBridge({
    manifestUrl,
    fetchImplementation: fixtureFetch(requests, { credentials: "omit" }),
    credentials: "omit",
  });

  const point = await bridge.confirm({
    sourceIdentity: manifest.source.source_identity,
    pointOrdinal: 0n,
    generation: 2,
  });

  assert.deepEqual(requests, ["manifest", "bytes=0-255", "bytes=227-260"]);
  assert.equal(point.authority, "exact_source_record");
  assert.equal(point.sourceIdentity, manifest.source.source_identity);
  assert.equal(point.pointOrdinal, "0");
  assert.deepEqual(point.ticks, [-4_350, -2_500, 158]);
  assert.deepEqual(point.position, [499_956.5, 4_599_975, 101.58]);
  assert.equal(point.intensity, 0);
  assert.equal(point.classification, 2);
  assert.deepEqual(point.rgb, [0, 0, 0]);
  assert.equal(Object.isFrozen(point), true);
});

test("LAS layout and record decoders reject incompatible widths and preserve exact values", () => {
  const deployment = validateManifest(manifest, manifestUrl);
  const layout = decodeLasLayout(source.subarray(0, 256), deployment);
  const record = source.subarray(layout.pointDataOffset, layout.pointDataOffset + layout.pointRecordBytes);

  assert.equal(layout.pointFormat, 3);
  assert.equal(layout.pointRecordBytes, 34);
  assert.equal(layout.pointCount, 70_000);
  assert.equal(decodeLasPointRecord(record, deployment.source.sourceIdentity, 0, 1, layout).classification, 2);
  const highClassificationRecord = new Uint8Array(record);
  highClassificationRecord[15] = 200;
  assert.equal(
    decodeLasPointRecord(
      highClassificationRecord,
      deployment.source.sourceIdentity,
      0,
      1,
      layout,
    ).classification,
    200,
  );
  assert.throws(
    () => decodeLasPointRecord(record.subarray(0, 33), deployment.source.sourceIdentity, 0, 1, layout),
    (error) => error instanceof ExactQueryError && error.code === "exact_query_truncated",
  );
});

test("exact-query public exports are declared and documented", async () => {
  const declaration = await readFile(new URL("exact-query.d.ts", import.meta.url), "utf8");
  const guide = await readFile(new URL("../../../docs/guides/browser-viewer.md", import.meta.url), "utf8");

  for (const name of [
    "ExactQueryError",
    "createLasExactQueryBridge",
    "decodeLasLayout",
    "decodeLasPointRecord",
  ]) {
    assert.match(declaration, new RegExp(`export (?:class|function) ${name}\\b`));
    assert.ok(guide.includes(`\`${name}\``));
  }
});

test("exact confirmation classifies validator drift, cancellation, and stale Source input", async () => {
  const drift = createLasExactQueryBridge({
    manifestUrl,
    fetchImplementation: fixtureFetch([], { etag: '"changed"' }),
  });
  await assert.rejects(
    drift.confirm({
      sourceIdentity: manifest.source.source_identity,
      pointOrdinal: 0,
      generation: 1,
    }),
    (error) => error.code === "exact_query_source_changed",
  );

  const controller = new AbortController();
  controller.abort();
  const cancelled = createLasExactQueryBridge({ manifestUrl, fetchImplementation: fixtureFetch([]) });
  await assert.rejects(
    cancelled.confirm({
      sourceIdentity: manifest.source.source_identity,
      pointOrdinal: 0,
      generation: 1,
      signal: controller.signal,
    }),
    (error) => error.code === "exact_query_cancelled",
  );

  const mismatch = createLasExactQueryBridge({ manifestUrl, fetchImplementation: fixtureFetch([]) });
  await assert.rejects(
    mismatch.confirm({ sourceIdentity: "aa".repeat(32), pointOrdinal: 0, generation: 1 }),
    (error) => error.code === "exact_query_source_mismatch",
  );
});

test("exact-query errors do not expose raw external exceptions", async () => {
  const bridge = createLasExactQueryBridge({
    manifestUrl,
    fetchImplementation: async () => { throw new Error("private transport details"); },
  });

  await assert.rejects(
    bridge.confirm({
      sourceIdentity: manifest.source.source_identity,
      pointOrdinal: 0,
      generation: 1,
    }),
    (error) => error instanceof ExactQueryError
      && error.code === "exact_query_failed"
      && !("cause" in error),
  );
});

function fixtureFetch(requests, options = {}) {
  return async (input, init = {}) => {
    if (options.credentials !== undefined) {
      assert.equal(init.credentials, options.credentials);
    }
    const url = String(input);
    if (url === manifestUrl) {
      requests.push("manifest");
      return new Response(JSON.stringify(manifest), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    assert.equal(url, "https://fixtures.test/v1/representative.las");
    const range = init.headers.Range;
    requests.push(range);
    const match = /^bytes=(\d+)-(\d+)$/.exec(range);
    assert.ok(match);
    const first = Number(match[1]);
    const last = Number(match[2]);
    const body = source.slice(first, last + 1);
    return new Response(body, {
      status: 206,
      headers: {
        "Accept-Ranges": "bytes",
        "Content-Encoding": "identity",
        "Content-Length": String(body.byteLength),
        "Content-Range": `bytes ${first}-${last}/${source.byteLength}`,
        ETag: options.etag ?? manifest.source.strong_etag,
      },
    });
  };
}
