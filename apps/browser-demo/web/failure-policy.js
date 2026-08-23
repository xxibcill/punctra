export const UNSUPPORTED_INITIALIZATION_CODES = Object.freeze([
  "missing_window",
  "insecure_context",
  "webgpu_unavailable",
  "capability_inspection",
  "canvas_surface",
  "webgpu_adapter",
  "webgpu_device",
  "surface_format",
  "presentation_mode",
  "surface_alpha_mode",
  "surface_configuration",
  "renderer_capability",
]);

export const RECOVERABLE_VIEWER_FAILURE_CODES = Object.freeze([
  "surface_timeout",
  "surface_occluded",
  "surface_outdated",
  "worker_failed",
]);

const unsupportedInitializationCodes = new Set(UNSUPPORTED_INITIALIZATION_CODES);
const recoverableViewerFailureCodes = new Set(RECOVERABLE_VIEWER_FAILURE_CODES);

export function failureState(record) {
  return unsupportedInitializationCodes.has(record.code) ? "unsupported" : "failed";
}

export function preservesCurrentViewer(record) {
  return recoverableViewerFailureCodes.has(record.code);
}
