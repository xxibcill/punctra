export function createVisualValidator(errorPrefix) {
  const requireCondition = (condition, message) => {
    if (!condition) throw new Error(`${errorPrefix}: ${message}`);
  };

  const requireRecord = (value, label) => {
    requireCondition(
      value !== null && typeof value === "object" && !Array.isArray(value),
      `${label} must be an object`,
    );
  };

  return Object.freeze({ requireCondition, requireRecord });
}

export function cloneJson(value) {
  return JSON.parse(JSON.stringify(value));
}

export function jsonEqual(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

/**
 * Serializes a JSON value with recursively sorted object keys so that records
 * built in the browser compare equal to the same values served by the local
 * server, which emits `sort_keys=True` JSON.
 */
export function canonicalJson(value) {
  return JSON.stringify(value, (_key, entry) => {
    if (entry === null || typeof entry !== "object" || Array.isArray(entry)) return entry;
    return Object.fromEntries(Object.keys(entry).sort().map((key) => [key, entry[key]]));
  });
}

export function canonicalJsonEqual(left, right) {
  return canonicalJson(left) === canonicalJson(right);
}

export function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

export function parsePageUrl(value, errorPrefix) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${errorPrefix}: page URL is unavailable`);
  }
  try {
    return new URL(value);
  } catch (error) {
    throw new Error(`${errorPrefix}: page URL is invalid: ${errorMessage(error)}`);
  }
}
