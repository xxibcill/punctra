export const STREAM_SCHEMA = "punctra-browser-stream-v1";
export const WORKER_SCHEMA = "punctra-browser-worker-v1";
export const WORKER_OUTPUT_TYPES = Object.freeze([
  "state",
  "batch",
  "complete",
  "failure",
]);

export const LIMITS = Object.freeze({
  manifestBytes: 32 * 1024,
  activeOperations: 1,
  concurrentRequests: 1,
  queuedRanges: 2,
  queuedRangeBytes: 512 * 1024,
  rangeBytes: 256 * 1024,
  concurrentResponseBytes: 256 * 1024,
  workerStagingBytes: 320 * 1024,
  transferBatchPoints: 1024,
  transferRecordBytes: 24,
  transferBatches: 8,
  streamPoints: 8192,
  cacheEntries: 64,
  memoryCacheBytes: 512 * 1024,
  persistentCacheBytes: 4 * 1024 * 1024,
  cancellationMilliseconds: 1000,
  retries: 2,
  retryDelayMilliseconds: 25,
});

const RETRYABLE_STATUS = new Set([408, 429, 500, 502, 503, 504]);
const CACHE_MODES = new Set(["none", "memory", "persistent"]);
const CREDENTIAL_MODES = new Set(["omit", "same-origin", "include"]);
const INDEX_HEADER_BYTES = 240;
const NODE_RECORD_BYTES = 168;
const SAMPLE_RECORD_BYTES = 42;
const CACHE_LAYOUT = "bounded-ledger-v1";
const CACHE_LEDGER_ENTRIES_HEADER = "X-Punctra-Cache-Entries";
const CACHE_LEDGER_BYTES_HEADER = "X-Punctra-Cache-Bytes";
const CACHE_IDENTITY_FIELDS = Object.freeze([
  Object.freeze({ name: "layout", header: "X-Punctra-Cache-Layout", query: "__punctra_cache_layout", namespace: true }),
  Object.freeze({ name: "schema", header: "X-Punctra-Schema", query: "__punctra_schema", namespace: true }),
  Object.freeze({ name: "deployment", header: "X-Punctra-Deployment", query: "__punctra_deployment", namespace: true }),
  Object.freeze({ name: "source", header: "X-Punctra-Source", query: "__punctra_source", namespace: true }),
  Object.freeze({ name: "sourceValidator", header: "X-Punctra-Source-Validator", query: "__punctra_source_validator", namespace: true }),
  Object.freeze({ name: "index", header: "X-Punctra-Index", query: "__punctra_index", namespace: true }),
  Object.freeze({ name: "kind", header: "X-Punctra-Kind", query: "__punctra_kind" }),
  Object.freeze({ name: "resourceValidator", header: "X-Punctra-Resource-Validator", query: "__punctra_resource_validator" }),
  Object.freeze({ name: "byteRange", header: "X-Punctra-Range", query: "__punctra_range" }),
  Object.freeze({ name: "digest", header: "X-Punctra-Digest" }),
]);

const SAFE_ACTIONS = Object.freeze({
  manifest_invalid: "Repair or select a compatible deployment manifest before retrying.",
  unsupported_deployment: "Build and publish the accepted disk-v2 inspection index deployment before retrying.",
  range_unsupported: "Configure the host to return exact 206 byte-range responses before retrying.",
  cors_headers_hidden: "Expose Content-Length, Content-Range, ETag, Accept-Ranges, and Content-Encoding through CORS.",
  content_encoding: "Serve the immutable binary representation without content encoding or transformation.",
  source_changed: "Discard the old binding and publish a new manifest after rebuilding its compatible index.",
  range_truncated: "Repair the hosted representation, then start a new operation.",
  range_corrupt: "Discard the corrupt response or deployment and start a new operation after repair.",
  index_incompatible: "Publish the exact compatible disk-v2 Punctra index before retrying.",
  offline: "Wait for connectivity to return, then start a new operation.",
  retry_exhausted: "Wait for server recovery, then start a new operation.",
  cache_quota: "Free origin storage or explicitly retry with memory or no cache.",
  cache_unavailable: "Explicitly retry with memory or no cache.",
  cancelled: "Start a new operation only if the caller still wants the progressive View.",
  worker_failed: "Terminate the worker, keep the current frame, and create a new worker before retrying.",
  resource_limit: "Select a deployment inside the fixed browser streaming ceilings.",
});

export class StreamingFailure extends Error {
  constructor(code, message, options = {}) {
    super(message, options);
    this.name = "StreamingFailure";
    this.code = code;
    this.safeAction = SAFE_ACTIONS[code] ?? SAFE_ACTIONS.manifest_invalid;
  }

  toJSON() {
    return {
      schema: WORKER_SCHEMA,
      type: "failure",
      code: this.code,
      message: this.message,
      safe_action: this.safeAction,
    };
  }
}

export function workerFailure(message) {
  return new StreamingFailure("worker_failed", message).toJSON();
}

export function createWorkerMessage(operationId, type, facts = {}) {
  require(
    WORKER_OUTPUT_TYPES.includes(type),
    "manifest_invalid",
    `unsupported worker output message ${type}`,
  );
  return {
    ...facts,
    schema: WORKER_SCHEMA,
    type,
    operation_id: operationId,
  };
}

