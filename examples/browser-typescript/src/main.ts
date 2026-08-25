import type { BrowserViewer, ViewerState } from "@punctra/viewer";

const canvas = document.querySelector<HTMLCanvasElement>("#viewer")!;

let viewer: BrowserViewer | undefined;

async function mount(): Promise<ViewerState> {
  const { createViewer } = await import("@punctra/viewer");
  viewer = await createViewer({
    canvas,
    viewport: {
      cssWidth: 960,
      cssHeight: 540,
      devicePixelRatio: window.devicePixelRatio,
    },
    assets: { cacheKey: "typescript-trial" },
  });
  return viewer.render();
}

function pause(): ViewerState | undefined {
  return viewer?.pause();
}

function resume(): ViewerState | undefined {
  return viewer?.resume();
}

function dispose(): void {
  viewer?.dispose();
  viewer = undefined;
}

Object.assign(window, { punctraTrial: { mount, pause, resume, dispose } });
