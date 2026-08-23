import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  LIMITS,
  RangeTransport,
  StreamingFailure,
  cacheEntryUrl,
  cacheNamespace,
  decodeRootSamples,
  readBoundedBody,
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
  assert.equal(deployment.displayMapping, "rgb16_full_range_rounded_rgba8_v1");
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
  assert.equal(result.metrics.sourceRequestedBytes, 256);
  assert.equal(result.metrics.sourceReceivedBytes, 256);
  assert.equal(result.metrics.indexRequestedBytes, 172_440);
  assert.equal(result.metrics.indexReceivedBytes, 172_440);
  assert.equal(result.metrics.requestedBytes, 172_696);
  assert.equal(result.metrics.receivedBytes, 172_696);
  assert.ok(result.metrics.sourceNetworkBytes < result.deployment.source.byteLength);
  assert.equal(result.metrics.transferredBatches, 4);
  assert.equal(result.metrics.transferredPoints, 4_096);
  assert.equal(result.metrics.transferredBytes, 98_304);
  assert.equal(result.metrics.decodedStagingBytesHighWater, 196_608);
  assert.equal(result.decode.intensityMinimum, 22);
  assert.equal(result.decode.intensityMaximum, 65_519);
  assert.equal(result.decode.classificationMinimum, 2);
  assert.equal(result.decode.classificationMaximum, 2);
  assert.equal(batches.length, 4);
  assert.ok(strictlyIncreasingOrdinals(batches));
  assert.equal(server.binaryRequests.length, 3);
  assert.deepEqual(server.binaryRequests.map((request) => request.range), [
    "bytes=0-255",
    "bytes=0-407",
    "bytes=744-172775",
  ]);
});

test("worker RGB mapping matches the exact repository U16 conversion", async () => {
  const deployment = validateManifest(manifest, MANIFEST_URL);
  const range = deployment.index.root.sampleRange;
  const sampleBytes = indexBytes.subarray(range.offset, range.offset + range.length);
  const batches = [];

  await decodeRootSamples(sampleBytes, deployment, (buffer) => batches.push(buffer));

  let sampleIndex = 0;
  for (const buffer of batches) {
    const transferred = new DataView(buffer);
    for (let offset = 0; offset < buffer.byteLength; offset += 24) {
      for (let channel = 0; channel < 3; channel += 1) {
        const value = indexBytes.readUInt16LE(
          range.offset + sampleIndex * 42 + 36 + channel * 2,
        );
        const expected = Math.floor((value * 255 + 32_767) / 65_535);
        assert.equal(transferred.getUint8(offset + 20 + channel), expected);
      }
      sampleIndex += 1;
    }
  }
  assert.equal(sampleIndex, deployment.index.root.displayPointCount);
});

test("queued-range bytes accept the exact ceiling and reject one over", () => {
  const deployment = structuredClone(validateManifest(manifest, MANIFEST_URL));
  deployment.index.headerAndRoot.length = LIMITS.rangeBytes;
  deployment.index.root.sampleRange.length = LIMITS.rangeBytes;
  const options = {
    deployment,
    cacheMode: "none",
    credentials: "omit",
    fetchImplementation: () => {},
  };

  const exact = new RangeTransport(options);
  assert.equal(exact.snapshot().queuedRangeBytesHighWater, LIMITS.queuedRangeBytes);

  deployment.index.root.sampleRange.length += 1;
  assert.throws(
    () => new RangeTransport(options),
    (error) => error instanceof StreamingFailure && error.code === "resource_limit",
  );
});

