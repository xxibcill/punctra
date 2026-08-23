import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  StreamingFailure,
  cacheEntryUrl,
  cacheNamespace,
  runStreamingOperation,
  validateManifest,
  workerFailure,
} from "./streaming-protocol.js";

const MANIFEST_URL = "https://fixtures.test/v1/deployment.json";
const fixtureDirectory = new URL("./fixtures/v1/", import.meta.url);
const manifestBytes = await readFile(new URL("deployment.json", fixtureDirectory));
const sourceBytes = await readFile(new URL("representative.las", fixtureDirectory));
const indexBytes = await readFile(new URL("representative.pidx", fixtureDirectory));
const manifest = JSON.parse(manifestBytes.toString("utf8"));

test("manifest fixes one immutable LAS and sampled disk-v2 root", () => {
  const deployment = validateManifest(manifest, MANIFEST_URL);

  assert.equal(deployment.source.byteLength, 2_380_227);
  assert.equal(deployment.source.pointCount, 70_000);
  assert.equal(deployment.index.root.coverage, "sampled");
  assert.equal(deployment.index.root.displayPointCount, 4_096);
  assert.equal(deployment.index.root.sampleRange.length, 172_032);
  assert.equal(deployment.source.sourceIdentity, deployment.index.sourceIdentity);
});

test("cold stream verifies bounded ranges and transfers four ordered batches", async () => {
  const server = fixtureServer();
  const batches = [];
  const result = await run(server, {
    onBatch: (buffer, facts) => batches.push({ buffer, facts }),
  });

  assert.equal(result.metrics.requestCount, 3);
  assert.equal(result.metrics.sourceNetworkBytes, 256);
  assert.equal(result.metrics.indexNetworkBytes, 172_440);
  assert.equal(result.metrics.networkBytes, 172_696);
  assert.ok(result.metrics.sourceNetworkBytes < result.deployment.source.byteLength);
  assert.equal(result.metrics.transferredBatches, 4);
  assert.equal(result.metrics.transferredPoints, 4_096);
  assert.equal(result.metrics.transferredBytes, 98_304);
  assert.equal(result.metrics.decodedStagingBytesHighWater, 196_608);
  assert.equal(batches.length, 4);
  assert.ok(strictlyIncreasingOrdinals(batches));
  assert.equal(server.binaryRequests.length, 3);
  assert.deepEqual(server.binaryRequests.map((request) => request.range), [
    "bytes=0-255",
    "bytes=0-407",
    "bytes=744-172775",
  ]);
});

test("persistent warm stream reuses only the exact identity-versioned ranges", async () => {
  const cacheStorage = new TestCacheStorage();
  const server = fixtureServer();
  const cold = await run(server, {}, { cacheMode: "persistent", invalidate: true, cacheStorage });
  const binaryRequestsAfterCold = server.binaryRequests.length;
  const warm = await run(server, {}, { cacheMode: "persistent", cacheStorage });

  assert.equal(cold.metrics.requestCount, 3);
  assert.equal(warm.metrics.requestCount, 0);
  assert.equal(warm.metrics.cacheHits, 3);
  assert.equal(warm.metrics.cacheBytes, 172_696);
  assert.equal(server.binaryRequests.length, binaryRequestsAfterCold);

  const deployment = validateManifest(manifest, MANIFEST_URL);
  const changedManifest = structuredClone(manifest);
  changedManifest.source.source_identity = "aa".repeat(32);
  changedManifest.index.source_identity = "aa".repeat(32);
  const changed = validateManifest(changedManifest, MANIFEST_URL);
  assert.notEqual(cacheNamespace(deployment), cacheNamespace(changed));
  assert.notEqual(
    cacheEntryUrl(deployment, "source", deployment.source.probe),
    cacheEntryUrl(changed, "source", changed.source.probe),
  );
});

test("full response to a Range request fails before its body is read", async () => {
  const server = fixtureServer({ scenario: "full_response" });
  await rejectsWithCode(run(server), "range_unsupported");
  assert.equal(server.bodyReads, 0);
});

test("validator drift has a distinct changed-Source outcome", async () => {
  await rejectsWithCode(run(fixtureServer({ scenario: "validator_drift" })), "source_changed");
});

test("truncation, oversized bodies, and digest corruption remain distinct", async () => {
  await rejectsWithCode(run(fixtureServer({ scenario: "truncated" })), "range_truncated");
  await rejectsWithCode(run(fixtureServer({ scenario: "oversized" })), "resource_limit");
  await rejectsWithCode(run(fixtureServer({ scenario: "corrupt" })), "range_corrupt");
});

test("retryable server failures stop at the declared bounded retry count", async () => {
  const recovers = fixtureServer({ scenario: "retry_twice" });
  const result = await run(recovers, {}, { delay: async () => {} });
  assert.equal(result.metrics.retries, 2);
  assert.equal(recovers.attempts, 6);

  const exhausted = fixtureServer({ scenario: "retry_forever" });
  await rejectsWithCode(run(exhausted, {}, { delay: async () => {} }), "retry_exhausted");
  assert.equal(exhausted.attempts, 4);
});