export async function runStreamingOperation(configuration, hooks = {}) {
  const fetchImplementation =
    configuration.fetchImplementation ?? globalThis.fetch?.bind(globalThis);
  const manifest = await loadManifest(
    configuration.manifestUrl,
    fetchImplementation,
    configuration.signal,
  );
  const deployment = validateManifest(manifest, configuration.manifestUrl);
  hooks.onDeployment?.(deployment);
  assertNotCancelled(configuration.signal);
  const transport = new RangeTransport({
    deployment,
    cacheMode: configuration.cacheMode,
    invalidate: configuration.invalidate,
    credentials: configuration.credentials,
    fetchImplementation,
    cacheStorage: configuration.cacheStorage ?? globalThis.caches,
    memoryCacheStorage: configuration.memoryCacheStorage,
    signal: configuration.signal,
    delay: configuration.delay,
  });
  await transport.initialize();
  assertNotCancelled(configuration.signal);
  hooks.onState?.("probing_source", transport.snapshot());
  await transport.fetchRange("source", deployment.source.probe, true);
  hooks.onState?.("validating_index", transport.snapshot());
  const header = await transport.fetchRange(
    "index",
    deployment.index.headerAndRoot,
    false,
  );
  validateIndexHeaderAndRoot(header, deployment);
  hooks.onState?.("decoding_samples", transport.snapshot());
  const samples = await transport.fetchRange(
    "index",
    deployment.index.root.sampleRange,
    false,
  );
  const decodeFacts = await decodeRootSamples(
    samples,
    deployment,
    hooks.onBatch,
    configuration.signal,
  );
  transport.recordDecode(decodeFacts);
  return {
    deployment,
    metrics: transport.snapshot(),
    decode: decodeFacts,
  };
}

