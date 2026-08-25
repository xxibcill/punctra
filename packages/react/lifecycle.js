/**
 * Starts one async viewer mount with an immediate cleanup handle. This module
 * stays package-private so React is only translating the viewer lifecycle.
 */
export function startViewerLifecycle(createViewer, options, publish) {
  let disposed = false;
  let viewer;
  let unsubscribe;

  publish({ status: "loading", viewer: null, state: null, error: null });
  const ready = Promise.resolve()
    .then(() => createViewer(options))
    .then((createdViewer) => {
      if (disposed) {
        createdViewer.dispose();
        return null;
      }
      viewer = createdViewer;
      unsubscribe = viewer.subscribe((state) => {
        publish({ status: "ready", viewer, state, error: null });
      });
      return viewer;
    })
    .catch((error) => {
      if (!disposed) publish({ status: "failed", viewer: null, state: null, error });
      return null;
    });

  return Object.freeze({
    ready,
    dispose() {
      if (disposed) return;
      disposed = true;
      unsubscribe?.();
      viewer?.dispose();
      unsubscribe = undefined;
      viewer = undefined;
    },
  });
}

export function applyViewerUpdate(viewer, update, publish) {
  try {
    update(viewer);
  } catch (error) {
    publish({ status: "failed", viewer, state: viewer.state(), error });
  }
}
