use std::sync::{Arc, Mutex};

use js_sys::Reflect;
use render_protocol::Viewport;
use render_wgpu::{
    Frame, FrameReport, PickHit, PickPoll, PickRequest, PickTicket, RecordedFrame, RendererConfig,
    WgpuRenderer,
};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::diagnostics::{
    CapabilityFacts, Diagnostics, Failure, FailureCode, FrameFacts, LimitFacts, PickFacts,
};
use crate::host::{
    CssViewportRequest, HostModelError, Lifecycle, MAX_RENDER_TRANSIENT_BYTES,
    PRESENTATION_LATENCY_FRAMES, PhysicalViewport, RESIZE_VIEWPORT_ACTION, RenderDisposition,
};
use crate::scene::{
    BATCH_KEY, BATCH_VERSION, PreparedScene, VIEW_GENERATION, centre_point_id, render_limits,
};
use crate::streaming::{StreamingLimitFacts, StreamingScene};

const INITIALIZATION_ACTION: &str = "Keep the canvas unavailable, use a secure context with a WebGPU-capable browser and device, then retry initialization.";
const INITIAL_VIEWPORT_ACTION: &str = "Keep the canvas unavailable, choose finite positive CSS dimensions and a device-pixel ratio at most four so the physical canvas remains within 4,096 pixels per dimension and 8,388,608 pixels total, then retry initialization.";
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
    let viewport = PhysicalViewport::from_css(CssViewportRequest::new(
        css_width,
        css_height,
        device_pixel_ratio,
    ))
    .map_err(initial_viewport_failure)?;
    let mut scene = PreparedScene::new()
        .map_err(|error| failure(FailureCode::SceneValidation, error, INITIALIZATION_ACTION))?;
    let (resources, capabilities) =
        BrowserResources::initialize(&canvas, viewport, &mut scene).await?;
    Ok(BrowserViewer {
        resources: Some(resources),
        scene,
        viewport,
        lifecycle: Lifecycle::ready(),
        capabilities,
        last_frame: None,
        pick: PickFacts::not_requested(),
        stream: StreamingScene::idle(),
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
    stream: StreamingScene,
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
        let viewport = PhysicalViewport::from_css(CssViewportRequest::new(
            css_width,
            css_height,
            device_pixel_ratio,
        ))
        .map_err(resize_viewport_failure)?;
        self.resources_mut()?.reconfigure(viewport)?;
        self.viewport = viewport;
        self.last_frame = None;
        self.pick = PickFacts::not_requested();
        self.diagnostics()
    }

    /// Declares whether the host considers the canvas visible.
    #[wasm_bindgen(js_name = setVisible)]
    pub fn set_visible(&mut self, visible: bool) -> Result<String, JsValue> {
        self.ensure_active()?;
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
        self.ensure_active()?;
        match self.lifecycle.begin_render().map_err(model_failure)? {
            RenderDisposition::SkipHidden => return self.diagnostics(),
            RenderDisposition::Record => {}
        }

        let frame = self.scene_frame()?;
        let (report, suboptimal) = self.resources_mut()?.render(&frame)?;
        validate_transient_bytes(report.transient_texture_bytes())?;
        self.lifecycle.record_frame().map_err(model_failure)?;
        self.last_frame = Some(FrameFacts::from_report(report, suboptimal));
        self.pick = PickFacts::not_requested();
        self.diagnostics()
    }

    /// Begins one nonblocking provisional pick against the last recorded frame.
    #[wasm_bindgen(js_name = beginPick)]
    pub fn begin_pick(&mut self, x: u32, y: u32) -> Result<String, JsValue> {
        self.ensure_ready()?;
        if self.stream.view_generation().is_some() {
            return Err(failure(
                FailureCode::StreamPickUnsupported,
                "the v0.16 streaming slice does not define a stable provisional-pick fixture",
                "Keep the progressive frame visible and wait for the later browser viewer API before relying on remote picking.",
            ));
        }
        let dimensions = self.viewport.dimensions();
        if x >= dimensions[0] || y >= dimensions[1] {
            return Err(failure(
                FailureCode::PickOutsideViewport,
                format!("pick pixel [{x}, {y}] is outside viewport {dimensions:?}"),
                "Choose a physical pixel inside the current viewport and begin a new provisional pick.",
            ));
        }
        self.last_frame
            .as_ref()
            .ok_or_else(missing_recorded_frame_failure)?;
        let transient_texture_bytes = self
            .viewport
            .renderer_transient_bytes_with_pick()
            .map_err(model_failure)?;
        validate_transient_bytes(transient_texture_bytes)?;
        self.resources_mut()?.begin_pick([x, y])?;
        self.last_frame
            .as_mut()
            .ok_or_else(missing_recorded_frame_failure)?
            .record_pick_transient_bytes(transient_texture_bytes);
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
        self.ensure_active()?;
        self.diagnostics_unchecked()
    }

    /// Drops renderer and GPU state and fuses the viewer against later work.
    #[wasm_bindgen]
    pub fn shutdown(&mut self) -> Result<String, JsValue> {
        self.lifecycle.shutdown().map_err(model_failure)?;
        self.resources.take();
        self.last_frame = None;
        self.pick = PickFacts::not_requested();
        self.diagnostics_unchecked()
    }

    /// Validates and publishes the first batch with its identity-bound reset.
    #[wasm_bindgen(js_name = beginStreamBatch)]
    #[allow(clippy::too_many_arguments)] // The private Wasm ABI carries explicit deployment and batch facts.
    pub fn begin_stream_batch(
        &mut self,
        source_identity: &str,
        expected_points: u32,
        origin_x: f64,
        origin_y: f64,
        origin_z: f64,
        batch_index: u32,
        payload: &[u8],
    ) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let mut next = self.stream.clone();
        let (reset, upsert) = next
            .begin_with_batch(
                source_identity,
                u64::from(expected_points),
                [origin_x, origin_y, origin_z],
                batch_index,
                payload,
            )
            .map_err(stream_validation_failure)?;
        let resources = self.resources_mut()?;
        resources.apply_update(&reset)?;
        resources.apply_update(&upsert)?;
        self.stream = next;
        self.reset_interaction_facts();
        self.diagnostics()
    }

    /// Publishes one bounded worker-decoded transfer batch.
    #[wasm_bindgen(js_name = publishStreamBatch)]
    pub fn publish_stream_batch(
        &mut self,
        batch_index: u32,
        payload: &[u8],
    ) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let mut next = self.stream.clone();
        let update = next
            .publish(batch_index, payload)
            .map_err(stream_validation_failure)?;
        self.resources_mut()?.apply_update(&update)?;
        self.stream = next;
        self.reset_interaction_facts();
        self.diagnostics()
    }

    /// Seals the sampled root after every declared Point is published.
    #[wasm_bindgen(js_name = completeStream)]
    pub fn complete_stream(&mut self) -> Result<String, JsValue> {
        self.ensure_ready()?;
        self.stream.complete().map_err(stream_validation_failure)?;
        self.diagnostics()
    }
}

