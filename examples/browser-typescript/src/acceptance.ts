import type { ViewerState } from "@punctra/viewer";

import { QUICKSTART_DISPLAY_MODES } from "./display-modes.ts";
import type { PackedRuntimeProof } from "./packed-runtime.ts";
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
  readonly recoverableFailureCode: "offline";
  readonly retryRetainedViewer: true;
  readonly retrySucceeded: true;
  readonly recreationFailureCode: "cancelled";
  readonly recreationRequired: true;
  readonly recreationSucceeded: true;
  readonly provisionalAuthority: "provisional_gpu_hint";
  readonly exactAuthority: "exact_source_record";
  readonly disposed: true;
  readonly packedRuntime: PackedRuntimeProof;
}

export async function runQuickstartAcceptance(
  controller: QuickstartController,
  manifestUrl: string,
  packedRuntime: PackedRuntimeProof,
): Promise<QuickstartAcceptanceRecord> {
  await controller.mount();
  const generationBeforeCancellation = requiredState(controller).generation;
  await cancelDelayedLoad(controller, manifestUrl);
  if (requiredState(controller).generation !== generationBeforeCancellation) {
    throw new Error("Cancelled load changed the active generation.");
  }

  const retry = await exerciseRecoverableRetry(controller, manifestUrl);
  const recreation = await exerciseRecreationRequiredRecovery(controller, manifestUrl);
  const displayModes = exerciseDisplayModes(controller);
  const projections = exerciseProjections(controller);
  exerciseNavigation(controller);
  await controller.settlePresentation();

  const state = requiredState(controller);
  const provisional = await pickResidentPoint(controller, state);
  if (!provisional) throw new Error("The fixed quickstart pick grid missed the accepted fixture.");
  const highlighted = controller.highlightSelected();
  if (highlighted.highlights.pointCount !== 1) {
    throw new Error("The quickstart did not publish the accepted presentation highlight.");
  }
  const exact = await controller.confirmSelected();
  const cleared = controller.clearHighlights();
  if (cleared.highlights.pointCount !== 0) {
    throw new Error("The quickstart did not clear its presentation highlight.");
  }
  if (controller.pause().lifecycle !== "hidden") {
    throw new Error("The quickstart did not pause presentation.");
  }
  if (controller.resume().lifecycle !== "ready") {
    throw new Error("The quickstart did not resume presentation.");
  }

  const settled = requiredState(controller);
  if (settled.packageVersion !== packedRuntime.viewerVersion) {
    throw new Error("The running viewer does not match the packed runtime proof.");
  }
  const completed = {
    schema: "punctra-browser-quickstart-acceptance-v1" as const,
    packageVersion: settled.packageVersion,
    sourceIdentity: requiredSourceIdentity(settled),
    generation: settled.generation,
    displayedPoints: settled.source.publishedPoints,
    displayModes: Object.freeze(displayModes),
    projections: Object.freeze(projections),
    cancellationRetainedViewer: true as const,
    ...retry,
    ...recreation,
    provisionalAuthority: provisional.authority,
    exactAuthority: exact.authority,
    packedRuntime: Object.freeze({ ...packedRuntime }),
  };
  controller.dispose();
  if (controller.state() !== null) {
    throw new Error("The quickstart retained its viewer after disposal.");
  }
  return Object.freeze({ ...completed, disposed: true as const });
}

async function exerciseRecoverableRetry(
  controller: QuickstartController,
  manifestUrl: string,
): Promise<Pick<
  QuickstartAcceptanceRecord,
  "recoverableFailureCode" | "retryRetainedViewer" | "retrySucceeded"
>> {
  const before = requiredState(controller);
  const disconnectedManifest = acceptanceUrl(manifestUrl, "fault", "disconnect");
  const failure = await expectViewerFailure(controller.load({
    manifestUrl: disconnectedManifest,
    invalidate: true,
  }));
  if (failure.code !== "offline" || failure.recoverable !== true) {
    throw new Error("The disconnected manifest was not a recoverable offline failure.");
  }
  const retained = requiredState(controller);
  if (retained.lifecycle !== "ready" || retained.generation !== before.generation) {
    throw new Error("The recoverable failure did not retain the active viewer generation.");
  }
  await controller.load({ invalidate: true });
  return Object.freeze({
    recoverableFailureCode: "offline" as const,
    retryRetainedViewer: true as const,
    retrySucceeded: true as const,
  });
}

