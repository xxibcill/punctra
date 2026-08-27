export function runWorkerOperation({
  WorkerConstructor = globalThis.Worker,
  workerFactory,
  workerUrl,
  workerName,
  timeoutMilliseconds,
  timeoutFailure,
  errorFailure,
  messageErrorFailure,
  initialMessage,
  signal,
  cancellationMessage,
  onMessage,
}) {
  return new Promise((resolve, reject) => {
    const workerOptions = { type: "module", name: workerName };
    const worker = workerFactory
      ? workerFactory(workerUrl, workerOptions)
      : new WorkerConstructor(workerUrl, workerOptions);
    let settled = false;
    let timeout = globalThis.setTimeout(
      () => settle(reject, timeoutFailure),
      timeoutMilliseconds,
    );
    const onAbort = () => {
      if (cancellationMessage !== undefined) worker.postMessage(cancellationMessage);
    };
    signal?.addEventListener("abort", onAbort, { once: true });

    function settle(callback, value) {
      if (settled) return;
      settled = true;
      if (timeout !== undefined) globalThis.clearTimeout(timeout);
      signal?.removeEventListener("abort", onAbort);
      worker.terminate();
      callback(value);
    }

    const controls = Object.freeze({
      resolve: (value) => settle(resolve, value),
      reject: (error) => settle(reject, error),
      postMessage: (message) => worker.postMessage(message),
      pauseTimeout: () => {
        if (settled || timeout === undefined) return;
        globalThis.clearTimeout(timeout);
        timeout = undefined;
      },
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
    if (signal?.aborted) onAbort();
  });
}