impl BrowserViewer {
    fn diagnostics_unchecked(&self) -> Result<String, JsValue> {
        if let Some(resources) = &self.resources {
            resources.ensure_device_available()?;
        }
        let limits = LimitFacts::new(render_limits());
        let diagnostics = Diagnostics {
            schema: "punctra-browser-streaming-v1",
            package_version: env!("CARGO_PKG_VERSION"),
            phase: self.lifecycle.phase(),
            rendered_frames: self.lifecycle.rendered_frames(),
            hidden_frame_skips: self.lifecycle.hidden_frame_skips(),
            capabilities: &self.capabilities,
            limits,
            viewport: self.viewport,
            scene: self.scene.facts(),
            streaming: self.stream.facts(),
            streaming_limits: StreamingLimitFacts::fixed(),
            frame: self.last_frame,
            pick: &self.pick,
            display_authority: "progressive_gpu_non_authoritative",
            safe_shutdown_action: RECREATE_ACTION,
        };
        diagnostics.to_json().map_err(|error| {
            failure(
                FailureCode::DiagnosticSerialization,
                error,
                "Keep the canvas unavailable and recreate the viewer before relying on diagnostics.",
            )
        })
    }

    fn ensure_active(&self) -> Result<(), JsValue> {
        self.lifecycle.ensure_active().map_err(model_failure)
    }

