const TRANSFER_RECORD_BYTES = 24;

export function appendTransferredOrdinals(target, payload) {
  if (!(payload instanceof ArrayBuffer) || payload.byteLength % TRANSFER_RECORD_BYTES !== 0) {
    throw new TypeError("stream payload must contain complete transfer records");
  }
  const view = new DataView(payload);
  for (let offset = 0; offset < payload.byteLength; offset += TRANSFER_RECORD_BYTES) {
    const ordinal = view.getBigUint64(offset, true);
    if (ordinal > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new RangeError("stream Point ordinal is not safely addressable");
    }
    target.push(Number(ordinal));
  }
}

export function samePointOrdinals(left, right) {
  return left.length === right.length
    && left.every((ordinal, index) => ordinal === right[index]);
}
