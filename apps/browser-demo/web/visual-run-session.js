import { cloneJson, createVisualValidator } from "./visual-validation.js";

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

export class VisualTrialRunner {
  #context;

  constructor(context) {
    requireCondition(context?.mode === "record" || context?.mode === "verify", "visual trial mode is invalid");
    requireCondition(Array.isArray(context.corpus?.trials), "visual trial corpus is invalid");
    requireCondition(Number.isSafeInteger(context.recreationCount) && context.recreationCount > 0, "visual recreation count is invalid");
    for (const name of [
      "materializeTrial",
      "loadBaseline",
      "runRecreation",
      "updateRunnerState",
      "repositoryBaselinePath",
      "observationArtifactPath",
      "buildBatchFacts",
    ]) {
      requireCondition(typeof context[name] === "function", `visual trial ${name} collaborator is invalid`);
    }
    this.#context = Object.freeze({ ...context });
  }

  async run(trialId) {
    const trial = this.#checkedInTrial(trialId);
    const materialized = await this.#context.materializeTrial(
      this.#context.corpus,
      trial.id,
      { corpusUrl: this.#context.corpusUrl },
    );
    const baselinePath = this.#context.repositoryBaselinePath(trial);
    const baseline = await this.#loadBaseline(trial, baselinePath);
    const completed = await this.#runRecreations({
      trial,
      materialized,
      baselinePath,
      ...baseline,
    });
    return this.#buildResult(trial, materialized, completed);
  }

  #checkedInTrial(trialId) {
    requireCondition(typeof trialId === "string" && trialId.length > 0, "visual trial id is invalid");
    const trial = this.#context.corpus.trials.find(({ id }) => id === trialId);
    requireCondition(trial !== undefined, `checked-in trial ${trialId} is unavailable`);
    return trial;
  }

  async #loadBaseline(trial, baselinePath) {
    if (this.#context.mode === "record") {
      return { baselinePng: undefined, baselineMetadata: undefined };
    }
    const loaded = await this.#context.loadBaseline(
      trial,
      this.#context.corpusUrl,
      baselinePath,
    );
    this.#context.artifacts.recordMetadata(loaded.metadata, loaded.bytes);
    return { baselinePng: loaded.bytes, baselineMetadata: loaded.metadata };
  }

  async #runRecreations(options) {
    let { baselinePng, baselineMetadata } = options;
    const recreations = [];
    for (let recreationIndex = 0; recreationIndex < this.#context.recreationCount; recreationIndex += 1) {
      this.#context.updateRunnerState({
        trial_id: options.trial.id,
        recreation_index: recreationIndex,
        message: `${options.trial.id}: recreation ${recreationIndex + 1}/${this.#context.recreationCount}`,
      });
      const adoptAsBaseline = this.#context.mode === "record" && recreationIndex === 0;
      const recreation = await this.#context.runRecreation({
        corpus: this.#context.corpus,
        trial: options.trial,
        materialized: options.materialized,
        recreationIndex,
        baselinePng,
        finalArtifact: adoptAsBaseline
          ? { kind: "baseline_png", path: options.baselinePath }
          : {
              kind: "recreation_png",
              path: this.#context.observationArtifactPath(options.trial.id, recreationIndex, "settled"),
            },
        environmentTracker: this.#context.environmentTracker,
      });
      if (adoptAsBaseline) {
        baselinePng = recreation.internal_final_png;
        baselineMetadata = recreation.record.capture.artifact;
      }
      recreations.push(recreation.record);
    }
    return { baselineMetadata, recreations };
  }

  #buildResult(trial, materialized, completed) {
    const failures = completed.recreations.flatMap((recreation) => recreation.failures.map(
      (failure) => `recreation:${recreation.index}:${failure}`,
    ));
    return {
      trial_id: trial.id,
      source_id: trial.source_id,
      display_mode: trial.display_mode,
      projection: materialized.camera.projection,
      conditions: [...trial.conditions],
      coverage: {
        declared: trial.coverage,
        raw_stream: materialized.source.expected_view.stream_coverage,
        expected_points: materialized.source.expected_view.published_points,
        settled_drawn_points: materialized.source.expected_view.settled_drawn_points,
        declared_authority: "source_or_authored_facts_only",
        settled_draw_authority: "presentation_only",
        query_completion: "not_inferred_from_visual_evidence",
      },
      input_facts: materialized.input_facts,
      camera: cloneJson(materialized.camera),
      selection: cloneJson(trial.selection),
      features: cloneJson(trial.features),
      expected_view: cloneJson(materialized.source.expected_view),
      batch_facts: this.#context.buildBatchFacts(trial, materialized.source.expected_view),
      tolerance_profile: trial.tolerance_profile,
      temporal_tolerance_profile: trial.temporal_tolerance_profile,
      baseline: completed.baselineMetadata,
      recreations: completed.recreations,
      passed: failures.length === 0 && completed.recreations.length === this.#context.recreationCount,
      failures,
    };
  }
}
