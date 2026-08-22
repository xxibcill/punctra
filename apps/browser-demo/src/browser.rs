use js_sys::Reflect;
use render_protocol::{RenderLimits, Viewport};
use render_wgpu::{
    Frame, FrameReport, PickHit, PickPoll, PickRequest, PickTicket, RecordedFrame, RendererConfig,
    WgpuRenderer,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::host::{
    HostModelError, Lifecycle, MAX_CANVAS_DIMENSION, MAX_CANVAS_PIXELS, MAX_DEVICE_PIXEL_RATIO,
    MAX_RENDER_TRANSIENT_BYTES, PhysicalViewport, RenderDisposition, ViewerPhase,
};
use crate::scene::{
    BATCH_KEY, BATCH_VERSION, PreparedScene, SceneFacts, VIEW_GENERATION, centre_point_id,
    render_limits,
};

const INITIALIZATION_ACTION: &str = "Keep the canvas unavailable, use a secure context with a WebGPU-capable browser and device, then retry initialization.";
const RECREATE_ACTION: &str =
    "Destroy this viewer and explicitly create a new viewer before rendering again.";
const RETRY_FRAME_ACTION: &str = "Keep the last presented frame and request another frame after the browser reports the canvas visible.";

/// Creates the private browser acceptance viewer after complete capability and
/// deterministic scene validation.
///
/// # Errors
///
/// Returns a structured JSON error string in a JavaScript value when the
/// browser, canvas, WebGPU adapter/device, renderer, scene, or requested size is
/// unsupported. No viewer is returned after partial initialization.
#[wasm_bindgen(js_name = createViewer)]
pub async fn create_viewer(
    canvas: HtmlCanvasElement,
    css_width: f64,
    css_height: f64,
    device_pixel_ratio: f64,
) -> Result<BrowserViewer, JsValue> {
    console_error_panic_hook::set_once();
    preflight_browser()?;
    let viewport = PhysicalViewport::from_css(css_width, css_height, device_pixel_ratio)
        .map_err(model_failure)?;
    let scene = PreparedScene::new()
        .map_err(|error| failure("scene_validation", error, INITIALIZATION_ACTION))?;
    let (resources, capabilities) = BrowserResources::initialize(&canvas, viewport, &scene).await?;
    Ok(BrowserViewer {
        resources: Some(resources),
        scene,
        viewport,
        lifecycle: Lifecycle::ready(),
        capabilities,
        last_frame: None,
        pick: PickFacts::not_requested(),
    })
}

/// Example-only browser viewer handle.
///
/// The JavaScript host owns lifecycle policy and calls these explicit methods.
/// This private acceptance boundary is not the supported SDK planned for v0.18.
#[wasm_bindgen]
pub struct BrowserViewer {
    resources: Option<BrowserResources>,
    scene: PreparedScene,
    viewport: PhysicalViewport,
    lifecycle: Lifecycle,
    capabilities: CapabilityFacts,
    last_frame: Option<FrameFacts>,
    pick: PickFacts,
}

#[wasm_bindgen]
impl BrowserViewer {
    /// Reconfigures the physical canvas from caller-owned CSS and DPR facts.
    #[wasm_bindgen]
    pub fn resize(
        &mut self,
        css_width: f64,
        css_height: f64,
        device_pixel_ratio: f64,
    ) -> Result<String, JsValue> {
        self.ensure_active()?;
        let viewport = PhysicalViewport::from_css(css_width, css_height, device_pixel_ratio)
            .map_err(model_failure)?;
        self.resources_mut()?.reconfigure(viewport);
        self.viewport = viewport;
        self.last_frame = None;
        self.pick = PickFacts::not_requested();
        self.diagnostics()
    }

    /// Declares whether the host considers the canvas visible.
    #[wasm_bindgen(js_name = setVisible)]
    pub fn set_visible(&mut self, visible: bool) -> Result<String, JsValue> {
        self.lifecycle.set_visible(visible).map_err(model_failure)?;
        if !visible {
            self.resources_mut()?.discard_interaction_state();
            self.pick = PickFacts::not_requested();
        }
        self.diagnostics()
    }

    /// Records, submits, and presents one frame when the host is visible.
    #[wasm_bindgen]
    pub fn render(&mut self) -> Result<String, JsValue> {
        match self.lifecycle.begin_render().map_err(model_failure)? {
            RenderDisposition::SkipHidden => return self.diagnostics(),
            RenderDisposition::Record => {}
        }

        let frame = self.scene_frame()?;
        let (report, suboptimal) = self.resources_mut()?.render(&frame)?;
        validate_transient_bytes(report)?;
        self.lifecycle.record_frame().map_err(model_failure)?;
        self.last_frame = Some(FrameFacts::from_report(report, suboptimal));
        self.pick = PickFacts::not_requested();
        self.diagnostics()
    }

    /// Begins one nonblocking provisional pick against the last recorded frame.
    #[wasm_bindgen(js_name = beginPick)]
    pub fn begin_pick(&mut self, x: u32, y: u32) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let dimensions = self.viewport.dimensions();
        if x >= dimensions[0] || y >= dimensions[1] {
            return Err(failure(
                "pick_outside_viewport",
                format!("pick pixel [{x}, {y}] is outside viewport {dimensions:?}"),
                "Choose a physical pixel inside the current viewport and begin a new provisional pick.",
            ));
        }
        self.resources_mut()?.begin_pick([x, y])?;
        self.pick = PickFacts::pending();
        self.diagnostics()
    }

    /// Polls the current provisional pick without blocking the browser event loop.
    #[wasm_bindgen(js_name = pollPick)]
    pub fn poll_pick(&mut self) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let outcome = self.resources_mut()?.poll_pick()?;
        match outcome {
            PickPoll::Pending => self.pick = PickFacts::pending(),
            PickPoll::Ready(None) => self.pick = PickFacts::miss(),
            PickPoll::Ready(Some(hit)) => self.accept_pick(hit)?,
        }
        self.diagnostics()
    }

    /// Returns the complete bounded host diagnostics as JSON.
    #[wasm_bindgen]
    pub fn diagnostics(&self) -> Result<String, JsValue> {
        let limits = LimitFacts::new(render_limits());
        let diagnostics = Diagnostics {
            schema: "punctra-browser-foundation-v1",
            package_version: env!("CARGO_PKG_VERSION"),
            phase: self.lifecycle.phase(),
            rendered_frames: self.lifecycle.rendered_frames(),
            hidden_frame_skips: self.lifecycle.hidden_frame_skips(),
            capabilities: &self.capabilities,
            limits,
            viewport: self.viewport,
            scene: self.scene.facts(),
            frame: self.last_frame,
            pick: &self.pick,
            display_authority: "progressive_gpu_non_authoritative",
            safe_shutdown_action: RECREATE_ACTION,
        };
        serde_json::to_string(&diagnostics).map_err(|error| {
            failure(
                "diagnostic_serialization",
                error,
                "Keep the canvas unavailable and recreate the viewer before relying on diagnostics.",
            )
        })
    }

    /// Drops renderer and GPU state and fuses the viewer against later work.
    #[wasm_bindgen]
    pub fn shutdown(&mut self) -> Result<String, JsValue> {
        self.resources.take();
        self.last_frame = None;
        self.pick = PickFacts::not_requested();
        self.lifecycle.shutdown();
        self.diagnostics()
    }
}

