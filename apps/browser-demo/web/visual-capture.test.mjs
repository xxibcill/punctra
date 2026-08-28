import assert from "node:assert/strict";
import test from "node:test";

import {
  captureCanonicalFrame,
  captureVisualEnvironment,
  derivePendingWorkEvidence,
  parseRawJson,
  renderQuietFrames,
  summarizeCaptureTimingSamples,
  visualEnvironmentFingerprint,
} from "./visual-capture.js";

test("capture wrapper polls without blocking and owns canonical bytes and explicit facts", async () => {
  const completedBytes = Uint8Array.of(1, 2, 3, 4, 5, 6, 7, 8);
  const raw = new FakeCaptureViewer(completedBytes, 2);
  let now = 0;
  let requestedFrames = 0;
  const result = await captureCanonicalFrame(raw, {
    width: 2,
    height: 1,
    pollFrameCeiling: 5,
    capturePolicy: {
      canonical_format: "rgba8",
      canonical_channel_order: "rgba",
      canonical_encoding: "linear",
      origin: "top_left",
      presentation_claim: "offscreen_not_presented",
    },
    requestFrame: async () => { requestedFrames += 1; },
    monotonicNow: () => { now += 0.25; return now; },
  });

  assert.equal(result.schema, "punctra-browser-canonical-capture-v1");
  assert.equal(result.timing.poll_count, 3);
  assert.ok(result.timing.begin_submission_milliseconds >= 0);
  assert.ok(result.timing.poll_wait_milliseconds >= 0);
  assert.ok(result.timing.poll_call_milliseconds >= 0);
  assert.ok(result.timing.canonical_copy_milliseconds >= 0);
  assert.equal(result.timing.submitted_work_done_callback_milliseconds, 1.25);
  assert.equal(result.timing.readback_mapping_callback_milliseconds, 1.5);
  assert.equal(result.timing.callback_ordering, "not_inferred");
  assert.equal(result.timing.physical_gpu_timing, "not_observed");
  assert.deepEqual(result.facts.completion_callbacks, {
    schema: "punctra-browser-frame-capture-completion-v1",
    origin: "begin_frame_capture_monotonic_clock",
    submitted_work_done_callback_milliseconds: 1.25,
    readback_mapping_callback_milliseconds: 1.5,
  });
  assert.equal(requestedFrames, 3);
  assert.equal(result.facts.normalization, "bgra_to_rgba");
  assert.equal(result.facts.presentation, "offscreen_not_presented");
  assert.equal(result.facts.physical_presentation_observed, false);
  assert.deepEqual(result.resource_facts, {
    capture_texture_bytes: 8,
    row_aligned_readback_bytes: 256,
    canonical_pixel_bytes: 8,
    peak_live_canonical_images_during_capture: 1,
  });
  completedBytes.fill(0);
  assert.deepEqual([...result.image.data], [1, 2, 3, 4, 5, 6, 7, 8]);
});

test("capture wrapper rejects malformed facts and bounded polling exhaustion", async () => {
  const malformed = new FakeCaptureViewer(Uint8Array.of(1, 2, 3, 4, 5, 6, 7, 8), 0);
  malformed.pending.canonical_channel_order = "bgra";
  await assert.rejects(
    captureCanonicalFrame(malformed, {
      width: 2,
      height: 1,
      requestFrame: async () => {},
      monotonicNow: () => 0,
    }),
    /canonical_channel_order differs/,
  );

  const batchTamper = new FakeCaptureViewer(Uint8Array.of(1, 2, 3, 4, 5, 6, 7, 8), 0);
  batchTamper.pending.batches[0].version = 0;
  await assert.rejects(
    captureCanonicalFrame(batchTamper, {
      width: 2,
      height: 1,
      requestFrame: async () => {},
      monotonicNow: () => 0,
    }),
    /batch version is invalid/,
  );

  const callbackTamper = new FakeCaptureViewer(Uint8Array.of(1, 2, 3, 4, 5, 6, 7, 8), 0);
  callbackTamper.completionOrigin = "unknown_clock";
  await assert.rejects(
    captureCanonicalFrame(callbackTamper, {
      width: 2,
      height: 1,
      requestFrame: async () => {},
      monotonicNow: () => 0,
    }),
    /completion origin differs/,
  );

  const pending = new FakeCaptureViewer(Uint8Array.of(1, 2, 3, 4, 5, 6, 7, 8), 10);
  await assert.rejects(
    captureCanonicalFrame(pending, {
      width: 2,
      height: 1,
      pollFrameCeiling: 2,
      requestFrame: async () => {},
      monotonicNow: () => 0,
    }),
    /did not complete within 2 foreground frames/,
  );
});

