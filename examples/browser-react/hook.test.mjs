import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React, { StrictMode, act } from "react";
import { createRoot } from "react-dom/client";

import { usePunctraViewer } from "@punctra/react";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

test("React hook owns Strict Mode replay and updates without implicit recreation", async () => {
  const viewers = [];
  globalThis.__PUNCTRA_TEST_CREATE_VIEWER__ = async () => {
    const viewer = fakeViewer();
    viewers.push(viewer);
    return viewer;
  };
  const dom = new JSDOM("<main id=trial></main>");
  globalThis.document = dom.window.document;
  globalThis.window = dom.window;
  const canvas = document.createElement("canvas");
  const firstViewport = { cssWidth: 640, cssHeight: 480, devicePixelRatio: 2 };
  const secondViewport = { cssWidth: 800, cssHeight: 600, devicePixelRatio: 1 };
  let binding;
  const root = createRoot(document.querySelector("#trial"));

  function Harness({ options }) {
    binding = usePunctraViewer(options);
    return null;
  }

  await act(async () => {
    root.render(React.createElement(
      StrictMode,
      null,
      React.createElement(Harness, {
        options: { canvas, viewport: firstViewport, active: true, mountKey: "first" },
      }),
    ));
    await settleLifecycle();
  });

  assert.equal(viewers.length, 2);
  assert.equal(viewers[0].disposeCalls, 1);
  const mountedViewer = viewers[1];
  assert.equal(binding.viewer, mountedViewer);

  await act(async () => {
    root.render(React.createElement(
      StrictMode,
      null,
      React.createElement(Harness, {
        options: { canvas, viewport: secondViewport, active: false, mountKey: "first" },
      }),
    ));
    await settleLifecycle();
  });

  assert.equal(viewers.length, 2);
  assert.deepEqual(mountedViewer.resizeCalls.at(-1), secondViewport);
  assert.equal(mountedViewer.pauseCalls > 0, true);

  await act(async () => {
    root.render(React.createElement(
      StrictMode,
      null,
      React.createElement(Harness, {
        options: { canvas, viewport: secondViewport, active: false, mountKey: "second" },
      }),
    ));
    await settleLifecycle();
  });

  assert.equal(viewers.length, 3);
  assert.equal(mountedViewer.disposeCalls, 1);
  const replacementViewer = viewers[2];
  assert.equal(binding.viewer, replacementViewer);

  await act(async () => root.unmount());
  assert.equal(replacementViewer.unsubscribeCalls, 1);
  assert.equal(replacementViewer.disposeCalls, 1);
  delete globalThis.__PUNCTRA_TEST_CREATE_VIEWER__;
  dom.window.close();
});

function fakeViewer() {
  const readyState = { lifecycle: "ready" };
  return {
    disposeCalls: 0,
    pauseCalls: 0,
    resizeCalls: [],
    resumeCalls: 0,
    unsubscribeCalls: 0,
    dispose() {
      this.disposeCalls += 1;
    },
    pause() {
      this.pauseCalls += 1;
    },
    resize(viewport) {
      this.resizeCalls.push(viewport);
    },
    resume() {
      this.resumeCalls += 1;
    },
    state() {
      return readyState;
    },
    subscribe(listener) {
      listener(readyState);
      return () => {
        this.unsubscribeCalls += 1;
      };
    },
  };
}

async function settleLifecycle() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}
