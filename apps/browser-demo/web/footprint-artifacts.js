import { encodeVisualArchive } from "./visual-archive.js";
import {
  createPngArtifactMetadata,
  encodeRgba8Png,
  sha256Hex,
} from "./visual-png.js";
import { createVisualValidator } from "./visual-validation.js";

const MAX_ARCHIVE_ENTRIES = 128;
const MAX_ARCHIVE_BYTES = 134_217_728;
const { requireCondition } = createVisualValidator("Point-footprint runner failed");

export class ArtifactRegistry {
  #artifactAdded;
  #entries = [];
  #metadata = [];
  #paths = new Set();

  constructor(artifactAdded) {
    requireCondition(typeof artifactAdded === "function", "artifact callback is invalid");
    this.#artifactAdded = artifactAdded;
  }

  async addPng(image, descriptor) {
    const bytes = await encodeRgba8Png(image);
    const metadata = await createPngArtifactMetadata({ descriptor, encodedBytes: bytes, image });
    this.#add(descriptor.path, bytes, metadata);
    return { bytes, metadata };
  }

  async addBytes(path, bytes, kind) {
    const metadata = {
      kind,
      path,
      mime_type: path.endsWith(".json") ? "application/json" : "application/octet-stream",
      encoded_byte_length: bytes.byteLength,
      encoded_sha256: await sha256Hex(bytes),
      authority: "release_evidence",
    };
    this.#add(path, bytes, metadata);
    return metadata;
  }

  #add(path, bytes, metadata) {
    requireCondition(!this.#paths.has(path), `artifact path ${path} is duplicated`);
    this.#paths.add(path);
    this.#entries.push({ path, bytes });
    this.#metadata.push(metadata);
    this.#artifactAdded({ path, bytes, metadata });
  }

  entries() {
    return [...this.#entries];
  }

  metadata() {
    return structuredClone(this.#metadata);
  }
}

export async function encodePointFootprintArchive(entries) {
  const archive = encodeVisualArchive(entries, {
    maximumEntries: MAX_ARCHIVE_ENTRIES,
    maximumArchiveBytes: MAX_ARCHIVE_BYTES,
  });
  return {
    bytes: archive.bytes,
    sha256: await sha256Hex(archive.bytes),
    facts: archive.facts,
  };
}