    fn ensure_ready(&self) -> Result<(), JsValue> {
        self.lifecycle.ensure_ready().map_err(interaction_failure)
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
                FailureCode::ViewportValidation,
                error,
                "Choose a nonzero bounded canvas size.",
            )
        })?;
        let view_generation = self.stream.view_generation().unwrap_or(VIEW_GENERATION);
        self.scene
            .frame(viewport, view_generation)
            .map_err(|error| failure(FailureCode::FrameValidation, error, RECREATE_ACTION))
    }

    fn reset_interaction_facts(&mut self) {
        self.last_frame = None;
        self.pick = PickFacts::not_requested();
    }

    fn accept_pick(&mut self, hit: PickHit) -> Result<(), JsValue> {
        let invariant_matches = hit.view_generation() == VIEW_GENERATION
            && hit.batch() == BATCH_KEY
            && hit.version() == BATCH_VERSION
            && hit.point() == centre_point_id();
        if !invariant_matches {
            return Err(failure(
                FailureCode::PickInvariant,
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
    device_loss: DeviceLossState,
}

type DeviceLossState = Arc<Mutex<Option<String>>>;

impl BrowserResources {
    async fn initialize(
        canvas: &HtmlCanvasElement,
        viewport: PhysicalViewport,
        scene: &mut PreparedScene,
    ) -> Result<(Self, CapabilityFacts), JsValue> {
        let instance = browser_instance();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|error| failure(FailureCode::CanvasSurface, error, INITIALIZATION_ACTION))?;
        let adapter = request_adapter(&instance, &surface).await?;
        let capabilities = surface.get_capabilities(&adapter);
        let surface_configuration = surface_configuration(&capabilities, viewport)?;
        let adapter_info = adapter.get_info();
        let adapter_limits = adapter.limits();
        let (device, queue) = request_device(&adapter).await?;
        let device_loss = track_device_loss(&device);
        set_canvas_size(canvas, viewport);
        configure_surface(
            &surface,
            &device,
            &surface_configuration,
            FailureCode::SurfaceConfiguration,
            INITIALIZATION_ACTION,
        )?;
        let mut renderer = create_renderer(&device, surface_configuration.format)?;
        publish_scene(&mut renderer, scene)?;
        scene
            .settle_after_publication()
            .map_err(|error| failure(FailureCode::ScenePlanning, error, INITIALIZATION_ACTION))?;
        let facts = CapabilityFacts::new(
            &adapter_info,
            &adapter_limits,
            &capabilities,
            &surface_configuration,
            browser_user_agent(),
            browser_platform(),
        );
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
                device_loss,
            },
            facts,
        ))
    }

    fn reconfigure(&mut self, viewport: PhysicalViewport) -> Result<(), JsValue> {
        self.ensure_device_available()?;
        let dimensions = viewport.dimensions();
        self.surface_configuration.width = dimensions[0];
        self.surface_configuration.height = dimensions[1];
        set_canvas_size(&self.canvas, viewport);
        self.discard_interaction_state();
        configure_surface(
            &self.surface,
            &self.device,
            &self.surface_configuration,
            FailureCode::SurfaceReconfiguration,
            RECREATE_ACTION,
        )?;
        self.ensure_device_available()
    }

    fn render(&mut self, frame: &Frame) -> Result<(FrameReport, bool), JsValue> {
        self.ensure_device_available()?;
        self.discard_interaction_state();
        let (surface_texture, suboptimal) = self.acquire_surface_texture()?;
        let target = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encoder("Punctra browser frame");
        let recorded = self
            .renderer
            .render(&mut encoder, &target, frame)
            .map_err(|error| failure(FailureCode::FrameRecording, error, RECREATE_ACTION))?;
        let report = recorded.report();
        self.queue.submit([encoder.finish()]);
        self.queue.present(surface_texture);
        self.recorded_frame = Some(recorded);
        Ok((report, suboptimal))
    }

    fn apply_update(&mut self, update: &render_protocol::RenderUpdate) -> Result<(), JsValue> {
        self.ensure_device_available()?;
        self.discard_interaction_state();
        self.renderer
            .apply(update)
            .map(|_| ())
            .map_err(|error| failure(FailureCode::StreamPublication, error, RECREATE_ACTION))
    }

    fn begin_pick(&mut self, pixel: [u32; 2]) -> Result<(), JsValue> {
        self.ensure_device_available()?;
        if self.pick_ticket.is_some() {
            return Err(failure(
                FailureCode::PickPending,
                "a provisional pick is already pending",
                "Poll the current pick to completion before beginning another one.",
            ));
        }
        let recorded = self
            .recorded_frame
            .as_ref()
            .ok_or_else(missing_recorded_frame_failure)?;
        let mut encoder = self.encoder("Punctra browser provisional pick");
        let ticket = self
            .renderer
            .pick(&mut encoder, recorded, PickRequest::new(pixel))
            .map_err(|error| failure(FailureCode::PickRecording, error, RECREATE_ACTION))?;
        self.queue.submit([encoder.finish()]);
        self.pick_ticket = Some(ticket);
        Ok(())
    }

    fn poll_pick(&mut self) -> Result<PickPoll, JsValue> {
        self.ensure_device_available()?;
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|error| failure(FailureCode::DevicePoll, error, RECREATE_ACTION))?;
        let ticket = self.pick_ticket.as_mut().ok_or_else(|| {
            failure(
                FailureCode::PickNotRequested,
                "no provisional pick is pending",
                "Begin a provisional pick against the last visible frame before polling.",
            )
        })?;
        let outcome = ticket
            .poll()
            .map_err(|error| failure(FailureCode::PickReadback, error, RECREATE_ACTION))?;
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
                FailureCode::SurfaceTimeout,
                "the browser timed out while acquiring a canvas texture",
                RETRY_FRAME_ACTION,
            )),
            wgpu::CurrentSurfaceTexture::Occluded => Err(failure(
                FailureCode::SurfaceOccluded,
                "the browser reported the canvas occluded",
                RETRY_FRAME_ACTION,
            )),
            wgpu::CurrentSurfaceTexture::Outdated => Err(failure(
                FailureCode::SurfaceOutdated,
                "the browser canvas surface is outdated",
                "Repeat the bounded resize, then request a new frame.",
            )),
            wgpu::CurrentSurfaceTexture::Lost => Err(failure(
                FailureCode::SurfaceLost,
                "the browser canvas surface was lost",
                RECREATE_ACTION,
            )),
            wgpu::CurrentSurfaceTexture::Validation => Err(failure(
                FailureCode::SurfaceValidation,
                "WebGPU rejected canvas texture acquisition",
                RECREATE_ACTION,
            )),
        }
    }

    fn encoder(&self, label: &'static str) -> wgpu::CommandEncoder {
        self.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) })
    }

    fn ensure_device_available(&self) -> Result<(), JsValue> {
        let loss = self
            .device_loss
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match loss {
            Some(loss) => Err(failure(FailureCode::DeviceLost, loss, RECREATE_ACTION)),
            None => Ok(()),
        }
    }
}

