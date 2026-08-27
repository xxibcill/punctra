import type { CreateViewerOptions, DisplayMode } from "@punctra/viewer";
import { createLasExactQueryBridge } from "@punctra/viewer/exact-query";
import { createInputNormalizer } from "@punctra/viewer/input";

import { runQuickstartAcceptance } from "./acceptance.ts";
import { QUICKSTART_DISPLAY_MODES } from "./display-modes.ts";
import { parsePackedRuntimeProof, type PackedRuntimeProof } from "./packed-runtime.ts";
import { QuickstartController, type QuickstartSnapshot } from "./quickstart.ts";
import "./styles.css";

const manifestUrl = new URL("/fixtures/v1/deployment.json", location.href).href;
const canvas = requiredElement<HTMLCanvasElement>("viewer");
const controller = new QuickstartController({
  canvas,
  viewport: viewport(),
  manifestUrl,
  createViewer: createPackedViewer,
  createExactBridge: createLasExactQueryBridge,
  publish: renderSnapshot,
});

const inputNormalizer = createInputNormalizer(canvas, (input) => {
  runAction(() => controller.navigate(input));
}, { preventDefault: true });
let resizeFrame: number | undefined;
const resizeObserver = new ResizeObserver(() => {
  if (resizeFrame !== undefined) cancelAnimationFrame(resizeFrame);
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = undefined;
    if (controller.state()) runAction(() => controller.resize(viewport()));
  });
});
resizeObserver.observe(canvas);

bindButton("initialize", () => controller.mount());
bindButton("load-source", () => controller.load({ invalidate: true }));
bindButton("projection", () => controller.alternateProjection());
bindButton("highlight", () => controller.highlightSelected());
bindButton("confirm", () => controller.confirmSelected());
bindButton("clear", () => controller.clearHighlights());
bindButton("pause", () => controller.pause());
bindButton("resume", () => controller.resume());
bindButton("dispose", () => controller.dispose());
bindButton("run-acceptance", runAcceptance);
requiredElement<HTMLSelectElement>("display-mode").addEventListener("change", (event) => {
  runAction(() => controller.setDisplayMode((event.currentTarget as HTMLSelectElement).value as DisplayMode));
});
canvas.addEventListener("click", (event) => runAction(() => pickCanvasPoint(event)));
document.addEventListener("visibilitychange", synchronizeVisibility);
window.addEventListener("beforeunload", () => {
  document.removeEventListener("visibilitychange", synchronizeVisibility);
  resizeObserver.disconnect();
  if (resizeFrame !== undefined) cancelAnimationFrame(resizeFrame);
  inputNormalizer.dispose();
  controller.dispose();
}, { once: true });

populateDisplayModes();
runAction(() => controller.mount());

async function pickCanvasPoint(event: MouseEvent): Promise<void> {
  const state = controller.state();
  if (!state) throw new Error("Initialize the viewer before picking.");
  const bounds = canvas.getBoundingClientRect();
  const x = Math.floor((event.clientX - bounds.left) * state.viewport.devicePixelRatio);
  const y = Math.floor((event.clientY - bounds.top) * state.viewport.devicePixelRatio);
  await controller.pick(x, y);
}

function synchronizeVisibility(): void {
  if (!controller.state()) return;
  runAction(() => document.visibilityState === "hidden" ? controller.pause() : controller.resume());
}

async function runAcceptance(): Promise<void> {
  setAcceptance("running", "RUNNING");
  const packedRuntime = await loadPackedRuntimeProof();
  const record = await runQuickstartAcceptance(controller, manifestUrl, packedRuntime);
  requiredElement<HTMLOutputElement>("acceptance-record").textContent = JSON.stringify(record, null, 2);
  setAcceptance("passed", "PASS");
}

async function loadPackedRuntimeProof(): Promise<PackedRuntimeProof> {
  const response = await fetch("/punctra-packed-runtime.json", { cache: "no-store" });
  if (!response.ok) throw new Error(`Packed runtime proof returned HTTP ${response.status}.`);
  for (const [header, expected] of [
    ["accept-ranges", "bytes"],
    ["content-encoding", "identity"],
  ]) {
    if (response.headers.get(header) !== expected) {
      throw new Error(`Packed runtime proof is missing the strict ${header} response.`);
    }
  }
  if (!response.headers.get("etag")) {
    throw new Error("Packed runtime proof is missing its validator response.");
  }
  return parsePackedRuntimeProof(await response.json());
}

