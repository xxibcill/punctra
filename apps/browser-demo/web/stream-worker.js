const WORKER_CACHE_TOKEN = encodeURIComponent(
  new URL(import.meta.url).searchParams.get("punctra-v") ?? "unversioned",
);
const PRODUCTION_BUNDLE = typeof import.meta.env !== "undefined" && import.meta.env.PROD;

let active;
const memoryCacheStorage = new Map();
let StreamingFailure;
let createWorkerMessage;
let runStreamingOperation;
let WORKER_SCHEMA;
let boundedWorkerFailureMessage;
let isWorkerOperationId;
let workerFailure;
let workerOperationId;
const dependencyModules = PRODUCTION_BUNDLE
  ? Promise.all([
      import("./streaming-protocol.js"),
      import("./worker-protocol.js"),
    ])
  : Promise.all([
      import(`./streaming-protocol.js?v=${WORKER_CACHE_TOKEN}`),
      import(`./worker-protocol.js?v=${WORKER_CACHE_TOKEN}`),
    ]);
const dependenciesReady = dependencyModules.then(([streamingProtocol, workerProtocol]) => {
  ({ StreamingFailure, createWorkerMessage, runStreamingOperation } = streamingProtocol);
  ({
    WORKER_SCHEMA,
    boundedWorkerFailureMessage,
    isWorkerOperationId,
    workerFailure,
    workerOperationId,
  } = workerProtocol);
});

self.addEventListener("message", (event) => {
  return handleMessage(event);
});

async function handleMessage(event) {
  await dependenciesReady;
  const message = event.data;
  const operationId = workerOperationId(message?.operation_id);
  if (message?.schema !== WORKER_SCHEMA) {
    publishFailure(
      operationId,
      new StreamingFailure("manifest_invalid", "worker message schema differs"),
    );
    return;
  }
  if (!isWorkerOperationId(message.operation_id)) {
    publishFailure(
      operationId,
      new StreamingFailure("manifest_invalid", "worker operation identity is invalid"),
    );
    return;
  }
  if (message.type === "cancel") {
    if (active?.operationId === operationId) active.controller.abort();
    return;
  }
  if (message.type !== "start") {
    publishFailure(
      operationId,
      new StreamingFailure("manifest_invalid", "unsupported worker message type"),
    );
    return;
  }
  if (active) {
    publishFailure(
      operationId,
      new StreamingFailure("resource_limit", "one worker operation is already active"),
    );
    return;
  }
  void start(message, operationId);
}

async function start(message, operationId) {
  const controller = new AbortController();
  active = { operationId, controller };
  publish(operationId, "state", { phase: "starting" });
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
        onDeployment: (deployment) => publish(operationId, "state", {
          phase: "deployment",
          deployment: publicDeployment(deployment),
        }),
        onState: (phase, metrics) => publish(operationId, "state", { phase, metrics }),
        onBatch: (buffer, facts) => publishBatch(operationId, buffer, facts),
      },
    );
    publish(operationId, "complete", {
      deployment: publicDeployment(result.deployment),
      metrics: result.metrics,
      decode: result.decode,
    });
  } catch (error) {
    publishFailure(operationId, error);
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
  const failure = error instanceof StreamingFailure
    ? error.toJSON()
    : workerFailure(error?.message ?? String(error));
  self.postMessage(createWorkerMessage(operationId, "failure", {
    code: failure.code,
    message: boundedWorkerFailureMessage(failure.message),
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
    source_bounds: {
      min: deployment.index.root.bounds.minimum,
      max: deployment.index.root.bounds.maximum,
    },
  };
}

self.addEventListener("error", (event) => {
  if (active) {
    publishFailure(active.operationId, new Error(event.message));
    active.controller.abort();
  }
});
