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
  const source = await readFile(new URL("sdk.js", import.meta.url), "utf8");
  const sdk = await import("./sdk.js");
  const assets = resolveViewerAssets();

  assert.equal(packageManifest.name, "@punctra/viewer");
  assert.equal(packageManifest.version, "0.18.0-alpha.1");
  assert.deepEqual(DISPLAY_MODES, ["neutral", "elevation", "rgb", "intensity", "classification"]);
  assert.equal(typeof assets.wasmUrl, "string");
  assert.equal(typeof assets.workerUrl, "string");
  assert.equal(new URL(assets.wasmUrl).pathname.endsWith("/pkg/browser_demo_bg.wasm"), true);
  assert.equal(new URL(assets.workerUrl).pathname.endsWith("/stream-worker.js"), true);
  assert.equal(Object.isFrozen(assets), true);
  assert.deepEqual(Object.keys(sdk).sort(), [
    "DISPLAY_MODES",
    "VIEWER_ERROR_CODES",
    "ViewerError",
    "createViewer",
    "resolveViewerAssets",
  ]);
  assert.match(declaration, /createViewer\(options: CreateViewerOptions\)/);
  assert.doesNotMatch(declaration, /createInputNormalizer|createLasExactQueryBridge/);
  assert.doesNotMatch(declaration, /createBrowserViewer/);
  assert.doesNotMatch(declaration, /workerFactory/);
  assert.match(
    source,
    /const DEFAULT_WORKER_URL = new URL\("\.\/stream-worker\.js", import\.meta\.url\);/,
  );
  assert.doesNotMatch(source, /stream-worker\.js\?worker&url/);
});

test("explicit SDK assets preserve deployment URLs and bounded cache busting", () => {
  const assets = resolveViewerAssets({
    wasmUrl: "https://cdn.test/punctra.wasm?immutable=1",
    workerUrl: "https://cdn.test/punctra-worker.js?immutable=1",
    cacheKey: "release-42",
  });

  assert.equal(new URL(assets.wasmUrl).searchParams.get("immutable"), "1");
  assert.equal(new URL(assets.wasmUrl).searchParams.get("punctra-v"), "release-42");
  assert.equal(new URL(assets.workerUrl).searchParams.get("punctra-v"), "release-42");
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
  assert.match(
    workerSource,
    /import\(`\.\/module-loader\.js\?punctra-v=\$\{WORKER_CACHE_TOKEN\}`\)/,
  );
  assert.doesNotMatch(workerSource, /from "\.\/module-loader\.js"/);
  assert.doesNotMatch(workerSource, /searchParams\.get\("v"\)/);
});
