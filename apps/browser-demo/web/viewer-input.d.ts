export type NormalizedViewerInput =
  | Readonly<{
      kind: "orbit" | "pan";
      deltaX: number;
      deltaY: number;
      source: string;
    }>
  | Readonly<{
      kind: "zoom";
      delta: number;
      source: "wheel" | "touch";
    }>
  | Readonly<{
      kind: "keyboard";
      code: string;
      repeat: boolean;
      modifiers: Readonly<{
        alt: boolean;
        control: boolean;
        meta: boolean;
        shift: boolean;
      }>;
    }>;

export function createInputNormalizer(
  target: EventTarget,
  listener: (input: NormalizedViewerInput) => void,
  options?: { readonly preventDefault?: boolean },
): Readonly<{ dispose(): void }>;
