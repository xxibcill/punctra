export function createDeferredStreamPublication({
  viewer,
  assertFact,
  publishDiagnostics,
  parseDiagnostics = JSON.parse,
}) {
  let deployment;
  let begun = false;

  return Object.freeze({
    hasBegun() {
      return begun;
    },

    acceptDeployment(nextDeployment) {
      assertFact(nextDeployment.root_coverage === "sampled", "remote root sampled Coverage");
      deployment = nextDeployment;
    },

    publishBatch(message) {
      assertFact(deployment !== undefined, "deployment precedes remote batches");
      assertFact(message.payload instanceof ArrayBuffer, "transferable worker batch");
      let diagnostics;
      if (!begun) {
        const [x, y, z] = deployment.world_origin;
        const [minimumZ, maximumZ] = [
          deployment.source_bounds.min[2],
          deployment.source_bounds.max[2],
        ];
        diagnostics = parseDiagnostics(
          viewer.beginStreamBatch(
            deployment.source_identity,
            deployment.root_display_point_count,
            x,
            y,
            z,
            minimumZ,
            maximumZ,
            message.batch_index,
            new Uint8Array(message.payload),
          ),
        );
        begun = true;
      } else {
        diagnostics = parseDiagnostics(
          viewer.publishStreamBatch(message.batch_index, new Uint8Array(message.payload)),
        );
      }
      assertFact(
        diagnostics.streaming.main_thread_batch_points_high_water <= 1_024,
        "main-thread Point work ceiling",
      );
      assertFact(
        diagnostics.streaming.main_thread_batch_bytes_high_water <= 32_768,
        "main-thread byte work ceiling",
      );
      const rendered = parseDiagnostics(viewer.render());
      publishDiagnostics(rendered);
      return rendered;
    },

    complete() {
      assertFact(begun, "at least one remote batch precedes completion");
      parseDiagnostics(viewer.completeStream());
      return parseDiagnostics(viewer.render());
    },
  });
}
