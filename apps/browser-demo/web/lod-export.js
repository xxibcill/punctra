import { createVisualValidator, errorMessage, parsePageUrl } from "./visual-validation.js";

export const LOD_EXPORT_RECEIPT_SCHEMA = "punctra-browser-lod-export-receipt-v1";
export const LOD_EXPORT_ENDPOINT = "/qualification-lod-export";
export const LOD_EXPORT_ARCHIVE_FILENAME = "v0.23-browser-lod-evidence.tar";

const SHA256_HEX = /^[0-9a-f]{64}$/;
const LOCAL_HOSTNAMES = new Set(["127.0.0.1", "localhost", "[::1]"]);
const { requireCondition } = createVisualValidator("LOD export invalid");

export async function exportLodArchiveToLocalServer({
  archiveBytes,
  filename,
  sha256,
  pageUrl,
  fetchImpl = globalThis.fetch,
}) {
  requireCondition(
    archiveBytes instanceof Uint8Array && archiveBytes.byteLength > 0,
    "archive bytes must be nonempty Uint8Array",
  );
  requireCondition(
    filename === LOD_EXPORT_ARCHIVE_FILENAME,
    "archive filename differs from the qualified transport contract",
  );
  requireCondition(typeof sha256 === "string" && SHA256_HEX.test(sha256), "archive SHA-256 is invalid");
  requireCondition(typeof fetchImpl === "function", "fetch implementation is unavailable");

  const url = parsePageUrl(pageUrl, "LOD export invalid");
  requireLocalHttpPage(url);
  const response = await fetchImpl(new URL(LOD_EXPORT_ENDPOINT, url.origin).href, {
    method: "POST",
    mode: "same-origin",
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
    headers: { "Content-Type": "application/x-tar" },
    body: archiveBytes,
  });
  requireCondition(
    response?.status === 201,
    `local server export returned HTTP ${response?.status ?? "unknown"}`,
  );
  const contentType = response.headers?.get?.("content-type") ?? "";
  requireCondition(
    contentType.toLowerCase().startsWith("application/json"),
    "local server export receipt is not JSON",
  );
  let receipt;
  try {
    receipt = await response.json();
  } catch (error) {
    throw new Error(
      `LOD export invalid: local server export receipt is malformed JSON: ${errorMessage(error)}`,
    );
  }
  return validateLodExportReceipt(receipt, {
    filename,
    byteLength: archiveBytes.byteLength,
    sha256,
  });
}

export function validateLodExportReceipt(receipt, expected) {
  requireCondition(isPlainObject(receipt), "local server export receipt must be an object");
  requireCondition(receipt.schema === LOD_EXPORT_RECEIPT_SCHEMA, "local server export receipt schema differs");
  requireCondition(receipt.filename === expected.filename, "local server export receipt filename differs");
  requireCondition(receipt.byte_length === expected.byteLength, "local server export receipt byte length differs");
  requireCondition(receipt.sha256 === expected.sha256, "local server export receipt SHA-256 differs");
  requireCondition(
    typeof receipt.path === "string" && receipt.path.startsWith("/"),
    "local server export receipt path must be absolute",
  );
  return {
    schema: receipt.schema,
    filename: receipt.filename,
    path: receipt.path,
    byte_length: receipt.byte_length,
    sha256: receipt.sha256,
  };
}

function requireLocalHttpPage(url) {
  requireCondition(
    url.protocol === "http:" && LOCAL_HOSTNAMES.has(url.hostname),
    "local server export requires a loopback HTTP page",
  );
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
