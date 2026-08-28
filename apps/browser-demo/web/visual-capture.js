export const QUIET_WINDOW_SCHEMA = "punctra-browser-quiet-window-v1";
export const CAPTURE_RESULT_SCHEMA = "punctra-browser-canonical-capture-v1";
export const ENVIRONMENT_SCHEMA = "punctra-browser-visual-environment-v1";

/**
 * Waits for a bounded GPU readback without conflating map completion with
 * compositor or panel presentation. Returned bytes are copied out of Wasm.
 */
export async function captureCanonicalFrame(rawViewer, options = {}) {
  requireRawMethod(rawViewer, "beginFrameCapture");
  requireRawMethod(rawViewer, "pollFrameCapture");
  requireRawMethod(rawViewer, "frameCaptureCompletionFacts");
  const requestFrame = options.requestFrame ?? browserAnimationFrame;
  const monotonicNow = options.monotonicNow ?? defaultNow;
  const pollFrameCeiling = boundedInteger(options.pollFrameCeiling ?? 240, "capture poll frame ceiling", 1, 600);
  const expectedWidth = boundedInteger(options.width, "capture width", 1, 4_096);
  const expectedHeight = boundedInteger(options.height, "capture height", 1, 4_096);

  const started = monotonicNow();
  const pendingFacts = parseRawJson(rawViewer.beginFrameCapture(), "frame-capture pending facts");
  validatePendingCaptureFacts(pendingFacts, {
    width: expectedWidth,
    height: expectedHeight,
    policy: options.capturePolicy,
  });
  const submitted = monotonicNow();
  let pollMilliseconds = 0;
  for (let pollCount = 1; pollCount <= pollFrameCeiling; pollCount += 1) {
    await requestFrame();
    const pollStarted = monotonicNow();
    const result = rawViewer.pollFrameCapture();
    const pollCompleted = monotonicNow();
    pollMilliseconds += pollCompleted - pollStarted;
    if (result === undefined) continue;
    requireCondition(result instanceof Uint8Array, "completed frame capture must be Uint8Array");
    const expectedBytes = expectedWidth * expectedHeight * 4;
    requireCondition(result.byteLength === expectedBytes, `completed frame capture contains ${result.byteLength} bytes instead of ${expectedBytes}`);
    const completionFacts = parseRawJson(
      rawViewer.frameCaptureCompletionFacts(),
      "frame-capture completion facts",
    );
    validateCaptureCompletionFacts(completionFacts);
    const copyStarted = monotonicNow();
    const canonical = result.slice();
    const completed = monotonicNow();
    return {
      schema: CAPTURE_RESULT_SCHEMA,
      image: { width: expectedWidth, height: expectedHeight, data: canonical },
      facts: {
        ...pendingFacts,
        status: "ready",
        completion: "map_callback_completed_and_copied",
        normalization: pendingFacts.source_channel_order === "bgra" ? "bgra_to_rgba" : "rgba_identity",
        canonical_pixel_bytes: canonical.byteLength,
        physical_presentation_observed: false,
        completion_callbacks: completionFacts,
      },
      timing: {
        begin_submission_milliseconds: submitted - started,
        poll_wait_milliseconds: Math.max(
          0,
          pollCompleted - submitted - pollMilliseconds,
        ),
        poll_call_milliseconds: pollMilliseconds,
        canonical_copy_milliseconds: completed - copyStarted,
        submitted_work_done_callback_milliseconds: completionFacts.submitted_work_done_callback_milliseconds,
        readback_mapping_callback_milliseconds: completionFacts.readback_mapping_callback_milliseconds,
        callback_elapsed_origin: completionFacts.origin,
        callback_ordering: "not_inferred",
        physical_gpu_timing: "not_observed",
        total_milliseconds: completed - started,
        poll_count: pollCount,
        animation_frames: pollCount,
      },
      resource_facts: {
        capture_texture_bytes: pendingFacts.color_texture_bytes,
        row_aligned_readback_bytes: pendingFacts.staging_buffer_bytes,
        canonical_pixel_bytes: canonical.byteLength,
        peak_live_canonical_images_during_capture: 1,
      },
    };
  }
  throw new Error(`Frame capture did not complete within ${pollFrameCeiling} foreground frames`);
}