export async function loadManifest(url, fetchImplementation, signal) {
  require(typeof fetchImplementation === "function", "manifest_invalid", "Fetch is unavailable");
  try {
    const response = await fetchImplementation(url, {
      method: "GET",
      cache: "no-store",
      redirect: "error",
      signal,
    });
    if (!response.ok || response.status !== 200) {
      throw new StreamingFailure("manifest_invalid", `manifest returned HTTP ${response.status}`);
    }
    const bytes = await readBoundedBody(
      response,
      LIMITS.manifestBytes,
      "manifest",
      "manifest_invalid",
      signal,
    );
    return JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch (error) {
    throw classifyThrown(error, "manifest_invalid");
  }
}

export function validateManifest(value, manifestUrl) {
  try {
    requireObject(value, "manifest");
    equal(value.schema, STREAM_SCHEMA, "unsupported_deployment", "manifest schema");
    boundedString(value.deployment_id, 1, 128, "deployment_id");
    equal(
      value.display_mapping,
      "rgb16_full_range_rounded_rgba8_v1",
      "unsupported_deployment",
      "display mapping",
    );
    const source = validateSource(value.source, manifestUrl);
    const index = validateIndex(value.index, manifestUrl);
    equal(index.sourceIdentity, source.sourceIdentity, "manifest_invalid", "Source identity binding");
    equal(index.sourcePointCount, source.pointCount, "manifest_invalid", "Source Point count binding");
    equal(index.root.coveredPointCount, source.pointCount, "manifest_invalid", "root Source coverage");
    return Object.freeze({
      schema: value.schema,
      deploymentId: value.deployment_id,
      displayMapping: value.display_mapping,
      source,
      index,
    });
  } catch (error) {
    throw classifyThrown(error, "manifest_invalid");
  }
}

function validateSource(value, manifestUrl) {
  requireObject(value, "source");
  equal(value.media_type, "application/vnd.las", "unsupported_deployment", "Source media type");
  const source = {
    url: resolvedHttpUrl(value.url, manifestUrl, "Source URL"),
    byteLength: positiveSafeInteger(value.byte_length, "Source byte length"),
    etag: strongEtag(value.strong_etag, "Source ETag"),
    sha256: digest(value.sha256, "Source SHA-256"),
    sourceIdentity: digest(value.source_identity, "Source identity"),
    pointCount: positiveSafeInteger(value.point_count, "Source Point count"),
    probe: validateRange(value.probe, "Source probe"),
  };
  validateRangeBounds(source.probe, source.byteLength, "Source probe");
  return Object.freeze(source);
}

function validateIndex(value, manifestUrl) {
  requireObject(value, "index");
  equal(value.disk_version, 2, "unsupported_deployment", "index disk version");
  equal(value.recipe_version, 2, "unsupported_deployment", "index recipe version");
  equal(value.display_sample_schema, 1, "unsupported_deployment", "display sample schema");
  const index = {
    url: resolvedHttpUrl(value.url, manifestUrl, "index URL"),
    byteLength: positiveSafeInteger(value.byte_length, "index byte length"),
    etag: strongEtag(value.strong_etag, "index ETag"),
    sha256: digest(value.sha256, "index SHA-256"),
    sourceIdentity: digest(value.source_identity, "index Source identity"),
    sourcePointCount: positiveSafeInteger(value.source_point_count, "index Source Point count"),
    transform: validateTransform(value.position_transform),
    headerAndRoot: validateRange(value.header_and_root, "index header/root"),
    root: validateRoot(value.root),
  };
  equal(index.headerAndRoot.offset, 0, "manifest_invalid", "index header offset");
  equal(index.headerAndRoot.length, INDEX_HEADER_BYTES + NODE_RECORD_BYTES, "manifest_invalid", "index header/root length");
  validateRangeBounds(index.headerAndRoot, index.byteLength, "index header/root");
  validateRangeBounds(index.root.sampleRange, index.byteLength, "root sample range");
  return Object.freeze(index);
}

function validateTransform(value) {
  requireObject(value, "position transform");
  return Object.freeze({
    offset: finiteTriple(value.offset, "position offset"),
    scale: positiveFiniteTriple(value.scale, "position scale"),
  });
}

function validateRoot(value) {
  requireObject(value, "root");
  equal(value.node_id, 1, "manifest_invalid", "root node identity");
  equal(value.coverage, "sampled", "unsupported_deployment", "root Coverage");
  const displayPointCount = positiveSafeInteger(value.display_point_count, "root display Point count");
  require(displayPointCount <= LIMITS.streamPoints, "resource_limit", "root display Point count exceeds 8,192");
  const sampleRange = validateRange(value.sample_range, "root sample range");
  equal(value.sample_range.record_bytes, SAMPLE_RECORD_BYTES, "unsupported_deployment", "sample record width");
  equal(value.sample_range.count, displayPointCount, "manifest_invalid", "sample count");
  equal(sampleRange.length, displayPointCount * SAMPLE_RECORD_BYTES, "manifest_invalid", "sample range length");
  require(sampleRange.length <= LIMITS.rangeBytes, "resource_limit", "root sample range exceeds 256 KiB");
  requireObject(value.bounds, "root bounds");
  const minimum = finiteTriple(value.bounds.min, "root minimum bounds");
  const maximum = finiteTriple(value.bounds.max, "root maximum bounds");
  require(minimum.every((entry, axis) => entry <= maximum[axis]), "manifest_invalid", "root bounds are inverted");
  return Object.freeze({
    nodeId: 1,
    coverage: "sampled",
    coveredPointCount: positiveSafeInteger(value.covered_point_count, "root covered Point count"),
    displayPointCount,
    bounds: Object.freeze({ minimum, maximum }),
    worldOrigin: finiteTriple(value.world_origin, "root world origin"),
    diskChecksum: digest(value.sample_range.disk_checksum_blake3, "root disk BLAKE3 checksum"),
    sampleRange,
  });
}

function validateRange(value, label) {
  requireObject(value, label);
  const range = {
    offset: nonnegativeSafeInteger(value.offset, `${label} offset`),
    length: positiveSafeInteger(value.length, `${label} length`),
    sha256: digest(value.sha256, `${label} SHA-256`),
  };
  require(range.length <= LIMITS.rangeBytes, "resource_limit", `${label} exceeds 256 KiB`);
  return Object.freeze(range);
}

function validateRangeBounds(range, total, label) {
  require(range.offset + range.length <= total, "manifest_invalid", `${label} exceeds its representation`);
}

export class RangeTransport {
  constructor(options) {
    this.deployment = options.deployment;
    this.cacheMode = cacheMode(options.cacheMode);
    this.invalidate = options.invalidate === true;
    this.credentials = credentialMode(options.credentials);
    this.fetchImplementation = options.fetchImplementation;
    this.cacheStorage = options.cacheStorage;
    this.memoryCacheStorage = options.memoryCacheStorage ?? new Map();
    this.signal = options.signal;
    this.delay = options.delay ?? defaultDelay;
    this.memory = undefined;
    this.persistent = undefined;
    this.cachedEntries = 0;
    this.cachedBytes = 0;
    this.metrics = freshMetrics();
    this.metrics.queuedRangeBytesHighWater =
      this.deployment.index.headerAndRoot.length +
      this.deployment.index.root.sampleRange.length;
    require(
      this.metrics.queuedRangeBytesHighWater <= LIMITS.queuedRangeBytes,
      "resource_limit",
      "queued index ranges exceed 512 KiB",
    );
  }

  async initialize() {
    const namespace = cacheNamespace(this.deployment);
    if (this.cacheMode === "memory") {
      if (this.invalidate) this.memoryCacheStorage.delete(namespace);
      this.memory = this.memoryCacheStorage.get(namespace) ?? new Map();
      this.memoryCacheStorage.set(namespace, this.memory);
      for (const value of this.memory.values()) {
        this.recordExistingCacheEntry(value.byteLength, LIMITS.memoryCacheBytes);
      }
      this.recordLogicalCacheSize();
      return;
    }
    if (this.cacheMode !== "persistent") return;
    require(this.cacheStorage && typeof this.cacheStorage.open === "function", "cache_unavailable", "Cache API is unavailable");
    try {
      if (this.invalidate) await this.cacheStorage.delete(namespace);
      this.persistent = await this.cacheStorage.open(namespace);
    } catch (error) {
      throw cacheFailure(error);
    }
    const ledger = await this.readPersistentCacheLedger();
    this.cachedEntries = ledger.entries;
    this.cachedBytes = ledger.bytes;
    this.recordLogicalCacheSize();
  }

  recordExistingCacheEntry(bytes, byteCeiling) {
    const nextEntries = this.cachedEntries + 1;
    const nextBytes = this.cachedBytes + bytes;
    require(
      nextEntries <= LIMITS.cacheEntries,
      "resource_limit",
      `logical ${this.cacheMode} cache exceeds ${LIMITS.cacheEntries} entries`,
    );
    require(
      nextBytes <= byteCeiling,
      "resource_limit",
      `logical ${this.cacheMode} cache exceeds ${byteCeiling} bytes`,
    );
    this.cachedEntries = nextEntries;
    this.cachedBytes = nextBytes;
  }

  async readPersistentCacheLedger() {
    try {
      const response = await this.persistent.match(cacheLedgerUrl(this.deployment));
      if (!response) {
        await this.writePersistentCacheLedger(0, 0);
        return { entries: 0, bytes: 0 };
      }
      return validateCacheLedger(response, this.deployment);
    } catch (error) {
      if (error instanceof StreamingFailure) throw error;
      throw cacheFailure(error);
    }
  }

  async writePersistentCacheLedger(entries, bytes) {
    await this.persistent.put(
      cacheLedgerUrl(this.deployment),
      cacheLedgerResponse(this.deployment, entries, bytes),
    );
  }

  async fetchRange(kind, range, requireAcceptRanges) {
    assertNotCancelled(this.signal);
    const resource = kind === "source" ? this.deployment.source : this.deployment.index;
    const key = cacheEntryUrl(this.deployment, kind, range);
    const cached = await this.readCache(key, kind, resource, range);
    if (cached) {
      this.recordCacheHit(kind, cached.byteLength);
      return cached;
    }
    const bytes = await this.fetchNetwork(kind, resource, range, requireAcceptRanges);
    await this.writeCache(key, kind, resource, range, bytes);
    assertNotCancelled(this.signal);
    this.recordNetwork(kind, bytes.byteLength);
    return bytes;
  }

  async fetchNetwork(kind, resource, range, requireAcceptRanges) {
    const start = range.offset;
    const end = start + range.length - 1;
    for (let attempt = 0; attempt <= LIMITS.retries; attempt += 1) {
      assertNotCancelled(this.signal);
      try {
        this.recordRequest(kind, range.length);
        const response = await this.fetchImplementation(resource.url, {
          method: "GET",
          headers: { Range: `bytes=${start}-${end}` },
          credentials: this.credentials,
          cache: "no-store",
          redirect: "manual",
          signal: this.signal,
        });
        if (RETRYABLE_STATUS.has(response.status)) {
          if (attempt === LIMITS.retries) throw new StreamingFailure("retry_exhausted", `HTTP ${response.status} persisted after ${attempt + 1} attempts`);
          await this.retry(attempt);
          continue;
        }
        validateRangeResponse(response, resource, range, requireAcceptRanges);
        const bytes = await readBoundedBody(
          response,
          range.length,
          "Range response",
          "range_truncated",
          this.signal,
        );
        this.recordResponse(kind, bytes.byteLength);
        if (bytes.byteLength !== range.length) throw new StreamingFailure("range_truncated", `received ${bytes.byteLength} bytes instead of ${range.length}`);
        await verifyDigest(bytes, range.sha256);
        return bytes;
      } catch (error) {
        const classified = classifyThrown(error, "offline");
        if (classified.code !== "offline" || attempt === LIMITS.retries) throw classified;
        await this.retry(attempt);
      }
    }
    throw new StreamingFailure("retry_exhausted", "bounded Range retries were exhausted");
  }

  async retry(attempt) {
    this.metrics.retries += 1;
    await this.delay(LIMITS.retryDelayMilliseconds * (attempt + 1), this.signal);
  }

  async readCache(key, kind, resource, range) {
    assertNotCancelled(this.signal);
    if (this.cacheMode === "none") return undefined;
    if (this.cacheMode === "memory") {
      const value = this.memory.get(key);
      if (!value) return undefined;
      await verifyDigest(value, range.sha256);
      assertNotCancelled(this.signal);
      return value.slice();
    }
    try {
      const response = await this.persistent.match(key);
      assertNotCancelled(this.signal);
      if (!response) return undefined;
      validateCachedMetadata(response, this.deployment, kind, resource, range);
      const bytes = await readBoundedBody(
        response,
        range.length,
        "cached range",
        "range_corrupt",
        this.signal,
      );
      if (bytes.byteLength !== range.length) throw new StreamingFailure("range_corrupt", "cached range length differs");
      await verifyDigest(bytes, range.sha256);
      assertNotCancelled(this.signal);
      return bytes;
    } catch (error) {
      if (error instanceof StreamingFailure) throw error;
      throw cacheFailure(error);
    }
  }

  async writeCache(key, kind, resource, range, bytes) {
    if (this.cacheMode === "none") return;
    const ceiling = this.cacheMode === "memory" ? LIMITS.memoryCacheBytes : LIMITS.persistentCacheBytes;
    const nextEntries = this.cachedEntries + 1;
    const nextBytes = this.cachedBytes + bytes.byteLength;
    require(nextEntries <= LIMITS.cacheEntries, "resource_limit", `logical ${this.cacheMode} cache exceeds ${LIMITS.cacheEntries} entries`);
    require(nextBytes <= ceiling, "resource_limit", `logical ${this.cacheMode} cache exceeds ${ceiling} bytes`);
    if (this.cacheMode === "memory") {
      this.memory.set(key, bytes.slice());
    } else {
      try {
        // Reserve first so an interrupted or quota-failed body write can only
        // conservatively overcount, never permit the namespace to exceed a ceiling.
        await this.writePersistentCacheLedger(nextEntries, nextBytes);
        await this.persistent.put(
          key,
          cachedResponse(this.deployment, kind, resource, range, bytes),
        );
      } catch (error) {
        throw cacheFailure(error);
      }
    }
    this.cachedEntries = nextEntries;
    this.cachedBytes = nextBytes;
    this.recordLogicalCacheSize();
  }

  recordLogicalCacheSize() {
    this.metrics.logicalCacheEntries = this.cachedEntries;
    this.metrics.logicalCacheBytes = this.cachedBytes;
  }

  recordCacheHit(kind, bytes) {
    this.metrics.cacheHits += 1;
    this.metrics.cacheBytes += bytes;
    this.metrics[`${kind}CacheBytes`] += bytes;
  }

  recordNetwork(kind, bytes) {
    this.metrics.requestCount += 1;
    this.metrics.networkBytes += bytes;
    this.metrics[`${kind}NetworkBytes`] += bytes;
    this.metrics.concurrentResponseBytesHighWater = Math.max(this.metrics.concurrentResponseBytesHighWater, bytes);
  }

  recordRequest(kind, bytes) {
    this.metrics.requestedBytes += bytes;
    this.metrics[`${kind}RequestedBytes`] += bytes;
  }

  recordResponse(kind, bytes) {
    this.metrics.receivedBytes += bytes;
    this.metrics[`${kind}ReceivedBytes`] += bytes;
  }

  recordDecode(facts) {
    this.metrics.decodedStagingBytesHighWater = facts.stagingBytesHighWater;
    this.metrics.transferredBatches = facts.batchCount;
    this.metrics.transferredPoints = facts.pointCount;
    this.metrics.transferredBytes = facts.transferredBytes;
  }

  snapshot() {
    return { ...this.metrics };
  }
}

export function validateIndexHeaderAndRoot(bytes, deployment) {
  require(bytes.byteLength === INDEX_HEADER_BYTES + NODE_RECORD_BYTES, "index_incompatible", "index header/root byte length differs");
  const view = dataView(bytes);
  equal(ascii(bytes, 0, 8), "PNIDX004", "index_incompatible", "index magic");
  equal(view.getUint32(8, true), 2, "index_incompatible", "index disk version");
  equal(view.getUint32(12, true), 2, "index_incompatible", "index recipe version");
  equal(hex(bytes.subarray(16, 48)), deployment.source.sourceIdentity, "index_incompatible", "index Source identity");
  equal(u64(view, 48), deployment.source.pointCount, "index_incompatible", "index Source Point count");
  compareF64Triple(view, 56, deployment.index.transform.offset, "index transform offset");
  compareF64Triple(view, 80, deployment.index.transform.scale, "index transform scale");
  equal(u64(view, 104), 1, "index_incompatible", "index bounds marker");
  compareF64Triple(view, 112, deployment.index.root.bounds.minimum, "index minimum bounds");
  compareF64Triple(view, 136, deployment.index.root.bounds.maximum, "index maximum bounds");
  const nodeCount = u64(view, 160);
  const leafCount = u64(view, 168);
  require(nodeCount >= 3 && leafCount >= 2, "index_incompatible", "sampled index node/leaf counts are incomplete");
  equal(u64(view, 176), INDEX_HEADER_BYTES, "index_incompatible", "index node table offset");
  equal(u64(view, 184), nodeCount * NODE_RECORD_BYTES, "index_incompatible", "index node table bytes");
  const sampleOffset = INDEX_HEADER_BYTES + nodeCount * NODE_RECORD_BYTES;
  equal(u64(view, 192), sampleOffset, "index_incompatible", "index sample offset");
  equal(sampleOffset, deployment.index.root.sampleRange.offset, "index_incompatible", "root sample layout");
  equal(
    u64(view, 200),
    deployment.index.byteLength - sampleOffset - 32,
    "index_incompatible",
    "index sample bytes",
  );
  equal(view.getUint32(208, true), 1, "index_incompatible", "display sample schema");
  equal(view.getUint32(212, true), 7, "index_incompatible", "display sample capabilities");
  [1, 6, 16, 17, 18].forEach((id, index) => {
    equal(view.getUint32(216 + index * 4, true), id, "index_incompatible", `display Attribute ${index}`);
  });
  equal(view.getUint32(236, true), SAMPLE_RECORD_BYTES, "index_incompatible", "display sample width");
  const root = INDEX_HEADER_BYTES;
  equal(u64(view, root), 1, "index_incompatible", "root identity");
  equal(u64(view, root + 8), 0, "index_incompatible", "root parent");
  const left = u64(view, root + 16);
  const right = u64(view, root + 24);
  require(
    left > 1 && left <= nodeCount && right > 1 && right <= nodeCount && left !== right,
    "index_incompatible",
    "root children are invalid",
  );
  compareF64Triple(view, root + 32, deployment.index.root.bounds.minimum, "root minimum bounds");
  compareF64Triple(view, root + 56, deployment.index.root.bounds.maximum, "root maximum bounds");
  equal(u64(view, root + 80), deployment.index.root.coveredPointCount, "index_incompatible", "root covered Point count");
  equal(u64(view, root + 88), deployment.index.root.displayPointCount, "index_incompatible", "root display Point count");
  equal(u64(view, root + 96), 0, "index_incompatible", "root first ordinal");
  equal(u64(view, root + 104), 0, "index_incompatible", "root Source span");
  equal(u64(view, root + 112), deployment.index.root.sampleRange.offset, "index_incompatible", "root sample offset");
  const geometricError = view.getFloat64(root + 120, true);
  require(Number.isFinite(geometricError) && geometricError >= 0, "index_incompatible", "root geometric error is invalid");
  equal(view.getUint8(root + 128), 0, "index_incompatible", "root Coverage");
  require(bytes.subarray(root + 129, root + 136).every((value) => value === 0), "index_incompatible", "root reserved bytes are nonzero");
  equal(hex(bytes.subarray(root + 136, root + 168)), deployment.index.root.diskChecksum, "index_incompatible", "root disk sample checksum");
}

export async function decodeRootSamples(
  bytes,
  deployment,
  onBatch = () => {},
  signal,
) {
  assertNotCancelled(signal);
  const root = deployment.index.root;
  equal(bytes.byteLength, root.sampleRange.length, "range_truncated", "root sample bytes");
  const outputBytesHighWater =
    Math.min(root.displayPointCount, LIMITS.transferBatchPoints)
    * LIMITS.transferRecordBytes;
  const stagingBytesHighWater = bytes.byteLength + outputBytesHighWater;
  require(
    stagingBytesHighWater <= LIMITS.workerStagingBytes,
    "resource_limit",
    "worker decode staging exceeds 320 KiB",
  );
  const view = dataView(bytes);
  let previousOrdinal = -1;
  let batchCount = 0;
  let transferredBytes = 0;
  let intensityMinimum = 65_535;
  let intensityMaximum = 0;
  let classificationMinimum = 255;
  let classificationMaximum = 0;
  for (let first = 0; first < root.displayPointCount; first += LIMITS.transferBatchPoints) {
    const count = Math.min(LIMITS.transferBatchPoints, root.displayPointCount - first);
    const output = new ArrayBuffer(count * LIMITS.transferRecordBytes);
    const encoded = new DataView(output);
    for (let row = 0; row < count; row += 1) {
      const decoded = decodeSample(view, (first + row) * SAMPLE_RECORD_BYTES, deployment, previousOrdinal);
      previousOrdinal = decoded.ordinal;
      intensityMinimum = Math.min(intensityMinimum, decoded.intensity);
      intensityMaximum = Math.max(intensityMaximum, decoded.intensity);
      classificationMinimum = Math.min(classificationMinimum, decoded.classification);
      classificationMaximum = Math.max(classificationMaximum, decoded.classification);
      encodeTransfer(encoded, row * LIMITS.transferRecordBytes, decoded);
    }
    batchCount += 1;
    transferredBytes += output.byteLength;
    require(batchCount <= LIMITS.transferBatches, "resource_limit", "transferred batch count exceeds eight");
    onBatch(output, { batchIndex: batchCount - 1, pointCount: count });
    await yieldForCancellation(signal);
    assertNotCancelled(signal);
  }
  return {
    batchCount,
    pointCount: root.displayPointCount,
    transferredBytes,
    stagingBytesHighWater,
    intensityMinimum,
    intensityMaximum,
    classificationMinimum,
    classificationMaximum,
  };
}

function decodeSample(view, offset, deployment, previousOrdinal) {
  const ordinal = u64(view, offset);
  require(ordinal > previousOrdinal, "range_corrupt", "root sample ordinals are not sorted and unique");
  const ticks = [i64(view, offset + 8), i64(view, offset + 16), i64(view, offset + 24)];
  const intensity = view.getUint16(offset + 32, true);
  const classification = view.getUint8(offset + 34);
  equal(view.getUint8(offset + 35), 0, "range_corrupt", "sample reserved byte");
  const world = ticks.map((tick, axis) => deployment.index.transform.offset[axis] + tick * deployment.index.transform.scale[axis]);
  require(world.every(Number.isFinite), "range_corrupt", "sample world position is not finite");
  require(withinBounds(world, deployment.index.root.bounds), "range_corrupt", "sample is outside root bounds");
  const relative = world.map((value, axis) => Math.fround(value - deployment.index.root.worldOrigin[axis]));
  require(relative.every(Number.isFinite), "range_corrupt", "sample relative position is not finite f32");
  const color = [
    rgb16ToRgb8(view.getUint16(offset + 36, true)),
    rgb16ToRgb8(view.getUint16(offset + 38, true)),
    rgb16ToRgb8(view.getUint16(offset + 40, true)),
    255,
  ];
  return { ordinal, relative, intensity, classification, color };
}

function rgb16ToRgb8(value) {
  return Math.floor((value * 255 + 32_767) / 65_535);
}

function encodeTransfer(view, offset, sample) {
  view.setBigUint64(offset, BigInt(sample.ordinal), true);
  sample.relative.forEach((value, axis) => view.setFloat32(offset + 8 + axis * 4, value, true));
  sample.color.forEach((value, channel) => view.setUint8(offset + 20 + channel, value));
}

function validateRangeResponse(response, resource, range, requireAcceptRanges) {
  if (
    response.type === "opaqueredirect"
    || response.redirected
    || (response.status >= 300 && response.status < 400)
  ) {
    throw new StreamingFailure("range_unsupported", "Range request was redirected");
  }
  if (response.status === 200) throw new StreamingFailure("range_unsupported", "server returned a full 200 response to a Range request");
  if (response.status !== 206) throw new StreamingFailure("range_unsupported", `Range request returned terminal HTTP ${response.status}`);
  const headers = response.headers;
  const required = ["content-length", "content-range", "etag"];
  if (required.some((name) => headers.get(name) === null)) throw new StreamingFailure("cors_headers_hidden", "required Range response headers are unavailable");
  equal(headers.get("etag"), resource.etag, "source_changed", "representation ETag");
  equal(headers.get("content-length"), String(range.length), "range_truncated", "Content-Length");
  const end = range.offset + range.length - 1;
  equal(headers.get("content-range"), `bytes ${range.offset}-${end}/${resource.byteLength}`, "range_truncated", "Content-Range");
  const encoding = headers.get("content-encoding");
  if (encoding !== null && encoding.toLowerCase() !== "identity") throw new StreamingFailure("content_encoding", `unexpected Content-Encoding ${encoding}`);
  if (requireAcceptRanges && headers.get("accept-ranges")?.toLowerCase() !== "bytes") throw new StreamingFailure("range_unsupported", "Source response does not declare Accept-Ranges: bytes");
}

async function verifyDigest(bytes, expected) {
  const subtle = globalThis.crypto?.subtle;
  require(subtle, "unsupported_deployment", "Web Crypto SHA-256 is unavailable");
  const actual = hex(new Uint8Array(await subtle.digest("SHA-256", bytes)));
  equal(actual, expected, "range_corrupt", "range SHA-256");
}

export async function readBoundedBody(
  response,
  maximumBytes,
  label,
  invalidLengthCode = "manifest_invalid",
  signal,
) {
  assertNotCancelled(signal);
  const declared = response.headers.get("content-length");
  if (declared !== null) {
    const length = Number(declared);
    require(
      Number.isSafeInteger(length) && length >= 0,
      invalidLengthCode,
      `${label} Content-Length is invalid`,
    );
    require(length <= maximumBytes, "resource_limit", `${label} exceeds ${maximumBytes} bytes`);
  }
  require(
    typeof response.body?.getReader === "function",
    "unsupported_deployment",
    `${label} requires a readable response stream`,
  );
  const reader = response.body.getReader();
  const bytes = new Uint8Array(maximumBytes);
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    assertNotCancelled(signal);
    if (done) break;
    const nextLength = length + value.byteLength;
    if (nextLength > maximumBytes) {
      await reader.cancel();
      throw new StreamingFailure("resource_limit", `${label} exceeds ${maximumBytes} bytes`);
    }
    bytes.set(value, length);
    length = nextLength;
  }
  return bytes.subarray(0, length);
}

