import { createVisualValidator, errorMessage, parsePageUrl } from "./visual-validation.js";

export const VISUAL_EXPORT_RECEIPT_SCHEMA = "punctra-browser-visual-export-receipt-v1";
export const VISUAL_EXPORT_ENDPOINT = "/qualification-visual-export";
export const VISUAL_EXPORT_ARCHIVE_FILENAME = "v0.21-browser-visual-evidence.tar";

const SHA256_HEX = /^[0-9a-f]{64}$/;
const LOCAL_HOSTNAMES = new Set(["127.0.0.1", "localhost", "[::1]"]);
const { requireCondition } = createVisualValidator("Visual export invalid");

/** Selects the private archive transport explicitly requested by the page URL. */
export function visualArchiveTransportFromUrl(pageUrl) {
  const url = parsePageUrl(pageUrl, "Visual export invalid");
  const requested = url.searchParams.get("transport");
  if (requested === null) return "browser-download";
  requireCondition(requested === "server", `unsupported archive transport ${JSON.stringify(requested)}`);
  requireLocalHttpPage(url);
  return "same-origin-local-server";
}

/** Sends one qualified USTAR archive to the opt-in loopback server export endpoint. */
export async function exportVisualArchiveToLocalServer({
  archiveBytes,
  filename,
  sha256,
  pageUrl,
  fetchImpl = globalThis.fetch,
}) {
  requireCondition(archiveBytes instanceof Uint8Array && archiveBytes.byteLength > 0, "archive bytes must be nonempty Uint8Array");
  requireCondition(filename === VISUAL_EXPORT_ARCHIVE_FILENAME, "archive filename differs from the qualified transport contract");
  requireCondition(typeof sha256 === "string" && SHA256_HEX.test(sha256), "archive SHA-256 is invalid");
  requireCondition(typeof fetchImpl === "function", "fetch implementation is unavailable");

  const url = parsePageUrl(pageUrl, "Visual export invalid");
  requireLocalHttpPage(url);
  const endpoint = new URL(VISUAL_EXPORT_ENDPOINT, url.origin);
  const response = await fetchImpl(endpoint.href, {
    method: "POST",
    mode: "same-origin",
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
    headers: { "Content-Type": "application/x-tar" },
    body: archiveBytes,
  });
  requireCondition(response?.status === 201, `local server export returned HTTP ${response?.status ?? "unknown"}`);
  const contentType = response.headers?.get?.("content-type") ?? "";
  requireCondition(contentType.toLowerCase().startsWith("application/json"), "local server export receipt is not JSON");
  let receipt;
  try {
    receipt = await response.json();
  } catch (error) {
    throw new Error(`Visual export invalid: local server export receipt is malformed JSON: ${errorMessage(error)}`);
  }
  return validateVisualExportReceipt(receipt, {
    filename,
    byteLength: archiveBytes.byteLength,
    sha256,
  });
}

export function validateVisualExportReceipt(receipt, expected) {
  requireCondition(isPlainObject(receipt), "local server export receipt must be an object");
  requireCondition(receipt.schema === VISUAL_EXPORT_RECEIPT_SCHEMA, "local server export receipt schema differs");
  requireCondition(receipt.filename === expected.filename, "local server export receipt filename differs");
  requireCondition(receipt.byte_length === expected.byteLength, "local server export receipt byte length differs");
  requireCondition(receipt.sha256 === expected.sha256, "local server export receipt SHA-256 differs");
  requireCondition(typeof receipt.path === "string" && receipt.path.startsWith("/"), "local server export receipt path must be absolute");
  return {
    schema: receipt.schema,
    filename: receipt.filename,
    path: receipt.path,
    byte_length: receipt.byte_length,
    sha256: receipt.sha256,
  };
}

function requireLocalHttpPage(url) {
  requireCondition(url.protocol === "http:" && LOCAL_HOSTNAMES.has(url.hostname), "local server export requires a loopback HTTP page");
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
