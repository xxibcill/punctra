import assert from "node:assert/strict";
import test from "node:test";

import {
  createPngArtifactMetadata,
  decodeRgba8Png,
  encodeRgba8Png,
  sha256Hex,
} from "./visual-png.js";

const PNG_SIGNATURE = Uint8Array.of(137, 80, 78, 71, 13, 10, 26, 10);

test("PNG encoding is deterministic and uses filter type zero", async () => {
  const image = fixtureImage();
  const first = await encodeRgba8Png(image);
  const second = await encodeRgba8Png(image);

  assert(first instanceof Uint8Array);
  assert.deepEqual(first, second);
  assert.deepEqual(first.subarray(0, PNG_SIGNATURE.length), PNG_SIGNATURE);

  const chunks = parseChunks(first);
  assert.deepEqual(chunks.map(({ type }) => type), ["IHDR", "IDAT", "IEND"]);
  const scanlines = await transformBytes(chunks[1].data, DecompressionStream, "deflate");
  assert.deepEqual(
    scanlines,
    Uint8Array.of(
      0,
      255, 0, 0, 255,
      0, 255, 0, 128,
      0,
      0, 0, 255, 64,
      255, 255, 255, 0,
    ),
  );
});

test("RGBA8 PNGs round trip through split IDAT chunks", async () => {
  const image = fixtureImage();
  const encoded = await encodeRgba8Png(image);
  const decoded = await decodeRgba8Png(encoded);

  assert.equal(decoded.width, image.width);
  assert.equal(decoded.height, image.height);
  assert(decoded.data instanceof Uint8Array);
  assert.deepEqual(decoded.data, image.data);
  assert.notEqual(decoded.data.buffer, image.data.buffer);

  const chunks = parseChunks(encoded);
  const midpoint = Math.floor(chunks[1].data.length / 2);
  const split = concatenateBytes(
    PNG_SIGNATURE,
    chunks[0].encoded,
    createChunk("IDAT", chunks[1].data.subarray(0, midpoint)),
    createChunk("IDAT", chunks[1].data.subarray(midpoint)),
    chunks[2].encoded,
  );
  assert.deepEqual(await decodeRgba8Png(split), decoded);
});

test("SHA-256 identities use lowercase Web Crypto hexadecimal", async () => {
  const bytes = new TextEncoder().encode("abc");
  assert.equal(
    await sha256Hex(bytes),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
  );
  await assert.rejects(() => sha256Hex("abc"), /byte source/);
});

test("PNG artifact metadata has one shared identity and timing schema", async () => {
  const image = fixtureImage();
  const encodedBytes = await encodeRgba8Png(image);
  const metadata = await createPngArtifactMetadata({
    descriptor: {
      kind: "recreation_png",
      trial_id: "trial-a",
      recreation_index: 1,
      frame_index: 29,
      path: "docs/releases/trial-a-recreation-1.png",
    },
    encodedBytes,
    image,
    timing: {
      encode_milliseconds: 1.25,
      png_encode_milliseconds: 1,
      artifact_encoding_milliseconds: 1.5,
    },
  });

  assert.deepEqual(metadata, {
    kind: "recreation_png",
    trial_id: "trial-a",
    recreation_index: 1,
    frame_index: 29,
    path: "docs/releases/trial-a-recreation-1.png",
    filename: "trial-a-recreation-1.png",
    mime_type: "image/png",
    encoding: "png-rgba8-filter-0",
    width: image.width,
    height: image.height,
    encoded_byte_length: encodedBytes.byteLength,
    encoded_sha256: await sha256Hex(encodedBytes),
    decoded_byte_length: image.data.byteLength,
    decoded_sha256: await sha256Hex(image.data),
    encode_milliseconds: 1.25,
    png_encode_milliseconds: 1,
    artifact_encoding_milliseconds: 1.5,
    authority: "presentation_only",
  });
});

test("the decoder rejects malformed signatures, truncation, and trailing bytes", async () => {
  const encoded = await encodeRgba8Png(fixtureImage());
  const badSignature = encoded.slice();
  badSignature[0] ^= 0xff;

  await assert.rejects(() => decodeRgba8Png(badSignature), /signature/);
  await assert.rejects(() => decodeRgba8Png(encoded.subarray(0, encoded.length - 1)), /truncated/);
  await assert.rejects(
    () => decodeRgba8Png(concatenateBytes(encoded, Uint8Array.of(0))),
    /IEND.*final|after IEND/,
  );
  await assert.rejects(() => decodeRgba8Png(new Uint8Array()), /signature/);
});

test("the decoder validates every chunk CRC and immutable chunk order", async () => {
  const encoded = await encodeRgba8Png(fixtureImage());
  const chunks = parseChunks(encoded);
  const tampered = encoded.slice();
  tampered[chunks[1].dataOffset] ^= 1;

  await assert.rejects(() => decodeRgba8Png(tampered), /CRC/);

  const reordered = concatenateBytes(
    PNG_SIGNATURE,
    chunks[1].encoded,
    chunks[0].encoded,
    chunks[2].encoded,
  );
  await assert.rejects(() => decodeRgba8Png(reordered), /IHDR.*first/);

  const unsupported = concatenateBytes(
    PNG_SIGNATURE,
    chunks[0].encoded,
    createChunk("tEXt", new TextEncoder().encode("not canonical")),
    chunks[1].encoded,
    chunks[2].encoded,
  );
  await assert.rejects(() => decodeRgba8Png(unsupported), /unsupported PNG chunk tEXt/);
});