function cachedResponse(deployment, kind, resource, range, bytes) {
  const identity = cacheIdentity(deployment, kind, range, resource);
  const headers = new Headers({ "Content-Length": String(bytes.byteLength) });
  for (const field of CACHE_IDENTITY_FIELDS) {
    headers.set(field.header, identity[field.name]);
  }
  return new Response(bytes.slice(), {
    status: 200,
    headers,
  });
}

function cacheLedgerResponse(deployment, entries, bytes) {
  const identity = cacheIdentity(deployment);
  const headers = new Headers({
    "Content-Length": "0",
    [CACHE_LEDGER_ENTRIES_HEADER]: String(entries),
    [CACHE_LEDGER_BYTES_HEADER]: String(bytes),
  });
  for (const field of CACHE_IDENTITY_FIELDS.filter((candidate) => candidate.namespace)) {
    headers.set(field.header, identity[field.name]);
  }
  return new Response(null, { status: 200, headers });
}

function validateCacheLedger(response, deployment) {
  const identity = cacheIdentity(deployment);
  for (const field of CACHE_IDENTITY_FIELDS.filter((candidate) => candidate.namespace)) {
    equal(
      response.headers.get(field.header),
      identity[field.name],
      "range_corrupt",
      `cache ledger ${field.name}`,
    );
  }
  equal(response.headers.get("content-length"), "0", "range_corrupt", "cache ledger Content-Length");
  return {
    entries: cacheLedgerInteger(response, CACHE_LEDGER_ENTRIES_HEADER, LIMITS.cacheEntries),
    bytes: cacheLedgerInteger(response, CACHE_LEDGER_BYTES_HEADER, LIMITS.persistentCacheBytes),
  };
}

