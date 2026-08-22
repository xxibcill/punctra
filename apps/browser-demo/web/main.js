const canvas = document.querySelector("#punctra-canvas");
const canvasShell = document.querySelector("#canvas-shell");
const statusBlock = document.querySelector("#status-block");
const statusMessage = document.querySelector("#status-message");
const diagnosticOutput = document.querySelector("#diagnostic-output");
const recoveryBlock = document.querySelector("#recovery-block");
const recoveryMessage = document.querySelector("#recovery-message");
const capabilityFacts = document.querySelector("#capability-facts");
const resourceFacts = document.querySelector("#resource-facts");
const pickFacts = document.querySelector("#pick-facts");
const restartButton = document.querySelector("#restart-button");
const visibilityButton = document.querySelector("#visibility-button");
const pickButton = document.querySelector("#pick-button");
const shutdownButton = document.querySelector("#shutdown-button");

let viewer = null;
let createViewer = null;
let wasmReady = false;
let suspended = false;
let smokeRunning = false;
let smokePassed = false;
let resizeFrame = null;
let smokeRecord = null;
let moduleLoadAttempt = 0;

function requestedViewport() {
  const bounds = canvasShell.getBoundingClientRect();
  return {
    cssWidth: Math.max(1, bounds.width),
    cssHeight: Math.max(1, bounds.height),
    dpr: Math.min(window.devicePixelRatio || 1, 4),
  };
}

function parseDiagnostics(json) {
  return JSON.parse(json);
}

