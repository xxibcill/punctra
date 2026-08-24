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
const PRESERVE_VIEWER_FAILURE = Symbol("preserve-viewer-failure");

export function failureState(record) {
  return unsupportedInitializationCodes.has(record.code) ? "unsupported" : "failed";
}

export function preserveViewerFailure(cause) {
  return Object.freeze({
    [PRESERVE_VIEWER_FAILURE]: true,
    cause,
  });
}

export function failureCause(error) {
  return isPreserveViewerFailure(error) ? error.cause : error;
}

export function isPreserveViewerFailure(error) {
  return error?.[PRESERVE_VIEWER_FAILURE] === true;
}

export function preservesCurrentViewer(record, error = record) {
  return isPreserveViewerFailure(error)
    || recoverableViewerFailureCodes.has(record.code);
}