function cacheLedgerInteger(response, header, ceiling) {
  const encoded = response.headers.get(header) ?? "";
  require(/^(0|[1-9]\d*)$/.test(encoded), "range_corrupt", `${header} is invalid`);
  const value = Number(encoded);
  require(Number.isSafeInteger(value), "range_corrupt", `${header} is not safely addressable`);
  require(value <= ceiling, "resource_limit", `${header} exceeds ${ceiling}`);
  return value;
}

function validateCachedMetadata(response, deployment, kind, resource, range) {
  const identity = cacheIdentity(deployment, kind, range, resource);
  for (const field of CACHE_IDENTITY_FIELDS) {
    equal(
      response.headers.get(field.header),
      identity[field.name],
      "range_corrupt",
      `cache ${field.name}`,
    );
  }
}

export function cacheNamespace(deployment) {
  const identity = cacheIdentity(deployment);
  return CACHE_IDENTITY_FIELDS
    .filter((field) => field.namespace)
    .map((field) => identity[field.name])
    .join(":");
}

function cacheLedgerUrl(deployment) {
  const identity = cacheIdentity(deployment);
  const url = new URL(deployment.source.url);
  for (const field of CACHE_IDENTITY_FIELDS.filter((candidate) => candidate.namespace)) {
    url.searchParams.set(field.query, identity[field.name]);
  }
  url.searchParams.set("__punctra_cache_ledger", CACHE_LAYOUT);
  return url.href;
}

