import { errorMessage } from "./visual-validation.js";

const PNG_SIGNATURE = Uint8Array.of(137, 80, 78, 71, 13, 10, 26, 10);
const IHDR = Uint8Array.of(73, 72, 68, 82);
const IDAT = Uint8Array.of(73, 68, 65, 84);
const IEND = Uint8Array.of(73, 69, 78, 68);
const IHDR_LENGTH = 13;
const BYTES_PER_PIXEL = 4;
const MAX_DIMENSION = 4_096;
const MAX_PIXELS = 8_388_608;
const DEFLATE_OVERHEAD_LIMIT = 65_536;
const ARTIFACT_TIMING_FIELDS = Object.freeze([
  "encode_milliseconds",
  "png_encode_milliseconds",
  "artifact_encoding_milliseconds",
]);
const CRC_TABLE = createCrcTable();

export async function encodeRgba8Png(image) {
  const { width, height, data } = validateImage(image);
  const scanlines = createUnfilteredScanlines(width, height, data);
  const compressed = await compressZlib(scanlines);
  return concatenateBytes(
    PNG_SIGNATURE,
    createChunk(IHDR, createIhdr(width, height)),
    createChunk(IDAT, compressed),
    createChunk(IEND, new Uint8Array()),
  );
}

export async function decodeRgba8Png(png) {
  requireUint8Array(png, "PNG");
  const parsed = parsePng(png);
  const scanlines = await decompressZlib(parsed.compressed, parsed.scanlineBytes);
  return {
    width: parsed.width,
    height: parsed.height,
    data: removeScanlineFilters(parsed.width, parsed.height, scanlines),
  };
}

