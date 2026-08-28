import assert from "node:assert/strict";
import test from "node:test";

import { VisualRunSession } from "./visual-run-session.js";

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