export function cacheEntryUrl(deployment, kind, range) {
  const resource = kind === "source" ? deployment.source : deployment.index;
  const identity = cacheIdentity(deployment, kind, range, resource);
  const url = new URL(resource.url);
  for (const field of CACHE_IDENTITY_FIELDS) {
    if (field.query) url.searchParams.set(field.query, identity[field.name]);
  }
  return url.href;
}

function cacheIdentity(deployment, kind, range, resource) {
  return Object.freeze({
    layout: CACHE_LAYOUT,
    schema: STREAM_SCHEMA,
    deployment: deployment.deploymentId,
    source: deployment.source.sourceIdentity,
    sourceValidator: deployment.source.etag,
    index: deployment.index.sha256,
    kind,
    resourceValidator: resource?.etag,
    byteRange: range ? `${range.offset}:${range.length}` : undefined,
    digest: range?.sha256,
  });
}

function freshMetrics() {
  return {
    requestCount: 0,
    networkBytes: 0,
    sourceNetworkBytes: 0,
    indexNetworkBytes: 0,
    requestedBytes: 0,
    receivedBytes: 0,
    sourceRequestedBytes: 0,
    sourceReceivedBytes: 0,
    indexRequestedBytes: 0,
    indexReceivedBytes: 0,
    cacheHits: 0,
    cacheBytes: 0,
    sourceCacheBytes: 0,
    indexCacheBytes: 0,
    logicalCacheEntries: 0,
    logicalCacheBytes: 0,
    retries: 0,
    concurrentRequestsHighWater: 1,
    concurrentResponseBytesHighWater: 0,
    queuedRangesHighWater: 2,
    queuedRangeBytesHighWater: 0,
    decodedStagingBytesHighWater: 0,
    transferredBatches: 0,
    transferredPoints: 0,
    transferredBytes: 0,
  };
}

