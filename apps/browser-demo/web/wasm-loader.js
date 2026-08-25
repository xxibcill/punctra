export function createWasmModuleLoader({ createRawViewer, initializeWasm, ViewerError }) {
  let initializedWasmUrl;
  let wasmInitialization;

  return async function loadBindings(wasmUrl) {
    const requestedUrl = wasmUrl.href;
    if (initializedWasmUrl !== undefined && initializedWasmUrl !== requestedUrl) {
      throw new ViewerError(
        "invalid_argument",
        "one imported SDK module cannot be initialized from two different Wasm asset URLs",
        {
          safeAction: "Reuse the first Wasm URL or import an independently versioned SDK module.",
        },
      );
    }
    if (!wasmInitialization) {
      initializedWasmUrl = requestedUrl;
      wasmInitialization = Promise.resolve()
        .then(() => initializeWasm({ module_or_path: wasmUrl }))
        .catch((error) => {
          initializedWasmUrl = undefined;
          wasmInitialization = undefined;
          throw error;
        });
    }
    try {
      await wasmInitialization;
    } catch (error) {
      throw error instanceof ViewerError
        ? error
        : new ViewerError("internal", error?.message ?? "WebAssembly initialization failed", {
            safeAction: "Verify the Wasm asset URL, MIME type, Content Security Policy, and response body before retrying.",
          });
    }
    return { createViewer: createRawViewer };
  };
}
