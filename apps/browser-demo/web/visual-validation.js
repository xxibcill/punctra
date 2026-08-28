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
