export const WORKER_SCHEMA = "punctra-browser-worker-v1";
export const INVALID_WORKER_OPERATION_ID = "invalid-operation";
export const MAX_WORKER_OPERATION_ID_CHARACTERS = 128;
export const MAX_WORKER_FAILURE_MESSAGE_CHARACTERS = 512;
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
    message: boundedWorkerFailureMessage(message),
    safe_action: WORKER_FAILURE_SAFE_ACTION,
  };
}

export function workerOperationId(value) {
  return isWorkerOperationId(value) ? value : INVALID_WORKER_OPERATION_ID;
}

export function isWorkerOperationId(value) {
  return typeof value === "string"
    && value.length > 0
    && value.length <= MAX_WORKER_OPERATION_ID_CHARACTERS;
}

export function boundedWorkerFailureMessage(value) {
  const message = typeof value === "string" ? value : String(value);
  if (message.length <= MAX_WORKER_FAILURE_MESSAGE_CHARACTERS) return message;
  return `${message.slice(0, MAX_WORKER_FAILURE_MESSAGE_CHARACTERS - 1)}…`;
}