impl BrowserViewer {
    fn ensure_active(&self) -> Result<(), JsValue> {
        if self.lifecycle.phase() == ViewerPhase::Shutdown {
            Err(model_failure(HostModelError::ViewerShutdown))
        } else {
            Ok(())
        }
    }

    fn ensure_ready(&self) -> Result<(), JsValue> {
        self.ensure_active()?;
        if self.lifecycle.phase() == ViewerPhase::Hidden {
            Err(failure(
                "viewer_hidden",
                "the host declared the canvas hidden",
                RETRY_FRAME_ACTION,
            ))
        } else {
            Ok(())
        }
    }

    fn resources_mut(&mut self) -> Result<&mut BrowserResources, JsValue> {
        self.resources
            .as_mut()
            .ok_or_else(|| model_failure(HostModelError::ViewerShutdown))
    }

    fn scene_frame(&self) -> Result<Frame, JsValue> {
        let dimensions = self.viewport.dimensions();
        let viewport = Viewport::new(dimensions[0], dimensions[1]).map_err(|error| {
            failure(
                "viewport_validation",
                error,
                "Choose a nonzero bounded canvas size.",
            )
        })?;
        self.scene
            .frame(viewport)
            .map_err(|error| failure("frame_validation", error, RECREATE_ACTION))
    }

