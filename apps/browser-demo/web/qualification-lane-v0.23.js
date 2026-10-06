// Browser-reported screen facts changed in the current session. Hardware and
// browser identity still require fresh observations; declarations qualify nothing.
import { QUALIFICATION_LANE as predecessorLane, QUALIFICATION_RUNTIME_LANE as predecessorRuntime } from "./qualification-lane-v0.22.js";

const id = "codex-iab-chromium-154-macos-26-apple-m5-pro-screen-1920";
export const QUALIFICATION_LANE = Object.freeze({
  ...predecessorLane, id,
  display: Object.freeze({ ...predecessorLane.display, screen_css_pixels: Object.freeze([1920, 1080]),
    physical_viewport: Object.freeze([1384, 865]), css_viewport: Object.freeze([692, 432.5]), canvas_bytes: 4_788_640,
    color_depth: 24, pixel_depth: 24,
    screen_note: "Exact browser-reported current session facts; this does not qualify physical panel composition or color." }),
});
export const QUALIFICATION_RUNTIME_LANE = Object.freeze({
  ...predecessorRuntime, id,
  screen: Object.freeze({ width: 1920, height: 1080, colorDepth: 24, pixelDepth: 24 }),
  display: Object.freeze({ physicalWidth: 1384, physicalHeight: 865, cssWidth: 692, cssHeight: 432.5,
    devicePixelRatio: 2, surfaceBytes: 4_788_640 }),
  host: Object.freeze({ ...predecessorRuntime.host,
    package: Object.freeze({ ...predecessorRuntime.host.package, version: "0.23.0-alpha.1" }) }),
});