test("the decoder validates IHDR dimensions and RGBA8 format facts", async () => {
  const encoded = await encodeRgba8Png(fixtureImage());

  await assert.rejects(
    () => decodeRgba8Png(rewriteIhdr(encoded, (ihdr) => ihdr.fill(0, 0, 4))),
    /width/,
  );
  await assert.rejects(
    () => decodeRgba8Png(rewriteIhdr(encoded, (ihdr) => { ihdr[9] = 2; })),
    /color type/,
  );
  await assert.rejects(
    () => decodeRgba8Png(rewriteIhdr(encoded, (ihdr) => { ihdr[11] = 1; })),
    /filter method/,
  );
});

test("the decoder rejects nonzero scanline filters and decompressed-length drift", async () => {
  const encoded = await encodeRgba8Png(fixtureImage());
  const chunks = parseChunks(encoded);
  const scanlines = await transformBytes(chunks[1].data, DecompressionStream, "deflate");
  const nonzeroFilter = scanlines.slice();
  nonzeroFilter[0] = 1;
  const filteredPng = await replaceIdat(encoded, nonzeroFilter);
  const shortPng = await replaceIdat(encoded, scanlines.subarray(0, scanlines.length - 1));

  await assert.rejects(
    () => decodeRgba8Png(filteredPng),
    /filter type 0/,
  );
  await assert.rejects(
    () => decodeRgba8Png(shortPng),
    /decompressed length/,
  );

  const invalidDeflate = replaceIdatBytes(encoded, Uint8Array.of(1, 2, 3));
  await assert.rejects(() => decodeRgba8Png(invalidDeflate), /decompress/);
});

test("the encoder validates dimensions, area, RGBA8 type, and exact data length", async () => {
  await assert.rejects(() => encodeRgba8Png(null), /image must be an object/);
  await assert.rejects(
    () => encodeRgba8Png({ width: 0, height: 1, data: new Uint8Array() }),
    /width.*positive integer/,
  );
  await assert.rejects(
    () => encodeRgba8Png({ width: 1.5, height: 1, data: new Uint8Array() }),
    /width.*positive integer/,
  );
  await assert.rejects(
    () => encodeRgba8Png({ width: 4_097, height: 1, data: new Uint8Array() }),
    /width.*4,096/,
  );
  await assert.rejects(
    () => encodeRgba8Png({ width: 4_096, height: 4_096, data: new Uint8Array() }),
    /8,388,608 pixels/,
  );
  await assert.rejects(
    () => encodeRgba8Png({ width: 1, height: 1, data: [0, 0, 0, 0] }),
    /Uint8Array/,
  );
  await assert.rejects(
    () => encodeRgba8Png({ width: 1, height: 1, data: new Uint8Array(3) }),
    /4 RGBA8 bytes/,
  );
});

function fixtureImage() {
  return {
    width: 2,
    height: 2,
    data: Uint8Array.of(
      255, 0, 0, 255,
      0, 255, 0, 128,
      0, 0, 255, 64,
      255, 255, 255, 0,
    ),
  };
}

async function replaceIdat(png, scanlines) {
  const compressed = await transformBytes(scanlines, CompressionStream, "deflate");
  return replaceIdatBytes(png, compressed);
}

function replaceIdatBytes(png, compressed) {
  const chunks = parseChunks(png);
  return concatenateBytes(
    PNG_SIGNATURE,
    chunks.find(({ type }) => type === "IHDR").encoded,
    createChunk("IDAT", compressed),
    chunks.find(({ type }) => type === "IEND").encoded,
  );
}

function rewriteIhdr(png, mutate) {
  const chunks = parseChunks(png);
  const ihdr = chunks[0].data.slice();
  mutate(ihdr);
  return concatenateBytes(
    PNG_SIGNATURE,
    createChunk("IHDR", ihdr),
    ...chunks.slice(1).map(({ encoded }) => encoded),
  );
}

function parseChunks(png) {
  const chunks = [];
  let offset = PNG_SIGNATURE.length;
  while (offset < png.length) {
    const length = readUint32(png, offset);
    const end = offset + 12 + length;
    const type = String.fromCharCode(...png.subarray(offset + 4, offset + 8));
    chunks.push({
      type,
      dataOffset: offset + 8,
      data: png.slice(offset + 8, offset + 8 + length),
      encoded: png.slice(offset, end),
    });
    offset = end;
  }
  return chunks;
}

function createChunk(type, data) {
  const typeBytes = new TextEncoder().encode(type);
  const chunk = new Uint8Array(12 + data.length);
  writeUint32(chunk, 0, data.length);
  chunk.set(typeBytes, 4);
  chunk.set(data, 8);
  writeUint32(chunk, 8 + data.length, crc32(typeBytes, data));
  return chunk;
}

async function transformBytes(bytes, Constructor, format) {
  const stream = new Blob([bytes]).stream().pipeThrough(new Constructor(format));
  return new Uint8Array(await new Response(stream).arrayBuffer());
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
  for (const byte of concatenateBytes(type, data)) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function readUint32(bytes, offset) {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(offset);
}

function writeUint32(bytes, offset, value) {
  new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).setUint32(offset, value);
}