function formatBytes(bytes) {
  if (!Number.isFinite(bytes)) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / 1024 ** 2).toFixed(2)} MiB`;
}

function replaceFacts(list, entries) {
  list.replaceChildren(
    ...entries.map(([label, value]) => {
      const row = document.createElement("div");
      const term = document.createElement("dt");
      const detail = document.createElement("dd");
      term.textContent = label;
      detail.textContent = String(value);
      row.append(term, detail);
      return row;
    }),
  );
}

function publishDiagnostics(diagnostics) {
  diagnosticOutput.textContent = JSON.stringify(diagnostics, null, 2);
  replaceFacts(capabilityFacts, [
    ["Secure context", diagnostics.capabilities.secure_context ? "available" : "unavailable"],
    ["WebGPU", diagnostics.capabilities.webgpu ? "available" : "unavailable"],
    ["Browser", diagnostics.capabilities.browser_user_agent],
    ["Adapter", diagnostics.capabilities.adapter_name],
    ["Backend", diagnostics.capabilities.backend],
    ["Surface", diagnostics.capabilities.surface_format],
    ["Composite alpha", diagnostics.capabilities.composite_alpha_mode],
    [
      "Render attachment",
      diagnostics.capabilities.surface_format_support.render_attachment ? "available" : "unavailable",
    ],
    [
      "Blendable surface",
      diagnostics.capabilities.surface_format_support.blendable ? "available" : "unavailable",
    ],
    ["Physical viewport", `${diagnostics.viewport.physical_width} × ${diagnostics.viewport.physical_height}`],
  ]);
  replaceFacts(resourceFacts, [
    ["Resident Points", `${diagnostics.scene.point_count} / ${diagnostics.limits.points}`],
    ["Logical vertex bytes", `${formatBytes(diagnostics.scene.estimated_gpu_bytes)} / ${formatBytes(diagnostics.limits.estimated_gpu_bytes)}`],
    ["Surface bytes / pixel", diagnostics.limits.surface_bytes_per_pixel],
    ["Canvas bytes", formatBytes(diagnostics.viewport.surface_bytes)],
    ["Transient texture bytes", formatBytes(diagnostics.frame?.transient_texture_bytes)],
    ["Presentation latency hint", `${diagnostics.limits.presentation_latency_frames} frames`],
    ["Rendered frames", diagnostics.rendered_frames],
    ["Hidden frame skips", diagnostics.hidden_frame_skips],
  ]);
  replaceFacts(pickFacts, [
    ["Pick state", diagnostics.pick.status.replaceAll("_", " ")],
    ["Point ordinal", diagnostics.pick.point_ordinal ?? "—"],
    ["Generation / batch / version", diagnostics.pick.generation === null
      ? "—"
      : `${diagnostics.pick.generation} / ${diagnostics.pick.batch_key} / ${diagnostics.pick.batch_version}`],
  ]);
}

function setHarnessState(state, message, safeAction = "") {
  document.body.dataset.browserSmoke = state;
  statusBlock.dataset.state = state;
  statusMessage.textContent = message;
  recoveryBlock.hidden = safeAction.length === 0;
  recoveryMessage.textContent = safeAction;
}

function setControls(enabled) {
  visibilityButton.disabled = !enabled;
  pickButton.disabled = !enabled;
  shutdownButton.disabled = !enabled;
}

function failureRecord(error) {
  if (error?.schema === "punctra-browser-failure-v1") return error;
  const message = typeof error === "string" ? error : String(error?.message ?? error);
  try {
    return JSON.parse(message);
  } catch {
    return {
      schema: "punctra-browser-failure-v1",
      code: "browser_module",
      message,
      safe_action: "Build the browser package again, serve it from localhost, and recreate the viewer.",
    };
  }
}

function publishFailure(error, { disableControls = false, state = "failed" } = {}) {
  const record = failureRecord(error);
  diagnosticOutput.textContent = JSON.stringify(record, null, 2);
  if (disableControls) setControls(false);
  const label = state === "unsupported" ? "UNSUPPORTED" : "FAILED";
  setHarnessState(state, `${label} — ${record.message}`, record.safe_action);
}

function assertFact(condition, message) {
  if (!condition) throw new Error(`Browser acceptance invariant failed: ${message}`);
}

function verifyCapabilityDiagnostics(diagnostics) {
  const capabilities = diagnostics.capabilities;
  assertFact(capabilities.secure_context === true, "secure-context capability");
  assertFact(capabilities.webgpu === true, "WebGPU capability");
  assertFact(capabilities.adapter_name.length > 0, "adapter name");
  assertFact(capabilities.backend === "BrowserWebGpu", "browser WebGPU backend");
  assertFact(capabilities.device_type.length > 0, "adapter device type");
  assertFact(capabilities.surface_format.length > 0, "surface format");
  assertFact(
    ["Opaque", "PreMultiplied"].includes(capabilities.composite_alpha_mode),
    "supported composite alpha mode",
  );
  assertFact(capabilities.present_mode === "fifo", "FIFO presentation");
  assertFact(
    capabilities.surface_format_support.render_attachment === true,
    "render-attachment surface",
  );
  assertFact(capabilities.surface_format_support.blendable === true, "blendable surface format");
  assertFact(capabilities.required_feature_count === 0, "WebGPU core features only");
  assertFact(
    capabilities.adapter_max_buffer_size >= 268_435_456,
    "default adapter buffer limit",
  );
  assertFact(
    capabilities.adapter_max_texture_dimension_2d >= 8_192,
    "default adapter texture-dimension limit",
  );
  assertFact(capabilities.adapter_max_bind_groups >= 4, "default adapter bind-group limit");
  assertFact(capabilities.adapter_max_vertex_buffers >= 8, "default adapter vertex-buffer limit");
  assertFact(
    capabilities.adapter_max_color_attachments >= 8,
    "default adapter color-attachment limit",
  );
}

async function initializeViewer() {
  const requested = requestedViewport();
  const next = await createViewer(
    canvas,
    requested.cssWidth,
    requested.cssHeight,
    requested.dpr,
  );
  viewer = next;
  suspended = false;
  visibilityButton.textContent = "Suspend rendering";
  setControls(true);
  return requested;
}

async function pollCentrePick() {
  let diagnostics = parseDiagnostics(viewer.diagnostics());
  const x = Math.floor(diagnostics.viewport.physical_width / 2);
  const y = Math.floor(diagnostics.viewport.physical_height / 2);
  publishDiagnostics(parseDiagnostics(viewer.beginPick(x, y)));

  for (let attempt = 0; attempt < 180; attempt += 1) {
    await new Promise((resolve) => requestAnimationFrame(resolve));
    diagnostics = parseDiagnostics(viewer.pollPick());
    publishDiagnostics(diagnostics);
    if (diagnostics.pick.status !== "pending") return diagnostics;
  }
  throw new Error("Browser acceptance invariant failed: provisional pick remained pending");
}

function verifyBoundedResize(initialViewport) {
  const requested = {
    cssWidth: Math.max(1, initialViewport.cssWidth * 0.75),
    cssHeight: Math.max(1, initialViewport.cssHeight * 0.75),
    dpr: initialViewport.dpr,
  };
  const diagnostics = parseDiagnostics(
    viewer.resize(requested.cssWidth, requested.cssHeight, requested.dpr),
  );
  const viewport = diagnostics.viewport;
  assertFact(viewport.css_width === requested.cssWidth, "resized CSS width");
  assertFact(viewport.css_height === requested.cssHeight, "resized CSS height");
  assertFact(viewport.device_pixel_ratio === requested.dpr, "resized device-pixel ratio");
  assertFact(
    viewport.physical_width === Math.round(requested.cssWidth * requested.dpr),
    "resized physical width",
  );
  assertFact(
    viewport.physical_height === Math.round(requested.cssHeight * requested.dpr),
    "resized physical height",
  );
  assertFact(
    viewport.surface_bytes
      === viewport.physical_width * viewport.physical_height * diagnostics.limits.surface_bytes_per_pixel,
    "resized surface accounting",
  );
  assertFact(diagnostics.frame === null, "resize discards the recorded frame");
  publishDiagnostics(parseDiagnostics(viewer.render()));
  return viewport;
}

async function runSmokePath() {
  smokeRunning = true;
  smokePassed = false;
  setHarnessState("checking", "Running bounded browser lifecycle checks…");
  const initialViewport = await initializeViewer();

  let diagnostics = parseDiagnostics(viewer.render());
  publishDiagnostics(diagnostics);
  assertFact(diagnostics.schema === "punctra-browser-foundation-v1", "diagnostic schema");
  verifyCapabilityDiagnostics(diagnostics);
  assertFact(diagnostics.scene.point_count === 1089, "fixed scene Point count");
  assertFact(diagnostics.scene.initial_requests === 1, "initial planner request");
  assertFact(diagnostics.scene.retained_nodes === 1, "settled planner retention");
  assertFact(diagnostics.scene.generation === 1, "View generation");
  assertFact(diagnostics.scene.batch_version === 1, "batch version");
  assertFact(diagnostics.scene.estimated_gpu_bytes === 26_136, "fixed scene logical bytes");
  assertFact(diagnostics.frame.drawn_points === 1089, "drawn Point count");
  assertFact(diagnostics.frame.draw_calls === 1, "single generated draw call");
  assertFact(diagnostics.frame.resident_bytes === 26_136, "fixed resident bytes");
  assertFact(diagnostics.limits.estimated_gpu_bytes === 49_152, "logical byte ceiling");
  assertFact(diagnostics.limits.points === 2_048, "Point ceiling");
  assertFact(diagnostics.limits.batches === 4, "batch ceiling");
  assertFact(diagnostics.limits.highlight_points === 32, "highlight ceiling");
  assertFact(diagnostics.limits.canvas_dimension === 4_096, "canvas dimension ceiling");
  assertFact(diagnostics.limits.canvas_pixels === 8_388_608, "canvas area ceiling");
  assertFact(diagnostics.limits.device_pixel_ratio === 4, "device-pixel-ratio ceiling");
  assertFact(
    diagnostics.limits.renderer_transient_bytes === 67_108_864,
    "transient texture ceiling",
  );
  assertFact(
    diagnostics.viewport.surface_bytes
      <= diagnostics.limits.canvas_pixels * diagnostics.limits.surface_bytes_per_pixel,
    "canvas byte ceiling",
  );
  assertFact(diagnostics.limits.surface_bytes_per_pixel === 4, "surface byte factor");
  assertFact(diagnostics.limits.presentation_latency_frames === 2, "presentation latency hint");
  assertFact(
    diagnostics.frame.transient_texture_bytes === diagnostics.viewport.surface_bytes,
    "exact pre-pick transient texture accounting",
  );

  const resizedViewport = verifyBoundedResize(initialViewport);

  viewer.setVisible(false);
  diagnostics = parseDiagnostics(viewer.render());
  assertFact(diagnostics.phase === "hidden", "hidden phase");
  assertFact(diagnostics.hidden_frame_skips === 1, "hidden frame suppression");
  viewer.setVisible(true);
  publishDiagnostics(parseDiagnostics(viewer.render()));

  diagnostics = await pollCentrePick();
  assertFact(diagnostics.pick.status === "hit", "centre provisional pick");
  assertFact(diagnostics.pick.point_ordinal === 544, "centre Point identity");
  assertFact(diagnostics.pick.generation === 1, "pick generation");
  assertFact(diagnostics.pick.batch_key === 1, "pick batch key");
  assertFact(diagnostics.pick.batch_version === 1, "pick batch version");
  assertFact(
    diagnostics.frame.transient_texture_bytes
      === diagnostics.viewport.physical_width * diagnostics.viewport.physical_height * 8,
    "exact post-pick transient texture accounting",
  );

  smokeRecord = {
    state: "pick_verified",
    browser: diagnostics.capabilities.browser_user_agent,
    platform: diagnostics.capabilities.browser_platform,
    scene: diagnostics.scene,
    frame: diagnostics.frame,
    pick: diagnostics.pick,
    viewport: diagnostics.viewport,
    resized_viewport: resizedViewport,
  };

  parseDiagnostics(viewer.shutdown());
  let shutdownRejected = false;
  try {
    viewer.render();
  } catch (error) {
    shutdownRejected = failureRecord(error).code === "host_model";
  }
  assertFact(shutdownRejected, "fused shutdown");
  smokeRecord.shutdown_rejected = shutdownRejected;

  await initializeViewer();
  diagnostics = parseDiagnostics(viewer.render());
  publishDiagnostics(diagnostics);
  assertFact(diagnostics.phase === "ready", "explicit recreation");
  setHarnessState("passed", "PASS — browser WebGPU lifecycle and invariants verified locally.");
  smokePassed = true;
  smokeRunning = false;
}

async function start() {
  if (!window.isSecureContext || !navigator.gpu) {
    const missing = !window.isSecureContext ? "secure context" : "WebGPU";
    publishFailure(
      {
        schema: "punctra-browser-failure-v1",
        code: "browser_capability",
        message: `${missing} is unavailable.`,
        safe_action: "Serve this page from localhost or HTTPS in a WebGPU-capable browser, then recreate the viewer.",
      },
      { disableControls: true, state: "unsupported" },
    );
    return;
  }

  try {
    if (!wasmReady) {
      const attempt = moduleLoadAttempt;
      moduleLoadAttempt += 1;
      const browserBindings = await import(`./pkg/browser_demo.js?attempt=${attempt}`);
      await browserBindings.default();
      createViewer = browserBindings.createViewer;
      wasmReady = true;
    }
    await runSmokePath();
  } catch (error) {
    smokeRunning = false;
    smokePassed = false;
    try {
      viewer?.shutdown();
    } catch {
      // Preserve the original acceptance failure as the actionable diagnostic.
    }
    viewer = null;
    publishFailure(error, { disableControls: true });
  }
}

async function restart() {
  if (smokeRunning) return;
  if (!smokePassed) {
    await start();
    return;
  }
  try {
    viewer?.shutdown();
    await initializeViewer();
    const diagnostics = parseDiagnostics(viewer.render());
    publishDiagnostics(diagnostics);
    setHarnessState("passed", "READY — viewer explicitly recreated.");
  } catch (error) {
    publishFailure(error, { disableControls: true });
  }
}

async function toggleVisibility() {
  if (!viewer || smokeRunning) return;
  suspended = !suspended;
  try {
    publishDiagnostics(parseDiagnostics(viewer.setVisible(!suspended)));
    visibilityButton.textContent = suspended ? "Resume rendering" : "Suspend rendering";
    if (!suspended) publishDiagnostics(parseDiagnostics(viewer.render()));
  } catch (error) {
    publishFailure(error);
  }
}

async function checkPick() {
  if (!viewer || smokeRunning || suspended) return;
  try {
    const diagnostics = await pollCentrePick();
    assertFact(diagnostics.pick.status === "hit", "manual centre provisional pick");
    setHarnessState("passed", "READY — centre provisional pick retained the recorded identity.");
  } catch (error) {
    publishFailure(error);
  }
}

function shutdown() {
  if (!viewer || smokeRunning) return;
  publishDiagnostics(parseDiagnostics(viewer.shutdown()));
  viewer = null;
  suspended = false;
  setControls(false);
  setHarnessState("passed", "SHUT DOWN — recreate the viewer before more work.");
}

function scheduleResize() {
  if (!viewer || smokeRunning || suspended || resizeFrame !== null) return;
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = null;
    try {
      const requested = requestedViewport();
      viewer.resize(requested.cssWidth, requested.cssHeight, requested.dpr);
      publishDiagnostics(parseDiagnostics(viewer.render()));
    } catch (error) {
      publishFailure(error);
    }
  });
}

function synchronizeDocumentVisibility() {
  if (!viewer || smokeRunning) return;
  try {
    const visible = document.visibilityState === "visible" && !suspended;
    viewer.setVisible(visible);
    if (visible) publishDiagnostics(parseDiagnostics(viewer.render()));
  } catch (error) {
    publishFailure(error, { disableControls: true });
  }
}

restartButton.addEventListener("click", restart);
visibilityButton.addEventListener("click", toggleVisibility);
pickButton.addEventListener("click", checkPick);
shutdownButton.addEventListener("click", shutdown);
document.addEventListener("visibilitychange", synchronizeDocumentVisibility);
new ResizeObserver(scheduleResize).observe(canvasShell);

window.__PUNCTRA_BROWSER_FOUNDATION__ = {
  diagnostics: () => viewer ? parseDiagnostics(viewer.diagnostics()) : null,
  smoke: () => smokeRecord,
  state: () => document.body.dataset.browserSmoke,
};

start();
