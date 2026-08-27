import assert from "node:assert/strict";
import test from "node:test";

import { mapClientPointToViewport, viewportFromCanvasBounds } from "./viewport.ts";

test("canvas bounds remain the authoritative CSS viewport", () => {
  assert.deepEqual(
    viewportFromCanvasBounds({ left: 0, top: 0, width: 1_043, height: 652 }, 2),
    { cssWidth: 1_043, cssHeight: 652, devicePixelRatio: 2 },
  );
  assert.deepEqual(
    viewportFromCanvasBounds({ left: 0, top: 0, width: 219.5, height: 137.25 }, 1.25),
    { cssWidth: 219.5, cssHeight: 137.25, devicePixelRatio: 1.25 },
  );
});

test("client points map through the rendered-to-physical viewport ratio", () => {
  const bounds = { left: 10, top: 20, width: 1_043, height: 652 };
  const viewport = { physicalWidth: 960, physicalHeight: 600 };

  assert.deepEqual(
    mapClientPointToViewport(bounds, viewport, 531.5, 346),
    [480, 300],
  );
  assert.deepEqual(
    mapClientPointToViewport(bounds, viewport, 1_053, 672),
    [959, 599],
  );
  assert.deepEqual(
    mapClientPointToViewport(bounds, viewport, 9, 19),
    [0, 0],
  );
});

test("invalid canvas and viewport dimensions fail before coordinate mapping", () => {
  assert.throws(
    () => viewportFromCanvasBounds({ left: 0, top: 0, width: 0, height: 600 }, 1),
    /canvas width/,
  );
  assert.throws(
    () => viewportFromCanvasBounds({ left: 0, top: 0, width: 960, height: 600 }, Number.NaN),
    /device pixel ratio/,
  );
  assert.throws(
    () => mapClientPointToViewport(
      { left: 0, top: 0, width: 960, height: 600 },
      { physicalWidth: 0, physicalHeight: 600 },
      10,
      10,
    ),
    /physical width/,
  );
});