    fn accept_pick(&mut self, hit: PickHit) -> Result<(), JsValue> {
        let invariant_matches = hit.view_generation() == VIEW_GENERATION
            && hit.batch() == BATCH_KEY
            && hit.version() == BATCH_VERSION
            && hit.point() == centre_point_id();
        if !invariant_matches {
            return Err(failure(
                "pick_invariant",
                "the centre-pixel hit did not preserve the fixed generation, batch, version, and Point identity",
                RECREATE_ACTION,
            ));
        }
        self.pick = PickFacts::hit(hit);
        Ok(())
    }
}

struct BrowserResources {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    canvas: HtmlCanvasElement,
    surface_configuration: wgpu::SurfaceConfiguration,
    renderer: WgpuRenderer,
    recorded_frame: Option<RecordedFrame>,
    pick_ticket: Option<PickTicket>,
}

impl BrowserResources {
    async fn initialize(
        canvas: &HtmlCanvasElement,
        viewport: PhysicalViewport,
        scene: &PreparedScene,
    ) -> Result<(Self, CapabilityFacts), JsValue> {
        let instance = browser_instance();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| failure("canvas_surface", error, INITIALIZATION_ACTION))?;
        let adapter = request_adapter(&instance, &surface).await?;
        let capabilities = surface.get_capabilities(&adapter);
        let surface_configuration = surface_configuration(&capabilities, viewport)?;
        let adapter_info = adapter.get_info();
        let adapter_limits = adapter.limits();
        let (device, queue) = request_device(&adapter).await?;
        let renderer = create_renderer(&device, surface_configuration.format, scene)?;
        set_canvas_size(canvas, viewport);
        surface.configure(&device, &surface_configuration);
        let facts = CapabilityFacts::new(&adapter_info, &adapter_limits, &surface_configuration);
        Ok((
            Self {
                _instance: instance,
                surface,
                device,
                queue,
                canvas: canvas.clone(),
                surface_configuration,
                renderer,
                recorded_frame: None,
                pick_ticket: None,
            },
            facts,
        ))
    }

    fn reconfigure(&mut self, viewport: PhysicalViewport) {
        let dimensions = viewport.dimensions();
        self.surface_configuration.width = dimensions[0];
        self.surface_configuration.height = dimensions[1];
        set_canvas_size(&self.canvas, viewport);
        self.discard_interaction_state();
        self.surface
            .configure(&self.device, &self.surface_configuration);
    }

    fn render(&mut self, frame: &Frame) -> Result<(FrameReport, bool), JsValue> {
        self.discard_interaction_state();
        let (surface_texture, suboptimal) = self.acquire_surface_texture()?;
        let target = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encoder("Punctra browser frame");
        let recorded = self
            .renderer
            .render(&mut encoder, &target, frame)
            .map_err(|error| failure("frame_recording", error, RECREATE_ACTION))?;
        let report = recorded.report();
        self.queue.submit([encoder.finish()]);
        self.queue.present(surface_texture);
        if suboptimal {
            self.surface
                .configure(&self.device, &self.surface_configuration);
        }
        self.recorded_frame = Some(recorded);
        Ok((report, suboptimal))
    }

    fn begin_pick(&mut self, pixel: [u32; 2]) -> Result<(), JsValue> {
        if self.pick_ticket.is_some() {
            return Err(failure(
                "pick_pending",
                "a provisional pick is already pending",
                "Poll the current pick to completion before beginning another one.",
            ));
        }
        let recorded = self.recorded_frame.as_ref().ok_or_else(|| {
            failure(
                "missing_recorded_frame",
                "no recorded frame is available for provisional picking",
                "Render a visible frame before beginning a provisional pick.",
            )
        })?;
        let mut encoder = self.encoder("Punctra browser provisional pick");
        let ticket = self
            .renderer
            .pick(&mut encoder, recorded, PickRequest::new(pixel))
            .map_err(|error| failure("pick_recording", error, RECREATE_ACTION))?;
        self.queue.submit([encoder.finish()]);
        self.pick_ticket = Some(ticket);
        Ok(())
    }

    fn poll_pick(&mut self) -> Result<PickPoll, JsValue> {
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| failure("device_poll", error, RECREATE_ACTION))?;
        let ticket = self.pick_ticket.as_mut().ok_or_else(|| {
            failure(
                "pick_not_requested",
                "no provisional pick is pending",
                "Begin a provisional pick against the last visible frame before polling.",
            )
        })?;
        let outcome = ticket
            .poll()
            .map_err(|error| failure("pick_readback", error, RECREATE_ACTION))?;
        if matches!(outcome, PickPoll::Ready(_)) {
            self.pick_ticket = None;
        }
        Ok(outcome)
    }

    fn discard_interaction_state(&mut self) {
        self.pick_ticket = None;
        self.recorded_frame = None;
    }

    fn acquire_surface_texture(&self) -> Result<(wgpu::SurfaceTexture, bool), JsValue> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => Ok((texture, false)),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok((texture, true)),
            wgpu::CurrentSurfaceTexture::Timeout => Err(failure(
                "surface_timeout",
                "the browser timed out while acquiring a canvas texture",
                RETRY_FRAME_ACTION,
            )),
            wgpu::CurrentSurfaceTexture::Occluded => Err(failure(
                "surface_occluded",
                "the browser reported the canvas occluded",
                RETRY_FRAME_ACTION,
            )),
            wgpu::CurrentSurfaceTexture::Outdated => Err(failure(
                "surface_outdated",
                "the browser canvas surface is outdated",
                "Repeat the bounded resize, then request a new frame.",
            )),
            wgpu::CurrentSurfaceTexture::Lost => Err(failure(
                "surface_lost",
                "the browser canvas surface was lost",
                RECREATE_ACTION,
            )),
            wgpu::CurrentSurfaceTexture::Validation => Err(failure(
                "surface_validation",
                "WebGPU rejected canvas texture acquisition",
                RECREATE_ACTION,
            )),
        }
    }

    fn encoder(&self, label: &'static str) -> wgpu::CommandEncoder {
        self.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) })
    }
}

