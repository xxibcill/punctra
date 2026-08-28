import assert from "node:assert/strict";
import test from "node:test";

import { verifyNominalPickCoverage } from "./visual-selection.js";

const sourceIdentity = "21".repeat(32);
const trial = {
  id: "selected-trial",
  selection: {
    ordinals: [1866, 1913],
    point_identity_authority: "authored_source_fact",
    nominal_pick_coverage_authority: "projected_authored_point_fact",
    highlight_authority: "presentation_only",
  },
};
const expectations = [
  expectation(1866, "selected-1866", [178, 158], 4),
  expectation(1913, "selected-1913", [318, 154], 4),
];

test("nominal picks bind authored pixels to returned Point identities before highlights", async () => {
  const viewer = new FakePickViewer(expectations);
  const evidence = await verifyNominalPickCoverage(viewer, trial, expectations, {
    requestFrame: () => Promise.resolve(),
  });
  assert.equal(evidence.execution_order, "before_presentation_only_highlights");
  assert.equal(evidence.highlight_point_count_during_checks, 0);
  assert.deepEqual(evidence.checks.map(({ ordinal }) => ordinal), [1866, 1913]);
  assert(evidence.checks.every(({ passed, attempts }) => passed && attempts.at(-1).matched));
  assert.deepEqual(viewer.requestedPixels, [[178, 158], [178, 157], [318, 154]]);
  assert.deepEqual(evidence.checks[0].matched_pixel, [178, 157]);
  assert.equal(viewer.cancelCount, 3);
});

test("nominal picks reject a different provisional Point identity", async () => {
  const onePixelExpectation = {
    ...expectations[0],
    nominal_region: { x: 178, y: 158, width: 1, height: 1 },
  };
  const viewer = new FakePickViewer([onePixelExpectation]);
  viewer.pointOrdinalOverride = "999";
  await assert.rejects(
    verifyNominalPickCoverage(viewer, {
      ...trial,
      selection: { ...trial.selection, ordinals: [1866] },
    }, [onePixelExpectation], {
      requestFrame: () => Promise.resolve(),
    }),
    /was not pickable in its authored region/,
  );
});

test("unselected trials do not issue a pick", async () => {
  const viewer = new FakePickViewer([]);
  const evidence = await verifyNominalPickCoverage(viewer, {
    id: "unselected-trial",
    selection: { ordinals: [] },
  }, []);
  assert.equal(evidence, null);
  assert.deepEqual(viewer.requestedPixels, []);
});

function expectation(ordinal, featureId, expectedPixel, batchKey) {
  return {
    ordinal,
    feature_id: featureId,
    expected_pixel: expectedPixel,
    nominal_region: {
      x: expectedPixel[0] - 12,
      y: expectedPixel[1] - 12,
      width: 24,
      height: 24,
    },
    generation: 1,
    batch_key: batchKey,
    batch_version: 2,
    source_identity: sourceIdentity,
  };
}

class FakePickViewer {
  constructor(expected) {
    this.expected = expected;
    this.requestedPixels = [];
    this.cancelCount = 0;
    this.activeIndex = -1;
    this.pointOrdinalOverride = undefined;
    this.pick = emptyPick();
  }

  diagnostics() {
    return this.json();
  }

  render() {
    this.pick = emptyPick();
    return this.json();
  }

  beginPick(x, y) {
    this.activeIndex += 1;
    this.activePixel = [x, y];
    this.requestedPixels.push([x, y]);
    this.pick = { ...emptyPick(), status: "pending" };
    return this.json();
  }

  pollPick() {
    const expected = this.activePixel[0] > 250 ? this.expected.at(-1) : this.expected[0];
    const pointOrdinal = this.pointOrdinalOverride
      ?? (this.activePixel[0] === expected.expected_pixel[0]
        && this.activePixel[1] === expected.expected_pixel[1]
        && expected.ordinal === 1866 ? "999" : String(expected.ordinal));
    this.pick = {
      status: "hit",
      authority: "provisional_gpu_hint",
      generation: expected.generation,
      batch_key: expected.batch_key,
      batch_version: expected.batch_version,
      source_identity: expected.source_identity,
      point_ordinal: pointOrdinal,
    };
    return this.json();
  }

  cancelPick() {
    this.cancelCount += 1;
    this.pick = emptyPick();
    return this.json();
  }

  json() {
    return JSON.stringify({
      highlights: { point_count: 0 },
      pick: this.pick,
    });
  }
}

function emptyPick() {
  return {
    status: "not_requested",
    authority: "provisional_gpu_hint",
    generation: null,
    batch_key: null,
    batch_version: null,
    source_identity: null,
    point_ordinal: null,
  };
}
