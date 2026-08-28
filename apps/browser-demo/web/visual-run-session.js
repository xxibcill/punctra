import { createVisualValidator } from "./visual-validation.js";

const { requireCondition } = createVisualValidator("Visual runner failed");

export class VisualRunSession {
  #activeRun;
  #artifactRegistry;
  #wasmInitialization;
  #wasmRuntimeBytes;
  #wasmRuntimeIdentity;
  #report;
  #draft;
  #corpus;
  #transportPolicy;
  #artifactByteCeiling;
  #baselineInputsEntry;
  #rubricReview;
  #verifyProvenance = null;
  #runnerState;

  constructor({ artifactRegistry, runnerStateSchema }) {
    this.#artifactRegistry = artifactRegistry;
    this.#runnerState = {
      schema: runnerStateSchema,
      status: "idle",
      mode: null,
      trial_id: null,
      recreation_index: null,
      completed_trials: 0,
      total_trials: 0,
      message: "Idle",
    };
  }

  get artifacts() {
    return this.#artifactRegistry;
  }

  get report() {
    return this.#report;
  }

  get draft() {
    return this.#draft;
  }

  get corpus() {
    return this.#corpus;
  }

  get transportPolicy() {
    return this.#transportPolicy;
  }

  get artifactByteCeiling() {
    return this.#artifactByteCeiling;
  }

  get baselineInputsEntry() {
    return this.#baselineInputsEntry;
  }

  get rubricReview() {
    return this.#rubricReview;
  }

  set rubricReview(review) {
    this.#rubricReview = review;
  }

  get verifyProvenance() {
    return this.#verifyProvenance;
  }

  set verifyProvenance(provenance) {
    this.#verifyProvenance = provenance;
  }

  runnerState() {
    return cloneJson(this.#runnerState);
  }

  updateRunnerState(patch) {
    this.#runnerState = { ...this.#runnerState, ...patch };
    return this.runnerState();
  }

  start(run) {
    if (this.#activeRun !== undefined) return this.#activeRun;
    requireCondition(this.#draft === undefined, "submit the pending post-capture review before starting another run");
    this.#activeRun = Promise.resolve()
      .then(run)
      .finally(() => {
        this.#activeRun = undefined;
      });
    return this.#activeRun;
  }

  resetForRun() {
    this.#artifactRegistry.clear();
    this.#report = undefined;
    this.#draft = undefined;
    this.#corpus = undefined;
    this.#transportPolicy = undefined;
    this.#artifactByteCeiling = undefined;
    this.#baselineInputsEntry = undefined;
  }

  configureTransport(policy, artifactByteCeiling) {
    this.#transportPolicy = cloneJson(policy);
    this.#artifactByteCeiling = artifactByteCeiling;
  }

  setBaselineInputsEntry(entry) {
    this.#baselineInputsEntry = entry;
  }

  stageReview(record, corpus) {
    this.#draft = record;
    this.#corpus = corpus;
  }

  completeRun(record) {
    this.#report = record;
    this.#draft = undefined;
    this.#corpus = undefined;
  }

  completeReview() {
    requireCondition(this.#draft !== undefined && this.#corpus !== undefined, "post-capture evidence draft is unavailable");
    this.#report = this.#draft;
    this.#draft = undefined;
    this.#corpus = undefined;
    this.#rubricReview = undefined;
    return this.#report;
  }

  bindWasmRuntime(identity, bytes) {
    if (this.#wasmRuntimeIdentity === undefined) {
      this.#wasmRuntimeIdentity = identity;
      this.#wasmRuntimeBytes = bytes.slice();
      return;
    }
    requireCondition(identity === this.#wasmRuntimeIdentity, "Wasm runtime bytes changed after module initialization");
  }

  initializeWasm(initialize) {
    requireCondition(this.#wasmRuntimeBytes instanceof Uint8Array, "Wasm runtime bytes were not captured before viewer creation");
    this.#wasmInitialization ??= initialize(this.#wasmRuntimeBytes);
    return this.#wasmInitialization;
  }
}

function cloneJson(value) {
  return JSON.parse(JSON.stringify(value));
}
