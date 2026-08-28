import { createVisualValidator } from "./visual-validation.js";

export const VISUAL_ARCHIVE_SCHEMA = "punctra-browser-visual-transport-archive-v1";
export const VISUAL_ARCHIVE_FORMAT = "ustar-uncompressed";

const BLOCK_BYTES = 512;
const FINAL_ZERO_BLOCKS = 2;
const ASCII_PATH = /^[A-Za-z0-9._/-]+$/;
const { requireCondition } = createVisualValidator("Visual archive invalid");

/** Encodes repository-relative byte artifacts into one deterministic USTAR archive. */
export function encodeVisualArchive(entries, options = {}) {
  const maximumEntries = boundedInteger(options.maximumEntries, "maximum archive entries", 1, 4_096);
  const maximumArchiveBytes = boundedInteger(
    options.maximumArchiveBytes,
    "maximum archive bytes",
    BLOCK_BYTES * FINAL_ZERO_BLOCKS,
    Number.MAX_SAFE_INTEGER,
  );
  requireCondition(Array.isArray(entries) && entries.length > 0, "archive entries must be nonempty");
  requireCondition(entries.length <= maximumEntries, "archive entry count exceeds its ceiling");
  const normalized = entries.map(normalizeEntry).sort((left, right) => {
    if (left.path < right.path) return -1;
    if (left.path > right.path) return 1;
    return 0;
  });
  requireCondition(new Set(normalized.map(({ path }) => path)).size === normalized.length, "archive paths must be unique");

  const payloadBytes = normalized.reduce((total, entry) => total + entry.bytes.byteLength, 0);
  const archiveBytes = normalized.reduce(
    (total, entry) => total + BLOCK_BYTES + paddedLength(entry.bytes.byteLength),
    BLOCK_BYTES * FINAL_ZERO_BLOCKS,
  );
  requireCondition(Number.isSafeInteger(archiveBytes), "archive byte length exceeds the exact integer range");
  requireCondition(archiveBytes <= maximumArchiveBytes, "archive byte length exceeds its ceiling");

  const archive = new Uint8Array(archiveBytes);
  let offset = 0;
  for (const entry of normalized) {
    archive.set(createHeader(entry.path, entry.bytes.byteLength), offset);
    offset += BLOCK_BYTES;
    archive.set(entry.bytes, offset);
    offset += paddedLength(entry.bytes.byteLength);
  }
  requireCondition(offset + BLOCK_BYTES * FINAL_ZERO_BLOCKS === archive.byteLength, "archive layout differs");
  return {
    bytes: archive,
    facts: {
      schema: VISUAL_ARCHIVE_SCHEMA,
      format: VISUAL_ARCHIVE_FORMAT,
      entry_count: normalized.length,
      payload_bytes: payloadBytes,
      archive_bytes: archive.byteLength,
      archive_structure_bytes: archive.byteLength - payloadBytes,
      paths: normalized.map(({ path }) => path),
    },
  };
}

function normalizeEntry(entry) {
  requireCondition(entry !== null && typeof entry === "object" && !Array.isArray(entry), "archive entry must be an object");
  const path = validatePath(entry.path);
  requireCondition(entry.bytes instanceof Uint8Array, `archive entry ${path} bytes must be Uint8Array`);
  return { path, bytes: entry.bytes };
}

function validatePath(path) {
  requireCondition(typeof path === "string" && path.length > 0, "archive path must be nonempty");
  requireCondition(ASCII_PATH.test(path), `archive path ${JSON.stringify(path)} must be portable ASCII`);
  requireCondition(!path.startsWith("/") && !path.endsWith("/") && !path.includes("//"), `archive path ${JSON.stringify(path)} is not canonical`);
  const segments = path.split("/");
  requireCondition(segments.every((segment) => segment !== "." && segment !== ".."), `archive path ${JSON.stringify(path)} escapes its root`);
  splitUstarPath(path);
  return path;
}

function createHeader(path, size) {
  const header = new Uint8Array(BLOCK_BYTES);
  const { name, prefix } = splitUstarPath(path);
  writeAscii(header, 0, 100, name);
  writeOctal(header, 100, 8, 0o644);
  writeOctal(header, 108, 8, 0);
  writeOctal(header, 116, 8, 0);
  writeOctal(header, 124, 12, size);
  writeOctal(header, 136, 12, 0);
  header.fill(0x20, 148, 156);
  header[156] = 0x30;
  writeAscii(header, 257, 6, "ustar\0");
  writeAscii(header, 263, 2, "00");
  writeAscii(header, 345, 155, prefix);
  const checksum = header.reduce((total, byte) => total + byte, 0);
  writeChecksum(header, checksum);
  return header;
}

function splitUstarPath(path) {
  if (path.length <= 100) return { name: path, prefix: "" };
  for (let separator = path.lastIndexOf("/"); separator > 0; separator = path.lastIndexOf("/", separator - 1)) {
    const prefix = path.slice(0, separator);
    const name = path.slice(separator + 1);
    if (prefix.length <= 155 && name.length <= 100) return { name, prefix };
  }
  throw new Error(`Visual archive invalid: archive path ${JSON.stringify(path)} exceeds USTAR fields`);
}

function writeAscii(target, offset, length, value) {
  requireCondition(value.length <= length, "archive ASCII field exceeds its width");
  for (let index = 0; index < value.length; index += 1) target[offset + index] = value.charCodeAt(index);
}

function writeOctal(target, offset, length, value) {
  requireCondition(Number.isSafeInteger(value) && value >= 0, "archive numeric field is invalid");
  const octal = value.toString(8).padStart(length - 1, "0");
  requireCondition(octal.length < length, "archive numeric field exceeds its width");
  writeAscii(target, offset, length - 1, octal);
}

function writeChecksum(target, checksum) {
  const octal = checksum.toString(8).padStart(6, "0");
  requireCondition(octal.length === 6, "archive checksum exceeds its width");
  writeAscii(target, 148, 6, octal);
  target[154] = 0;
  target[155] = 0x20;
}

function paddedLength(length) {
  return Math.ceil(length / BLOCK_BYTES) * BLOCK_BYTES;
}

function boundedInteger(value, label, minimum, maximum) {
  requireCondition(Number.isSafeInteger(value) && value >= minimum && value <= maximum, `${label} is invalid`);
  return value;
}