function cacheMode(value) {
  const mode = value ?? "none";
  require(CACHE_MODES.has(mode), "manifest_invalid", `unsupported cache mode ${mode}`);
  return mode;
}

function credentialMode(value) {
  const mode = value ?? "same-origin";
  require(CREDENTIAL_MODES.has(mode), "manifest_invalid", `unsupported credentials mode ${mode}`);
  return mode;
}

function cacheFailure(error) {
  const code = error?.name === "QuotaExceededError" ? "cache_quota" : "cache_unavailable";
  return new StreamingFailure(code, error?.message ?? String(error), { cause: error });
}

function classifyThrown(error, fallbackCode) {
  if (error instanceof StreamingFailure) return error;
  if (error?.name === "AbortError") return new StreamingFailure("cancelled", "the stream operation was cancelled", { cause: error });
  return new StreamingFailure(fallbackCode, error?.message ?? String(error), { cause: error });
}

function assertNotCancelled(signal) {
  if (signal?.aborted) throw new StreamingFailure("cancelled", "the stream operation was cancelled");
}

function defaultDelay(milliseconds, signal) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(resolve, milliseconds);
    signal?.addEventListener("abort", () => {
      clearTimeout(timeout);
      reject(new DOMException("cancelled", "AbortError"));
    }, { once: true });
  });
}

