import initializeWasm, {
  createViewer as createRawViewer,
} from "./pkg/browser_demo.js";
import {
  DISPLAY_MODES,
  VIEWER_ERROR_CODES,
  ViewerError,
  createBrowserViewer,
} from "./viewer-api.js";

const DEFAULT_WASM_URL = new URL("./pkg/browser_demo_bg.wasm", import.meta.url);
const DEFAULT_WORKER_URL = new URL("./stream-worker.js", import.meta.url);
const MAX_CACHE_KEY_CHARACTERS = 128;

let initializedWasmUrl;
let wasmInitialization;

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

async function loadBindings(wasmUrl) {
  const requestedUrl = wasmUrl.href;
  if (initializedWasmUrl !== undefined && initializedWasmUrl !== requestedUrl) {
    throw new ViewerError(
      "invalid_argument",
      "one imported SDK module cannot be initialized from two different Wasm asset URLs",
      {
        safeAction: "Reuse the first Wasm URL or import an independently versioned SDK module.",
      },
    );
  }
  if (!wasmInitialization) {
    initializedWasmUrl = requestedUrl;
    wasmInitialization = initializeWasm({ module_or_path: wasmUrl }).catch((error) => {
      initializedWasmUrl = undefined;
      wasmInitialization = undefined;
      throw error;
    });
  }
  try {
    await wasmInitialization;
  } catch (error) {
    throw error instanceof ViewerError
      ? error
      : new ViewerError("internal", error?.message ?? "WebAssembly initialization failed", {
          safeAction: "Verify the Wasm asset URL, MIME type, Content Security Policy, and response body before retrying.",
        });
  }
  return { createViewer: createRawViewer };
}

function createBundledWorker(_workerUrl, _options) {
  return new Worker(new URL("./stream-worker.js", import.meta.url), { type: "module" });
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
  return url;
}
