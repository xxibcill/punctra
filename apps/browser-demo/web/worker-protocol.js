export const WORKER_SCHEMA = "punctra-browser-worker-v1";
export const WORKER_OUTPUT_TYPES = Object.freeze([
  "state",
  "batch",
  "complete",
  "failure",
]);
export const WORKER_FAILURE_SAFE_ACTION =
  "Terminate the worker, keep the current frame, and create a new worker before retrying.";

export function workerFailure(message) {
  return {
    schema: WORKER_SCHEMA,
    type: "failure",
    code: "worker_failed",
    message,
    safe_action: WORKER_FAILURE_SAFE_ACTION,
  };
}
