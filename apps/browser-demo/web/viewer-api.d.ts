export type DisplayMode =
  | "neutral"
  | "elevation"
  | "rgb"
  | "intensity"
  | "classification";

export type ViewerErrorCode =
  | "invalid_argument"
  | "viewer_destroyed"
  | "load_busy"
  | "render_cancelled"
  | "internal"
  | "capability_inspection"
  | "canvas_surface"
  | "device_lost"
  | "device_poll"
  | "diagnostic_serialization"
  | "frame_recording"
  | "frame_validation"
  | "camera_validation"
  | "host_model"
  | "initial_viewport"
  | "insecure_context"
  | "missing_recorded_frame"
  | "missing_window"
  | "pick_invariant"
  | "pick_not_requested"
  | "pick_outside_viewport"
  | "pick_pending"
  | "pick_readback"
  | "pick_recording"
  | "highlight_validation"
  | "presentation_mode"
  | "renderer_capability"
  | "resize_viewport"
  | "scene_planning"
  | "scene_publication"
  | "scene_validation"
  | "stream_publication"
  | "stream_validation"
  | "stale_generation"
  | "display_mode"
  | "surface_alpha_mode"
  | "surface_configuration"
  | "surface_format"
  | "surface_lost"
  | "surface_occluded"
  | "surface_outdated"
  | "surface_reconfiguration"
  | "surface_timeout"
  | "surface_validation"
  | "transient_texture_limit"
  | "viewer_hidden"
  | "viewport_validation"
  | "webgpu_adapter"
  | "webgpu_device"
  | "webgpu_unavailable"
  | "manifest_invalid"
  | "unsupported_deployment"
  | "range_unsupported"
  | "cors_headers_hidden"
  | "content_encoding"
  | "source_changed"
  | "range_truncated"
  | "range_corrupt"
  | "index_incompatible"
  | "offline"
  | "retry_exhausted"
  | "cache_quota"
  | "cache_unavailable"
  | "cancelled"
  | "worker_failed"
  | "resource_limit"
  | "exact_query_invalid"
  | "exact_query_unavailable"
  | "exact_query_busy"
  | "exact_query_cancelled"
  | "exact_query_source_mismatch"
  | "exact_query_source_changed"
  | "exact_query_incompatible"
  | "exact_query_corrupt"
  | "exact_query_truncated"
  | "exact_query_range_unsupported"
  | "exact_query_content_encoding"
  | "exact_query_failed";

export const DISPLAY_MODES: readonly DisplayMode[];
export const VIEWER_ERROR_CODES: readonly ViewerErrorCode[];

export class ViewerError extends Error {
  readonly schema: "punctra-viewer-error-v1";
  readonly code: ViewerErrorCode;
  readonly safeAction: string;
  readonly recoverable: boolean;
  constructor(
    code: ViewerErrorCode,
    message: string,
    options?: { cause?: unknown; safeAction?: string; recoverable?: boolean },
  );
}

export interface ViewportInput {
  readonly cssWidth: number;
  readonly cssHeight: number;
  readonly devicePixelRatio: number;
}

interface CameraBase {
  readonly eye: readonly [number, number, number];
  readonly target: readonly [number, number, number];
  readonly up: readonly [number, number, number];
  readonly nearDistance: number;
  readonly farDistance: number;
}

export interface PerspectiveCamera extends CameraBase {
  readonly projection: "perspective";
  readonly verticalFieldOfViewRadians: number;
}

export interface OrthographicCamera extends CameraBase {
  readonly projection: "orthographic";
  readonly verticalWorldHeight: number;
}

export type ViewerCamera = PerspectiveCamera | OrthographicCamera;

export interface PointIdentity {
  readonly sourceIdentity: string;
  readonly pointOrdinal: bigint | string | number;
}

export interface ProvisionalPick extends PointIdentity {
  readonly status: "hit";
  readonly authority: "provisional_gpu_hint";
  readonly generation: number;
  readonly batchKey: number;
  readonly batchVersion: number;
}

export interface ExactPoint extends PointIdentity {
  readonly authority: "exact_source_record";
  readonly pointOrdinal: string;
  readonly generation: number;
  readonly ticks: readonly [number, number, number];
  readonly position: readonly [number, number, number];
  readonly intensity: number;
  readonly classification: number;
  readonly rgb: readonly [number, number, number];
}