function validateCaptureCompletionFacts(facts) {
  requireRecord(facts, "frame-capture completion facts");
  requireCondition(facts.schema === "punctra-browser-frame-capture-completion-v1", "frame-capture completion schema differs");
  requireCondition(facts.origin === "begin_frame_capture_monotonic_clock", "frame-capture completion origin differs");
  for (const field of ["submitted_work_done_callback_milliseconds", "readback_mapping_callback_milliseconds"]) {
    requireCondition(Number.isFinite(facts[field]) && facts[field] >= 0, `frame-capture completion ${field} is invalid`);
  }
}

/** Renders exactly the declared foreground quiet window and rejects drift. */
export async function renderQuietFrames(rawViewer, options = {}) {
  requireRawMethod(rawViewer, "render");
  requireRawMethod(rawViewer, "diagnostics");
  const frameCount = boundedInteger(options.frameCount ?? 30, "quiet frame count", 1, 300);
  const requestFrame = options.requestFrame ?? browserAnimationFrame;
  const monotonicNow = options.monotonicNow ?? defaultNow;
  const expected = options.expected ?? {};
  const observeFrame = options.observeFrame;
  requireCondition(observeFrame === undefined || typeof observeFrame === "function", "quiet frame observer must be a function");
  const before = parseRawJson(rawViewer.diagnostics(), "pre-quiet diagnostics");
  validateSettledDiagnostics(before, expected, false);
  const frameIntervals = [];
  const submissionMilliseconds = [];
  let previousTimestamp = monotonicNow();
  let stableFacts;
  let firstRenderedFrame;
  let lastRenderedFrame;
  let scheduledAnimationFrames = 0;
  let resolvedAnimationFrames = 0;
  for (let frameIndex = 0; frameIndex < frameCount; frameIndex += 1) {
    scheduledAnimationFrames += 1;
    await requestFrame();
    resolvedAnimationFrames += 1;
    const frameTimestamp = monotonicNow();
    frameIntervals.push(frameTimestamp - previousTimestamp);
    previousTimestamp = frameTimestamp;
    const submissionStarted = monotonicNow();
    const diagnostics = parseRawJson(rawViewer.render(), `quiet frame ${frameIndex + 1} diagnostics`);
    submissionMilliseconds.push(monotonicNow() - submissionStarted);
    validateSettledDiagnostics(diagnostics, expected, true);
    const currentStableFacts = quietStableFacts(diagnostics);
    if (stableFacts === undefined) {
      stableFacts = currentStableFacts;
      firstRenderedFrame = diagnostics.rendered_frames;
    } else {
      requireCondition(deepEqual(stableFacts, currentStableFacts), `quiet frame ${frameIndex + 1} changed stable renderer facts`);
    }
    lastRenderedFrame = diagnostics.rendered_frames;
    if (observeFrame !== undefined) {
      await observeFrame({ index: frameIndex, diagnostics });
    }
  }
  const after = parseRawJson(rawViewer.diagnostics(), "post-quiet diagnostics");
  validateSettledDiagnostics(after, expected, true);
  requireCondition(deepEqual(stableFacts, quietStableFacts(after)), "renderer facts changed after the quiet window");
  return {
    schema: QUIET_WINDOW_SCHEMA,
    complete: true,
    quiet_frames: frameCount,
    first_settled_frame: firstRenderedFrame,
    quiet_window_complete_frame: lastRenderedFrame,
    animation_frame_scheduler: {
      authority: "runner_owned_request_animation_frame_tracker",
      scheduled: scheduledAnimationFrames,
      resolved: resolvedAnimationFrames,
      pending: scheduledAnimationFrames - resolvedAnimationFrames,
    },
    generation: stableFacts.streaming.generation,
    coverage: stableFacts.streaming.coverage,
    required_frames: frameCount,
    observed_frames: frameCount,
    first_rendered_frame: firstRenderedFrame,
    last_rendered_frame: lastRenderedFrame,
    stable_facts: stableFacts,
    observed_frame_captures: observeFrame === undefined ? 0 : frameCount,
    frame_interval_milliseconds: summarizeSamples(frameIntervals),
    frame_submission_milliseconds: summarizeSamples(submissionMilliseconds),
    frame_interval_samples_milliseconds: frameIntervals,
    frame_submission_samples_milliseconds: submissionMilliseconds,
  };
}