export async function sha256Hex(bytes) {
  requireUint8Array(bytes, "SHA-256 input");
  const subtle = globalThis.crypto?.subtle;
  if (subtle === undefined) throw new Error("Web Crypto SHA-256 is unavailable");
  const digest = new Uint8Array(await subtle.digest("SHA-256", bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function createPngArtifactMetadata({
  descriptor,
  encodedBytes,
  image,
  identities = {},
  timing = {},
}) {
  const validatedDescriptor = validateArtifactDescriptor(descriptor);
  requireUint8Array(encodedBytes, "encoded PNG");
  const validatedImage = validateImage(image);
  const timingMetadata = {};
  for (const field of ARTIFACT_TIMING_FIELDS) {
    if (timing[field] === undefined) continue;
    if (!Number.isFinite(timing[field]) || timing[field] < 0) {
      throw new TypeError(`${field} must be finite and nonnegative`);
    }
    timingMetadata[field] = timing[field];
  }
  return {
    kind: validatedDescriptor.kind,
    trial_id: validatedDescriptor.trial_id ?? null,
    recreation_index: validatedDescriptor.recreation_index ?? null,
    frame_index: validatedDescriptor.frame_index ?? null,
    path: validatedDescriptor.path,
    filename: validatedDescriptor.path.split("/").at(-1),
    mime_type: "image/png",
    encoding: "png-rgba8-filter-0",
    width: validatedImage.width,
    height: validatedImage.height,
    encoded_byte_length: encodedBytes.byteLength,
    encoded_sha256: identities.encoded_sha256 ?? await sha256Hex(encodedBytes),
    decoded_byte_length: validatedImage.data.byteLength,
    decoded_sha256: identities.decoded_sha256 ?? await sha256Hex(validatedImage.data),
    ...timingMetadata,
    authority: "presentation_only",
  };
}

function validateArtifactDescriptor(descriptor) {
  if (descriptor === null || typeof descriptor !== "object" || Array.isArray(descriptor)) {
    throw new TypeError("PNG artifact descriptor must be an object");
  }
  if (typeof descriptor.kind !== "string" || descriptor.kind.length === 0) {
    throw new TypeError("PNG artifact kind must be nonempty");
  }
  if (typeof descriptor.path !== "string" || descriptor.path.length === 0) {
    throw new TypeError("PNG artifact path must be nonempty");
  }
  return descriptor;
}

function validateImage(image) {
  if (image === null || typeof image !== "object" || Array.isArray(image)) {
    throw new TypeError("image must be an object");
  }
  const width = validateDimension(image.width, "width");
  const height = validateDimension(image.height, "height");
  const pixels = width * height;
  if (pixels > MAX_PIXELS) {
    throw new RangeError("image must not exceed 8,388,608 pixels");
  }
  requireUint8Array(image.data, "image data");
  const expectedLength = pixels * BYTES_PER_PIXEL;
  if (image.data.length !== expectedLength) {
    throw new RangeError(`image data must contain exactly ${expectedLength} RGBA8 bytes`);
  }
  return { width, height, data: image.data };
}

function validateDimension(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError(`${label} must be a positive integer`);
  }
  if (value > MAX_DIMENSION) {
    throw new RangeError(`${label} must not exceed 4,096 pixels`);
  }
  return value;
}

function requireUint8Array(value, label) {
  if (!(value instanceof Uint8Array)) {
    throw new TypeError(`${label} must be a Uint8Array byte source`);
  }
}

function createUnfilteredScanlines(width, height, rgba) {
  const rgbaBytesPerRow = width * BYTES_PER_PIXEL;
  const scanlineBytesPerRow = rgbaBytesPerRow + 1;
  const scanlines = new Uint8Array(scanlineBytesPerRow * height);
  for (let row = 0; row < height; row += 1) {
    const sourceOffset = row * rgbaBytesPerRow;
    const targetOffset = row * scanlineBytesPerRow + 1;
    scanlines.set(rgba.subarray(sourceOffset, sourceOffset + rgbaBytesPerRow), targetOffset);
  }
  return scanlines;
}

function createIhdr(width, height) {
  const ihdr = new Uint8Array(IHDR_LENGTH);
  const view = dataView(ihdr);
  view.setUint32(0, width);
  view.setUint32(4, height);
  ihdr.set([8, 6, 0, 0, 0], 8);
  return ihdr;
}

function createChunk(type, data) {
  const chunk = new Uint8Array(12 + data.length);
  const view = dataView(chunk);
  view.setUint32(0, data.length);
  chunk.set(type, 4);
  chunk.set(data, 8);
  view.setUint32(8 + data.length, crc32(type, data));
  return chunk;
}

function parsePng(png) {
  validateSignature(png);
  const state = { phase: "before_ihdr", idat: [], compressedBytes: 0, header: null };
  let offset = PNG_SIGNATURE.length;
  while (offset < png.length) {
    const chunk = readChunk(png, offset);
    acceptChunk(state, chunk);
    offset = chunk.end;
    if (chunk.type === "IEND") {
      if (offset !== png.length) throw new PngValidationError("PNG IEND chunk must be final");
      return parsedImage(state);
    }
  }
  throw new PngValidationError("PNG is truncated: IEND chunk is missing");
}

function validateSignature(png) {
  if (png.length < PNG_SIGNATURE.length) {
    throw new PngValidationError("PNG signature is truncated");
  }
  for (let index = 0; index < PNG_SIGNATURE.length; index += 1) {
    if (png[index] !== PNG_SIGNATURE[index]) {
      throw new PngValidationError("PNG signature is invalid");
    }
  }
}

function readChunk(png, offset) {
  if (png.length - offset < 12) {
    throw new PngValidationError("PNG is truncated in a chunk header");
  }
  const length = dataView(png).getUint32(offset);
  if (length > png.length - offset - 12) {
    throw new PngValidationError("PNG is truncated in a chunk payload");
  }
  const typeBytes = png.subarray(offset + 4, offset + 8);
  const type = readChunkType(typeBytes);
  const data = png.subarray(offset + 8, offset + 8 + length);
  const expectedCrc = dataView(png).getUint32(offset + 8 + length);
  if (crc32(typeBytes, data) !== expectedCrc) {
    throw new PngValidationError(`PNG ${type} chunk CRC is invalid`);
  }
  return { type, data, end: offset + 12 + length };
}

function readChunkType(bytes) {
  if (!Array.from(bytes).every(isAsciiLetter)) {
    throw new PngValidationError("PNG chunk type must contain four ASCII letters");
  }
  return String.fromCharCode(bytes[0], bytes[1], bytes[2], bytes[3]);
}

function isAsciiLetter(byte) {
  return (byte >= 65 && byte <= 90) || (byte >= 97 && byte <= 122);
}

function acceptChunk(state, chunk) {
  if (chunk.type === "IHDR") return acceptIhdr(state, chunk.data);
  if (chunk.type === "IDAT") return acceptIdat(state, chunk.data);
  if (chunk.type === "IEND") return acceptIend(state, chunk.data);
  throw new PngValidationError(`unsupported PNG chunk ${chunk.type}`);
}

function acceptIhdr(state, data) {
  if (state.phase !== "before_ihdr") {
    throw new PngValidationError("PNG IHDR chunk must be first and unique");
  }
  state.header = parseIhdr(data);
  state.phase = "after_ihdr";
}

function acceptIdat(state, data) {
  if (state.phase === "before_ihdr") {
    throw new PngValidationError("PNG IHDR chunk must be first");
  }
  if (state.phase !== "after_ihdr" && state.phase !== "idat") {
    throw new PngValidationError("PNG IDAT chunks must be contiguous and precede IEND");
  }
  state.compressedBytes += data.length;
  if (state.compressedBytes > state.header.scanlineBytes + DEFLATE_OVERHEAD_LIMIT) {
    throw new PngValidationError("PNG IDAT data exceeds the bounded canonical-image limit");
  }
  state.idat.push(data);
  state.phase = "idat";
}

function acceptIend(state, data) {
  if (state.phase !== "idat") {
    throw new PngValidationError("PNG IEND chunk must follow one or more IDAT chunks");
  }
  if (data.length !== 0) throw new PngValidationError("PNG IEND chunk must be empty");
  state.phase = "iend";
}

function parseIhdr(data) {
  if (data.length !== IHDR_LENGTH) {
    throw new PngValidationError("PNG IHDR chunk must contain exactly 13 bytes");
  }
  const view = dataView(data);
  const width = validatePngDimension(view.getUint32(0), "width");
  const height = validatePngDimension(view.getUint32(4), "height");
  if (width * height > MAX_PIXELS) {
    throw new PngValidationError("PNG must not exceed 8,388,608 pixels");
  }
  validateIhdrFormat(data);
  return { width, height, scanlineBytes: (width * BYTES_PER_PIXEL + 1) * height };
}

function validatePngDimension(value, label) {
  if (value === 0) throw new PngValidationError(`PNG ${label} must be positive`);
  if (value > MAX_DIMENSION) {
    throw new PngValidationError(`PNG ${label} must not exceed 4,096 pixels`);
  }
  return value;
}

function validateIhdrFormat(data) {
  const facts = [
    [data[8], 8, "bit depth"],
    [data[9], 6, "color type"],
    [data[10], 0, "compression method"],
    [data[11], 0, "filter method"],
    [data[12], 0, "interlace method"],
  ];
  for (const [actual, expected, label] of facts) {
    if (actual !== expected) {
      throw new PngValidationError(`PNG ${label} must be ${expected}, received ${actual}`);
    }
  }
}

function parsedImage(state) {
  if (state.header === null || state.idat.length === 0) {
    throw new PngValidationError("PNG requires IHDR and IDAT chunks before IEND");
  }
  return {
    ...state.header,
    compressed: concatenateBytes(...state.idat),
  };
}

function removeScanlineFilters(width, height, scanlines) {
  const rgbaBytesPerRow = width * BYTES_PER_PIXEL;
  const scanlineBytesPerRow = rgbaBytesPerRow + 1;
  const rgba = new Uint8Array(rgbaBytesPerRow * height);
  for (let row = 0; row < height; row += 1) {
    const sourceOffset = row * scanlineBytesPerRow;
    if (scanlines[sourceOffset] !== 0) {
      throw new PngValidationError(`PNG scanline ${row} must use filter type 0`);
    }
    rgba.set(
      scanlines.subarray(sourceOffset + 1, sourceOffset + scanlineBytesPerRow),
      row * rgbaBytesPerRow,
    );
  }
  return rgba;
}

async function compressZlib(scanlines) {
  return transformBytes({
    bytes: scanlines,
    constructorName: "CompressionStream",
    maximumLength: scanlines.length + DEFLATE_OVERHEAD_LIMIT,
    failureLabel: "PNG scanline compression",
  });
}

async function decompressZlib(compressed, expectedLength) {
  const result = await transformBytes({
    bytes: compressed,
    constructorName: "DecompressionStream",
    maximumLength: expectedLength,
    failureLabel: "PNG IDAT decompression",
  });
  if (result.length !== expectedLength) {
    throw new PngValidationError(
      `PNG decompressed length must be ${expectedLength} bytes, received ${result.length}`,
    );
  }
  return result;
}

async function transformBytes(options) {
  const Constructor = globalThis[options.constructorName];
  if (typeof Constructor !== "function") {
    throw new Error(`${options.constructorName} is unavailable`);
  }
  try {
    const transformed = new Blob([options.bytes])
      .stream()
      .pipeThrough(new Constructor("deflate"));
    return await readBoundedStream(transformed, options.maximumLength);
  } catch (error) {
    if (error instanceof PngValidationError) throw error;
    throw new PngValidationError(`${options.failureLabel} failed: ${errorMessage(error)}`, {
      cause: error,
    });
  }
}

async function readBoundedStream(stream, maximumLength) {
  const reader = stream.getReader();
  const chunks = [];
  let length = 0;
  while (true) {
    const { value, done } = await reader.read();
    if (done) return concatenateBytes(...chunks);
    requireUint8Array(value, "compressed stream chunk");
    length += value.length;
    if (length > maximumLength) {
      await cancelQuietly(reader);
      throw new PngValidationError(
        `PNG transformed data exceeds the ${maximumLength}-byte bounded limit`,
      );
    }
    chunks.push(value);
  }
}

async function cancelQuietly(reader) {
  try {
    await reader.cancel();
  } catch {
    // The size violation remains the actionable failure.
  }
}


function concatenateBytes(...parts) {
  const length = parts.reduce((total, part) => total + part.length, 0);
  const result = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.length;
  }
  return result;
}

function crc32(type, data) {
  let crc = 0xffffffff;
  crc = updateCrc(crc, type);
  crc = updateCrc(crc, data);
  return (crc ^ 0xffffffff) >>> 0;
}

function updateCrc(crc, bytes) {
  for (const byte of bytes) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return crc;
}

function createCrcTable() {
  const table = new Uint32Array(256);
  for (let index = 0; index < table.length; index += 1) {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = (value >>> 1) ^ (0xedb88320 & -(value & 1));
    }
    table[index] = value >>> 0;
  }
  return table;
}

function dataView(bytes) {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

class PngValidationError extends Error {
  constructor(message, options) {
    super(message, options);
    this.name = "PngValidationError";
  }
}
