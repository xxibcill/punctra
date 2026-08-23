import {
  StreamingFailure,
  createWorkerMessage,
  runStreamingOperation,
} from "./streaming-protocol.js?v=16-qualified";
import { WORKER_SCHEMA, workerFailure } from "./worker-protocol.js?v=16-qualified";

let active;
const memoryCacheStorage = new Map();

self.addEventListener("message", (event) => {
  const message = event.data;
  if (message?.schema !== WORKER_SCHEMA) {
    publishFailure(message?.operation_id, new StreamingFailure("manifest_invalid", "worker message schema differs"));
    return;
  }
  if (message.type === "cancel") {
    if (active?.operationId === message.operation_id) active.controller.abort();
    return;
  }
  if (message.type !== "start") {
    publishFailure(message.operation_id, new StreamingFailure("manifest_invalid", `unsupported worker message ${message.type}`));
    return;
  }
  if (active) {
    publishFailure(message.operation_id, new StreamingFailure("resource_limit", "one worker operation is already active"));
    return;
  }
  void start(message);
});

async function start(message) {
  const controller = new AbortController();
  active = { operationId: message.operation_id, controller };
  publish(message.operation_id, "state", { phase: "starting" });
  try {
    const result = await runStreamingOperation(
      {
        manifestUrl: new URL(message.manifest_url, self.location.href).href,
        cacheMode: message.cache_mode,
        memoryCacheStorage,
        invalidate: message.invalidate,
        credentials: message.credentials,
        signal: controller.signal,
      },
      {
        onDeployment: (deployment) => publish(message.operation_id, "state", {
          phase: "deployment",
          deployment: publicDeployment(deployment),
        }),
        onState: (phase, metrics) => publish(message.operation_id, "state", { phase, metrics }),
        onBatch: (buffer, facts) => publishBatch(message.operation_id, buffer, facts),
      },
    );
    publish(message.operation_id, "complete", {
      deployment: publicDeployment(result.deployment),
      metrics: result.metrics,
      decode: result.decode,
    });
  } catch (error) {
    publishFailure(message.operation_id, error);
  } finally {
    active = undefined;
  }
}

function publishBatch(operationId, buffer, facts) {
  self.postMessage(
    createWorkerMessage(operationId, "batch", {
      batch_index: facts.batchIndex,
      point_count: facts.pointCount,
      payload: buffer,
    }),
    [buffer],
  );
}

function publish(operationId, type, facts) {
  self.postMessage(createWorkerMessage(operationId, type, facts));
}

function publishFailure(operationId, error) {
  const failure = error instanceof StreamingFailure ? error.toJSON() : workerFailure(error?.message ?? String(error));
  self.postMessage(createWorkerMessage(operationId, "failure", {
    code: failure.code,
    message: failure.message,
    safe_action: failure.safe_action,
  }));
}

function publicDeployment(deployment) {
  return {
    schema: deployment.schema,
    deployment_id: deployment.deploymentId,
    source_identity: deployment.source.sourceIdentity,
    source_byte_length: deployment.source.byteLength,
    source_point_count: deployment.source.pointCount,
    root_display_point_count: deployment.index.root.displayPointCount,
    root_coverage: deployment.index.root.coverage,
    world_origin: deployment.index.root.worldOrigin,
  };
}

self.addEventListener("error", (event) => {
  if (active) {
    publishFailure(active.operationId, new Error(event.message));
    active.controller.abort();
  }
});
