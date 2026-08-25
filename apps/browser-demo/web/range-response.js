const REQUIRED_RANGE_HEADERS = Object.freeze([
  "Content-Length",
  "Content-Range",
  "ETag",
]);

export class RangeResponseError extends Error {
  constructor(kind, message, options = {}) {
    super(message);
    this.name = "RangeResponseError";
    this.kind = kind;
    this.headerName = options.headerName;
  }
}

export function validateBoundRangeResponse(response, binding) {
  if (response.type === "opaqueredirect"
    || response.redirected
    || (response.status >= 300 && response.status < 400)) {
    throw rangeFailure("redirected", "Range request was redirected");
  }
  if (response.status === 200) {
    throw rangeFailure("full_response", "server returned a full 200 response to a Range request");
  }
  if (response.status !== 206) {
    throw rangeFailure("status", `Range request returned terminal HTTP ${response.status}`);
  }

  const missing = REQUIRED_RANGE_HEADERS.filter((name) => response.headers.get(name) === null);
  if (missing.length > 0) {
    throw new RangeResponseError(
      "header_unavailable",
      `required Range response header ${missing[0]} is unavailable`,
      { headerName: missing[0] },
    );
  }
  requireHeader(
    response,
    "ETag",
    binding.etag,
    "etag_mismatch",
  );
  requireHeader(
    response,
    "Content-Length",
    String(binding.length),
    "content_length_mismatch",
  );
  const end = binding.offset + binding.length - 1;
  requireHeader(
    response,
    "Content-Range",
    `bytes ${binding.offset}-${end}/${binding.totalLength}`,
    "content_range_mismatch",
  );
  const encoding = response.headers.get("Content-Encoding");
  if (encoding !== null && encoding.toLowerCase() !== "identity") {
    throw rangeFailure("content_encoding", `unexpected Content-Encoding ${encoding}`);
  }
  if (binding.requireAcceptRanges
    && response.headers.get("Accept-Ranges")?.toLowerCase() !== "bytes") {
    throw rangeFailure("accept_ranges", "Source response does not declare Accept-Ranges: bytes");
  }
}

function requireHeader(response, name, expected, kind) {
  const actual = response.headers.get(name);
  if (actual !== expected) {
    throw rangeFailure(kind, `${name} ${actual ?? "is unavailable"}; expected ${expected}`);
  }
}

function rangeFailure(kind, message) {
  return new RangeResponseError(kind, message);
}
