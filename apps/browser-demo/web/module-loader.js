const PRODUCTION_BUNDLE = typeof import.meta.env !== "undefined" && import.meta.env.PROD;

export function loadExactQueryModules(cacheToken) {
  return PRODUCTION_BUNDLE
    ? Promise.all([
        import("./streaming-protocol.js"),
        import("./range-response.js"),
      ])
    : Promise.all([
        import(`./streaming-protocol.js?v=${cacheToken}`),
        import(`./range-response.js?v=${cacheToken}`),
      ]);
}

export function loadStreamingProtocolModules(cacheToken) {
  return PRODUCTION_BUNDLE
    ? Promise.all([
        import("./worker-protocol.js"),
        import("./range-response.js"),
      ])
    : Promise.all([
        import(`./worker-protocol.js?v=${cacheToken}`),
        import(`./range-response.js?v=${cacheToken}`),
      ]);
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

export function loadWorkerModules(cacheToken) {
  return PRODUCTION_BUNDLE
    ? Promise.all([
        import("./streaming-protocol.js"),
        import("./worker-protocol.js"),
      ])
    : Promise.all([
        import(`./streaming-protocol.js?v=${cacheToken}`),
        import(`./worker-protocol.js?v=${cacheToken}`),
      ]);
}
