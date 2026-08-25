import assert from "node:assert/strict";
import test from "node:test";

import { applyViewerUpdate, startViewerLifecycle } from "./lifecycle.js";

test("abandoned asynchronous mounts dispose every late viewer", async () => {
  const resolvers = [];
  const viewers = [];
  const publications = [];
  const lifecycles = [];
  const createViewer = () => new Promise((resolve) => resolvers.push(resolve));

  for (let index = 0; index < 64; index += 1) {
    const lifecycle = startViewerLifecycle(createViewer, { index }, (value) => publications.push(value));
    lifecycles.push(lifecycle);
    lifecycle.dispose();
  }
  await Promise.resolve();
  assert.equal(resolvers.length, 64);
  for (const resolve of resolvers) {
    const viewer = fakeViewer();
    viewers.push(viewer);
    resolve(viewer);
  }
  await Promise.all(lifecycles.map((lifecycle) => lifecycle.ready));

  assert.equal(viewers.every((viewer) => viewer.disposeCalls === 1), true);
  assert.equal(viewers.every((viewer) => viewer.subscribeCalls === 0), true);
  assert.equal(publications.filter((value) => value.status === "loading").length, 64);
  assert.equal(publications.some((value) => value.status === "ready"), false);
});

test("mounted lifecycle unsubscribes before idempotent viewer disposal", async () => {
  const order = [];
  const viewer = fakeViewer(order);
  const publications = [];
  const lifecycle = startViewerLifecycle(async () => viewer, {}, (value) => publications.push(value));
  assert.equal(await lifecycle.ready, viewer);

  lifecycle.dispose();
  lifecycle.dispose();

  assert.deepEqual(order, ["subscribe", "unsubscribe", "dispose"]);
  assert.equal(publications.at(-1).status, "ready");
});

test("viewer updates publish one consistent failure binding", () => {
  const error = new Error("resize failed");
  const viewer = { state: () => ({ lifecycle: "ready" }) };
  const publications = [];

  applyViewerUpdate(viewer, () => {
    throw error;
  }, (value) => publications.push(value));

  assert.deepEqual(publications, [{
    status: "failed",
    viewer,
    state: { lifecycle: "ready" },
    error,
  }]);
});

function fakeViewer(order = []) {
  return {
    subscribeCalls: 0,
    disposeCalls: 0,
    subscribe(listener) {
      this.subscribeCalls += 1;
      order.push("subscribe");
      listener({ lifecycle: "ready" });
      return () => order.push("unsubscribe");
    },
    dispose() {
      this.disposeCalls += 1;
      order.push("dispose");
    },
  };
}