function renderSnapshot(snapshot: QuickstartSnapshot): void {
  const state = snapshot.state;
  setText("operation", snapshot.operation);
  setText("lifecycle", state?.lifecycle ?? "not initialized");
  setText("package-version", state?.packageVersion ?? "—");
  setText("coverage", state?.source.coverage ?? "none");
  setText("displayed-points", formatNumber(state?.source.publishedPoints));
  setText("generation", formatNumber(state?.generation));
  setText("projection-value", state?.camera.projection ?? "—");
  setText("source-identity", abbreviatedIdentity(state?.source.identity));
  setText("pick-authority", snapshot.selectedPoint?.authority ?? "none");
  setText("exact-authority", snapshot.exactPoint?.authority ?? "not confirmed");
  setText("safe-action", state?.failure?.safeAction ?? "No recovery action required.");
  setText("resource-line", resourceLine(state));
  setDisabled("highlight", !snapshot.selectedPoint);
  setDisabled("confirm", !snapshot.selectedPoint);
  setDisabled("clear", !state || state.highlights.pointCount === 0);
  requiredElement<HTMLSelectElement>("display-mode").value = state?.displayMode ?? "rgb";
}

function resourceLine(state: QuickstartSnapshot["state"]): string {
  if (!state) return "No viewer resource facts.";
  return [
    `${state.render.residentBytes.toLocaleString()} renderer bytes`,
    `${state.source.retainedRecordBytes.toLocaleString()} decoded bytes`,
    `${state.viewport.surfaceBytes.toLocaleString()} canvas bytes`,
  ].join(" · ");
}

function viewport() {
  const cssWidth = Math.max(320, Math.min(960, canvas.clientWidth || 960));
  const cssHeight = Math.max(240, Math.min(600, canvas.clientHeight || 600));
  return {
    cssWidth,
    cssHeight,
    devicePixelRatio: Math.min(4, window.devicePixelRatio || 1),
  };
}

function populateDisplayModes(): void {
  const select = requiredElement<HTMLSelectElement>("display-mode");
  select.replaceChildren(...QUICKSTART_DISPLAY_MODES.map((mode) => new Option(mode, mode)));
  select.value = "rgb";
}

function bindButton(id: string, action: () => unknown | Promise<unknown>): void {
  requiredElement<HTMLButtonElement>(id).addEventListener("click", () => runAction(action));
}

function runAction(action: () => unknown | Promise<unknown>): void {
  try {
    void Promise.resolve(action()).catch(publishError);
  } catch (error) {
    publishError(error);
  }
}

function publishError(error: unknown): void {
  if (error instanceof DOMException && error.name === "AbortError") return;
  const record = isViewerError(error)
    ? error
    : {
        message: error instanceof Error ? error.message : String(error),
        safeAction: "Correct the reported condition and retry.",
      };
  setText("operation", record.message);
  setText("safe-action", record.safeAction);
  setAcceptance("failed", "FAIL");
}

async function createPackedViewer(options: CreateViewerOptions) {
  const { createViewer } = await import("@punctra/viewer");
  return createViewer(options);
}

function isViewerError(error: unknown): error is {
  readonly schema: "punctra-viewer-error-v1";
  readonly message: string;
  readonly safeAction: string;
} {
  return typeof error === "object"
    && error !== null
    && (error as { schema?: unknown }).schema === "punctra-viewer-error-v1"
    && typeof (error as { message?: unknown }).message === "string"
    && typeof (error as { safeAction?: unknown }).safeAction === "string";
}

function setAcceptance(state: string, label: string): void {
  const output = requiredElement<HTMLOutputElement>("acceptance-state");
  output.dataset.state = state;
  output.textContent = label;
}

function setDisabled(id: string, disabled: boolean): void {
  requiredElement<HTMLButtonElement>(id).disabled = disabled;
}

function setText(id: string, value: string): void {
  requiredElement<HTMLElement>(id).textContent = value;
}

function formatNumber(value: number | undefined): string {
  return value === undefined ? "—" : value.toLocaleString();
}

function abbreviatedIdentity(value: string | null | undefined): string {
  return value ? `${value.slice(0, 12)}…${value.slice(-8)}` : "none";
}

function requiredElement<ElementType extends HTMLElement>(id: string): ElementType {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Quickstart element #${id} is missing.`);
  return element as ElementType;
}

Object.assign(window, {
  punctraQuickstart: Object.freeze({
    controller,
    runAcceptance,
    dispose: () => controller.dispose(),
  }),
});