/** Derives every settled pending-work category from retained exact source facts. */
export function derivePendingWorkEvidence(quietWindow, options) {
  requireRecord(quietWindow, "quiet window");
  const stable = quietWindow.stable_facts;
  requireRecord(stable, "quiet-window stable facts");
  const expected = options?.expected;
  requireRecord(expected, "pending-work expectations");
  requireCondition(Array.isArray(options.observedBatches), "pending-work observed batches are absent");
  requireCondition(Array.isArray(expected.capture_batches), "pending-work expected batches are absent");
  const observedBatches = options.observedBatches;
  const expectedBatches = expected.capture_batches;
  const expectedIndices = new Set(expectedBatches.map(({ batch_index: batchIndex }) => batchIndex));
  const observedIndices = new Set(observedBatches.map(({ batch_index: batchIndex }) => batchIndex));
  const categories = {
    load: stable.phase === "ready" ? 0 : 1,
    request: options.requestPath === "private_direct_transfer_v2" ? 0 : 1,
    publication: stable.streaming.phase === "complete"
      && stable.streaming.expected_points === stable.streaming.published_points ? 0 : 1,
    replacement: [...expectedIndices].filter((batchIndex) => !observedIndices.has(batchIndex)).length,
    retirement: [...observedIndices].filter((batchIndex) => !expectedIndices.has(batchIndex)).length,
    recolor: stable.display_mode === expected.display_mode
      && observedBatches.every((batch) => expectedBatches.some((candidate) => (
        candidate.batch_index === batch.batch_index
          && candidate.version === batch.version
          && candidate.presentation_weight_u8 === batch.presentation_weight_u8
      ))) ? 0 : 1,
    highlight: stable.highlights.point_count === expected.highlight_points ? 0 : 1,
    scheduled_render: quietWindow.animation_frame_scheduler.pending,
  };
  return {
    schema: "punctra-browser-visual-pending-work-v1",
    categories,
    total: Object.values(categories).reduce((total, value) => total + value, 0),
    sources: {
      load: { viewer_phase: stable.phase },
      request: { transfer_path: options.requestPath },
      publication: {
        stream_phase: stable.streaming.phase,
        expected_points: stable.streaming.expected_points,
        published_points: stable.streaming.published_points,
      },
      replacement_and_retirement: {
        authority: "renderer_accepted_capture_batch_snapshot",
        expected_batches: cloneJson(expectedBatches),
        observed_batches: cloneJson(observedBatches),
      },
      recolor: {
        expected_display_mode: expected.display_mode,
        observed_display_mode: stable.display_mode,
        expected_batches: cloneJson(expectedBatches),
        observed_batches: cloneJson(observedBatches),
      },
      highlight: {
        expected_points: expected.highlight_points,
        observed_points: stable.highlights.point_count,
      },
      scheduled_render: cloneJson(quietWindow.animation_frame_scheduler),
    },
  };
}

