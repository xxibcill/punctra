import assert from "node:assert/strict";
import test from "node:test";

import {
  appendTransferredOrdinals,
  samePointOrdinals,
} from "./stream-ordinals.js";

test("cold and warm ordinal sequences compare exactly", () => {
  const cold = [];
  appendTransferredOrdinals(cold, payload([2n, 5n]));
  appendTransferredOrdinals(cold, payload([9n]));

  const warm = [];
  appendTransferredOrdinals(warm, payload([2n, 5n, 9n]));

  assert.deepEqual(cold, [2, 5, 9]);
  assert.equal(samePointOrdinals(cold, warm), true);
  assert.equal(samePointOrdinals(cold, [2, 6, 9]), false);
  assert.equal(samePointOrdinals(cold, [2, 5]), false);
  assert.throws(
    () => appendTransferredOrdinals([], payload([BigInt(Number.MAX_SAFE_INTEGER) + 1n])),
    /not safely addressable/,
  );
});

function payload(ordinals) {
  const bytes = new ArrayBuffer(ordinals.length * 32);
  const view = new DataView(bytes);
  ordinals.forEach((ordinal, index) => view.setBigUint64(index * 32, ordinal, true));
  return bytes;
}
