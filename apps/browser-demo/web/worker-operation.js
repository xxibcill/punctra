export function runWorkerOperation({
  WorkerConstructor = globalThis.Worker,
  workerUrl,
  workerName,
  timeoutMilliseconds,
  timeoutFailure,
  errorFailure,
  messageErrorFailure,
  initialMessage,
  onMessage,
}) {
  return new Promise((resolve, reject) => {
    const worker = new WorkerConstructor(workerUrl, { type: "module", name: workerName });
    let settled = false;
    const timeout = globalThis.setTimeout(
      () => settle(reject, timeoutFailure),
      timeoutMilliseconds,
    );

    function settle(callback, value) {
      if (settled) return;
      settled = true;
      globalThis.clearTimeout(timeout);
      worker.terminate();
      callback(value);
    }

    const controls = Object.freeze({
      resolve: (value) => settle(resolve, value),
      reject: (error) => settle(reject, error),
      postMessage: (message) => worker.postMessage(message),
    });
    worker.addEventListener("error", (event) => controls.reject(errorFailure(event)));
    worker.addEventListener("messageerror", () => controls.reject(messageErrorFailure));
    worker.addEventListener("message", (event) => {
      try {
        onMessage(event.data, controls);
      } catch (error) {
        controls.reject(error);
      }
    });
    worker.postMessage(initialMessage);
  });
}