/** Captures explicit browser, canvas, adapter, color, and unavailable facts. */
export function captureVisualEnvironment(options) {
  const diagnostics = typeof options?.diagnostics === "string"
    ? parseRawJson(options.diagnostics, "environment diagnostics")
    : options?.diagnostics;
  requireRecord(diagnostics, "environment diagnostics");
  const canvas = options.canvas;
  requireCondition(canvas !== null && typeof canvas === "object", "environment canvas is required");
  const navigatorObject = options.navigatorObject ?? globalThis.navigator ?? {};
  const windowObject = options.windowObject ?? globalThis.window ?? {};
  const screenObject = options.screenObject ?? globalThis.screen ?? {};
  const documentObject = options.documentObject ?? globalThis.document ?? {};
  const bounds = typeof canvas.getBoundingClientRect === "function"
    ? canvas.getBoundingClientRect()
    : { width: null, height: null };
  const matchMedia = typeof windowObject.matchMedia === "function"
    ? (query) => Boolean(windowObject.matchMedia(query).matches)
    : () => null;
  return {
    schema: ENVIRONMENT_SCHEMA,
    browser: {
      user_agent: navigatorObject.userAgent ?? null,
      platform: navigatorObject.platform ?? null,
      language: navigatorObject.language ?? null,
      logical_processors: navigatorObject.hardwareConcurrency ?? null,
    },
    document: {
      secure_context: Boolean(windowObject.isSecureContext ?? globalThis.isSecureContext),
      visibility_state: documentObject.visibilityState ?? null,
      cross_origin_isolated: Boolean(windowObject.crossOriginIsolated ?? globalThis.crossOriginIsolated),
    },
    screen: {
      width_css_pixels: finiteOrNull(screenObject.width),
      height_css_pixels: finiteOrNull(screenObject.height),
      color_depth_bits: finiteOrNull(screenObject.colorDepth),
      pixel_depth_bits: finiteOrNull(screenObject.pixelDepth),
    },
    viewport: {
      requested_css_width: diagnostics.viewport?.css_width ?? null,
      requested_css_height: diagnostics.viewport?.css_height ?? null,
      requested_device_pixel_ratio: diagnostics.viewport?.device_pixel_ratio ?? null,
      observed_window_device_pixel_ratio: finiteOrNull(windowObject.devicePixelRatio),
      observed_css_width: finiteOrNull(bounds.width),
      observed_css_height: finiteOrNull(bounds.height),
      canvas_bitmap_width: finiteOrNull(canvas.width),
      canvas_bitmap_height: finiteOrNull(canvas.height),
      visual_viewport_scale: finiteOrNull(windowObject.visualViewport?.scale),
      visual_viewport_width: finiteOrNull(windowObject.visualViewport?.width),
      visual_viewport_height: finiteOrNull(windowObject.visualViewport?.height),
    },
    color_capabilities: {
      gamut_srgb: matchMedia("(color-gamut: srgb)"),
      gamut_p3: matchMedia("(color-gamut: p3)"),
      gamut_rec2020: matchMedia("(color-gamut: rec2020)"),
      dynamic_range_high: matchMedia("(dynamic-range: high)"),
      video_dynamic_range_high: matchMedia("(video-dynamic-range: high)"),
      configured_surface_color_space: "srgb",
      display_icc_profile: null,
      physical_panel_hdr_state: null,
    },
    webgpu: diagnostics.capabilities,
    fallback: {
      allowed: false,
      requested: false,
      used: false,
    },
    host: options.host ?? null,
    unavailable_measurements: {
      driver_gpu_memory_bytes: null,
      energy: null,
      gpu_completion_time: null,
      physical_cache_allocation_bytes: null,
      physical_display_panel_presentation: null,
      process_resident_memory_bytes: null,
      thermal_state: null,
    },
  };
}

/** Freezes every environment field later claimed as recreation-stable. */
export function visualEnvironmentFingerprint(environment) {
  requireRecord(environment, "visual environment");
  return JSON.stringify({
    browser: environment.browser,
    document: environment.document,
    screen: environment.screen,
    viewport: environment.viewport,
    color_capabilities: environment.color_capabilities,
    webgpu: environment.webgpu,
    fallback: environment.fallback,
    host: environment.host,
    unavailable_measurements: environment.unavailable_measurements,
    attended_lane: environment.attended_lane,
    canonical_requirements: environment.canonical_requirements,
  });
}

export function parseRawJson(value, label = "raw diagnostics") {
  requireCondition(typeof value === "string", `${label} must be a JSON string`);
  try {
    const parsed = JSON.parse(value);
    requireRecord(parsed, label);
    return parsed;
  } catch (error) {
    throw new Error(`${label} is invalid JSON: ${error?.message ?? error}`);
  }
}

export function summarizeSamples(samples) {
  requireCondition(Array.isArray(samples) && samples.length > 0, "timing samples must be a nonempty array");
  requireCondition(samples.every((sample) => Number.isFinite(sample) && sample >= 0), "timing samples must be finite and nonnegative");
  const ordered = [...samples].sort((left, right) => left - right);
  return {
    count: ordered.length,
    p50: percentile(ordered, 50),
    p95: percentile(ordered, 95),
    maximum: ordered.at(-1),
  };
}