export interface ExactQueryBridge {
  confirm(request: {
    readonly sourceIdentity: string;
    readonly pointOrdinal: bigint;
    readonly generation: number;
    readonly signal?: AbortSignal;
  }): Promise<ExactPoint>;
}

export interface ViewerState {
  readonly schema: "punctra-viewer-state-v1";
  readonly packageVersion: string;
  readonly lifecycle: "ready" | "hidden" | "destroyed";
  readonly generation: number;
  readonly source: Readonly<{
    identity: string | null;
    coverage: string;
    expectedPoints: number;
    publishedPoints: number;
    publishedBatches: number;
    retainedRecordBytes: number;
  }>;
  readonly viewport: Readonly<{
    cssWidth: number;
    cssHeight: number;
    devicePixelRatio: number;
    physicalWidth: number;
    physicalHeight: number;
    surfaceBytes: number;
  }>;
  readonly camera: Readonly<ViewerCamera & {
    verticalFieldOfViewRadians: number | null;
    verticalWorldHeight: number | null;
  }>;
  readonly displayMode: DisplayMode;
  readonly render: Readonly<{
    scheduled: boolean;
    renderedFrames: number;
    hiddenFrameSkips: number;
    drawnPoints: number;
    drawCalls: number;
    residentBytes: number;
    transientTextureBytes: number;
    surfaceSuboptimal: boolean;
  }>;
  readonly pick: Readonly<{
    status: "not_requested" | "pending" | "miss" | "hit";
    authority: "provisional_gpu_hint";
    generation: number | null;
    sourceIdentity: string | null;
    pointOrdinal: string | null;
    batchKey: number | null;
    batchVersion: number | null;
  }>;
  readonly highlights: Readonly<{
    generation: number | null;
    sourceIdentity: string | null;
    pointCount: number;
    authority: "presentation_only";
  }>;
  readonly resources: Readonly<{
    pointLimit: number;
    batchLimit: number;
    highlightPointLimit: number;
    residentByteLimit: number;
    retainedRecordByteLimit: number;
    workerStagingByteLimit: number;
  }>;
  readonly capabilities: Readonly<Record<string, unknown>>;
  readonly load: Readonly<{ active: boolean; facts: Readonly<Record<string, unknown>> | null }>;
  readonly failure: Readonly<{
    code: ViewerErrorCode;
    message: string;
    safeAction: string;
    recoverable: boolean;
  }> | null;
}

export interface SourceLoadResult {
  readonly deployment: Readonly<Record<string, unknown>>;
  readonly metrics: Readonly<Record<string, number>>;
  readonly decode: Readonly<Record<string, number>>;
  readonly pointOrdinals: readonly number[];
  readonly mainThreadMillisecondsHighWater: number;
  readonly state: ViewerState;
}

export interface BrowserViewerOptions {
  readonly bindings: { createViewer(...arguments_: unknown[]): Promise<unknown> };
  readonly canvas: HTMLCanvasElement;
  readonly viewport: ViewportInput;
  readonly exactQueryBridge?: ExactQueryBridge;
  readonly WorkerConstructor?: typeof Worker;
  readonly workerUrl?: string | URL;
  readonly requestAnimationFrame?: typeof requestAnimationFrame;
  readonly cancelAnimationFrame?: typeof cancelAnimationFrame;
}

export function createBrowserViewer(options: BrowserViewerOptions): Promise<BrowserViewer>;

export class BrowserViewer {
  state(): ViewerState;
  subscribe(listener: (state: ViewerState) => void): () => boolean;
  resize(viewport: ViewportInput): ViewerState;
  setVisible(visible: boolean): ViewerState;
  setCamera(camera: ViewerCamera): ViewerState;
  setDisplayMode(mode: DisplayMode): ViewerState;
  render(): ViewerState;
  requestRender(): Promise<ViewerState>;
  loadSource(options: {
    readonly manifestUrl: string;
    readonly cacheMode?: "none" | "memory" | "persistent";
    readonly credentials?: RequestCredentials;
    readonly invalidate?: boolean;
    readonly signal?: AbortSignal;
  }): Promise<SourceLoadResult>;
  pick(request: { readonly x: number; readonly y: number; readonly signal?: AbortSignal }): Promise<ProvisionalPick | null>;
  setHighlights(points: readonly PointIdentity[], generation?: number): ViewerState;
  clearHighlights(generation?: number): ViewerState;
  confirmPoint(point: PointIdentity & { readonly generation?: number }, options?: { readonly signal?: AbortSignal }): Promise<ExactPoint>;
  destroy(): void;
}
