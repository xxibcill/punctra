import type {
  BrowserViewer,
  DisplayMode,
  ExactPoint,
  ExactQueryBridge,
  OrthographicCamera,
  PerspectiveCamera,
  PointIdentity,
  ProvisionalPick,
  SourceLoadResult,
  ViewerCamera,
  ViewerErrorCode,
  ViewerState,
  ViewportInput,
} from "./viewer-api.js";

export {
  DISPLAY_MODES,
  VIEWER_ERROR_CODES,
  ViewerError,
} from "./viewer-api.js";

export type {
  BrowserViewer,
  DisplayMode,
  ExactPoint,
  ExactQueryBridge,
  OrthographicCamera,
  PerspectiveCamera,
  PointIdentity,
  ProvisionalPick,
  SourceLoadResult,
  ViewerCamera,
  ViewerErrorCode,
  ViewerState,
  ViewportInput,
};

export interface ViewerAssetOptions {
  /** Override the packaged Wasm URL, for example when copying assets to a CDN. */
  readonly wasmUrl?: string | URL;
  /** Override the packaged module-Worker URL and opt out of bundler Worker discovery. */
  readonly workerUrl?: string | URL;
  /** Optional bounded query token for explicitly copied, non-hashed assets. */
  readonly cacheKey?: string;
}

export interface ViewerAssetUrls {
  readonly wasmUrl: string;
  readonly workerUrl: string;
}

export interface CreateViewerOptions {
  readonly canvas: HTMLCanvasElement;
  readonly viewport: ViewportInput;
  readonly exactQueryBridge?: ExactQueryBridge;
  readonly WorkerConstructor?: typeof Worker;
  readonly requestAnimationFrame?: typeof requestAnimationFrame;
  readonly cancelAnimationFrame?: typeof cancelAnimationFrame;
  readonly assets?: ViewerAssetOptions;
}

/** Resolve package-relative or explicitly deployed Wasm and Worker assets. */
export function resolveViewerAssets(options?: ViewerAssetOptions): ViewerAssetUrls;

/** Create one lifecycle-safe viewer. Call `dispose()` during host teardown. */
export function createViewer(options: CreateViewerOptions): Promise<BrowserViewer>;
