// Exact current browser screen and canvas observations. Earlier screen variants
// remain diagnostic records; declarations alone qualify no session.
import { QUALIFICATION_LANE as predecessorLane, QUALIFICATION_RUNTIME_LANE as predecessorRuntime } from "./qualification-lane-v0.22.js";

const id = "codex-iab-chromium-154-macos-26-apple-m5-pro-screen-1512-canvas-1384";
export const QUALIFICATION_LANE = Object.freeze({
  ...predecessorLane, id,
  display: Object.freeze({ ...predecessorLane.display, screen_css_pixels: Object.freeze([1512, 982]),
    physical_viewport: Object.freeze([1384, 865]), css_viewport: Object.freeze([692, 432.5]), canvas_bytes: 4_788_640,
    color_depth: 30, pixel_depth: 30,
    screen_note: "Exact browser-reported current session facts; this does not qualify physical panel composition or color." }),
});
export const QUALIFICATION_RUNTIME_LANE = Object.freeze({
  ...predecessorRuntime, id,
  screen: Object.freeze({ width: 1512, height: 982, colorDepth: 30, pixelDepth: 30 }),
  display: Object.freeze({ physicalWidth: 1384, physicalHeight: 865, cssWidth: 692, cssHeight: 432.5,
    devicePixelRatio: 2, surfaceBytes: 4_788_640 }),
  host: Object.freeze({ ...predecessorRuntime.host,
    package: Object.freeze({ ...predecessorRuntime.host.package, version: "0.23.0-alpha.1" }) }),
});
