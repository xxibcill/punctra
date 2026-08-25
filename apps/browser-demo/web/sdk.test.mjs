import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  DISPLAY_MODES,
  ViewerError,
  resolveViewerAssets,
} from "./sdk.js";

test("SDK exports the public viewer surface and package-relative assets", async () => {
  const packageManifest = JSON.parse(await readFile(new URL("package.json", import.meta.url)));
  const declaration = await readFile(new URL("sdk.d.ts", import.meta.url), "utf8");
  const assets = resolveViewerAssets();

  assert.equal(packageManifest.name, "@punctra/viewer");
  assert.equal(packageManifest.version, "0.18.0-alpha.1");
  assert.deepEqual(DISPLAY_MODES, ["neutral", "elevation", "rgb", "intensity", "classification"]);
  assert.equal(assets.wasmUrl.pathname.endsWith("/pkg/browser_demo_bg.wasm"), true);
  assert.equal(assets.workerUrl.pathname.endsWith("/stream-worker.js"), true);
  assert.equal(Object.isFrozen(assets), true);
  assert.match(declaration, /createViewer\(options: CreateViewerOptions\)/);
  assert.doesNotMatch(declaration, /createBrowserViewer/);
});

test("explicit SDK assets preserve deployment URLs and bounded cache busting", () => {
  const assets = resolveViewerAssets({
    wasmUrl: "https://cdn.test/punctra.wasm?immutable=1",
    workerUrl: "https://cdn.test/punctra-worker.js?immutable=1",
    cacheKey: "release-42",
  });

  assert.equal(assets.wasmUrl.searchParams.get("immutable"), "1");
  assert.equal(assets.wasmUrl.searchParams.get("punctra-v"), "release-42");
  assert.equal(assets.workerUrl.searchParams.get("punctra-v"), "release-42");
  assert.throws(
    () => resolveViewerAssets({ cacheKey: "" }),
    (error) => error instanceof ViewerError && error.code === "invalid_argument",
  );
  assert.throws(
    () => resolveViewerAssets({ cacheKey: "x".repeat(129) }),
    (error) => error instanceof ViewerError && error.code === "invalid_argument",
  );
});

test("copied Worker propagates the SDK cache token to its dependencies", async () => {
  const workerSource = await readFile(new URL("stream-worker.js", import.meta.url), "utf8");

  assert.match(workerSource, /searchParams\.get\("punctra-v"\)/);
  assert.doesNotMatch(workerSource, /searchParams\.get\("v"\)/);
});
