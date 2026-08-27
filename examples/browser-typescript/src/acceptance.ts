import type { DisplayMode, ViewerState } from "@punctra/viewer";

import { QuickstartController } from "./quickstart.ts";

export interface QuickstartAcceptanceRecord {
  readonly schema: "punctra-browser-quickstart-acceptance-v1";
  readonly packageVersion: string;
  readonly sourceIdentity: string;
  readonly generation: number;
  readonly displayedPoints: number;
  readonly displayModes: readonly string[];
  readonly projections: readonly string[];
  readonly cancellationRetainedViewer: true;
  readonly provisionalAuthority: "provisional_gpu_hint";
  readonly exactAuthority: "exact_source_record";
  readonly disposed: true;
}

const ACCEPTED_DISPLAY_MODES: readonly DisplayMode[] = Object.freeze([
  "neutral",
  "elevation",
  "rgb",
  "intensity",
  "classification",
]);

export async function runQuickstartAcceptance(
  controller: QuickstartController,
  manifestUrl: string,
): Promise<QuickstartAcceptanceRecord> {
  await controller.mount();
  const generationBeforeCancellation = requiredState(controller).generation;
  await cancelDelayedLoad(controller, manifestUrl);
  if (requiredState(controller).generation !== generationBeforeCancellation) {
    throw new Error("Cancelled load changed the active generation.");
  }

  await controller.load({ invalidate: true });
  for (const mode of ACCEPTED_DISPLAY_MODES) controller.setDisplayMode(mode);
  const projections = exerciseProjections(controller);
  exerciseNavigation(controller);
  await controller.settlePresentation();

  const state = requiredState(controller);
  const provisional = await pickResidentPoint(controller, state);
  if (!provisional) throw new Error("The fixed quickstart pick grid missed the accepted fixture.");
  controller.highlightSelected();
  const exact = await controller.confirmSelected();
  controller.clearHighlights();
  controller.pause();
  controller.resume();

  const settled = requiredState(controller);
  const record = Object.freeze({
    schema: "punctra-browser-quickstart-acceptance-v1" as const,
    packageVersion: settled.packageVersion,
    sourceIdentity: requiredSourceIdentity(settled),
    generation: settled.generation,
    displayedPoints: settled.source.publishedPoints,
    displayModes: Object.freeze([...ACCEPTED_DISPLAY_MODES]),
    projections: Object.freeze(projections),
    cancellationRetainedViewer: true as const,
    provisionalAuthority: provisional.authority,
    exactAuthority: exact.authority,
    disposed: true as const,
  });
  controller.dispose();
  return record;
}

async function pickResidentPoint(controller: QuickstartController, state: ViewerState) {
  const fractions = [0.5, 0.35, 0.65, 0.2, 0.8, 0.1, 0.9];
  for (const yFraction of fractions) {
    for (const xFraction of fractions) {
      const provisional = await controller.pick(
        Math.floor(state.viewport.physicalWidth * xFraction),
        Math.floor(state.viewport.physicalHeight * yFraction),
      );
      if (provisional) return provisional;
    }
  }
  return null;
}

async function cancelDelayedLoad(controller: QuickstartController, manifestUrl: string): Promise<void> {
  const cancellation = new AbortController();
  const delayedManifest = new URL(manifestUrl, globalThis.location?.href ?? "http://localhost/");
  delayedManifest.searchParams.set("delay_ms", "250");
  const pending = controller.load({ manifestUrl: delayedManifest.href, signal: cancellation.signal });
  globalThis.setTimeout(() => cancellation.abort(), 25);
  try {
    await pending;
    throw new Error("The delayed Source load completed before cancellation.");
  } catch (error) {
    if (error instanceof Error && error.message === "The delayed Source load completed before cancellation.") {
      throw error;
    }
    if ((error as { code?: string }).code !== "cancelled") throw error;
  }
}

function exerciseProjections(controller: QuickstartController): string[] {
  const projections = new Set<string>([requiredState(controller).camera.projection]);
  projections.add(controller.alternateProjection().camera.projection);
  projections.add(controller.alternateProjection().camera.projection);
  if (!projections.has("perspective") || !projections.has("orthographic")) {
    throw new Error("The quickstart did not exercise both accepted projections.");
  }
  return [...projections].sort();
}

function exerciseNavigation(controller: QuickstartController): void {
  controller.navigate({ kind: "orbit", deltaX: 4, deltaY: 2, source: "pointer" });
  controller.navigate({ kind: "pan", deltaX: 2, deltaY: -1, source: "pointer" });
  controller.navigate({ kind: "zoom", delta: 0.2, source: "wheel" });
  const keyboardInput = {
    kind: "keyboard" as const,
    code: "KeyP",
    repeat: false,
    modifiers: { alt: false, control: false, meta: false, shift: false },
  };
  controller.navigate(keyboardInput);
  controller.navigate(keyboardInput);
}

function requiredState(controller: QuickstartController): ViewerState {
  const state = controller.state();
  if (!state) throw new Error("The quickstart viewer is not initialized.");
  return state;
}

function requiredSourceIdentity(state: ViewerState): string {
  if (!state.source.identity) throw new Error("The quickstart Source identity is missing.");
  return state.source.identity;
}
