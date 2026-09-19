import assert from "node:assert/strict";
import test from "node:test";

import {
  ArtifactRegistry,
  encodePointFootprintArchive,
} from "./footprint-artifacts.js";
import { sha256Hex } from "./visual-png.js";

test("artifact registry retains ordered bytes and reports exact metadata", async () => {
  const added = [];
  const registry = new ArtifactRegistry((artifact) => added.push(artifact));
  const jsonBytes = new TextEncoder().encode("{\"passed\":true}\n");
  const jsonMetadata = await registry.addBytes(
    "docs/releases/evidence.json",
    jsonBytes,
    "evidence_json",
  );
  const png = await registry.addPng(
    { width: 1, height: 1, data: Uint8Array.of(10, 20, 30, 255) },
    {
      kind: "canonical_candidate_png",
      path: "docs/releases/candidate.png",
      trial_id: "trial-a",
      recreation_index: 0,
      frame_index: null,
    },
  );

  assert.deepEqual(registry.entries(), [
    { path: "docs/releases/evidence.json", bytes: jsonBytes },
    { path: "docs/releases/candidate.png", bytes: png.bytes },
  ]);
  assert.deepEqual(added, [
    { path: "docs/releases/evidence.json", bytes: jsonBytes, metadata: jsonMetadata },
    { path: "docs/releases/candidate.png", bytes: png.bytes, metadata: png.metadata },
  ]);
  assert.deepEqual(registry.metadata(), [jsonMetadata, png.metadata]);

  const copiedMetadata = registry.metadata();
  copiedMetadata[0].kind = "tampered";
  assert.equal(registry.metadata()[0].kind, "evidence_json");
});

test("artifact registry rejects a duplicate path without publishing it", async () => {
  const added = [];
  const registry = new ArtifactRegistry((artifact) => added.push(artifact));
  const path = "docs/releases/evidence.json";
  await registry.addBytes(path, Uint8Array.of(1), "evidence_json");

  await assert.rejects(
    registry.addBytes(path, Uint8Array.of(2), "evidence_json"),
    /artifact path docs\/releases\/evidence\.json is duplicated/,
  );
  assert.equal(added.length, 1);
  assert.deepEqual(registry.entries(), [{ path, bytes: Uint8Array.of(1) }]);
});

test("point-footprint archive is deterministic, sorted, hashed, and entry-bounded", async () => {
  const entries = [
    { path: "docs/releases/z.json", bytes: Uint8Array.of(9, 8, 7) },
    { path: "docs/releases/a.png", bytes: Uint8Array.of(1, 2, 3, 4) },
  ];
  const first = await encodePointFootprintArchive(entries);
  const second = await encodePointFootprintArchive([...entries].reverse());

  assert.deepEqual(second, first);
  assert.deepEqual(first.facts.paths, [
    "docs/releases/a.png",
    "docs/releases/z.json",
  ]);
  assert.equal(first.sha256, await sha256Hex(first.bytes));
  assert.match(first.sha256, /^[0-9a-f]{64}$/);

  const excessiveEntries = Array.from({ length: 129 }, (_, index) => ({
    path: `docs/releases/${index}.json`,
    bytes: Uint8Array.of(index % 256),
  }));
  await assert.rejects(
    encodePointFootprintArchive(excessiveEntries),
    /archive entry count exceeds its ceiling/,
  );
});
