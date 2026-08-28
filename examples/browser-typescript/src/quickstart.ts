import type {
  BrowserViewer,
  CreateViewerOptions,
  DisplayMode,
  ExactPoint,
  ExactQueryBridge,
  ProvisionalPick,
  ViewerState,
  ViewportInput,
} from "@punctra/viewer";
import type { NormalizedViewerInput } from "@punctra/viewer/input";

import { alternateProjection, applyNavigation, cameraFromState } from "./navigation.ts";

const ACCEPTANCE_QUERY_KEYS = new Set(["acceptance_phase", "delay_ms", "fault"]);
const MANIFEST_URL_BASE = "http://localhost/";

export interface QuickstartSnapshot {
  readonly state: ViewerState | null;
  readonly selectedPoint: ProvisionalPick | null;
  readonly exactPoint: ExactPoint | null;
  readonly operation: string;
}

interface QuickstartOptions {
  readonly canvas: HTMLCanvasElement;
  readonly readViewport: () => ViewportInput;
  readonly manifestUrl: string;
  readonly createViewer: (options: CreateViewerOptions) => Promise<BrowserViewer>;
  readonly createExactBridge: (options: { manifestUrl: string }) => ExactQueryBridge;
  readonly publish: (snapshot: QuickstartSnapshot) => void;
}

export class QuickstartController {
  readonly #options: QuickstartOptions;
  #viewer: BrowserViewer | null = null;
  #unsubscribe: (() => boolean) | null = null;
  #selectedPoint: ProvisionalPick | null = null;
  #exactPoint: ExactPoint | null = null;
  #operation = "Viewer not initialized";
  #mountRevision = 0;
  #asyncRevision = 0;

  constructor(options: QuickstartOptions) {
    this.#options = options;
  }

  state(): ViewerState | null {
    return this.#viewer?.state() ?? null;
  }

