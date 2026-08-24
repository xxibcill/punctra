const MAX_POINTERS = 2;
const MAX_KEY_CODE_CHARACTERS = 64;
const PIXELS_PER_WHEEL_LINE = 100;
const PAGE_WHEEL_LINES = 10;
const MAX_ABSOLUTE_WHEEL_LINES = 100;

export function createInputNormalizer(target, listener, options = {}) {
  if (typeof target?.addEventListener !== "function"
    || typeof target?.removeEventListener !== "function") {
    throw new TypeError("input target must support event listeners");
  }
  if (typeof listener !== "function") throw new TypeError("input listener must be a function");
  const preventDefault = options.preventDefault === true;
  const pointers = new Map();
  let disposed = false;

  const emit = (input, event) => {
    if (!input) return;
    if (preventDefault) event.preventDefault?.();
    listener(Object.freeze(input));
  };

  const pointerDown = (event) => {
    if (pointers.size >= MAX_POINTERS || !finitePointer(event)) return;
    pointers.set(event.pointerId, pointerPosition(event));
    target.setPointerCapture?.(event.pointerId);
  };

  const pointerMove = (event) => {
    const previous = pointers.get(event.pointerId);
    if (!previous || !finitePointer(event)) return;
    const previousPair = pairFacts(pointers);
    const current = pointerPosition(event);
    pointers.set(event.pointerId, current);
    if (pointers.size === 1) {
      const kind = event.shiftKey || (event.buttons & 4) !== 0 ? "pan" : "orbit";
      emit(pointerDelta(kind, previous, current, event.pointerType), event);
      return;
    }
    const currentPair = pairFacts(pointers);
    emit(pointerDelta("pan", previousPair.center, currentPair.center, "touch"), event);
    if (previousPair.distance > 0 && currentPair.distance > 0) {
      const delta = Math.log(previousPair.distance / currentPair.distance) / 0.12;
      if (Number.isFinite(delta) && delta !== 0) {
        emit({ kind: "zoom", delta, source: "touch" }, event);
      }
    }
  };

  const pointerEnd = (event) => {
    pointers.delete(event.pointerId);
    target.releasePointerCapture?.(event.pointerId);
  };

  const wheel = (event) => {
    if (!Number.isFinite(event.deltaY)) return;
    const delta = wheelLines(event.deltaY, event.deltaMode);
    if (delta !== 0) emit({ kind: "zoom", delta, source: "wheel" }, event);
  };

  const keyDown = (event) => {
    if (typeof event.code !== "string" || event.code.length === 0) return;
    emit({
      kind: "keyboard",
      code: event.code.slice(0, MAX_KEY_CODE_CHARACTERS),
      repeat: event.repeat === true,
      modifiers: Object.freeze({
        alt: event.altKey === true,
        control: event.ctrlKey === true,
        meta: event.metaKey === true,
        shift: event.shiftKey === true,
      }),
    }, event);
  };

  const listeners = [
    ["pointerdown", pointerDown],
    ["pointermove", pointerMove],
    ["pointerup", pointerEnd],
    ["pointercancel", pointerEnd],
    ["wheel", wheel],
    ["keydown", keyDown],
  ];
  for (const [type, callback] of listeners) {
    target.addEventListener(type, callback, type === "wheel" ? { passive: !preventDefault } : undefined);
  }

  return Object.freeze({
    dispose() {
      if (disposed) return;
      disposed = true;
      for (const [type, callback] of listeners) target.removeEventListener(type, callback);
      pointers.clear();
    },
  });
}

function pointerDelta(kind, previous, current, source) {
  const deltaX = current.x - previous.x;
  const deltaY = current.y - previous.y;
  return deltaX === 0 && deltaY === 0
    ? undefined
    : { kind, deltaX, deltaY, source: source || "pointer" };
}

function finitePointer(event) {
  return Number.isFinite(event.clientX)
    && Number.isFinite(event.clientY)
    && Number.isSafeInteger(event.pointerId);
}

function pointerPosition(event) {
  return { x: event.clientX, y: event.clientY };
}

function pairFacts(pointers) {
  const [first, second] = [...pointers.values()];
  if (!second) return { center: first, distance: 0 };
  return {
    center: { x: (first.x + second.x) / 2, y: (first.y + second.y) / 2 },
    distance: Math.hypot(second.x - first.x, second.y - first.y),
  };
}

function wheelLines(delta, mode) {
  const lines = mode === 1
    ? delta
    : mode === 2
      ? delta * PAGE_WHEEL_LINES
      : delta / PIXELS_PER_WHEEL_LINE;
  return Math.max(-MAX_ABSOLUTE_WHEEL_LINES, Math.min(MAX_ABSOLUTE_WHEEL_LINES, lines));
}
