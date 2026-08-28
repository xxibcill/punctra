export const QUALIFICATION_LANE = deepFreeze({
  id: "codex-iab-chromium-151-macos-26-apple-m5-pro",
  status: "repository_qualified_exact_lane",
  browser: {
    surface: "Codex in-app browser",
    engine: "Chromium",
    user_agent_version: "151.0.0.0",
    user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36",
    language: "en-US",
    logical_processors: 15,
  },
  operating_system: {
    name: "macOS",
    version: "26.5.2",
    build: "25F84",
    architecture: "arm64",
    user_agent_platform: "MacIntel",
    note: "The browser's reduced user-agent OS token is not the operating-system version authority.",
  },
  device: {
    class: "Apple silicon laptop",
    gpu: "Apple M5 Pro",
    gpu_cores: 16,
    gpu_class: "integrated",
    metal_support: "Metal 4",
    mapping_note: "The browser exposed only a generic WebGPU adapter name; the physical GPU mapping is a local-system inference from the sole installed GPU.",
  },
  webgpu: {
    adapter_name: "browser WebGPU adapter",
    backend: "BrowserWebGpu",
    device_type: "Other",
    surface_format: "Bgra8Unorm",
    composite_alpha_mode: "Opaque",
    present_mode: "fifo",
    render_attachment: true,
    blendable: true,
    required_feature_count: 0,
    max_buffer_size: 4_294_967_292,
    max_texture_dimension_2d: 16_384,
    max_bind_groups: 4,
    max_vertex_buffers: 8,
    max_color_attachments: 8,
  },
  display: {
    physical_viewport: [1_749, 1_093],
    css_viewport: [874.28125, 546.421875],
    device_pixel_ratio: 2,
    screen_css_pixels: [1_920, 1_080],
    color_depth: 24,
    pixel_depth: 24,
    canvas_bytes: 7_646_628,
    display_path: "built-in Retina display",
    screen_note: "The browser-reported CSS screen size and bit depth are exact session facts; the host independently reported the built-in Retina display path.",
  },
  workload: {
    deployment_id: "repository-las-v1",
    source_identity: "c459ff39717b7d6994aaebf344641f5a3add7faf65e249b85933ebd066d1c26e",
    source_points: 70_000,
    coverage: "sampled",
    displayed_points: 4_096,
    displayed_batches: 4,
  },
});

export const QUALIFICATION_RUNTIME_LANE = deepFreeze({
  id: QUALIFICATION_LANE.id,
  host: {
    schema: "punctra-qualification-host-v1",
    operatingSystem: {
      name: QUALIFICATION_LANE.operating_system.name,
      version: QUALIFICATION_LANE.operating_system.version,
      build: QUALIFICATION_LANE.operating_system.build,
      architecture: QUALIFICATION_LANE.operating_system.architecture,
    },
    device: {
      class: QUALIFICATION_LANE.device.class,
      gpu: QUALIFICATION_LANE.device.gpu,
      gpuCores: QUALIFICATION_LANE.device.gpu_cores,
      gpuClass: QUALIFICATION_LANE.device.gpu_class,
      metalSupport: QUALIFICATION_LANE.device.metal_support,
    },
    displayPath: QUALIFICATION_LANE.display.display_path,
    package: {
      name: "@punctra/viewer",
      version: "0.21.0-alpha.1",
    },
  },
  browser: {
    userAgent: QUALIFICATION_LANE.browser.user_agent,
    platform: QUALIFICATION_LANE.operating_system.user_agent_platform,
    language: QUALIFICATION_LANE.browser.language,
    logicalProcessors: QUALIFICATION_LANE.browser.logical_processors,
  },
  screen: {
    width: QUALIFICATION_LANE.display.screen_css_pixels[0],
    height: QUALIFICATION_LANE.display.screen_css_pixels[1],
    colorDepth: QUALIFICATION_LANE.display.color_depth,
    pixelDepth: QUALIFICATION_LANE.display.pixel_depth,
  },
  display: {
    physicalWidth: QUALIFICATION_LANE.display.physical_viewport[0],
    physicalHeight: QUALIFICATION_LANE.display.physical_viewport[1],
    cssWidth: QUALIFICATION_LANE.display.css_viewport[0],
    cssHeight: QUALIFICATION_LANE.display.css_viewport[1],
    devicePixelRatio: QUALIFICATION_LANE.display.device_pixel_ratio,
    surfaceBytes: QUALIFICATION_LANE.display.canvas_bytes,
  },
  capabilities: {
    secure_context: true,
    webgpu: true,
    browser_user_agent: QUALIFICATION_LANE.browser.user_agent,
    browser_platform: QUALIFICATION_LANE.operating_system.user_agent_platform,
    adapter_name: QUALIFICATION_LANE.webgpu.adapter_name,
    backend: QUALIFICATION_LANE.webgpu.backend,
    device_type: QUALIFICATION_LANE.webgpu.device_type,
    surface_format: QUALIFICATION_LANE.webgpu.surface_format,
    composite_alpha_mode: QUALIFICATION_LANE.webgpu.composite_alpha_mode,
    present_mode: QUALIFICATION_LANE.webgpu.present_mode,
    surface_format_support: {
      render_attachment: QUALIFICATION_LANE.webgpu.render_attachment,
      blendable: QUALIFICATION_LANE.webgpu.blendable,
    },
    required_feature_count: QUALIFICATION_LANE.webgpu.required_feature_count,
    adapter_max_buffer_size: QUALIFICATION_LANE.webgpu.max_buffer_size,
    adapter_max_texture_dimension_2d: QUALIFICATION_LANE.webgpu.max_texture_dimension_2d,
    adapter_max_bind_groups: QUALIFICATION_LANE.webgpu.max_bind_groups,
    adapter_max_vertex_buffers: QUALIFICATION_LANE.webgpu.max_vertex_buffers,
    adapter_max_color_attachments: QUALIFICATION_LANE.webgpu.max_color_attachments,
  },
});

function deepFreeze(value) {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    for (const nested of Object.values(value)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}
