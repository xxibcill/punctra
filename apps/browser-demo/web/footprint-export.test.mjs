import assert from "node:assert/strict";
import test from "node:test";

import {
  FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
  FOOTPRINT_EXPORT_ENDPOINT,
  FOOTPRINT_EXPORT_RECEIPT_SCHEMA,
  exportFootprintArchiveToLocalServer,
  footprintArchiveTransportFromUrl,
  validateFootprintExportReceipt,
} from "./footprint-export.js";

const digest = "a".repeat(64);

test("point-footprint export transport is explicit and loopback only", () => {
  assert.equal(footprintArchiveTransportFromUrl("http://127.0.0.1:8000/footprint.html"), "browser-download");
  assert.equal(
    footprintArchiveTransportFromUrl("http://localhost:8000/footprint.html?transport=server"),
    "same-origin-local-server",
  );
  assert.throws(
    () => footprintArchiveTransportFromUrl("https://example.test/footprint.html?transport=server"),
    /loopback HTTP page/,
  );
});

test("point-footprint export posts exact TAR bytes and validates the receipt", async () => {
  const archiveBytes = Uint8Array.of(1, 2, 3, 4);
  let request;
  const receipt = await exportFootprintArchiveToLocalServer({
    archiveBytes,
    filename: FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
    sha256: digest,
    pageUrl: "http://127.0.0.1:8123/footprint.html?transport=server",
    fetchImpl: async (url, options) => {
      request = { url, options };
      return new Response(JSON.stringify({
        schema: FOOTPRINT_EXPORT_RECEIPT_SCHEMA,
        filename: FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
        path: "/tmp/export/v0.22-browser-point-footprint-evidence.tar",
        byte_length: archiveBytes.byteLength,
        sha256: digest,
      }), { status: 201, headers: { "Content-Type": "application/json" } });
    },
  });

  assert.equal(request.url, `http://127.0.0.1:8123${FOOTPRINT_EXPORT_ENDPOINT}`);
  assert.equal(request.options.method, "POST");
  assert.equal(request.options.headers["Content-Type"], "application/x-tar");
  assert.deepEqual(request.options.body, archiveBytes);
  assert.equal(receipt.sha256, digest);
});

test("point-footprint export rejects drifted receipt fields", () => {
  const expected = {
    filename: FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
    byteLength: 4,
    sha256: digest,
  };
  const receipt = {
    schema: FOOTPRINT_EXPORT_RECEIPT_SCHEMA,
    filename: FOOTPRINT_EXPORT_ARCHIVE_FILENAME,
    path: "/tmp/export.tar",
    byte_length: 4,
    sha256: digest,
  };
  assert.equal(validateFootprintExportReceipt(receipt, expected).byte_length, 4);
  assert.throws(
    () => validateFootprintExportReceipt({ ...receipt, byte_length: 5 }, expected),
    /byte length differs/,
  );
});