/** Aggregates capture intervals without allowing a frame to disappear. */
export function summarizeCaptureTimingSamples(samples, options = {}) {
  requireCondition(Array.isArray(samples), "capture timing samples must be an array");
  const expectedCount = boundedInteger(
    options.expectedCount ?? samples.length,
    "expected capture timing sample count",
    0,
    600,
  );
  requireCondition(samples.length === expectedCount, "capture timing sample count differs");
  const fields = [
    "begin_submission_milliseconds",
    "poll_wait_milliseconds",
    "poll_call_milliseconds",
    "canonical_copy_milliseconds",
    "submitted_work_done_callback_milliseconds",
    "readback_mapping_callback_milliseconds",
    "total_milliseconds",
    "poll_count",
    "animation_frames",
  ];
  for (const sample of samples) {
    requireRecord(sample, "capture timing sample");
    for (const field of fields) {
      requireCondition(Number.isFinite(sample[field]) && sample[field] >= 0, `capture timing ${field} is invalid`);
    }
    requireCondition(sample.callback_elapsed_origin === "begin_frame_capture_monotonic_clock", "capture timing callback origin differs");
    requireCondition(sample.callback_ordering === "not_inferred", "capture timing callback ordering differs");
    requireCondition(sample.physical_gpu_timing === "not_observed", "capture timing physical-GPU claim differs");
  }
  return {
    sample_count: samples.length,
    samples: cloneJson(samples),
    totals: Object.fromEntries(fields.map((field) => [
      field,
      samples.reduce((total, sample) => total + sample[field], 0),
    ])),
  };
}

function validatePendingCaptureFacts(facts, expected) {
  requireRecord(facts, "frame-capture pending facts");
  const exact = {
    schema: "punctra-browser-frame-capture-v1",
    status: "pending",
    completion: "map_callback_pending",
    presentation: "offscreen_not_presented",
    width: expected.width,
    height: expected.height,
    configured_surface_color_space: "srgb",
    canonical_format: "rgba8",
    canonical_channel_order: "rgba",
    origin: "top_left",
    bytes_per_pixel: 4,
    tight_bytes_per_row: expected.width * 4,
    output_bytes: expected.width * expected.height * 4,
    color_texture_bytes: expected.width * expected.height * 4,
  };
  for (const [field, value] of Object.entries(exact)) {
    requireCondition(facts[field] === value, `frame-capture ${field} differs`);
  }
  requireCondition(facts.source_channel_order === "rgba" || facts.source_channel_order === "bgra", "frame-capture source channel order differs");
  requireCondition(facts.source_encoding === "linear" || facts.source_encoding === "srgb", "frame-capture source encoding differs");
  requireCondition(facts.canonical_encoding === facts.source_encoding, "frame-capture canonical encoding differs from its source");
  requireCondition(facts.batch_state_authority === "renderer_accepted_updates", "frame-capture batch-state authority differs");
  requireCondition(Array.isArray(facts.batches), "frame-capture batches are absent");
  const batchIndices = new Set();
  for (const batch of facts.batches) {
    requireRecord(batch, "frame-capture batch");
    requireCondition(Number.isSafeInteger(batch.batch_index) && batch.batch_index >= 0, "frame-capture batch batch_index is invalid");
    for (const field of ["key", "version", "point_count"]) {
      requireCondition(Number.isSafeInteger(batch[field]) && batch[field] >= 1, `frame-capture batch ${field} is invalid`);
    }
    requireCondition(Number.isSafeInteger(batch.presentation_weight_u8) && batch.presentation_weight_u8 >= 0, "frame-capture batch presentation_weight_u8 is invalid");
    requireCondition(batch.presentation_weight_u8 <= 255, "frame-capture batch presentation weight is invalid");
    requireCondition(batch.state === "resident", "frame-capture batch state differs");
    requireCondition(!batchIndices.has(batch.batch_index), "frame-capture batch index is duplicated");
    batchIndices.add(batch.batch_index);
  }
  requireCondition(Number.isInteger(facts.row_alignment_bytes) && facts.row_alignment_bytes === 256, "frame-capture row alignment differs");
  requireCondition(Number.isInteger(facts.padded_bytes_per_row) && facts.padded_bytes_per_row >= facts.tight_bytes_per_row && facts.padded_bytes_per_row % facts.row_alignment_bytes === 0, "frame-capture padded row layout differs");
  requireCondition(facts.staging_buffer_bytes === facts.padded_bytes_per_row * expected.height, "frame-capture staging length differs");
  for (const field of ["view_generation", "drawn_points", "draw_calls", "resident_bytes", "renderer_transient_texture_bytes"]) {
    requireCondition(Number.isSafeInteger(facts[field]) && facts[field] >= 0, `frame-capture ${field} is invalid`);
  }
  if (expected.policy !== undefined) {
    const policy = expected.policy;
    const policyFacts = {
      canonical_format: policy.canonical_format,
      canonical_channel_order: policy.canonical_channel_order,
      canonical_encoding: policy.canonical_encoding,
      origin: policy.origin,
      presentation: policy.presentation_claim,
    };
    for (const [field, value] of Object.entries(policyFacts)) {
      requireCondition(facts[field] === value, `frame-capture policy ${field} differs`);
    }
  }
}

