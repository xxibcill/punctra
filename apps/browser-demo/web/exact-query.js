import { loadExactQueryModules } from "./module-loader.js";
import { ExactQueryError } from "./exact-query-error.js";
import { decodeLasLayout, decodeLasPointRecord } from "./las-exact-decoder.js";

const MODULE_CACHE_TOKEN = encodeURIComponent(
  new URL(import.meta.url).searchParams.get("v") ?? "unversioned",
);
const dependencyModules = loadExactQueryModules(MODULE_CACHE_TOKEN);
const [
  {
    loadManifest,
    readBoundedBody,
    validateManifest,
  },
  { RangeResponseError, validateBoundRangeResponse },
] = await dependencyModules;

export { ExactQueryError };

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
  const manifest = await loadManifest(
    manifestUrl,
    fetchImplementation,
    signal,
    credentials,
  );
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
  return decodeLasPointRecord(
    bytes,
    deployment.source.sourceIdentity,
    pointOrdinal,
    generation,
    layout,
  );
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
  validateExactRangeResponse(response, source, offset, length, requireAcceptRanges);
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

function validateExactRangeResponse(response, source, offset, length, requireAcceptRanges) {
  try {
    validateBoundRangeResponse(response, {
      etag: source.etag,
      offset,
      length,
      totalLength: source.byteLength,
      requireAcceptRanges,
    });
  } catch (error) {
    if (!(error instanceof RangeResponseError)) throw error;
    throw exactRangeFailure(error);
  }
}

function exactRangeFailure(error) {
  if (error.kind === "etag_mismatch"
    || (error.kind === "header_unavailable" && error.headerName === "ETag")) {
    return new ExactQueryError("exact_query_source_changed", error.message);
  }
  const code = error.kind === "content_encoding"
    ? "exact_query_content_encoding"
    : "exact_query_range_unsupported";
  return new ExactQueryError(code, error.message);
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
  if ((typeof value !== "string" || value.trim().length === 0) && !(value instanceof URL)) {
    throw new ExactQueryError("exact_query_invalid", `${label} must be a valid URL`);
  }
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

function boundedMessage(message) {
  const text = String(message);
  return text.length <= 512 ? text : `${text.slice(0, 511)}…`;
}
