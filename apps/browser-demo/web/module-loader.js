const PRODUCTION_BUNDLE = typeof import.meta.env !== "undefined" && import.meta.env.PROD;

export async function loadExactQueryModules(cacheToken) {
  const [streamingProtocol, , rangeResponse] = await loadStreamingGraph(cacheToken);
  return [streamingProtocol, rangeResponse];
}

export async function loadStreamingProtocol(cacheToken) {
  const [streamingProtocol] = await loadStreamingGraph(cacheToken);
  return streamingProtocol;
}

export function loadViewerModules(cacheToken) {
  return PRODUCTION_BUNDLE
    ? Promise.all([
        import("./stream-ordinals.js"),
        import("./worker-operation.js"),
        import("./worker-protocol.js"),
        import("./camera-policy.js"),
      ])
    : Promise.all([
        import(`./stream-ordinals.js?v=${cacheToken}`),
        import(`./worker-operation.js?v=${cacheToken}`),
        import(`./worker-protocol.js?v=${cacheToken}`),
        import(`./camera-policy.js?v=${cacheToken}`),
      ]);
}

export async function loadWorkerModules(cacheToken) {
  const [streamingProtocol, workerProtocol] = await loadStreamingGraph(cacheToken);
  return [streamingProtocol, workerProtocol];
}

async function loadStreamingGraph(cacheToken) {
  const modules = await (PRODUCTION_BUNDLE
    ? Promise.all([
        import("./streaming-protocol.js"),
        import("./worker-protocol.js"),
        import("./range-response.js"),
      ])
    : Promise.all([
        import(`./streaming-protocol.js?v=${cacheToken}`),
        import(`./worker-protocol.js?v=${cacheToken}`),
        import(`./range-response.js?v=${cacheToken}`),
      ]));
  const [streamingProtocol, workerProtocol, rangeResponse] = modules;
  streamingProtocol.initializeStreamingProtocol(workerProtocol, rangeResponse);
  return modules;
}