fn preflight_browser() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| {
        failure(
            "missing_window",
            "the WebAssembly module is not running in a browser Window",
            INITIALIZATION_ACTION,
        )
    })?;
    if !window.is_secure_context() {
        return Err(failure(
            "insecure_context",
            "WebGPU requires a secure browser context; serve this host from localhost or HTTPS",
            INITIALIZATION_ACTION,
        ));
    }
    let navigator = window.navigator();
    let has_webgpu =
        Reflect::has(navigator.as_ref(), &JsValue::from_str("gpu")).map_err(|error| {
            failure(
                "capability_inspection",
                format!("could not inspect navigator.gpu: {error:?}"),
                INITIALIZATION_ACTION,
            )
        })?;
    if !has_webgpu {
        return Err(failure(
            "webgpu_unavailable",
            "navigator.gpu is unavailable in this browser",
            INITIALIZATION_ACTION,
        ));
    }
    Ok(())
}

fn browser_instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    wgpu::Instance::new(descriptor)
}

async fn request_adapter(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
) -> Result<wgpu::Adapter, JsValue> {
    instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: Some(surface),
            apply_limit_buckets: false,
        })
        .await
        .map_err(|error| failure("webgpu_adapter", error, INITIALIZATION_ACTION))
}

async fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), JsValue> {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("Punctra browser foundation device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })
        .await
        .map_err(|error| failure("webgpu_device", error, INITIALIZATION_ACTION))
}

fn surface_configuration(
    capabilities: &wgpu::SurfaceCapabilities,
    viewport: PhysicalViewport,
) -> Result<wgpu::SurfaceConfiguration, JsValue> {
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| capabilities.formats.first().copied())
        .ok_or_else(|| {
            failure(
                "surface_format",
                "the canvas exposes no surface format",
                INITIALIZATION_ACTION,
            )
        })?;
    if !capabilities
        .present_modes
        .contains(&wgpu::PresentMode::Fifo)
    {
        return Err(failure(
            "presentation_mode",
            "the canvas does not expose required FIFO presentation",
            INITIALIZATION_ACTION,
        ));
    }
    let alpha_mode = capabilities.alpha_modes.first().copied().ok_or_else(|| {
        failure(
            "surface_alpha_mode",
            "the canvas exposes no composite alpha mode",
            INITIALIZATION_ACTION,
        )
    })?;
    let dimensions = viewport.dimensions();
    Ok(wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Srgb,
        width: dimensions[0],
        height: dimensions[1],
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: Vec::new(),
    })
}

