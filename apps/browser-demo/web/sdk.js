import initializeWasm, {
  createViewer as createRawViewer,
} from "./pkg/browser_demo.js";
import {
  DISPLAY_MODES,
  VIEWER_ERROR_CODES,
  ViewerError,
  createBrowserViewer,
} from "./viewer-api.js";
import { createWasmModuleLoader } from "./wasm-loader.js";

const DEFAULT_WASM_URL = new URL("./pkg/browser_demo_bg.wasm", import.meta.url);
const DEFAULT_WORKER_URL = new URL("./stream-worker.js", import.meta.url);
const MAX_CACHE_KEY_CHARACTERS = 128;
const PRODUCTION_BUNDLE = typeof import.meta.env !== "undefined" && import.meta.env.PROD;

const loadBindings = createWasmModuleLoader({
  createRawViewer,
  initializeWasm,
  ViewerError,
});

export { DISPLAY_MODES, VIEWER_ERROR_CODES, ViewerError };

/**
 * Resolves the two deployable SDK assets without assuming a public path.
 * Explicit URLs support CDNs and copied assets; the defaults remain visible to
 * standards-aware bundlers through static `new URL(..., import.meta.url)` calls.
 */
export function resolveViewerAssets(options = {}) {
  const cacheKey = cacheKeyInput(options.cacheKey);
  return Object.freeze({
    wasmUrl: versionedUrl(options.wasmUrl ?? DEFAULT_WASM_URL, cacheKey),
    workerUrl: versionedUrl(options.workerUrl ?? DEFAULT_WORKER_URL, cacheKey),
  });
}

/** Creates one independently disposable viewer from the packaged Wasm module. */
export async function createViewer(options) {
  if (!options || typeof options !== "object") {
    throw new ViewerError("invalid_argument", "viewer options are required");
  }
  const assets = resolveViewerAssets(options.assets);
  const bindings = await loadBindings(assets.wasmUrl);
  const defaultWorkerAsset = options.assets?.workerUrl === undefined;
  return createBrowserViewer({
    ...options,
    bindings,
    workerUrl: assets.workerUrl,
    workerFactory: options.workerFactory
      ?? (defaultWorkerAsset ? createBundledWorker : undefined),
  });
}

function createBundledWorker(workerUrl, options) {
  if (PRODUCTION_BUNDLE) {
    return new Worker(new URL("./stream-worker.js", import.meta.url), { type: "module" });
  }
  return new Worker(workerUrl, options);
}

function cacheKeyInput(value) {
  if (value === undefined) return undefined;
  if (typeof value !== "string" || value.length === 0 || value.length > MAX_CACHE_KEY_CHARACTERS) {
    throw new ViewerError(
      "invalid_argument",
      `asset cacheKey must contain 1-${MAX_CACHE_KEY_CHARACTERS} characters`,
    );
  }
  return value;
}

function versionedUrl(value, cacheKey) {
  let url;
  try {
    url = new URL(value, import.meta.url);
  } catch {
    throw new ViewerError("invalid_argument", "SDK asset URLs must be valid absolute or package-relative URLs");
  }
  if (cacheKey !== undefined) url.searchParams.set("punctra-v", cacheKey);
  return url.href;
}