fn track_device_loss(device: &wgpu::Device) -> DeviceLossState {
    let state = Arc::new(Mutex::new(None));
    let callback_state = Arc::clone(&state);
    device.set_device_lost_callback(move |reason, message| {
        let loss = format!("WebGPU device lost ({reason:?}): {message}");
        *callback_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(loss);
    });
    state
}

fn preflight_browser() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| {
        failure(
            FailureCode::MissingWindow,
            "the WebAssembly module is not running in a browser Window",
            INITIALIZATION_ACTION,
        )
    })?;
    if !window.is_secure_context() {
        return Err(failure(
            FailureCode::InsecureContext,
            "WebGPU requires a secure browser context; serve this host from localhost or HTTPS",
            INITIALIZATION_ACTION,
        ));
    }
    let navigator = window.navigator();
    let has_webgpu =
        Reflect::has(navigator.as_ref(), &JsValue::from_str("gpu")).map_err(|error| {
            failure(
                FailureCode::CapabilityInspection,
                format!("could not inspect navigator.gpu: {error:?}"),
                INITIALIZATION_ACTION,
            )
        })?;
    if !has_webgpu {
        return Err(failure(
            FailureCode::WebGpuUnavailable,
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
        .map_err(|error| failure(FailureCode::WebGpuAdapter, error, INITIALIZATION_ACTION))
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
        .map_err(|error| failure(FailureCode::WebGpuDevice, error, INITIALIZATION_ACTION))
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
                FailureCode::SurfaceFormat,
                "the canvas exposes no surface format",
                INITIALIZATION_ACTION,
            )
        })?;
    if !capabilities
        .present_modes
        .contains(&wgpu::PresentMode::Fifo)
    {
        return Err(failure(
            FailureCode::PresentationMode,
            "the canvas does not expose required FIFO presentation",
            INITIALIZATION_ACTION,
        ));
    }
    let alpha_mode = capabilities
        .alpha_modes
        .iter()
        .copied()
        .find(|mode| {
            matches!(
                mode,
                wgpu::CompositeAlphaMode::Opaque | wgpu::CompositeAlphaMode::PreMultiplied
            )
        })
        .ok_or_else(|| {
            failure(
                FailureCode::SurfaceAlphaMode,
                "the canvas exposes no supported opaque or premultiplied composite alpha mode",
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
        desired_maximum_frame_latency: PRESENTATION_LATENCY_FRAMES,
        alpha_mode,
        view_formats: Vec::new(),
    })
}

