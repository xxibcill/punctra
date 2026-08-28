import assert from "node:assert/strict";
import test from "node:test";

import { VisualRunSession, VisualTrialRunner } from "./visual-run-session.js";

function createSession() {
  const artifacts = {
    clearCount: 0,
    clear() {
      this.clearCount += 1;
    },
  };
  return {
    artifacts,
    session: new VisualRunSession({
      artifactRegistry: artifacts,
      runnerStateSchema: "runner-state-v1",
    }),
  };
}

test("VisualRunSession owns runner and evidence transitions", () => {
  const { artifacts, session } = createSession();
  assert.deepEqual(session.runnerState(), {
    schema: "runner-state-v1",
    status: "idle",
    mode: null,
    trial_id: null,
    recreation_index: null,
    completed_trials: 0,
    total_trials: 0,
    message: "Idle",
  });

  session.resetForRun();
  session.configureTransport({ maximum_entries: 5 }, 1024);
  session.setBaselineInputsEntry({ manifest: { schema: "baseline-v1" } });
  const draft = { summary: null };
  const corpus = { rubric: { schema: "rubric-v1" } };
  session.stageReview(draft, corpus);
  session.rubricReview = { ready: true };

  assert.equal(artifacts.clearCount, 1);
  assert.equal(session.draft, draft);
  assert.equal(session.corpus, corpus);
  assert.deepEqual(session.transportPolicy, { maximum_entries: 5 });
  assert.equal(session.artifactByteCeiling, 1024);
  assert.equal(session.baselineInputsEntry.manifest.schema, "baseline-v1");

  assert.equal(session.completeReview(), draft);
  assert.equal(session.report, draft);
  assert.equal(session.draft, undefined);
  assert.equal(session.corpus, undefined);
  assert.equal(session.rubricReview, undefined);
});

test("VisualRunSession coalesces concurrent runs and blocks a pending review", async () => {
  const { session } = createSession();
  let release;
  let calls = 0;
  const pending = session.start(() => {
    calls += 1;
    return new Promise((resolve) => {
      release = resolve;
    });
  });
  const duplicate = session.start(() => Promise.resolve("duplicate"));
  assert.equal(pending, duplicate);
  await Promise.resolve();
  assert.equal(calls, 1);
  release("complete");
  assert.equal(await pending, "complete");

  session.stageReview({}, {});
  assert.throws(
    () => session.start(() => Promise.resolve()),
    /submit the pending post-capture review/,
  );
});

test("VisualRunSession pins one Wasm runtime and initializes it once", async () => {
  const { session } = createSession();
  const bytes = new Uint8Array([1, 2, 3]);
  session.bindWasmRuntime("3:abc", bytes);
  bytes[0] = 9;
  let initializations = 0;
  const initialize = (boundBytes) => {
    initializations += 1;
    assert.deepEqual([...boundBytes], [1, 2, 3]);
    return Promise.resolve();
  };
  await session.initializeWasm(initialize);
  await session.initializeWasm(initialize);
  assert.equal(initializations, 1);
  assert.throws(
    () => session.bindWasmRuntime("3:def", new Uint8Array([1, 2, 3])),
    /Wasm runtime bytes changed/,
  );
});

test("VisualTrialRunner accepts one checked-in trial id and owns its complete recreation lifecycle", async () => {
  const trial = {
    id: "generated-neutral",
    source_id: "generated-source",
    display_mode: "neutral",
    conditions: ["dense"],
    coverage: "authored",
    selection: { ordinals: [] },
    features: [{ id: "centre" }],
    tolerance_profile: "canonical",
    temporal_tolerance_profile: "stable",
  };
  const materialized = {
    camera: { projection: "perspective", eye: [1, 2, 3] },
    input_facts: { source: "generated" },
    source: {
      expected_view: {
        stream_coverage: "authored",
        published_points: 4,
        settled_drawn_points: 4,
      },
    },
  };
  const baselinePng = new Uint8Array([1, 2, 3]);
  const baselineArtifact = { path: "baseline.png" };
  const recreationCalls = [];
  const runner = new VisualTrialRunner({
    corpus: { trials: [trial] },
    corpusUrl: new URL("http://127.0.0.1:8000/corpus.json"),
    mode: "record",
    environmentTracker: { id: "environment" },
    artifacts: { recordMetadata: () => assert.fail("record mode must not load a baseline") },
    materializeTrial: async (corpus, trialId, options) => {
      assert.equal(corpus.trials[0], trial);
      assert.equal(trialId, trial.id);
      assert.equal(options.corpusUrl.href, "http://127.0.0.1:8000/corpus.json");
      return materialized;
    },
    loadBaseline: async () => assert.fail("record mode must not load a baseline"),
    runRecreation: async (options) => {
      recreationCalls.push(options);
      return {
        internal_final_png: options.recreationIndex === 0 ? baselinePng : undefined,
        record: {
          index: options.recreationIndex,
          capture: { artifact: options.recreationIndex === 0 ? baselineArtifact : { path: `recreation-${options.recreationIndex}.png` } },
          failures: [],
        },
      };
    },
    updateRunnerState: () => {},
    repositoryBaselinePath: () => "fixtures/baseline.png",
    observationArtifactPath: (_trialId, recreationIndex) => `evidence/recreation-${recreationIndex}.png`,
    buildBatchFacts: () => ({ schema: "batch-facts-v1" }),
    recreationCount: 3,
  });

  const result = await runner.run(trial.id);

  assert.equal(recreationCalls.length, 3);
  assert.equal(recreationCalls[0].baselinePng, undefined);
  assert.equal(recreationCalls[0].finalArtifact.kind, "baseline_png");
  assert.equal(recreationCalls[1].baselinePng, baselinePng);
  assert.equal(recreationCalls[1].finalArtifact.kind, "recreation_png");
  assert.equal(recreationCalls[2].environmentTracker.id, "environment");
  assert.deepEqual(result, {
    trial_id: trial.id,
    source_id: trial.source_id,
    display_mode: trial.display_mode,
    projection: materialized.camera.projection,
    conditions: trial.conditions,
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
    camera: materialized.camera,
    selection: trial.selection,
    features: trial.features,
    expected_view: materialized.source.expected_view,
    batch_facts: { schema: "batch-facts-v1" },
    tolerance_profile: trial.tolerance_profile,
    temporal_tolerance_profile: trial.temporal_tolerance_profile,
    baseline: baselineArtifact,
    recreations: recreationCalls.map((_call, index) => ({
      index,
      capture: { artifact: index === 0 ? baselineArtifact : { path: `recreation-${index}.png` } },
      failures: [],
    })),
    passed: true,
    failures: [],
  });
  await assert.rejects(() => runner.run("missing-trial"), /checked-in trial missing-trial is unavailable/);
});
