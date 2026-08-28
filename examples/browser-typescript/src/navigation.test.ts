import assert from "node:assert/strict";
import test from "node:test";

import type { PerspectiveCamera } from "@punctra/viewer";

import { alternateProjection, applyNavigation } from "./navigation.ts";

const perspective: PerspectiveCamera = {
  projection: "perspective",
  eye: [0, -10, 10],
  target: [0, 0, 0],
  up: [0, 0, 1],
  verticalFieldOfViewRadians: Math.PI / 3,
  nearDistance: 0.1,
  farDistance: 100,
};

test("quickstart navigation preserves complete cameras across every input channel", () => {
  const orbited = applyNavigation(
    perspective,
    { kind: "orbit", deltaX: 5, deltaY: 3, source: "pointer" },
    600,
  );
  assert.equal(orbited?.projection, "perspective");
  assert.notDeepEqual(orbited?.eye, perspective.eye);

  const panned = applyNavigation(
    perspective,
    { kind: "pan", deltaX: 4, deltaY: -2, source: "touch" },
    600,
  );
  assert.notDeepEqual(panned?.target, perspective.target);

  const zoomed = applyNavigation(
    perspective,
    { kind: "zoom", delta: 0.5, source: "wheel" },
    600,
  );
  assert.notDeepEqual(zoomed?.eye, perspective.eye);

  const ignored = applyNavigation(
    perspective,
    {
      kind: "keyboard",
      code: "KeyX",
      repeat: false,
      modifiers: { alt: false, control: false, meta: false, shift: false },
    },
    600,
  );
  assert.equal(ignored, null);
});

test("quickstart projection changes preserve position and clipping", () => {
  const orthographic = alternateProjection(perspective);
  if (orthographic.projection !== "orthographic") assert.fail("expected orthographic camera");
  assert.deepEqual(orthographic.eye, perspective.eye);
  assert.deepEqual(orthographic.target, perspective.target);
  assert.equal(orthographic.nearDistance, perspective.nearDistance);
  assert.equal(orthographic.farDistance, perspective.farDistance);
  assert(orthographic.verticalWorldHeight > 0);

  const restored = alternateProjection(orthographic);
  assert.equal(restored.projection, "perspective");
});