test("quiet window renders exactly 30 stable frames and publishes timing summaries", async () => {
  const raw = new FakeQuietViewer();
  let tick = 0;
  const observedFrames = [];
  const result = await renderQuietFrames(raw, {
    frameCount: 30,
    requestFrame: async () => {},
    monotonicNow: () => { tick += 1; return tick; },
    observeFrame: async ({ index, diagnostics }) => {
      observedFrames.push([index, diagnostics.rendered_frames]);
    },
    expected: {
      source_identity: "21".repeat(32),
      point_count: 10,
      published_batches: 2,
      view_id: 16,
      generation: 1,
      display_mode: "rgb",
      projection: "perspective",
      highlight_points: 0,
      physical_width: 2,
      physical_height: 1,
      drawn_points: 8,
      draw_calls: 1,
      resident_bytes: 192,
    },
  });

  assert.equal(result.complete, true);
  assert.equal(result.quiet_frames, 30);
  assert.equal(result.first_settled_frame, 1);
  assert.equal(result.quiet_window_complete_frame, 30);
  assert.deepEqual(result.animation_frame_scheduler, {
    authority: "runner_owned_request_animation_frame_tracker",
    scheduled: 30,
    resolved: 30,
    pending: 0,
  });
  const pending = derivePendingWorkEvidence(result, {
    requestPath: "private_direct_transfer_v2",
    observedBatches: [{ batch_index: 1, version: 1, presentation_weight_u8: 255 }],
    expected: {
      display_mode: "rgb",
      highlight_points: 0,
      capture_batches: [{ batch_index: 1, version: 1, presentation_weight_u8: 255 }],
    },
  });
  assert.deepEqual(pending.categories, {
    load: 0,
    request: 0,
    publication: 0,
    replacement: 0,
    retirement: 0,
    recolor: 0,
    highlight: 0,
    scheduled_render: 0,
  });
  assert.equal(pending.total, 0);
  assert.equal(result.generation, 1);
  assert.equal(result.coverage, "sampled");
  assert.equal(result.observed_frame_captures, 30);
  assert.equal(result.observed_frames, 30);
  assert.equal(result.first_rendered_frame, 1);
  assert.equal(result.last_rendered_frame, 30);
  assert.equal(result.frame_interval_milliseconds.count, 30);
  assert.equal(result.frame_submission_milliseconds.count, 30);
  assert.equal(result.frame_interval_samples_milliseconds.length, 30);
  assert.equal(result.frame_submission_samples_milliseconds.length, 30);
  assert.equal(raw.renderCalls, 30);
  assert.equal(observedFrames.length, 30);
  assert.deepEqual(observedFrames.at(-1), [29, 30]);
});

test("quiet window rejects resource or presentation drift", async () => {
  const raw = new FakeQuietViewer({ driftAt: 3 });
  await assert.rejects(
    renderQuietFrames(raw, {
      frameCount: 5,
      requestFrame: async () => {},
      monotonicNow: () => 0,
      expected: { point_count: 10 },
    }),
    /changed stable renderer facts/,
  );
});