test("decode staging rejects the first record-width step over its ceiling before allocation", async () => {
  const acceptedCount = 7_216;
  const acceptedBytes = sampleRecords(acceptedCount);
  const acceptedDeployment = decodeBoundaryDeployment(acceptedCount);
  const accepted = await decodeRootSamples(acceptedBytes, acceptedDeployment);
  assert.equal(accepted.stagingBytesHighWater, LIMITS.workerStagingBytes - 32);
  assert.equal(accepted.batchCount, LIMITS.transferBatches);

  const rejectedCount = acceptedCount + 1;
  const rejectedBytes = new Uint8Array(rejectedCount * 42);
  await assert.rejects(
    decodeRootSamples(rejectedBytes, decodeBoundaryDeployment(rejectedCount)),
    (error) => error instanceof StreamingFailure && error.code === "resource_limit",
  );
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
  const entries = cacheStorage.namespaces.get(cacheNamespace(deployment));
  const rangeEntries = Array.from(entries).filter(([key]) =>
    new URL(key).searchParams.has("__punctra_kind")
  );
  assert.equal(entries.size, 4);
  assert.equal(rangeEntries.length, 3);
  for (const [key, response] of rangeEntries) {
    const entry = new URL(key);
    const kind = entry.searchParams.get("__punctra_kind");
    assert.ok(kind === "source" || kind === "index");
    assert.equal(entry.searchParams.get("__punctra_schema"), deployment.schema);
    assert.equal(entry.searchParams.get("__punctra_deployment"), deployment.deploymentId);
    assert.equal(entry.searchParams.get("__punctra_source"), deployment.source.sourceIdentity);
    assert.equal(entry.searchParams.get("__punctra_source_validator"), deployment.source.etag);
    assert.equal(entry.searchParams.get("__punctra_index"), deployment.index.sha256);
    assert.equal(entry.searchParams.get("__punctra_cache_layout"), "bounded-ledger-v1");
    assert.equal(response.headers.get("x-punctra-cache-layout"), "bounded-ledger-v1");
    assert.equal(response.headers.get("x-punctra-schema"), deployment.schema);
    assert.equal(response.headers.get("x-punctra-deployment"), deployment.deploymentId);
    assert.equal(response.headers.get("x-punctra-source"), deployment.source.sourceIdentity);
    assert.equal(response.headers.get("x-punctra-source-validator"), deployment.source.etag);
    assert.equal(response.headers.get("x-punctra-index"), deployment.index.sha256);
    assert.equal(response.headers.get("x-punctra-kind"), kind);
    assert.equal(response.headers.get("x-punctra-range"), entry.searchParams.get("__punctra_range"));
    assert.match(response.headers.get("x-punctra-digest"), /^[0-9a-f]{64}$/);
  }
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

test("persistent cache ceiling includes compatible ranges from prior operations", async () => {
  const cacheStorage = new TestCacheStorage();
  for (let offset = 0; offset < 15; offset += 1) {
    const rangedManifest = manifestWithSourceProbe(offset, 256 * 1024);
    await run(fixtureServer({ manifestOverride: rangedManifest }), {}, {
      cacheMode: "persistent",
      cacheStorage,
    });
  }

  const overLimitManifest = manifestWithSourceProbe(15, 256 * 1024);
  await rejectsWithCode(
    run(fixtureServer({ manifestOverride: overLimitManifest }), {}, {
      cacheMode: "persistent",
      cacheStorage,
    }),
    "resource_limit",
  );
});

test("an orphaned persistent range invalidates its unaccounted namespace", async () => {
  const cacheStorage = new TestCacheStorage();
  const server = fixtureServer();
  await run(server, {}, { cacheMode: "persistent", cacheStorage });
  const requestsAfterCold = server.binaryRequests.length;
  const deployment = validateManifest(manifest, MANIFEST_URL);
  const entries = cacheStorage.namespaces.get(cacheNamespace(deployment));
  const ledgerKey = Array.from(entries.keys()).find((key) =>
    new URL(key).searchParams.has("__punctra_cache_ledger")
  );
  assert.ok(ledgerKey);
  entries.delete(ledgerKey);

  const recovered = await run(server, {}, { cacheMode: "persistent", cacheStorage });

  assert.equal(recovered.metrics.cacheHits, 0);
  assert.equal(recovered.metrics.requestCount, 3);
  assert.equal(server.binaryRequests.length, requestsAfterCold + 3);
  assert.equal(
    cacheStorage.namespaces.get(cacheNamespace(deployment)).size,
    4,
  );
});

test("cache entry metadata accepts the exact ceiling and rejects one over", async (context) => {
  for (const cacheMode of ["memory", "persistent"]) {
    await context.test(cacheMode, async () => {
      const deployment = validateManifest(manifest, MANIFEST_URL);
      const server = fixtureServer();
      const transport = new RangeTransport({
        deployment,
        cacheMode,
        credentials: "omit",
        fetchImplementation: server.fetch,
        cacheStorage: new TestCacheStorage(),
        memoryCacheStorage: new Map(),
      });
      await transport.initialize();
      for (let offset = 0; offset < LIMITS.cacheEntries; offset += 1) {
        await transport.fetchRange("source", sourceRange(offset, 1), true);
      }

      assert.equal(transport.snapshot().logicalCacheEntries, LIMITS.cacheEntries);
      await rejectsWithCode(
        transport.fetchRange("source", sourceRange(LIMITS.cacheEntries, 1), true),
        "resource_limit",
      );
    });
  }
});

test("memory cache survives sequential operations in one worker and invalidates exactly", async () => {
  const memoryCacheStorage = new Map();
  const server = fixtureServer();
  const cold = await run(server, {}, { cacheMode: "memory", memoryCacheStorage });
  const requestsAfterCold = server.binaryRequests.length;
  const warm = await run(server, {}, { cacheMode: "memory", memoryCacheStorage });

  assert.equal(cold.metrics.requestCount, 3);
  assert.equal(warm.metrics.requestCount, 0);
  assert.equal(warm.metrics.cacheHits, 3);
  assert.equal(server.binaryRequests.length, requestsAfterCold);

  const invalidated = await run(server, {}, {
    cacheMode: "memory",
    memoryCacheStorage,
    invalidate: true,
  });
  assert.equal(invalidated.metrics.requestCount, 3);
  assert.equal(server.binaryRequests.length, requestsAfterCold + 3);
});

test("full response to a Range request fails before its body is read", async () => {
  const server = fixtureServer({ scenario: "full_response" });
  await rejectsWithCode(run(server), "range_unsupported");
  assert.equal(server.bodyReads, 0);
});

test("redirects and non-retryable HTTP statuses are terminal Range failures", async () => {
  const redirected = fixtureServer({ scenario: "redirect" });
  await rejectsWithCode(run(redirected), "range_unsupported");
  assert.equal(redirected.attempts, 2);
  assert.equal(redirected.binaryRequests[0].redirect, "manual");

  const missing = fixtureServer({ scenario: "not_found" });
  await rejectsWithCode(run(missing), "range_unsupported");
  assert.equal(missing.attempts, 2);
});

test("validator drift has a distinct changed-Source outcome", async () => {
  await rejectsWithCode(run(fixtureServer({ scenario: "validator_drift" })), "source_changed");
});

test("truncation, oversized bodies, and digest corruption remain distinct", async () => {
  await rejectsWithCode(run(fixtureServer({ scenario: "truncated" })), "range_truncated");
  await rejectsWithCode(run(fixtureServer({ scenario: "oversized" })), "resource_limit");
  await rejectsWithCode(run(fixtureServer({ scenario: "corrupt" })), "range_corrupt");
});

test("bounded body reading copies chunks into one fixed-capacity buffer", async () => {
  const response = new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(Uint8Array.of(1, 2));
      controller.enqueue(Uint8Array.of(3));
      controller.close();
    },
  }));

  const bytes = await readBoundedBody(response, 8, "test response");

  assert.deepEqual([...bytes], [1, 2, 3]);
  assert.equal(bytes.byteLength, 3);
  assert.equal(bytes.buffer.byteLength, 8);
});

