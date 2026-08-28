import assert from "node:assert/strict";
import test from "node:test";

import {
  VISUAL_ARCHIVE_FORMAT,
  encodeVisualArchive,
} from "./visual-archive.js";

test("transport archive is deterministic, sorted, and preserves repository paths and bytes", () => {
  const entries = [
    { path: "docs/releases/z.png", bytes: Uint8Array.of(9, 8, 7) },
    {
      path: "apps/browser-demo/web/fixtures/visual-v1/baselines/a.png",
      bytes: Uint8Array.of(1, 2, 3, 4),
    },
  ];
  const options = { maximumEntries: 8, maximumArchiveBytes: 8_192 };
  const first = encodeVisualArchive(entries, options);
  const second = encodeVisualArchive([...entries].reverse(), options);
  assert.deepEqual(second, first);
  assert.equal(first.facts.format, VISUAL_ARCHIVE_FORMAT);
  assert.deepEqual(first.facts.paths, [
    "apps/browser-demo/web/fixtures/visual-v1/baselines/a.png",
    "docs/releases/z.png",
  ]);
  assert.equal(first.facts.payload_bytes, 7);
  assert.equal(first.facts.archive_bytes, 3_072);
  assert.deepEqual(readTar(first.bytes), [
    {
      path: "apps/browser-demo/web/fixtures/visual-v1/baselines/a.png",
      bytes: Uint8Array.of(1, 2, 3, 4),
    },
    { path: "docs/releases/z.png", bytes: Uint8Array.of(9, 8, 7) },
  ]);
});

test("transport archive rejects traversal, duplicates, non-bytes, and size overflow", () => {
  const valid = { path: "docs/releases/evidence.json", bytes: Uint8Array.of(1) };
  assert.throws(
    () => encodeVisualArchive([{ ...valid, path: "../evidence.json" }], { maximumEntries: 2, maximumArchiveBytes: 4_096 }),
    /escapes its root/,
  );
  assert.throws(
    () => encodeVisualArchive([valid, valid], { maximumEntries: 2, maximumArchiveBytes: 4_096 }),
    /paths must be unique/,
  );
  assert.throws(
    () => encodeVisualArchive([{ ...valid, bytes: [1] }], { maximumEntries: 2, maximumArchiveBytes: 4_096 }),
    /must be Uint8Array/,
  );
  assert.throws(
    () => encodeVisualArchive([valid], { maximumEntries: 2, maximumArchiveBytes: 1_024 }),
    /byte length exceeds its ceiling/,
  );
});

function readTar(bytes) {
  const entries = [];
  let offset = 0;
  while (bytes.slice(offset, offset + 512).some((byte) => byte !== 0)) {
    const header = bytes.slice(offset, offset + 512);
    const name = readAscii(header, 0, 100);
    const prefix = readAscii(header, 345, 155);
    const path = prefix.length === 0 ? name : `${prefix}/${name}`;
    const size = Number.parseInt(readAscii(header, 124, 12), 8);
    assert.equal(readAscii(header, 257, 6), "ustar");
    assert.equal(headerChecksum(header), Number.parseInt(readAscii(header, 148, 8), 8));
    offset += 512;
    entries.push({ path, bytes: bytes.slice(offset, offset + size) });
    offset += Math.ceil(size / 512) * 512;
  }
  assert(bytes.slice(offset).every((byte) => byte === 0));
  return entries;
}

function readAscii(bytes, offset, length) {
  const field = bytes.slice(offset, offset + length);
  const terminator = field.indexOf(0);
  return String.fromCharCode(...field.slice(0, terminator < 0 ? field.length : terminator)).trimEnd();
}

function headerChecksum(header) {
  return header.reduce(
    (total, byte, index) => total + (index >= 148 && index < 156 ? 0x20 : byte),
    0,
  );
}
