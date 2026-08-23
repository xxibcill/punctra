import assert from "node:assert/strict";
import test from "node:test";

import { createDeferredStreamPublication } from "./stream-publication.js";

test("a pre-batch worker failure leaves the prior frame untouched", () => {
  const viewer = new FakeViewer();
  const publication = createPublication(viewer);

  publication.acceptDeployment(deployment());

  assert.equal(viewer.frame, "prior");
  assert.deepEqual(viewer.calls, []);
});

test("the first batch resets and publishes as one host event", () => {
  const viewer = new FakeViewer();
  const published = [];
  const publication = createPublication(viewer, published);
  publication.acceptDeployment(deployment());

  const rendered = publication.publishBatch({
    batch_index: 0,
    payload: new ArrayBuffer(24),
  });

  assert.deepEqual(viewer.calls, ["begin_batch", "render"]);
  assert.equal(rendered.frame, "replacement");
  assert.equal(published.at(-1).frame, "replacement");
});

test("an invalid first batch cannot reset the prior frame", () => {
  const viewer = new FakeViewer();
  const publication = createPublication(viewer);
  publication.acceptDeployment(deployment());

  assert.throws(
    () => publication.publishBatch({ batch_index: 0, payload: new ArrayBuffer(0) }),
    /invalid first batch/,
  );
  assert.equal(viewer.frame, "prior");
});

function createPublication(viewer, published = []) {
  return createDeferredStreamPublication({
    viewer,
    assertFact: (condition, message) => assert.ok(condition, message),
    publishDiagnostics: (diagnostics) => published.push(diagnostics),
  });
}

function deployment() {
  return {
    root_coverage: "sampled",
    source_identity: "01".repeat(32),
    root_display_point_count: 1,
    world_origin: [0, 0, 0],
  };
}

class FakeViewer {
  constructor() {
    this.frame = "prior";
    this.calls = [];
  }

  beginStreamBatch(_source, _points, _x, _y, _z, _batchIndex, payload) {
    this.calls.push("begin_batch");
    if (payload.byteLength !== 24) throw new Error("invalid first batch");
    this.frame = "replacement";
    return JSON.stringify({ streaming: streamingFacts() });
  }

  publishStreamBatch() {
    this.calls.push("publish");
    return JSON.stringify({ streaming: streamingFacts() });
  }

  render() {
    this.calls.push("render");
    return JSON.stringify({ frame: this.frame });
  }

  completeStream() {
    this.calls.push("complete");
    return JSON.stringify({});
  }
}

function streamingFacts() {
  return {
    main_thread_batch_points_high_water: 1,
    main_thread_batch_bytes_high_water: 24,
  };
}