test("environment capture separates requested and observed viewport, color, and unavailable facts", () => {
  const diagnostics = baseDiagnostics();
  const environment = captureVisualEnvironment({
    diagnostics,
    canvas: {
      width: 640,
      height: 480,
      getBoundingClientRect: () => ({ width: 320, height: 240 }),
    },
    navigatorObject: {
      userAgent: "Test Browser",
      platform: "Test OS",
      language: "en",
      hardwareConcurrency: 8,
    },
    windowObject: {
      isSecureContext: true,
      crossOriginIsolated: false,
      devicePixelRatio: 2,
      visualViewport: { scale: 1, width: 960, height: 540 },
      matchMedia: (query) => ({ matches: query.includes("srgb") }),
    },
    screenObject: { width: 1920, height: 1080, colorDepth: 24, pixelDepth: 24 },
    documentObject: { visibilityState: "visible" },
    host: { schema: "punctra-qualification-host-v1" },
  });

  assert.equal(environment.viewport.requested_device_pixel_ratio, 2);
  assert.equal(environment.viewport.observed_window_device_pixel_ratio, 2);
  assert.equal(environment.viewport.canvas_bitmap_width, 640);
  assert.equal(environment.color_capabilities.gamut_srgb, true);
  assert.deepEqual(environment.unavailable_measurements, {
    driver_gpu_memory_bytes: null,
    energy: null,
    gpu_completion_time: null,
    physical_cache_allocation_bytes: null,
    physical_display_panel_presentation: null,
    process_resident_memory_bytes: null,
    thermal_state: null,
  });
  assert.equal(environment.fallback.used, false);
  environment.attended_lane = { id: "attended" };
  environment.canonical_requirements = { webgpu: true };
  const fingerprint = visualEnvironmentFingerprint(environment);
  for (const mutate of [
    (value) => { value.screen.width_css_pixels += 1; },
    (value) => { value.color_capabilities.gamut_p3 = !value.color_capabilities.gamut_p3; },
    (value) => { value.unavailable_measurements.energy = 1; },
  ]) {
    const changed = structuredClone(environment);
    mutate(changed);
    assert.notEqual(visualEnvironmentFingerprint(changed), fingerprint);
  }
});

test("raw JSON parsing rejects non-string and malformed evidence", () => {
  assert.throws(() => parseRawJson({}), /must be a JSON string/);
  assert.throws(() => parseRawJson("[1]"), /must be an object/);
  assert.throws(() => parseRawJson("{"), /invalid JSON/);
});

test("capture timing aggregation retains every subsystem sample and rejects omission or authority tampering", () => {
  const sample = {
    begin_submission_milliseconds: 1,
    poll_wait_milliseconds: 2,
    poll_call_milliseconds: 3,
    canonical_copy_milliseconds: 4,
    submitted_work_done_callback_milliseconds: 5,
    readback_mapping_callback_milliseconds: 6,
    total_milliseconds: 10,
    poll_count: 2,
    animation_frames: 2,
    callback_elapsed_origin: "begin_frame_capture_monotonic_clock",
    callback_ordering: "not_inferred",
    physical_gpu_timing: "not_observed",
  };
  const summary = summarizeCaptureTimingSamples([sample, sample], { expectedCount: 2 });
  assert.equal(summary.sample_count, 2);
  assert.equal(summary.totals.total_milliseconds, 20);
  assert.equal(summary.totals.readback_mapping_callback_milliseconds, 12);
  assert.throws(
    () => summarizeCaptureTimingSamples([sample], { expectedCount: 2 }),
    /sample count differs/,
  );
  assert.throws(
    () => summarizeCaptureTimingSamples([{ ...sample, physical_gpu_timing: "estimated" }]),
    /physical-GPU claim differs/,
  );
});

class FakeCaptureViewer {
  constructor(bytes, pendingPolls) {
    this.bytes = bytes;
    this.pendingPolls = pendingPolls;
    this.polls = 0;
    this.pending = pendingCaptureFacts();
    this.completionOrigin = "begin_frame_capture_monotonic_clock";
  }