  async mount(): Promise<ViewerState> {
    const mountRevision = ++this.#mountRevision;
    this.#releaseViewer();
    this.#selectedPoint = null;
    this.#exactPoint = null;
    this.#operation = "Initializing WebGPU viewer";
    this.#publish();
    const exactQueryBridge = this.#options.createExactBridge({
      manifestUrl: this.#options.manifestUrl,
    });
    const creationViewport = this.#options.readViewport();
    let viewer: BrowserViewer;
    try {
      viewer = await this.#options.createViewer({
        canvas: this.#options.canvas,
        viewport: creationViewport,
        exactQueryBridge,
        assets: { cacheKey: "v0.20-quickstart" },
      });
    } catch (error) {
      if (mountRevision === this.#mountRevision) throw error;
      const activeState = this.#viewer?.state();
      if (activeState) return activeState;
      throw cancelledOperation("Viewer initialization was superseded before publication.");
    }
    if (mountRevision !== this.#mountRevision) {
      viewer.dispose();
      const activeState = this.#viewer?.state();
      if (activeState) return activeState;
      throw cancelledOperation("Viewer initialization was cancelled before publication.");
    }
    this.#viewer = viewer;
    try {
      const publicationViewport = this.#options.readViewport();
      if (!sameViewport(creationViewport, publicationViewport)) {
        viewer.resize(publicationViewport);
      }
      this.#unsubscribe = viewer.subscribe(() => this.#publish());
      const state = viewer.render();
      this.#operation = "Viewer ready; immutable Source not loaded";
      this.#publish();
      return state;
    } catch (error) {
      this.#releaseViewer();
      throw error;
    }
  }

  async load(options: {
    readonly manifestUrl?: string;
    readonly invalidate?: boolean;
    readonly signal?: AbortSignal;
    readonly onState?: (state: ViewerState) => void;
  } = {}): Promise<ViewerState> {
    const viewer = this.#requireViewer();
    const manifestUrl = boundManifestUrl(this.#options.manifestUrl, options.manifestUrl);
    const asyncRevision = ++this.#asyncRevision;
    const unsubscribe = options.onState ? viewer.subscribe(options.onState) : undefined;
    this.#operation = "Streaming verified sampled Coverage";
    this.#publish();
    let result;
    try {
      result = await viewer.loadSource({
        manifestUrl,
        cacheMode: "persistent",
        credentials: "same-origin",
        invalidate: options.invalidate,
        signal: options.signal,
      });
    } catch (error) {
      this.#assertAsyncCurrent(viewer, asyncRevision);
      throw error;
    } finally {
      unsubscribe?.();
    }
    this.#assertAsyncCurrent(viewer, asyncRevision);
    this.#selectedPoint = null;
    this.#exactPoint = null;
    this.#operation = "Sampled Coverage settled";
    this.#publish();
    return result.state;
  }

  setDisplayMode(mode: DisplayMode): ViewerState {
    const viewer = this.#requireViewer();
    const state = viewer.setDisplayMode(mode);
    viewer.render();
    this.#operation = `Display mapping: ${mode}`;
    this.#publish();
    return state;
  }

  resize(viewport: ViewportInput): ViewerState {
    const viewer = this.#requireViewer();
    const state = viewer.resize(viewport);
    viewer.render();
    this.#operation = `Viewport: ${state.viewport.physicalWidth} × ${state.viewport.physicalHeight}`;
    this.#publish();
    return state;
  }

  alternateProjection(): ViewerState {
    const viewer = this.#requireViewer();
    const state = viewer.setCamera(alternateProjection(cameraFromState(viewer.state())));
    viewer.render();
    this.#operation = `Projection: ${state.camera.projection}`;
    this.#publish();
    return state;
  }

  navigate(input: NormalizedViewerInput): ViewerState | null {
    const viewer = this.#requireViewer();
    const camera = applyNavigation(
      cameraFromState(viewer.state()),
      input,
      viewer.state().viewport.cssHeight,
    );
    if (!camera) return null;
    const state = viewer.setCamera(camera);
    void viewer.requestRender();
    this.#operation = `Navigation: ${input.kind}`;
    this.#publish();
    return state;
  }

  settlePresentation(): Promise<ViewerState> {
    return this.#requireViewer().requestRender();
  }

  async pick(x: number, y: number): Promise<ProvisionalPick | null> {
    const viewer = this.#requireViewer();
    const asyncRevision = ++this.#asyncRevision;
    this.#operation = "Reading provisional GPU pick";
    this.#publish();
    const selectedPoint = await viewer.pick({ x, y });
    this.#assertAsyncCurrent(viewer, asyncRevision);
    this.#selectedPoint = selectedPoint;
    this.#exactPoint = null;
    this.#operation = this.#selectedPoint
      ? "Provisional pick available; exact confirmation required"
      : "No resident display Point at that pixel";
    this.#publish();
    return this.#selectedPoint;
  }

  highlightSelected(): ViewerState {
    if (!this.#selectedPoint) throw new Error("Pick a resident display Point before highlighting.");
    const viewer = this.#requireViewer();
    const state = viewer.setHighlights([this.#selectedPoint]);
    viewer.render();
    this.#operation = "One presentation-only highlight active";
    this.#publish();
    return state;
  }

  clearHighlights(): ViewerState {
    const viewer = this.#requireViewer();
    const state = viewer.clearHighlights();
    viewer.render();
    this.#operation = "Presentation highlights cleared";
    this.#publish();
    return state;
  }

  async confirmSelected(options: { readonly signal?: AbortSignal } = {}): Promise<ExactPoint> {
    if (!this.#selectedPoint) throw new Error("Pick a resident display Point before exact confirmation.");
    const selectedPoint = this.#selectedPoint;
    const viewer = this.#requireViewer();
    const asyncRevision = ++this.#asyncRevision;
    this.#operation = "Confirming exact immutable Source record";
    this.#publish();
    const exactPoint = await viewer.confirmPoint(selectedPoint, options);
    this.#assertAsyncCurrent(viewer, asyncRevision);
    this.#exactPoint = exactPoint;
    this.#operation = "Exact Source record confirmed";
    this.#publish();
    return this.#exactPoint;
  }

  pause(): ViewerState {
    const state = this.#requireViewer().pause();
    this.#operation = "Presentation paused; viewer state retained";
    this.#publish();
    return state;
  }

  resume(): ViewerState {
    const viewer = this.#requireViewer();
    const state = viewer.resume();
    viewer.render();
    this.#operation = "Presentation resumed";
    this.#publish();
    return state;
  }

  dispose(): void {
    this.#mountRevision += 1;
    this.#releaseViewer();
    this.#selectedPoint = null;
    this.#exactPoint = null;
    this.#operation = "Viewer disposed";
    this.#publish();
  }

  #releaseViewer(): void {
    this.#asyncRevision += 1;
    this.#unsubscribe?.();
    this.#unsubscribe = null;
    this.#viewer?.dispose();
    this.#viewer = null;
  }

  #requireViewer(): BrowserViewer {
    if (!this.#viewer) throw new Error("Initialize the viewer before using it.");
    return this.#viewer;
  }

  #assertAsyncCurrent(viewer: BrowserViewer, asyncRevision: number): void {
    if (viewer !== this.#viewer || asyncRevision !== this.#asyncRevision) {
      throw cancelledOperation("The viewer operation was superseded before publication.");
    }
  }

  #publish(): void {
    this.#options.publish(Object.freeze({
      state: this.state(),
      selectedPoint: this.#selectedPoint,
      exactPoint: this.#exactPoint,
      operation: this.#operation,
    }));
  }
}

function sameViewport(left: ViewportInput, right: ViewportInput): boolean {
  return left.cssWidth === right.cssWidth
    && left.cssHeight === right.cssHeight
    && left.devicePixelRatio === right.devicePixelRatio;
}

function boundManifestUrl(configuredManifestUrl: string, requestedManifestUrl?: string): string {
  const configured = new URL(configuredManifestUrl, globalThis.location?.href ?? MANIFEST_URL_BASE);
  const requested = new URL(
    requestedManifestUrl ?? configured.href,
    globalThis.location?.href ?? MANIFEST_URL_BASE,
  );
  if (
    requested.origin !== configured.origin
    || requested.pathname !== configured.pathname
    || requested.hash !== configured.hash
  ) {
    throw new Error("Source manifest URL must remain bound to the mounted manifest.");
  }
  for (const key of requested.searchParams.keys()) {
    if (!ACCEPTANCE_QUERY_KEYS.has(key) || requested.searchParams.getAll(key).length !== 1) {
      throw new Error("Source manifest URL contains an unsupported identity variation.");
    }
  }
  return requested.href;
}

function cancelledOperation(message: string): DOMException {
  return new DOMException(message, "AbortError");
}
