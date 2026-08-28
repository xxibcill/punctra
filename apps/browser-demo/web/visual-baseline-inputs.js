import { createVisualValidator } from "./visual-validation.js";

export const BASELINE_INPUTS_SCHEMA = "punctra-browser-visual-baseline-inputs-v1";
export const BASELINE_INPUTS_PATH = "apps/browser-demo/web/fixtures/visual-v1/baseline-inputs.json";

const BASELINE_IDENTITY_FIELDS = Object.freeze([
  "trial_id",
  "path",
  "width",
  "height",
  "encoded_byte_length",
  "encoded_sha256",
  "decoded_byte_length",
  "decoded_sha256",
]);
const { requireCondition, requireRecord } = createVisualValidator("Visual baseline inputs invalid");

/** Creates the deterministic pre-pin input manifest from one record-mode run. */
export function createBaselineInputsManifest(options) {
  const { release, trials, artifacts, runtimeArtifacts } = options;
  requireCondition(release === "0.21.0-alpha.1", "baseline-input release differs");
  requireCondition(Array.isArray(trials) && trials.length > 0, "baseline-input trials are absent");
  requireCondition(Array.isArray(artifacts), "baseline-input artifact registry is absent");
  const registry = new Map(artifacts.map((artifact) => [artifact.path, artifact]));
  requireCondition(registry.size === artifacts.length, "baseline-input artifact paths are duplicated");
  const canonicalBaselines = trials.map((trial) => {
    const path = `apps/browser-demo/web/fixtures/visual-v1/${trial.baseline_path.replace(/^\.\//, "")}`;
    const artifact = registry.get(path);
    requireRecord(artifact, `baseline-input artifact ${path}`);
    requireCondition(artifact.kind === "baseline_png", `baseline-input artifact ${path} kind differs`);
    requireCondition(artifact.trial_id === trial.id, `baseline-input artifact ${path} trial differs`);
    return selectFields(artifact, BASELINE_IDENTITY_FIELDS);
  });
  return validateBaselineInputsManifest({
    schema: BASELINE_INPUTS_SCHEMA,
    release,
    package_artifact: {
      package_name: "@punctra/viewer",
      package_version: release,
      runtime_artifacts: cloneJson(runtimeArtifacts),
    },
    canonical_baselines: canonicalBaselines,
  }, { trials, runtimeArtifacts });
}

/** Validates the checked-in pre-pin record without accepting extra identities. */
export function validateBaselineInputsManifest(value, options) {
  requireRecord(value, "baseline-input manifest");
  requireCondition(value.schema === BASELINE_INPUTS_SCHEMA, "baseline-input schema differs");
  requireCondition(value.release === "0.21.0-alpha.1", "baseline-input release differs");
  requireCondition(!Object.hasOwn(value, "implementation_commit"), "baseline-input manifest must not contain an implementation commit");
  requireRecord(value.package_artifact, "baseline-input package artifact");
  requireCondition(value.package_artifact.package_name === "@punctra/viewer", "baseline-input package name differs");
  requireCondition(value.package_artifact.package_version === value.release, "baseline-input package version differs");
  validateDigestRecords(value.package_artifact.runtime_artifacts, "baseline-input runtime artifacts");
  if (options?.runtimeArtifacts !== undefined) {
    requireCondition(deepEqual(value.package_artifact.runtime_artifacts, options.runtimeArtifacts), "baseline-input runtime artifacts differ from fetched bytes");
  }
  requireCondition(Array.isArray(value.canonical_baselines) && value.canonical_baselines.length > 0, "baseline-input canonical baselines are absent");
  for (const baseline of value.canonical_baselines) validateBaselineIdentity(baseline);
  if (options?.trials !== undefined) {
    requireCondition(value.canonical_baselines.length === options.trials.length, "baseline-input trial count differs");
    for (let index = 0; index < options.trials.length; index += 1) {
      const trial = options.trials[index];
      const baseline = value.canonical_baselines[index];
      const path = `apps/browser-demo/web/fixtures/visual-v1/${trial.baseline_path.replace(/^\.\//, "")}`;
      requireCondition(baseline.trial_id === trial.id && baseline.path === path, `baseline-input trial ${trial.id} binding differs`);
    }
  }
  if (options?.artifacts !== undefined) {
    const registry = new Map(options.artifacts.map((artifact) => [artifact.path, artifact]));
    for (const baseline of value.canonical_baselines) {
      const artifact = registry.get(baseline.path);
      requireRecord(artifact, `baseline-input registry artifact ${baseline.path}`);
      requireCondition(deepEqual(baseline, selectFields(artifact, BASELINE_IDENTITY_FIELDS)), `baseline-input artifact ${baseline.path} identity differs`);
    }
  }
  return value;
}

export function encodeBaselineInputsManifest(value) {
  return new TextEncoder().encode(`${JSON.stringify(value, null, 2)}\n`);
}

function validateBaselineIdentity(value) {
  requireRecord(value, "baseline-input canonical baseline");
  requireCondition(Object.keys(value).length === BASELINE_IDENTITY_FIELDS.length, "baseline-input canonical baseline fields differ");
  requireCondition(typeof value.trial_id === "string" && value.trial_id.length > 0, "baseline-input trial identity differs");
  requireCondition(typeof value.path === "string" && value.path.startsWith("apps/browser-demo/web/fixtures/visual-v1/baselines/") && value.path.endsWith(".png"), "baseline-input path differs");
  for (const field of ["width", "height", "encoded_byte_length", "decoded_byte_length"]) {
    requireCondition(Number.isSafeInteger(value[field]) && value[field] > 0, `baseline-input ${field} differs`);
  }
  for (const field of ["encoded_sha256", "decoded_sha256"]) {
    requireCondition(/^[0-9a-f]{64}$/.test(value[field]), `baseline-input ${field} differs`);
  }
}

function validateDigestRecords(values, label) {
  requireCondition(Array.isArray(values) && values.length > 0, `${label} are absent`);
  const paths = new Set();
  for (const value of values) {
    requireRecord(value, label);
    requireCondition(Object.keys(value).length === 3, `${label} fields differ`);
    requireCondition(typeof value.path === "string" && value.path.length > 0 && !paths.has(value.path), `${label} path differs`);
    paths.add(value.path);
    requireCondition(Number.isSafeInteger(value.byte_length) && value.byte_length > 0, `${label} byte length differs`);
    requireCondition(/^[0-9a-f]{64}$/.test(value.sha256), `${label} SHA-256 differs`);
  }
}

function selectFields(value, fields) {
  return Object.fromEntries(fields.map((field) => [field, value[field]]));
}

function cloneJson(value) {
  return JSON.parse(JSON.stringify(value));
}

function deepEqual(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}