async function exerciseRecreationRequiredRecovery(
  controller: QuickstartController,
  manifestUrl: string,
): Promise<Pick<
  QuickstartAcceptanceRecord,
  "recreationFailureCode" | "recreationRequired" | "recreationSucceeded"
>> {
  const before = requiredState(controller);
  const cancellation = new AbortController();
  const partialPublicationManifest = acceptanceUrl(
    manifestUrl,
    "acceptance_phase",
    "partial-publication",
  );
  const failure = await expectViewerFailure(controller.load({
    manifestUrl: partialPublicationManifest,
    invalidate: true,
    signal: cancellation.signal,
    onState: (state) => {
      if (state.generation !== before.generation && state.source.publishedPoints > 0) {
        cancellation.abort();
      }
    },
  }));
  if (failure.code !== "cancelled" || failure.recoverable !== false) {
    throw new Error("The post-publication cancellation did not require viewer recreation.");
  }
  if (requiredState(controller).lifecycle !== "destroyed") {
    throw new Error("The post-publication failure did not fuse the viewer.");
  }
  await controller.mount();
  await controller.load({ invalidate: true });
  return Object.freeze({
    recreationFailureCode: "cancelled" as const,
    recreationRequired: true as const,
    recreationSucceeded: true as const,
  });
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

function acceptanceUrl(manifestUrl: string, name: string, value: string): string {
  const url = new URL(manifestUrl, globalThis.location?.href ?? "http://localhost/");
  url.searchParams.set(name, value);
  return url.href;
}

interface StructuredViewerFailure {
  readonly code: string;
  readonly recoverable: boolean;
}

async function expectViewerFailure(operation: Promise<unknown>): Promise<StructuredViewerFailure> {
  try {
    await operation;
  } catch (error) {
    if (
      typeof error === "object"
      && error !== null
      && typeof (error as { code?: unknown }).code === "string"
      && typeof (error as { recoverable?: unknown }).recoverable === "boolean"
    ) {
      return error as StructuredViewerFailure;
    }
    throw error;
  }
  throw new Error("The deterministic quickstart fault completed successfully.");
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

function exerciseDisplayModes(controller: QuickstartController): string[] {
  return QUICKSTART_DISPLAY_MODES.map((mode) => {
    const state = controller.setDisplayMode(mode);
    if (state.displayMode !== mode || requiredState(controller).displayMode !== mode) {
      throw new Error(`The quickstart did not apply the ${mode} display mapping.`);
    }
    return state.displayMode;
  });
}

function exerciseNavigation(controller: QuickstartController): void {
  requireNavigationChange(
    controller,
    { kind: "orbit", deltaX: 4, deltaY: 2, source: "pointer" },
    "orbit",
  );
  requireNavigationChange(
    controller,
    { kind: "pan", deltaX: 2, deltaY: -1, source: "pointer" },
    "pan",
  );
  requireNavigationChange(
    controller,
    { kind: "zoom", delta: 0.2, source: "wheel" },
    "zoom",
  );
  const keyboardInput = {
    kind: "keyboard" as const,
    code: "KeyP",
    repeat: false,
    modifiers: { alt: false, control: false, meta: false, shift: false },
  };
  requireNavigationChange(controller, keyboardInput, "keyboard projection change");
  requireNavigationChange(controller, keyboardInput, "keyboard projection restore");
}

function requireNavigationChange(
  controller: QuickstartController,
  input: Parameters<QuickstartController["navigate"]>[0],
  label: string,
): void {
  const before = requiredState(controller).camera;
  const after = controller.navigate(input);
  if (!after || sameCamera(before, after.camera)) {
    throw new Error(`The quickstart ${label} navigation did not change the camera.`);
  }
  if (!sameCamera(after.camera, requiredState(controller).camera)) {
    throw new Error(`The quickstart ${label} navigation did not publish its camera.`);
  }
}

function sameCamera(left: ViewerState["camera"], right: ViewerState["camera"]): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
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
