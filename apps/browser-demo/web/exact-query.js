const MODULE_CACHE_TOKEN = encodeURIComponent(
  new URL(import.meta.url).searchParams.get("v") ?? "unversioned",
);
const {
  loadManifest,
  readBoundedBody,
  validateManifest,
} = await import(`./streaming-protocol.js?v=${MODULE_CACHE_TOKEN}`);

const LAS_HEADER_BYTES = 256;
const LAS_POINT_FORMAT = 3;
const LAS_POINT_RECORD_BYTES = 34;
const EXACT_QUERY_SAFE_ACTION =
  "Keep the current display as non-authoritative and retry exact confirmation against the active immutable Source.";

export class ExactQueryError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "ExactQueryError";
    this.code = code;
    this.safeAction = EXACT_QUERY_SAFE_ACTION;
  }
}

export function createLasExactQueryBridge(options) {
  const manifestUrl = requiredUrl(options?.manifestUrl, "manifestUrl");
  const fetchImplementation = options?.fetchImplementation ?? globalThis.fetch?.bind(globalThis);
  if (typeof fetchImplementation !== "function") {
    throw new ExactQueryError("exact_query_unavailable", "Fetch is unavailable");
  }
  const credentials = credentialMode(options?.credentials ?? "same-origin");
  let binding;
  let active = false;

  return Object.freeze({
    async confirm(request) {
      if (active) {
        throw new ExactQueryError(
          "exact_query_busy",
          "one exact Point confirmation is already active",
        );
      }
      active = true;
      try {
        assertNotCancelled(request?.signal);
        binding ??= await loadBinding(
          manifestUrl,
          fetchImplementation,
          credentials,
          request?.signal,
        );
        return await confirmPoint(
          binding,
          request,
          fetchImplementation,
          credentials,
        );
      } catch (error) {
        if (error?.name === "AbortError" || request?.signal?.aborted) {
          throw new ExactQueryError("exact_query_cancelled", "exact Point confirmation was cancelled");
        }
        if (error instanceof ExactQueryError) throw error;
        throw new ExactQueryError(
          "exact_query_failed",
          boundedMessage(error?.message ?? String(error)),
        );
      } finally {
        active = false;
      }
    },
  });
}

async function loadBinding(manifestUrl, fetchImplementation, credentials, signal) {
  const manifest = await loadManifest(manifestUrl, fetchImplementation, signal);
  const deployment = validateManifest(manifest, manifestUrl);
  const probe = await fetchExactRange(
    deployment.source,
    deployment.source.probe.offset,
    deployment.source.probe.length,
    fetchImplementation,
    credentials,
    signal,
    true,
  );
  const layout = decodeLasLayout(probe, deployment);
  return Object.freeze({ deployment, layout });
}

async function confirmPoint(binding, request, fetchImplementation, credentials) {
  const { deployment, layout } = binding;
  const sourceIdentity = requiredSourceIdentity(request?.sourceIdentity);
  if (sourceIdentity !== deployment.source.sourceIdentity) {
    throw new ExactQueryError(
      "exact_query_source_mismatch",
      "exact Query Source identity differs from the immutable deployment",
    );
  }
  const pointOrdinal = pointOrdinalValue(request?.pointOrdinal, layout.pointCount);
  const generation = positiveSafeInteger(request?.generation, "View generation");
  const offset = layout.pointDataOffset + pointOrdinal * layout.pointRecordBytes;
  const bytes = await fetchExactRange(
    deployment.source,
    offset,
    layout.pointRecordBytes,
    fetchImplementation,
    credentials,
    request?.signal,
    false,
  );
  assertNotCancelled(request?.signal);
  return exactPointResult(
    bytes,
    deployment.source.sourceIdentity,
    pointOrdinal,
    generation,
    layout,
  );
}

export function decodeLasLayout(bytes, deployment) {
  const view = dataView(bytes, LAS_HEADER_BYTES, "LAS Source probe");
  if (ascii(bytes, 0, 4) !== "LASF") {
    throw new ExactQueryError("exact_query_incompatible", "Source probe has no LAS signature");
  }
  if (view.getUint8(24) !== 1 || view.getUint8(25) !== 2) {
    throw new ExactQueryError("exact_query_incompatible", "exact bridge requires LAS 1.2");
  }
  const headerBytes = view.getUint16(94, true);
  const pointDataOffset = view.getUint32(96, true);
  const pointFormat = view.getUint8(104);
  const pointRecordBytes = view.getUint16(105, true);
  const pointCount = view.getUint32(107, true);
  if (headerBytes > bytes.byteLength || pointDataOffset < headerBytes) {
    throw new ExactQueryError("exact_query_incompatible", "LAS point-data offset is invalid");
  }
  if (pointFormat !== LAS_POINT_FORMAT || pointRecordBytes !== LAS_POINT_RECORD_BYTES) {
    throw new ExactQueryError(
      "exact_query_incompatible",
      "exact bridge requires uncompressed LAS point format 3 with 34-byte records",
    );
  }
  if (pointCount !== deployment.source.pointCount) {
    throw new ExactQueryError("exact_query_source_changed", "LAS Point count differs from deployment");
  }
  const scale = [view.getFloat64(131, true), view.getFloat64(139, true), view.getFloat64(147, true)];
  const offset = [view.getFloat64(155, true), view.getFloat64(163, true), view.getFloat64(171, true)];
  if (!scale.every((value) => Number.isFinite(value) && value > 0) || !offset.every(Number.isFinite)) {
    throw new ExactQueryError("exact_query_incompatible", "LAS position transform is invalid");
  }
  if (!sameTriple(scale, deployment.index.transform.scale)
    || !sameTriple(offset, deployment.index.transform.offset)) {
    throw new ExactQueryError("exact_query_source_changed", "LAS transform differs from deployment");
  }
  if (pointDataOffset + pointCount * pointRecordBytes > deployment.source.byteLength) {
    throw new ExactQueryError("exact_query_incompatible", "LAS Point records exceed Source length");
  }
  return Object.freeze({
    pointDataOffset,
    pointFormat,
    pointRecordBytes,
    pointCount,
    scale: Object.freeze(scale),
    offset: Object.freeze(offset),
  });
}

