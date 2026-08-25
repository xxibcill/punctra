import assert from "node:assert/strict";
import test from "node:test";

import {
  RangeResponseError,
  validateBoundRangeResponse,
} from "./range-response.js";

const BINDING = Object.freeze({
  etag: '"fixture"',
  offset: 256,
  length: 34,
  totalLength: 1_000,
  requireAcceptRanges: true,
});

test("shared Range validation accepts the exact immutable binding", () => {
  assert.doesNotThrow(() => validateBoundRangeResponse(response(), BINDING));
});

test("shared Range validation publishes closed failure kinds", () => {
  const cases = [
    ["redirected", response({ redirected: true })],
    ["full_response", response({ status: 200 })],
    ["status", response({ status: 503 })],
    ["header_unavailable", response({ omitHeader: "Content-Length" })],
    ["etag_mismatch", response({ headers: { ETag: '"changed"' } })],
    ["content_length_mismatch", response({ headers: { "Content-Length": "35" } })],
    ["content_range_mismatch", response({ headers: { "Content-Range": "bytes 0-33/1000" } })],
    ["content_encoding", response({ headers: { "Content-Encoding": "gzip" } })],
    ["accept_ranges", response({ headers: { "Accept-Ranges": "none" } })],
  ];

  for (const [kind, value] of cases) {
    assert.throws(
      () => validateBoundRangeResponse(value, BINDING),
      (error) => error instanceof RangeResponseError && error.kind === kind,
    );
  }
});

function response(options = {}) {
  const values = new Headers({
    "Accept-Ranges": "bytes",
    "Content-Length": String(BINDING.length),
    "Content-Range": `bytes ${BINDING.offset}-${BINDING.offset + BINDING.length - 1}/${BINDING.totalLength}`,
    ETag: BINDING.etag,
    ...options.headers,
  });
  if (options.omitHeader) values.delete(options.omitHeader);
  return {
    status: options.status ?? 206,
    redirected: options.redirected ?? false,
    type: options.type ?? "basic",
    headers: values,
  };
}