  beginFrameCapture() {
    return JSON.stringify(this.pending);
  }

  pollFrameCapture() {
    this.polls += 1;
    return this.polls <= this.pendingPolls ? undefined : this.bytes;
  }

  frameCaptureCompletionFacts() {
    return JSON.stringify({
      schema: "punctra-browser-frame-capture-completion-v1",
      origin: this.completionOrigin,
      submitted_work_done_callback_milliseconds: 1.25,
      readback_mapping_callback_milliseconds: 1.5,
    });
  }
}

class FakeQuietViewer {
  constructor(options = {}) {
    this.renderCalls = 0;
    this.driftAt = options.driftAt;
    this.current = baseDiagnostics();
  }

  diagnostics() {
    return JSON.stringify(this.current);
  }

  render() {
    this.renderCalls += 1;
    this.current = baseDiagnostics({
      renderedFrames: this.renderCalls,
      drawnPoints: this.renderCalls === this.driftAt ? 7 : 8,
    });
    return JSON.stringify(this.current);
  }
}

function pendingCaptureFacts() {
  return {
    schema: "punctra-browser-frame-capture-v1",
    status: "pending",
    completion: "map_callback_pending",
    presentation: "offscreen_not_presented",
    width: 2,
    height: 1,
    view_generation: 1,
    drawn_points: 8,
    draw_calls: 1,
    resident_bytes: 192,
    renderer_transient_texture_bytes: 8,
    batch_state_authority: "renderer_accepted_updates",
    batches: [{
      batch_index: 0,
      key: 1,
      version: 1,
      point_count: 8,
      state: "resident",
      presentation_weight_u8: 255,
    }],
    source_format: "bgra8_unorm",
    source_channel_order: "bgra",
    source_encoding: "linear",
    configured_surface_color_space: "srgb",
    canonical_format: "rgba8",
    canonical_channel_order: "rgba",
    canonical_encoding: "linear",
    origin: "top_left",
    bytes_per_pixel: 4,
    row_alignment_bytes: 256,
    tight_bytes_per_row: 8,
    padded_bytes_per_row: 256,
    output_bytes: 8,
    color_texture_bytes: 8,
    staging_buffer_bytes: 256,
  };
}

function baseDiagnostics(options = {}) {
  const renderedFrames = options.renderedFrames ?? 0;
  const drawnPoints = options.drawnPoints ?? 8;
  return {
    schema: "punctra-browser-viewer-v1",
    package_version: "0.21.0-alpha.1",
    phase: "ready",
    rendered_frames: renderedFrames,
    hidden_frame_skips: 0,
    capabilities: {
      secure_context: true,
      webgpu: true,
      surface_format: "Bgra8Unorm",
      composite_alpha_mode: "Opaque",
      present_mode: "fifo",
    },
    viewport: {
      css_width: 320,
      css_height: 240,
      device_pixel_ratio: 2,
      physical_width: 2,
      physical_height: 1,
      surface_bytes: 8,
    },
    streaming: {
      phase: "complete",
      source_identity: "21".repeat(32),
      view_id: 16,
      generation: 1,
      coverage: "sampled",
      expected_points: 10,
      published_points: 10,
      published_batches: 2,
      transferred_bytes: 320,
      retained_record_bytes: 320,
      presentation_version: 1,
    },
    capture_resources: {
      pending_tickets: 0,
      owned_textures: 0,
      owned_readback_buffers: 0,
    },
    camera: { projection: "perspective", eye: [0, -1, 1], target: [0, 0, 0], up: [0, 0, 1] },
    display_mode: "rgb",
    highlights: { point_count: 0, authority: "presentation_only" },
    frame: renderedFrames === 0 ? null : {
      view_generation: 1,
      drawn_points: drawnPoints,
      draw_calls: 1,
      resident_bytes: 192,
      transient_texture_bytes: 8,
      surface_suboptimal: false,
    },
    display_authority: "progressive_gpu_non_authoritative",
  };
}
