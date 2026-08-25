import type {
  BrowserViewer,
  CreateViewerOptions,
  ViewerState,
  ViewportInput,
} from "@punctra/viewer";

export interface UsePunctraViewerOptions extends Omit<CreateViewerOptions, "canvas" | "viewport"> {
  readonly canvas: HTMLCanvasElement | null;
  readonly viewport: ViewportInput;
  /** Pause presentation without destroying Source loading or viewer state. */
  readonly active?: boolean;
  /** Change this identity to recreate the viewer after non-viewport options change. */
  readonly mountKey?: string | number;
}

export type PunctraViewerBinding =
  | Readonly<{ status: "idle" | "loading"; viewer: null; state: null; error: null }>
  | Readonly<{ status: "ready"; viewer: BrowserViewer; state: ViewerState; error: null }>
  | Readonly<{ status: "failed"; viewer: BrowserViewer | null; state: ViewerState | null; error: unknown }>;

/** Bind one caller-owned canvas to the Punctra viewer lifecycle. */
export function usePunctraViewer(options: UsePunctraViewerOptions): PunctraViewerBinding;