test("offline, cancellation, quota, and worker failure have safe recovery codes", async () => {
  await rejectsWithCode(run(fixtureServer({ scenario: "offline" }), {}, { delay: async () => {} }), "offline");

  const controller = new AbortController();
  controller.abort();
  await rejectsWithCode(run(fixtureServer(), {}, { signal: controller.signal }), "cancelled");

  const quotaStorage = new TestCacheStorage({ quotaFailure: true });
  await rejectsWithCode(
    run(fixtureServer(), {}, { cacheMode: "persistent", cacheStorage: quotaStorage }),
    "cache_quota",
  );

  const failure = workerFailure("worker crashed");
  assert.equal(failure.code, "worker_failed");
  assert.match(failure.safe_action, /create a new worker/);
});

test("unsupported bare or mismatched deployments fail before binary Fetch", async () => {
  const invalid = structuredClone(manifest);
  invalid.index = undefined;
  const server = fixtureServer({ manifestOverride: invalid });
  await rejectsWithCode(run(server), "manifest_invalid");
  assert.equal(server.binaryRequests.length, 0);
});

async function run(server, hooks = {}, overrides = {}) {
  return runStreamingOperation(
    {
      manifestUrl: MANIFEST_URL,
      cacheMode: "none",
      credentials: "omit",
      fetchImplementation: server.fetch,
      ...overrides,
    },
    hooks,
  );
}

function fixtureServer(options = {}) {
  const state = {
    attempts: 0,
    bodyReads: 0,
    binaryRequests: [],
  };
  const servedManifest = options.manifestOverride ?? manifest;
  state.fetch = async (input, init = {}) => {
    state.attempts += 1;
    const url = String(input);
    if (url === MANIFEST_URL) {
      return new Response(JSON.stringify(servedManifest), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (options.scenario === "offline") throw new TypeError("offline");
    if (options.scenario === "retry_forever") return new Response(null, { status: 503 });
    if (options.scenario === "retry_twice" && state.binaryRequests.length === 0 && state.attempts <= 3) {
      return new Response(null, { status: 503 });
    }
    const descriptor = binaryDescriptor(url, servedManifest);
    const range = new Headers(init.headers).get("Range");
    state.binaryRequests.push({ url, range });
    const [start, end] = parseRange(range);
    const original = descriptor.bytes.subarray(start, end + 1);
    if (options.scenario === "full_response") {
      return unreadResponse(200, descriptor.bytes, {}, state);
    }
    let body = original;
    if (options.scenario === "truncated") body = original.subarray(0, original.length - 1);
    if (options.scenario === "oversized") {
      body = new Uint8Array(original.length + 1);
      body.set(original);
    }
    if (options.scenario === "corrupt") {
      body = Uint8Array.from(original);
      body[0] ^= 0xff;
    }
    const etag = options.scenario === "validator_drift" ? '"changed"' : descriptor.etag;
    return unreadResponse(
      206,
      body,
      {
        "Accept-Ranges": "bytes",
        "Content-Length": String(original.length),
        "Content-Range": `bytes ${start}-${end}/${descriptor.bytes.length}`,
        ETag: etag,
        "Content-Encoding": "identity",
      },
      state,
    );
  };
  return state;
}

function binaryDescriptor(url, servedManifest) {
  const sourceUrl = new URL(servedManifest.source.url, MANIFEST_URL).href;
  const indexUrl = new URL(servedManifest.index.url, MANIFEST_URL).href;
  if (url === sourceUrl) {
    return { bytes: sourceBytes, etag: servedManifest.source.strong_etag };
  }
  if (url === indexUrl) {
    return { bytes: indexBytes, etag: servedManifest.index.strong_etag };
  }
  throw new Error(`unexpected fixture URL ${url}`);
}

function parseRange(value) {
  const match = /^bytes=(\d+)-(\d+)$/.exec(value ?? "");
  assert.ok(match, `invalid Range ${value}`);
  return [Number(match[1]), Number(match[2])];
}

function unreadResponse(status, body, headers, state) {
  const response = new Response(body, { status, headers });
  const read = response.arrayBuffer.bind(response);
  response.arrayBuffer = async () => {
    state.bodyReads += 1;
    return read();
  };
  return response;
}

function strictlyIncreasingOrdinals(batches) {
  let previous = -1n;
  for (const { buffer } of batches) {
    const view = new DataView(buffer);
    for (let offset = 0; offset < buffer.byteLength; offset += 24) {
      const ordinal = view.getBigUint64(offset, true);
      if (ordinal <= previous) return false;
      previous = ordinal;
    }
  }
  return true;
}

async function rejectsWithCode(promise, code) {
  await assert.rejects(promise, (error) => {
    assert.ok(error instanceof StreamingFailure);
    assert.equal(error.code, code);
    assert.ok(error.safeAction.length > 0);
    return true;
  });
}

class TestCacheStorage {
  constructor(options = {}) {
    this.namespaces = new Map();
    this.quotaFailure = options.quotaFailure === true;
  }

  async delete(name) {
    return this.namespaces.delete(name);
  }

  async open(name) {
    const entries = this.namespaces.get(name) ?? new Map();
    this.namespaces.set(name, entries);
    return {
      match: async (key) => entries.get(String(key))?.clone(),
      put: async (key, response) => {
        if (this.quotaFailure) throw new DOMException("quota full", "QuotaExceededError");
        entries.set(String(key), response.clone());
      },
    };
  }
}
