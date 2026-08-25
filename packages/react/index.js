import { createViewer } from "@punctra/viewer";
import { createUsePunctraViewer } from "./hook.js";

/**
 * Mounts one Punctra viewer into a caller-owned canvas and disposes it across
 * unmount, Strict Mode replay, and hot replacement. Change `mountKey` when a
 * non-viewport creation option intentionally requires viewer recreation.
 */
export const usePunctraViewer = createUsePunctraViewer(createViewer);