test("retryable server failures stop at the declared bounded retry count", async () => {
  const recovers = fixtureServer({ scenario: "retry_twice" });
  const result = await run(recovers, {}, { delay: async () => {} });
  assert.equal(result.metrics.retries, 2);
  assert.equal(result.metrics.sourceRequestedBytes, 768);
  assert.equal(result.metrics.sourceReceivedBytes, 256);
  assert.equal(result.metrics.indexRequestedBytes, 172_440);
  assert.equal(result.metrics.indexReceivedBytes, 172_440);
  assert.equal(result.metrics.requestedBytes, 173_208);
  assert.equal(result.metrics.receivedBytes, 172_696);
  assert.equal(recovers.attempts, 6);

  const exhausted = fixtureServer({ scenario: "retry_forever" });
  await rejectsWithCode(run(exhausted, {}, { delay: async () => {} }), "retry_exhausted");
  assert.equal(exhausted.attempts, 4);
});

test("retryable responses are cancelled before the next Range attempt", async () => {
  const server = fixtureServer({ scenario: "retry_open_body" });

  await run(server, {}, { delay: async () => {} });

  assert.equal(server.retryBodyCancellations, 1);
  assert.equal(server.overlappingRetry, false);
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

test("manifest network failures preserve the offline recovery outcome", async () => {
  const server = fixtureServer({ scenario: "manifest_offline" });

  await rejectsWithCode(run(server), "offline");

  assert.equal(server.attempts, 1);
  assert.equal(server.binaryRequests.length, 0);
});

test("warm cache reads acknowledge cancellation instead of completing", async () => {
  const memoryCacheStorage = new Map();
  const server = fixtureServer();
  await run(server, {}, { cacheMode: "memory", memoryCacheStorage });
  const binaryRequestsAfterCold = server.binaryRequests.length;
  const controller = new AbortController();

  await rejectsWithCode(
    run(
      server,
      {
        onState: (phase) => {
          if (phase === "probing_source") controller.abort();
        },
      },
      {
        cacheMode: "memory",
        memoryCacheStorage,
        signal: controller.signal,
      },
    ),
    "cancelled",
  );
  assert.equal(server.binaryRequests.length, binaryRequestsAfterCold);
});

test("decode yields between batches so cancellation cannot race completion", async () => {
  const controller = new AbortController();
  let publishedBatches = 0;

  await rejectsWithCode(
    run(
      fixtureServer(),
      {
        onBatch: () => {
          publishedBatches += 1;
          setTimeout(() => controller.abort(), 0);
        },
      },
      { signal: controller.signal },
    ),
    "cancelled",
  );
  assert.equal(publishedBatches, 1);
});

test("stalled persistent cache work cannot block cancellation acknowledgement", async () => {
  const cacheStorage = new TestCacheStorage({ stalledPut: 1 });
  const controller = new AbortController();
  const operation = run(fixtureServer(), {}, {
    cacheMode: "persistent",
    cacheStorage,
    signal: controller.signal,
  });
  await cacheStorage.putStalled;
  const started = performance.now();
  controller.abort();

  await completesWithin(
    rejectsWithCode(operation, "cancelled"),
    LIMITS.cancellationMilliseconds,
  );
  assert.ok(performance.now() - started < LIMITS.cancellationMilliseconds);
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
    overlappingRetry: false,
    retryBodyCancellations: 0,
    retryBodyOpen: false,
  };
  const servedManifest = options.manifestOverride ?? manifest;
  state.fetch = async (input, init = {}) => {
    state.attempts += 1;
    const url = String(input);
    if (url === MANIFEST_URL) {
      if (options.scenario === "manifest_offline") throw new TypeError("offline");
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
    if (options.scenario === "retry_open_body" && state.attempts === 2) {
      state.retryBodyOpen = true;
      return new Response(new ReadableStream({
        cancel() {
          state.retryBodyOpen = false;
          state.retryBodyCancellations += 1;
        },
      }), { status: 503 });
    }
    if (options.scenario === "retry_open_body" && state.retryBodyOpen) {
      state.overlappingRetry = true;
    }
    const descriptor = binaryDescriptor(url, servedManifest);
    const range = new Headers(init.headers).get("Range");
    state.binaryRequests.push({ url, range, redirect: init.redirect });
    const [start, end] = parseRange(range);
    const original = descriptor.bytes.subarray(start, end + 1);
    if (options.scenario === "redirect") {
      return new Response(null, {
        status: 302,
        headers: { Location: "https://fixtures.test/redirected" },
      });
    }
    if (options.scenario === "not_found") return new Response(null, { status: 404 });
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

function manifestWithSourceProbe(offset, length) {
  const rangedManifest = structuredClone(manifest);
  const bytes = sourceBytes.subarray(offset, offset + length);
  rangedManifest.source.probe = {
    offset,
    length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
  return rangedManifest;
}

function sourceRange(offset, length) {
  const bytes = sourceBytes.subarray(offset, offset + length);
  return Object.freeze({
    offset,
    length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  });
}

function decodeBoundaryDeployment(pointCount) {
  return {
    index: {
      transform: { offset: [0, 0, 0], scale: [1, 1, 1] },
      root: {
        displayPointCount: pointCount,
        sampleRange: { length: pointCount * 42 },
        bounds: { minimum: [0, 0, 0], maximum: [pointCount, 0, 0] },
        worldOrigin: [0, 0, 0],
      },
    },
  };
}

function sampleRecords(pointCount) {
  const bytes = new Uint8Array(pointCount * 42);
  const view = new DataView(bytes.buffer);
  for (let ordinal = 0; ordinal < pointCount; ordinal += 1) {
    const offset = ordinal * 42;
    view.setBigUint64(offset, BigInt(ordinal), true);
    view.setBigInt64(offset + 8, BigInt(ordinal), true);
  }
  return bytes;
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

async function completesWithin(promise, milliseconds) {
  let timeout;
  try {
    await Promise.race([
      promise,
      new Promise((_, reject) => {
        timeout = setTimeout(
          () => reject(new Error(`operation exceeded ${milliseconds} milliseconds`)),
          milliseconds,
        );
      }),
    ]);
  } finally {
    clearTimeout(timeout);
  }
}

class TestCacheStorage {
  constructor(options = {}) {
    this.namespaces = new Map();
    this.quotaFailure = options.quotaFailure === true;
    this.stalledPut = options.stalledPut;
    this.putCount = 0;
    this.putStalled = new Promise((resolve) => {
      this.markPutStalled = resolve;
    });
  }

  async delete(name) {
    return this.namespaces.delete(name);
  }

  async open(name) {
    const entries = this.namespaces.get(name) ?? new Map();
    this.namespaces.set(name, entries);
    return {
      match: async (key) => entries.get(typeof key === "string" ? key : key.url)?.clone(),
      put: async (key, response) => {
        this.putCount += 1;
        if (this.putCount === this.stalledPut) {
          this.markPutStalled();
          return new Promise(() => {});
        }
        if (this.quotaFailure) throw new DOMException("quota full", "QuotaExceededError");
        entries.set(String(key), response.clone());
      },
    };
  }
}