fn create_renderer(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    scene: &PreparedScene,
) -> Result<WgpuRenderer, JsValue> {
    let mut renderer = WgpuRenderer::new(device, RendererConfig::new(format, render_limits()))
        .map_err(|error| failure("renderer_capability", error, INITIALIZATION_ACTION))?;
    renderer
        .apply(&PreparedScene::reset_update())
        .and_then(|_| renderer.apply(&scene.batch_update()))
        .map_err(|error| failure("scene_publication", error, INITIALIZATION_ACTION))?;
    Ok(renderer)
}

fn set_canvas_size(canvas: &HtmlCanvasElement, viewport: PhysicalViewport) {
    let dimensions = viewport.dimensions();
    canvas.set_width(dimensions[0]);
    canvas.set_height(dimensions[1]);
}

fn validate_transient_bytes(report: FrameReport) -> Result<(), JsValue> {
    if report.transient_texture_bytes() <= MAX_RENDER_TRANSIENT_BYTES {
        Ok(())
    } else {
        Err(failure(
            "transient_texture_limit",
            format!(
                "renderer transient textures used {} bytes above the {}-byte ceiling",
                report.transient_texture_bytes(),
                MAX_RENDER_TRANSIENT_BYTES
            ),
            RECREATE_ACTION,
        ))
    }
}

#[derive(Serialize)]
struct Diagnostics<'a> {
    schema: &'static str,
    package_version: &'static str,
    phase: ViewerPhase,
    rendered_frames: u64,
    hidden_frame_skips: u64,
    capabilities: &'a CapabilityFacts,
    limits: LimitFacts,
    viewport: PhysicalViewport,
    scene: SceneFacts,
    frame: Option<FrameFacts>,
    pick: &'a PickFacts,
    display_authority: &'static str,
    safe_shutdown_action: &'static str,
}

#[derive(Serialize)]
struct CapabilityFacts {
    secure_context: bool,
    webgpu: bool,
    browser_user_agent: String,
    browser_platform: String,
    adapter_name: String,
    backend: String,
    device_type: String,
    surface_format: String,
    present_mode: &'static str,
    required_feature_count: u64,
    adapter_max_buffer_size: u64,
    adapter_max_texture_dimension_2d: u32,
}

impl CapabilityFacts {
    fn new(
        adapter: &wgpu::AdapterInfo,
        limits: &wgpu::Limits,
        surface: &wgpu::SurfaceConfiguration,
    ) -> Self {
        let adapter_name = if adapter.name.is_empty() {
            "browser WebGPU adapter".to_owned()
        } else {
            adapter.name.clone()
        };
        Self {
            secure_context: true,
            webgpu: true,
            browser_user_agent: browser_user_agent(),
            browser_platform: browser_platform(),
            adapter_name,
            backend: format!("{:?}", adapter.backend),
            device_type: format!("{:?}", adapter.device_type),
            surface_format: format!("{:?}", surface.format),
            present_mode: "fifo",
            required_feature_count: 0,
            adapter_max_buffer_size: limits.max_buffer_size,
            adapter_max_texture_dimension_2d: limits.max_texture_dimension_2d,
        }
    }
}