export function decodeLasPointRecord(
  bytes,
  sourceIdentity,
  pointOrdinal,
  generation,
  layout,
) {
  return exactPointResult(bytes, sourceIdentity, pointOrdinal, generation, layout);
}

function exactPointResult(bytes, sourceIdentity, pointOrdinal, generation, layout) {
  const view = dataView(bytes, layout.pointRecordBytes, "LAS Point record");
  const ticks = [view.getInt32(0, true), view.getInt32(4, true), view.getInt32(8, true)];
  const position = ticks.map((tick, axis) => layout.offset[axis] + tick * layout.scale[axis]);
  if (!position.every(Number.isFinite)) {
    throw new ExactQueryError("exact_query_corrupt", "exact Point position is non-finite");
  }
  return deepFreeze({
    authority: "exact_source_record",
    sourceIdentity,
    pointOrdinal: String(pointOrdinal),
    generation,
    ticks,
    position,
    intensity: view.getUint16(12, true),
    classification: view.getUint8(15),
    rgb: [
      view.getUint16(28, true),
      view.getUint16(30, true),
      view.getUint16(32, true),
    ],
  });
}

async function fetchExactRange(
  source,
  offset,
  length,
  fetchImplementation,
  credentials,
  signal,
  requireAcceptRanges,
) {
  assertNotCancelled(signal);
  const end = offset + length - 1;
  const response = await fetchImplementation(source.url, {
    method: "GET",
    headers: { Range: `bytes=${offset}-${end}` },
    credentials,
    cache: "no-store",
    redirect: "manual",
    signal,
  });
  validateExactRangeResponse(response, source, offset, end, length, requireAcceptRanges);
  const bytes = await readBoundedBody(
    response,
    length,
    "exact Source Range response",
    "range_truncated",
    signal,
  );
  if (bytes.byteLength !== length) {
    throw new ExactQueryError(
      "exact_query_truncated",
      `exact Source Range returned ${bytes.byteLength} bytes instead of ${length}`,
    );
  }
  return bytes;
}

function validateExactRangeResponse(response, source, offset, end, length, requireAcceptRanges) {
  if (response.status !== 206 || response.redirected || response.type === "opaqueredirect") {
    throw new ExactQueryError("exact_query_range_unsupported", "exact Source request was not a direct 206 response");
  }
  headerEquals(response, "Content-Range", `bytes ${offset}-${end}/${source.byteLength}`);
  headerEquals(response, "Content-Length", String(length));
  headerEquals(response, "ETag", source.etag, "exact_query_source_changed");
  const encoding = response.headers.get("Content-Encoding");
  if (encoding !== null && encoding.toLowerCase() !== "identity") {
    throw new ExactQueryError("exact_query_content_encoding", "exact Source response was transformed");
  }
  if (requireAcceptRanges) headerEquals(response, "Accept-Ranges", "bytes");
}

function headerEquals(response, name, expected, code = "exact_query_range_unsupported") {
  const actual = response.headers.get(name);
  if (actual !== expected) {
    throw new ExactQueryError(code, `${name} ${actual ?? "is unavailable"}; expected ${expected}`);
  }
}

function pointOrdinalValue(value, pointCount) {
  let ordinal;
  try {
    ordinal = typeof value === "bigint" ? value : BigInt(value);
  } catch {
    throw new ExactQueryError("exact_query_invalid", "Point ordinal must be a nonnegative integer");
  }
  if (ordinal < 0n || ordinal >= BigInt(pointCount) || ordinal > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new ExactQueryError("exact_query_invalid", "Point ordinal is outside the active Source");
  }
  return Number(ordinal);
}

function positiveSafeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new ExactQueryError("exact_query_invalid", `${label} must be a positive safe integer`);
  }
  return value;
}

function requiredSourceIdentity(value) {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) {
    throw new ExactQueryError("exact_query_invalid", "Source identity must be 64 lowercase hexadecimal characters");
  }
  return value;
}

function requiredUrl(value, label) {
  try {
    return new URL(value, globalThis.location?.href ?? "http://localhost/").href;
  } catch {
    throw new ExactQueryError("exact_query_invalid", `${label} must be a valid URL`);
  }
}

function credentialMode(value) {
  if (!["omit", "same-origin", "include"].includes(value)) {
    throw new ExactQueryError("exact_query_invalid", "credentials mode is unsupported");
  }
  return value;
}

function assertNotCancelled(signal) {
  if (signal?.aborted) throw signal.reason ?? new DOMException("cancelled", "AbortError");
}

function dataView(bytes, expectedLength, label) {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength !== expectedLength) {
    throw new ExactQueryError("exact_query_truncated", `${label} must contain exactly ${expectedLength} bytes`);
  }
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function ascii(bytes, offset, length) {
  return String.fromCharCode(...bytes.subarray(offset, offset + length));
}

function sameTriple(left, right) {
  return left.every((value, index) => Object.is(value, right[index]));
}

function boundedMessage(message) {
  const text = String(message);
  return text.length <= 512 ? text : `${text.slice(0, 511)}…`;
}

function deepFreeze(value) {
  for (const child of Object.values(value)) {
    if (child && typeof child === "object") Object.freeze(child);
  }
  return Object.freeze(value);
}
