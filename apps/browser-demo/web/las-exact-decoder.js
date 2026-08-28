import { ExactQueryError } from "./exact-query-error.js";

const LAS_HEADER_BYTES = 256;
const LAS_POINT_FORMAT = 3;
const LAS_POINT_RECORD_BYTES = 34;

export function decodeLasLayout(bytes, deployment) {
  const view = dataView(bytes, LAS_HEADER_BYTES, "LAS Source probe");
  if (ascii(bytes, 0, 4) !== "LASF") {
    throw new ExactQueryError("exact_query_incompatible", "Source probe has no LAS signature");
  }
  if (view.getUint8(24) !== 1 || view.getUint8(25) !== 2) {
    throw new ExactQueryError("exact_query_incompatible", "exact bridge requires LAS 1.2");
  }
  const headerBytes = view.getUint16(94, true);
  const pointDataOffset = view.getUint32(96, true);
  const pointFormat = view.getUint8(104);
  const pointRecordBytes = view.getUint16(105, true);
  const pointCount = view.getUint32(107, true);
  if (headerBytes > bytes.byteLength || pointDataOffset < headerBytes) {
    throw new ExactQueryError("exact_query_incompatible", "LAS point-data offset is invalid");
  }
  if (pointFormat !== LAS_POINT_FORMAT || pointRecordBytes !== LAS_POINT_RECORD_BYTES) {
    throw new ExactQueryError(
      "exact_query_incompatible",
      "exact bridge requires uncompressed LAS point format 3 with 34-byte records",
    );
  }
  if (pointCount !== deployment.source.pointCount) {
    throw new ExactQueryError("exact_query_source_changed", "LAS Point count differs from deployment");
  }
  const scale = [view.getFloat64(131, true), view.getFloat64(139, true), view.getFloat64(147, true)];
  const offset = [view.getFloat64(155, true), view.getFloat64(163, true), view.getFloat64(171, true)];
  if (!scale.every((value) => Number.isFinite(value) && value > 0) || !offset.every(Number.isFinite)) {
    throw new ExactQueryError("exact_query_incompatible", "LAS position transform is invalid");
  }
  if (!sameTriple(scale, deployment.index.transform.scale)
    || !sameTriple(offset, deployment.index.transform.offset)) {
    throw new ExactQueryError("exact_query_source_changed", "LAS transform differs from deployment");
  }
  if (pointDataOffset + pointCount * pointRecordBytes > deployment.source.byteLength) {
    throw new ExactQueryError("exact_query_incompatible", "LAS Point records exceed Source length");
  }
  return Object.freeze({
    pointDataOffset,
    pointFormat,
    pointRecordBytes,
    pointCount,
    scale: Object.freeze(scale),
    offset: Object.freeze(offset),
  });
}

export function decodeLasPointRecord(
  bytes,
  sourceIdentity,
  pointOrdinal,
  generation,
  layout,
) {
  const view = dataView(bytes, layout.pointRecordBytes, "LAS Point record");
  const ticks = [view.getInt32(0, true), view.getInt32(4, true), view.getInt32(8, true)];
  const position = ticks.map((tick, axis) => layout.offset[axis] + tick * layout.scale[axis]);
  if (!position.every(Number.isFinite)) {
    throw new ExactQueryError("exact_query_corrupt", "exact Point position is non-finite");
  }
  return deepFreeze({
    authority: "exact_source_record",
    sourceIdentity,
    pointOrdinal: String(pointOrdinal),
    generation,
    ticks,
    position,
    intensity: view.getUint16(12, true),
    classification: view.getUint8(15),
    rgb: [
      view.getUint16(28, true),
      view.getUint16(30, true),
      view.getUint16(32, true),
    ],
  });
}

function dataView(bytes, expectedLength, label) {
  if (!(bytes instanceof Uint8Array) || bytes.byteLength !== expectedLength) {
    throw new ExactQueryError("exact_query_truncated", `${label} must contain exactly ${expectedLength} bytes`);
  }
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function ascii(bytes, offset, length) {
  return String.fromCharCode(...bytes.subarray(offset, offset + length));
}

function sameTriple(left, right) {
  return left.every((value, index) => Object.is(value, right[index]));
}

function deepFreeze(value) {
  for (const child of Object.values(value)) {
    if (child && typeof child === "object") Object.freeze(child);
  }
  return Object.freeze(value);
}