function validateSettledDiagnostics(diagnostics, expected, requireFrame) {
  requireRecord(diagnostics, "settled diagnostics");
  requireCondition(diagnostics.phase === "ready", "viewer is not ready during settlement");
  requireRecord(diagnostics.streaming, "settled stream facts");
  requireCondition(diagnostics.streaming.phase === "complete", "stream is not complete during settlement");
  requireCondition(diagnostics.streaming.expected_points === diagnostics.streaming.published_points, "stream expected and published Points differ");
  requireCondition(diagnostics.streaming.coverage === "sampled", "stream Coverage differs");
  requireRecord(diagnostics.capture_resources, "settled capture-resource facts");
  for (const field of ["pending_tickets", "owned_textures", "owned_readback_buffers"]) {
    requireCondition(diagnostics.capture_resources[field] === 0, `settled capture resource ${field} differs`);
  }
  if (requireFrame) requireRecord(diagnostics.frame, "settled frame facts");
  const checks = [
    ["source_identity", diagnostics.streaming.source_identity],
    ["point_count", diagnostics.streaming.published_points],
    ["published_batches", diagnostics.streaming.published_batches],
    ["view_id", diagnostics.streaming.view_id],
    ["generation", diagnostics.streaming.generation],
    ["display_mode", diagnostics.display_mode],
    ["projection", diagnostics.camera?.projection],
    ["highlight_points", diagnostics.highlights?.point_count],
    ["physical_width", diagnostics.viewport?.physical_width],
    ["physical_height", diagnostics.viewport?.physical_height],
  ];
  if (requireFrame) {
    checks.push(
      ["drawn_points", diagnostics.frame.drawn_points],
      ["draw_calls", diagnostics.frame.draw_calls],
      ["resident_bytes", diagnostics.frame.resident_bytes],
    );
  }
  for (const [field, actual] of checks) {
    if (expected[field] !== undefined) requireCondition(actual === expected[field], `settled ${field} differs`);
  }
}

function quietStableFacts(diagnostics) {
  return {
    phase: diagnostics.phase,
    capabilities: diagnostics.capabilities,
    viewport: diagnostics.viewport,
    streaming: diagnostics.streaming,
    capture_resources: diagnostics.capture_resources,
    camera: diagnostics.camera,
    display_mode: diagnostics.display_mode,
    highlights: diagnostics.highlights,
    frame: diagnostics.frame,
    display_authority: diagnostics.display_authority,
  };
}

function percentile(ordered, percentileValue) {
  const index = Math.max(0, Math.ceil(ordered.length * percentileValue / 100) - 1);
  return ordered[index];
}

function finiteOrNull(value) {
  return Number.isFinite(value) ? value : null;
}

function browserAnimationFrame() {
  return new Promise((resolve) => globalThis.requestAnimationFrame(resolve));
}

function defaultNow() {
  return globalThis.performance.now();
}

function boundedInteger(value, label, minimum, maximum) {
  requireCondition(Number.isInteger(value) && value >= minimum && value <= maximum, `${label} must be an integer from ${minimum} through ${maximum}`);
  return value;
}

function requireRawMethod(rawViewer, method) {
  requireCondition(typeof rawViewer?.[method] === "function", `raw viewer ${method} is unavailable`);
}

function deepEqual(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function cloneJson(value) {
  return JSON.parse(JSON.stringify(value));
}

function requireRecord(value, label) {
  requireCondition(value !== null && typeof value === "object" && !Array.isArray(value), `${label} must be an object`);
}

function requireCondition(condition, message) {
  if (!condition) throw new Error(`Visual capture invalid: ${message}`);
}
