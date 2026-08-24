import assert from "node:assert/strict";
import test from "node:test";

import { createInputNormalizer } from "./viewer-input.js";

test("input normalizer emits bounded policy-free pointer, touch, wheel, and keyboard facts", () => {
  const target = new FakeTarget();
  const inputs = [];
  const normalizer = createInputNormalizer(target, (input) => inputs.push(input), {
    preventDefault: true,
  });

  target.dispatch("pointerdown", pointer(1, 10, 20));
  const orbit = pointer(1, 14, 27);
  target.dispatch("pointermove", orbit);
  target.dispatch("pointerup", pointer(1, 14, 27));
  target.dispatch("pointerdown", pointer(2, 20, 20, { shiftKey: true }));
  target.dispatch("pointermove", pointer(2, 15, 23, { shiftKey: true }));
  target.dispatch("pointerup", pointer(2, 15, 23));

  target.dispatch("pointerdown", pointer(3, 0, 0, { pointerType: "touch" }));
  target.dispatch("pointerdown", pointer(4, 10, 0, { pointerType: "touch" }));
  target.dispatch("pointermove", pointer(4, 20, 0, { pointerType: "touch" }));
  target.dispatch("pointerup", pointer(3, 0, 0, { pointerType: "touch" }));
  target.dispatch("pointerup", pointer(4, 20, 0, { pointerType: "touch" }));
  target.dispatch("wheel", { deltaY: 250, deltaMode: 0 });
  target.dispatch("keydown", { code: "KeyP", shiftKey: true, repeat: false });

  assert.deepEqual(inputs[0], {
    kind: "orbit",
    deltaX: 4,
    deltaY: 7,
    source: "mouse",
  });
  assert.equal(inputs[1].kind, "pan");
  assert.equal(inputs[2].kind, "pan");
  assert.equal(inputs[3].kind, "zoom");
  assert.equal(inputs[3].source, "touch");
  assert.deepEqual(inputs[4], { kind: "zoom", delta: 2.5, source: "wheel" });
  assert.deepEqual(inputs[5], {
    kind: "keyboard",
    code: "KeyP",
    repeat: false,
    modifiers: { alt: false, control: false, meta: false, shift: true },
  });
  assert.equal(orbit.prevented, true);
  assert.equal(Object.isFrozen(inputs[5]), true);
  assert.equal(Object.isFrozen(inputs[5].modifiers), true);

  normalizer.dispose();
  normalizer.dispose();
  target.dispatch("wheel", { deltaY: 100, deltaMode: 0 });
  assert.equal(inputs.length, 6);
});

test("input normalizer retains at most two active pointers", () => {
  const target = new FakeTarget();
  const inputs = [];
  createInputNormalizer(target, (input) => inputs.push(input));
  target.dispatch("pointerdown", pointer(1, 0, 0));
  target.dispatch("pointerdown", pointer(2, 1, 0));
  target.dispatch("pointerdown", pointer(3, 2, 0));
  target.dispatch("pointermove", pointer(3, 4, 0));

  assert.deepEqual(inputs, []);
});

function pointer(pointerId, clientX, clientY, options = {}) {
  return {
    pointerId,
    clientX,
    clientY,
    pointerType: options.pointerType ?? "mouse",
    buttons: options.buttons ?? 1,
    shiftKey: options.shiftKey ?? false,
  };
}

class FakeTarget {
  constructor() {
    this.listeners = new Map();
  }

  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }

  removeEventListener(type, listener) {
    if (this.listeners.get(type) === listener) this.listeners.delete(type);
  }

  dispatch(type, event) {
    event.preventDefault = () => { event.prevented = true; };
    this.listeners.get(type)?.(event);
  }
}
