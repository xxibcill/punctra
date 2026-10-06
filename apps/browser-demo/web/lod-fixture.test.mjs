import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { decodeTransferV2, projectAuthoredPointAtViewport } from "./visual-corpus.js";
import { materializeLodFixture } from "./lod-fixture.js";

test("the closed corpus pins exact authored payloads and accepted image/timing gates", async () => {
  const corpus = JSON.parse(await readFile(new URL("./fixtures/lod-v1/corpus.json", import.meta.url)));
  assert.equal(corpus.recreations, 3);
  assert.equal(corpus.presented_steps, 8);
  assert.deepEqual(corpus.profiles.map((profile) => profile.requested_device_pixel_ratio), [1, 2, 4]);
  assert.equal(corpus.image_limits.maximum_changed_common_fraction, 0.25);
  assert.equal(corpus.image_limits.maximum_temporal_rmse, 0.40);
  assert.equal(corpus.timing_limits.frame_callback_p95_milliseconds, 50);
  assert.equal(corpus.timing_limits.canonical_predecessor_p95_ratio, 2);
  for (const source of corpus.sources) {
    const fixture = await materializeLodFixture({ projection: source.projection, opaqueCoincident: source.opaque_coincident });
    assert.equal(fixture.payload_sha256, source.payload_sha256);
    assert.equal(fixture.source_identity, source.source_identity);
    assert.deepEqual(fixture.camera, source.camera);
    assert.deepEqual(fixture.batch_point_counts, source.batch_point_counts);
  }
});

test("authored cuts preserve global identities, bounds, duplicate residency, and reproducible bytes", async () => {
  for (const projection of ["perspective", "orthographic"]) {
    const fixture = await materializeLodFixture({ projection });
    const recreated = await materializeLodFixture({ projection });
    assert.equal(fixture.payload_sha256, recreated.payload_sha256);
    assert.deepEqual(fixture.batch_point_counts, [81, 45, 36]);
    assert.equal(fixture.point_count * 24, 3_888);
    const points = fixture.batches.flatMap((bytes) => decodeTransferV2(bytes));
    assert.deepEqual(points.map((point) => point.ordinal), Array.from({ length: 162 }, (_, index) => index));
    assert.ok(points.every((point) => point.relative_position.every(Number.isFinite)));
    const viewport = { css_width: 320, css_height: 240, requested_device_pixel_ratio: 2, physical_width: 640, physical_height: 480 };
    for (const parent of points.slice(0, 81)) {
      const child = points.slice(81).find((point) => Math.abs(point.relative_position[0] / (projection === "perspective" ? 5.2 / 4.8 : 1) - parent.relative_position[0]) < 1e-6
        && Math.abs(point.relative_position[2] / (projection === "perspective" ? 5.2 / 4.8 : 1) - parent.relative_position[2]) < 1e-6);
      assert.ok(child, "each parent sample has its authored child counterpart");
      const before = projectAuthoredPointAtViewport(parent, fixture.world_origin, fixture.camera, viewport);
      const after = projectAuthoredPointAtViewport(child, fixture.world_origin, fixture.camera, viewport);
      assert.ok(Math.hypot(before.exact_x - after.exact_x, before.exact_y - after.exact_y) < 0.001);
    }
    assert.equal(fixture.fixture_authority, "authored_private_cut_not_browser_child_lod_streaming");
    const opaque = await materializeLodFixture({ projection, opaqueCoincident: true });
    assert.notEqual(opaque.source_identity, fixture.source_identity);
  }
});
