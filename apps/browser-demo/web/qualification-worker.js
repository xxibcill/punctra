import "./stream-worker.js";

self.addEventListener("message", (event) => {
  const manifestUrl = event.data?.manifest_url;
  if (typeof manifestUrl !== "string") return;
  if (new URL(manifestUrl).searchParams.get("worker_fault") !== "crash") return;
  queueMicrotask(() => {
    throw new Error("intentional v0.19 qualification worker crash");
  });
});
