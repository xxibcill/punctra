import assert from "node:assert/strict";
import test from "node:test";

import {
  VISUAL_EXPORT_ARCHIVE_FILENAME,
  VISUAL_EXPORT_ENDPOINT,
  VISUAL_EXPORT_RECEIPT_SCHEMA,
  exportVisualArchiveToLocalServer,
  validateVisualExportReceipt,
  visualArchiveTransportFromUrl,
} from "./visual-export.js";

const SHA256 = "a".repeat(64);

test("archive transport defaults to one browser download and requires an explicit server opt-in", () => {
  assert.equal(visualArchiveTransportFromUrl("http://127.0.0.1:8000/visual.html?mode=record"), "browser-download");
  assert.equal(
    visualArchiveTransportFromUrl("http://localhost:8000/visual.html?mode=record&transport=server"),
    "same-origin-local-server",
  );
  assert.throws(
    () => visualArchiveTransportFromUrl("http://127.0.0.1:8000/visual.html?transport=other"),
    /unsupported archive transport/,
  );
  assert.throws(
    () => visualArchiveTransportFromUrl("https://example.com/visual.html?transport=server"),
    /loopback HTTP page/,
  );
});

test("local server export posts exact TAR bytes and accepts only an exact receipt", async () => {
  const archiveBytes = Uint8Array.of(1, 2, 3, 4);
  let request;
  const receipt = await exportVisualArchiveToLocalServer({
    archiveBytes,
    filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
    sha256: SHA256,
    pageUrl: "http://127.0.0.1:8000/visual.html?transport=server",
    fetchImpl: async (url, options) => {
      request = { url, options };
      return new Response(JSON.stringify({
        schema: VISUAL_EXPORT_RECEIPT_SCHEMA,
        filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
        path: "/tmp/qualified/v0.21-browser-visual-evidence.tar",
        byte_length: archiveBytes.byteLength,
        sha256: SHA256,
      }), {
        status: 201,
        headers: { "Content-Type": "application/json; charset=utf-8" },
      });
    },
  });

  assert.equal(request.url, `http://127.0.0.1:8000${VISUAL_EXPORT_ENDPOINT}`);
  assert.equal(request.options.method, "POST");
  assert.equal(request.options.mode, "same-origin");
  assert.equal(request.options.credentials, "same-origin");
  assert.equal(request.options.cache, "no-store");
  assert.equal(request.options.redirect, "error");
  assert.equal(request.options.headers["Content-Type"], "application/x-tar");
  assert.equal(request.options.body, archiveBytes);
  assert.deepEqual(receipt, {
    schema: VISUAL_EXPORT_RECEIPT_SCHEMA,
    filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
    path: "/tmp/qualified/v0.21-browser-visual-evidence.tar",
    byte_length: 4,
    sha256: SHA256,
  });
});

test("local server export rejects non-success and mismatched receipts", async () => {
  const base = {
    archiveBytes: Uint8Array.of(1),
    filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
    sha256: SHA256,
    pageUrl: "http://127.0.0.1:8000/visual.html?transport=server",
  };
  await assert.rejects(
    exportVisualArchiveToLocalServer({
      ...base,
      fetchImpl: async () => new Response("conflict", { status: 409 }),
    }),
    /HTTP 409/,
  );
  await assert.rejects(
    exportVisualArchiveToLocalServer({
      ...base,
      fetchImpl: async () => new Response(JSON.stringify({
        schema: VISUAL_EXPORT_RECEIPT_SCHEMA,
        filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
        path: "/tmp/archive.tar",
        byte_length: 2,
        sha256: SHA256,
      }), { status: 201, headers: { "Content-Type": "application/json" } }),
    }),
    /byte length differs/,
  );
});

test("receipt validation rejects relative output paths and identity mismatches", () => {
  const expected = {
    filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
    byteLength: 1,
    sha256: SHA256,
  };
  assert.throws(
    () => validateVisualExportReceipt({
      schema: VISUAL_EXPORT_RECEIPT_SCHEMA,
      filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
      path: "relative/archive.tar",
      byte_length: 1,
      sha256: SHA256,
    }, expected),
    /path must be absolute/,
  );
  assert.throws(
    () => validateVisualExportReceipt({
      schema: VISUAL_EXPORT_RECEIPT_SCHEMA,
      filename: VISUAL_EXPORT_ARCHIVE_FILENAME,
      path: "/tmp/archive.tar",
      byte_length: 1,
      sha256: "b".repeat(64),
    }, expected),
    /SHA-256 differs/,
  );
});
