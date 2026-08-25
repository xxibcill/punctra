import { useEffect, useRef, useState } from "react";

import { applyViewerUpdate, startViewerLifecycle } from "./lifecycle.js";

const IDLE_BINDING = Object.freeze({
  status: "idle",
  viewer: null,
  state: null,
  error: null,
});

export function createUsePunctraViewer(createViewer) {
  return function usePunctraViewer(options) {
    const [binding, setBinding] = useState(IDLE_BINDING);
    const creationOptions = useRef(options);
    creationOptions.current = options;

    useEffect(() => {
      if (!options.canvas) {
        setBinding(IDLE_BINDING);
        return undefined;
      }
      const current = creationOptions.current;
      const { active: _active, mountKey: _mountKey, ...viewerOptions } = current;
      const lifecycle = startViewerLifecycle(
        createViewer,
        {
          ...viewerOptions,
          canvas: current.canvas,
          viewport: current.viewport,
        },
        setBinding,
      );
      return () => lifecycle.dispose();
    }, [options.canvas, options.mountKey]);

    useEffect(() => {
      if (!binding.viewer) return;
      applyViewerUpdate(binding.viewer, (viewer) => viewer.resize(options.viewport), setBinding);
    }, [
      binding.viewer,
      options.viewport.cssWidth,
      options.viewport.cssHeight,
      options.viewport.devicePixelRatio,
    ]);

    useEffect(() => {
      if (!binding.viewer) return;
      applyViewerUpdate(binding.viewer, (viewer) => {
        if (options.active === false) viewer.pause();
        else viewer.resume();
      }, setBinding);
    }, [binding.viewer, options.active]);

    return binding;
  };
}
