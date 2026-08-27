import type { ViewportInput, ViewerState } from "@punctra/viewer";

type CanvasBounds = Pick<DOMRectReadOnly, "left" | "top" | "width" | "height">;
type PhysicalViewport = Pick<ViewerState["viewport"], "physicalWidth" | "physicalHeight">;

export function viewportFromCanvasBounds(
  bounds: CanvasBounds,
  devicePixelRatio: number,
): ViewportInput {
  return {
    cssWidth: positiveFinite(bounds.width, "canvas width"),
    cssHeight: positiveFinite(bounds.height, "canvas height"),
    devicePixelRatio: positiveFinite(devicePixelRatio, "device pixel ratio"),
  };
}

export function mapClientPointToViewport(
  bounds: CanvasBounds,
  viewport: PhysicalViewport,
  clientX: number,
  clientY: number,
): readonly [number, number] {
  const cssWidth = positiveFinite(bounds.width, "canvas width");
  const cssHeight = positiveFinite(bounds.height, "canvas height");
  const physicalWidth = positiveInteger(viewport.physicalWidth, "physical width");
  const physicalHeight = positiveInteger(viewport.physicalHeight, "physical height");
  return [
    mapAxis(finite(clientX, "client x") - finite(bounds.left, "canvas left"), cssWidth, physicalWidth),
    mapAxis(finite(clientY, "client y") - finite(bounds.top, "canvas top"), cssHeight, physicalHeight),
  ];
}

function mapAxis(offset: number, cssLength: number, physicalLength: number): number {
  const mapped = Math.floor(offset * physicalLength / cssLength);
  return Math.max(0, Math.min(physicalLength - 1, mapped));
}

function positiveInteger(value: number, label: string): number {
  if (!Number.isInteger(value)) throw new Error(`${label} must be a positive integer.`);
  return positiveFinite(value, label);
}

function positiveFinite(value: number, label: string): number {
  if (!(value > 0) || !Number.isFinite(value)) {
    throw new Error(`${label} must be finite and positive.`);
  }
  return value;
}

function finite(value: number, label: string): number {
  if (!Number.isFinite(value)) throw new Error(`${label} must be finite.`);
  return value;
}