async function yieldForCancellation(signal) {
  try {
    await defaultDelay(0, signal);
  } catch (error) {
    throw classifyThrown(error, "cancelled");
  }
}

function resolvedHttpUrl(value, base, label) {
  boundedString(value, 1, 2048, label);
  const url = new URL(value, base);
  require(url.protocol === "http:" || url.protocol === "https:", "manifest_invalid", `${label} must use HTTP or HTTPS`);
  url.hash = "";
  return url.href;
}

function strongEtag(value, label) {
  boundedString(value, 3, 256, label);
  require(!value.startsWith("W/") && value.startsWith('"') && value.endsWith('"'), "manifest_invalid", `${label} must be strong and quoted`);
  return value;
}

function digest(value, label) {
  require(typeof value === "string" && /^[0-9a-f]{64}$/.test(value), "manifest_invalid", `${label} must be 64 lowercase hexadecimal characters`);
  return value;
}

function finiteTriple(value, label) {
  require(Array.isArray(value) && value.length === 3 && value.every(Number.isFinite), "manifest_invalid", `${label} must contain three finite numbers`);
  return Object.freeze([...value]);
}

function positiveFiniteTriple(value, label) {
  const result = finiteTriple(value, label);
  require(result.every((entry) => entry > 0), "manifest_invalid", `${label} must be positive`);
  return result;
}

function positiveSafeInteger(value, label) {
  require(Number.isSafeInteger(value) && value > 0, "manifest_invalid", `${label} must be a positive safe integer`);
  return value;
}

function nonnegativeSafeInteger(value, label) {
  require(Number.isSafeInteger(value) && value >= 0, "manifest_invalid", `${label} must be a nonnegative safe integer`);
  return value;
}

function boundedString(value, minimum, maximum, label) {
  require(typeof value === "string" && value.length >= minimum && value.length <= maximum, "manifest_invalid", `${label} length is invalid`);
}

function requireObject(value, label) {
  require(value && typeof value === "object" && !Array.isArray(value), "manifest_invalid", `${label} must be an object`);
}

function require(condition, code, message) {
  if (!condition) throw new StreamingFailure(code, message);
}

function equal(actual, expected, code, label) {
  require(Object.is(actual, expected), code, `${label} differs: received ${String(actual)}, expected ${String(expected)}`);
}

function compareF64Triple(view, offset, expected, label) {
  expected.forEach((value, axis) => equal(view.getFloat64(offset + axis * 8, true), value, "index_incompatible", `${label} axis ${axis}`));
}

function withinBounds(world, bounds) {
  return world.every((value, axis) => bounds.minimum[axis] <= value && value <= bounds.maximum[axis]);
}

function u64(view, offset) {
  const value = view.getBigUint64(offset, true);
  require(value <= BigInt(Number.MAX_SAFE_INTEGER), "range_corrupt", "unsigned 64-bit value is not safely addressable");
  return Number(value);
}

function i64(view, offset) {
  const value = view.getBigInt64(offset, true);
  require(value >= BigInt(Number.MIN_SAFE_INTEGER) && value <= BigInt(Number.MAX_SAFE_INTEGER), "range_corrupt", "signed 64-bit tick is not safely addressable");
  return Number(value);
}

function dataView(bytes) {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function ascii(bytes, offset, length) {
  return String.fromCharCode(...bytes.subarray(offset, offset + length));
}

function hex(bytes) {
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}
