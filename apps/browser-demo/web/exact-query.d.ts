import type { ExactPoint } from "./viewer-api.js";

export type ExactQueryErrorCode =
  | "exact_query_invalid"
  | "exact_query_unavailable"
  | "exact_query_busy"
  | "exact_query_cancelled"
  | "exact_query_source_mismatch"
  | "exact_query_source_changed"
  | "exact_query_incompatible"
  | "exact_query_corrupt"
  | "exact_query_truncated"
  | "exact_query_range_unsupported"
  | "exact_query_content_encoding"
  | "exact_query_failed";

export class ExactQueryError extends Error {
  readonly code: ExactQueryErrorCode;
  readonly safeAction: string;
  constructor(code: ExactQueryErrorCode, message: string, options?: { cause?: unknown });
}

export interface LasExactQueryLayout {
  readonly pointDataOffset: number;
  readonly pointFormat: 3;
  readonly pointRecordBytes: 34;
  readonly pointCount: number;
  readonly scale: readonly [number, number, number];
  readonly offset: readonly [number, number, number];
}

export interface LasExactQueryDeployment {
  readonly source: Readonly<{
    sourceIdentity: string;
    byteLength: number;
    pointCount: number;
  }>;
  readonly index: Readonly<{
    transform: Readonly<{
      scale: readonly [number, number, number];
      offset: readonly [number, number, number];
    }>;
  }>;
}

export interface LasExactQueryBridge {
  confirm(request: {
    readonly sourceIdentity: string;
    readonly pointOrdinal: bigint | string | number;
    readonly generation: number;
    readonly signal?: AbortSignal;
  }): Promise<ExactPoint>;
}

export function createLasExactQueryBridge(options: {
  readonly manifestUrl: string | URL;
  readonly fetchImplementation?: typeof fetch;
  readonly credentials?: RequestCredentials;
}): Readonly<LasExactQueryBridge>;

export function decodeLasLayout(
  bytes: Uint8Array,
  deployment: LasExactQueryDeployment,
): Readonly<LasExactQueryLayout>;

export function decodeLasPointRecord(
  bytes: Uint8Array,
  sourceIdentity: string,
  pointOrdinal: number,
  generation: number,
  layout: LasExactQueryLayout,
): ExactPoint;
