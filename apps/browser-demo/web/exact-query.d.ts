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
  constructor(code: ExactQueryErrorCode, message: string);
}

export interface LasExactQueryBridge {
  confirm(request: {
    readonly sourceIdentity: string;
    readonly pointOrdinal: bigint | string | number;
    readonly generation: number;
    readonly signal?: AbortSignal;
  }): Promise<ExactPoint>;
}

export interface LasExactQueryBridgeOptions {
  readonly manifestUrl: string | URL;
  readonly fetchImplementation?: typeof fetch;
  readonly credentials?: RequestCredentials;
}

export function createLasExactQueryBridge(
  options: LasExactQueryBridgeOptions,
): Readonly<LasExactQueryBridge>;