fn create_renderer(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> Result<WgpuRenderer, JsValue> {
    WgpuRenderer::new(device, RendererConfig::new(format, render_limits())).map_err(|error| {
        failure(
            FailureCode::RendererCapability,
            error,
            INITIALIZATION_ACTION,
        )
    })
}

fn publish_scene(renderer: &mut WgpuRenderer, scene: &PreparedScene) -> Result<(), JsValue> {
    renderer
        .apply(&PreparedScene::reset_update())
        .and_then(|_| renderer.apply(&scene.batch_update()))
        .map(|_| ())
        .map_err(|error| failure(FailureCode::ScenePublication, error, INITIALIZATION_ACTION))
}

fn set_canvas_size(canvas: &HtmlCanvasElement, viewport: PhysicalViewport) {
    let dimensions = viewport.dimensions();
    canvas.set_width(dimensions[0]);
    canvas.set_height(dimensions[1]);
}

fn configure_surface(
    surface: &wgpu::Surface<'_>,
    device: &wgpu::Device,
    configuration: &wgpu::SurfaceConfiguration,
    failure_code: FailureCode,
    safe_action: &'static str,
) -> Result<(), JsValue> {
    surface.configure(device, configuration);
    // The WebGPU backend converts a thrown canvas `configure` into `Lost` on
    // the next acquisition, so probe once before publishing success.
    match surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(texture)
        | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
            drop(texture);
            Ok(())
        }
        wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => Ok(()),
        wgpu::CurrentSurfaceTexture::Outdated => Err(failure(
            failure_code,
            "the browser canvas surface remained outdated after configuration",
            safe_action,
        )),
        wgpu::CurrentSurfaceTexture::Lost => Err(failure(
            failure_code,
            "the browser rejected the canvas surface configuration",
            safe_action,
        )),
        wgpu::CurrentSurfaceTexture::Validation => Err(failure(
            failure_code,
            "WebGPU rejected canvas surface configuration validation",
            safe_action,
        )),
    }
}

fn validate_transient_bytes(transient_texture_bytes: u64) -> Result<(), JsValue> {
    if transient_texture_bytes <= MAX_RENDER_TRANSIENT_BYTES {
        Ok(())
    } else {
        Err(failure(
            FailureCode::TransientTextureLimit,
            format!(
                "renderer transient textures used {transient_texture_bytes} bytes above the {MAX_RENDER_TRANSIENT_BYTES}-byte ceiling"
            ),
            RECREATE_ACTION,
        ))
    }
}

fn missing_recorded_frame_failure() -> JsValue {
    failure(
        FailureCode::MissingRecordedFrame,
        "no recorded frame is available for provisional picking",
        "Render a visible frame before beginning a provisional pick.",
    )
}

fn stream_validation_failure(error: crate::streaming::StreamError) -> JsValue {
    failure(
        FailureCode::StreamValidation,
        error,
        "Terminate the current worker operation, keep the last complete frame, and start a new identity-bound stream.",
    )
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

fn model_failure(error: HostModelError) -> JsValue {
    failure(FailureCode::HostModel, error, RECREATE_ACTION)
}

fn initial_viewport_failure(error: HostModelError) -> JsValue {
    failure(FailureCode::InitialViewport, error, INITIAL_VIEWPORT_ACTION)
}

fn resize_viewport_failure(error: HostModelError) -> JsValue {
    failure(FailureCode::ResizeViewport, error, RESIZE_VIEWPORT_ACTION)
}

fn interaction_failure(error: HostModelError) -> JsValue {
    if error == HostModelError::ViewerHidden {
        failure(FailureCode::ViewerHidden, error, RETRY_FRAME_ACTION)
    } else {
        model_failure(error)
    }
}

fn failure(
    code: FailureCode,
    message: impl std::fmt::Display,
    safe_action: &'static str,
) -> JsValue {
    JsValue::from_str(&Failure::new(code, message, safe_action).to_json())
}
