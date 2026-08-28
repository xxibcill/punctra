const EXACT_QUERY_SAFE_ACTION =
  "Keep the current display as non-authoritative and retry exact confirmation against the active immutable Source.";

export class ExactQueryError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "ExactQueryError";
    this.code = code;
    this.safeAction = EXACT_QUERY_SAFE_ACTION;
  }
}