fn browser_user_agent() -> String {
    browser_navigator()
        .and_then(|navigator| navigator.user_agent().ok())
        .unwrap_or_else(|| "unreported browser user agent".to_owned())
}

fn browser_platform() -> String {
    browser_navigator()
        .and_then(|navigator| navigator.platform().ok())
        .unwrap_or_else(|| "unreported browser platform".to_owned())
}

fn browser_navigator() -> Option<web_sys::Navigator> {
    web_sys::window().map(|window| window.navigator())
}

#[derive(Clone, Copy, Serialize)]
struct LimitFacts {
    estimated_gpu_bytes: u64,
    points: u64,
    batches: u64,
    highlight_points: u64,
    canvas_dimension: u32,
    canvas_pixels: u64,
    device_pixel_ratio: f64,
    renderer_transient_bytes: u64,
}

impl LimitFacts {
    const fn new(render: RenderLimits) -> Self {
        Self {
            estimated_gpu_bytes: render.max_estimated_gpu_bytes(),
            points: render.max_points(),
            batches: render.max_batches(),
            highlight_points: render.max_highlight_points(),
            canvas_dimension: MAX_CANVAS_DIMENSION,
            canvas_pixels: MAX_CANVAS_PIXELS,
            device_pixel_ratio: MAX_DEVICE_PIXEL_RATIO,
            renderer_transient_bytes: MAX_RENDER_TRANSIENT_BYTES,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
struct FrameFacts {
    view_generation: u64,
    drawn_points: u64,
    draw_calls: u64,
    resident_bytes: u64,
    transient_texture_bytes: u64,
    surface_suboptimal: bool,
}

impl FrameFacts {
    const fn from_report(report: FrameReport, surface_suboptimal: bool) -> Self {
        Self {
            view_generation: report.view_generation().generation(),
            drawn_points: report.drawn_points(),
            draw_calls: report.draw_calls(),
            resident_bytes: report.resident_bytes(),
            transient_texture_bytes: report.transient_texture_bytes(),
            surface_suboptimal,
        }
    }
}

#[derive(Serialize)]
struct PickFacts {
    status: &'static str,
    authority: &'static str,
    generation: Option<u64>,
    batch_key: Option<u64>,
    batch_version: Option<u64>,
    point_ordinal: Option<u64>,
}

impl PickFacts {
    const fn not_requested() -> Self {
        Self::new("not_requested", None)
    }

    const fn pending() -> Self {
        Self::new("pending", None)
    }

    const fn miss() -> Self {
        Self::new("miss", None)
    }

    const fn new(status: &'static str, hit: Option<PickHit>) -> Self {
        match hit {
            Some(hit) => Self {
                status,
                authority: "provisional_gpu_hint",
                generation: Some(hit.view_generation().generation()),
                batch_key: Some(hit.batch().get()),
                batch_version: Some(hit.version().get()),
                point_ordinal: Some(hit.point().ordinal()),
            },
            None => Self {
                status,
                authority: "provisional_gpu_hint",
                generation: None,
                batch_key: None,
                batch_version: None,
                point_ordinal: None,
            },
        }
    }

    const fn hit(hit: PickHit) -> Self {
        Self::new("hit", Some(hit))
    }
}

#[derive(Serialize)]
struct Failure {
    schema: &'static str,
    code: &'static str,
    message: String,
    safe_action: &'static str,
}

fn model_failure(error: HostModelError) -> JsValue {
    failure("host_model", error, RECREATE_ACTION)
}

fn failure(
    code: &'static str,
    message: impl std::fmt::Display,
    safe_action: &'static str,
) -> JsValue {
    let failure = Failure {
        schema: "punctra-browser-failure-v1",
        code,
        message: message.to_string(),
        safe_action,
    };
    let json = serde_json::to_string(&failure).unwrap_or_else(|_| {
        format!(
            "{{\"schema\":\"punctra-browser-failure-v1\",\"code\":\"{code}\",\"message\":\"browser failure\",\"safe_action\":\"{safe_action}\"}}"
        )
    });
    JsValue::from_str(&json)
}
