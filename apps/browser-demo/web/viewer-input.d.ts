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

export interface InputNormalizerOptions {
  readonly preventDefault?: boolean;
}

export interface InputNormalizer {
  dispose(): void;
}

export function createInputNormalizer(
  target: EventTarget,
  listener: (input: NormalizedViewerInput) => void,
  options?: InputNormalizerOptions,
): Readonly<InputNormalizer>;
