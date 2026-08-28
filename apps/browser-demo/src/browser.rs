use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use js_sys::Reflect;
use render_protocol::{
    Camera, PointId, PresentationWeight, RenderUpdate, ViewGenerationKey, Viewport,
};
use render_wgpu::{
    Frame, FrameReport, PickHit, PickPoll, PickRequest, PickTicket, PointFootprintStatus,
    RecordedFrame, RendererConfig, WgpuRenderer,
};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::capture::{CaptureCompletionFacts, CaptureFrameFacts, CaptureLayout, CaptureSlot};
use crate::diagnostics::{
    CameraFacts, CapabilityFacts, CaptureResourceFacts, Diagnostics, Failure, FailureCode,
    FrameFacts, HighlightFacts, LimitFacts, PickFacts, PointFootprintFacts,
};
use crate::display::DisplayMode;
use crate::host::{
    CssViewportRequest, HostModelError, Lifecycle, MAX_RENDER_TRANSIENT_BYTES,
    PRESENTATION_LATENCY_FRAMES, PhysicalViewport, RESIZE_VIEWPORT_ACTION, RenderDisposition,
};
use crate::scene::{
    BATCH_KEY, BATCH_VERSION, MAX_HIGHLIGHT_POINTS, NOMINAL_PICK_SIZE_PHYSICAL_PIXELS,
    PreparedScene, REQUESTED_POINT_FOOTPRINT, VIEW_GENERATION, centre_point_id,
    projected_density_display_size, render_limits,
};
use crate::streaming::{StreamingLimitFacts, StreamingScene, parse_source_identity};

const INITIALIZATION_ACTION: &str = "Keep the canvas unavailable, use a secure context with a WebGPU-capable browser and device, then retry initialization.";
const INITIAL_VIEWPORT_ACTION: &str = "Keep the canvas unavailable, choose finite positive CSS dimensions and a device-pixel ratio at most four so the physical canvas remains within 4,096 pixels per dimension and 8,388,608 pixels total, then retry initialization.";
const RECREATE_ACTION: &str =
    "Destroy this viewer and explicitly create a new viewer before rendering again.";
const RETRY_FRAME_ACTION: &str = "Keep the last presented frame and request another frame after the browser reports the canvas visible.";
const RETRY_CAPTURE_ACTION: &str =
    "Keep the last presented frame, discard this capture, and begin a new bounded frame capture.";

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
    let point_footprint_status = resources.point_footprint_status(renderer_viewport(viewport)?);
    let camera = scene.camera();
    Ok(BrowserViewer {
        resources: Some(resources),
        scene,
        viewport,
        lifecycle: Lifecycle::ready(),
        capabilities,
        last_frame: None,
        pick: PickFacts::not_requested(),
        camera,
        highlights: HighlightFacts::empty(),
        stream: StreamingScene::idle(),
        point_footprint_status,
    })
}

/// Example-only browser viewer handle.
///
/// The JavaScript host owns lifecycle policy and calls these explicit methods.
/// The supported JavaScript SDK wraps this low-level generated binding.
#[wasm_bindgen]
pub struct BrowserViewer {
    resources: Option<BrowserResources>,
    scene: PreparedScene,
    viewport: PhysicalViewport,
    lifecycle: Lifecycle,
    capabilities: CapabilityFacts,
    last_frame: Option<FrameFacts>,
    pick: PickFacts,
    camera: Camera,
    highlights: HighlightFacts,
    stream: StreamingScene,
    point_footprint_status: PointFootprintStatus,
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
        let renderer_viewport = renderer_viewport(viewport)?;
        self.resources_mut()?.reconfigure(viewport)?;
        self.viewport = viewport;
        self.point_footprint_status = self
            .resources_mut()?
            .point_footprint_status(renderer_viewport);
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
            self.resources_mut()?.discard_frame_dependent_state();
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
        let minimum_transient_texture_bytes = self
            .viewport
            .renderer_transient_bytes_with_pick()
            .map_err(model_failure)?;
        validate_transient_bytes(minimum_transient_texture_bytes)?;
        let transient_texture_bytes = self.resources_mut()?.begin_pick([x, y])?;
        validate_transient_bytes(transient_texture_bytes)?;
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

    /// Cancels one pending provisional pick while preserving its recorded frame.
    #[wasm_bindgen(js_name = cancelPick)]
    pub fn cancel_pick(&mut self) -> Result<String, JsValue> {
        self.ensure_active()?;
        self.resources_mut()?.cancel_pick();
        self.pick = PickFacts::not_requested();
        self.diagnostics()
    }

    /// Begins one bounded offscreen frame capture without presenting a frame.
    #[wasm_bindgen(js_name = beginFrameCapture)]
    pub fn begin_frame_capture(&mut self) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let frame = self.scene_frame()?;
        let batches = self
            .stream
            .capture_batch_facts()
            .map_err(stream_validation_failure)?;
        self.resources_mut()?.begin_frame_capture(&frame, batches)
    }

    /// Polls the current capture without blocking the browser event loop.
    ///
    /// JavaScript receives `undefined` while mapping is pending and one tight
    /// top-left-origin RGBA8 `Uint8Array` after completion.
    #[wasm_bindgen(js_name = pollFrameCapture)]
    pub fn poll_frame_capture(&mut self) -> Result<Option<Vec<u8>>, JsValue> {
        self.ensure_ready()?;
        self.resources_mut()?.poll_frame_capture()
    }

    /// Returns callback timing for the most recently completed frame capture.
    #[wasm_bindgen(js_name = frameCaptureCompletionFacts)]
    pub fn frame_capture_completion_facts(&self) -> Result<String, JsValue> {
        self.ensure_ready()?;
        self.resources
            .as_ref()
            .ok_or_else(|| model_failure(HostModelError::ViewerShutdown))?
            .frame_capture_completion_facts()
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
        if let Some(resources) = self.resources.as_mut() {
            resources.cancel_frame_capture();
        }
        self.resources.take();
        self.last_frame = None;
        self.pick = PickFacts::not_requested();
        self.highlights = HighlightFacts::empty();
        self.diagnostics_unchecked()
    }

    /// Replaces the active camera with one validated perspective camera.
    #[wasm_bindgen(js_name = setPerspectiveCamera)]
    #[allow(clippy::too_many_arguments)] // The private Wasm ABI carries one explicit camera value.
    pub fn set_perspective_camera(
        &mut self,
        eye_x: f64,
        eye_y: f64,
        eye_z: f64,
        target_x: f64,
        target_y: f64,
        target_z: f64,
        up_x: f64,
        up_y: f64,
        up_z: f64,
        vertical_field_of_view_radians: f32,
        near_distance: f32,
        far_distance: f32,
    ) -> Result<String, JsValue> {
        let camera = Camera::perspective(
            [eye_x, eye_y, eye_z],
            [target_x, target_y, target_z],
            [up_x, up_y, up_z],
            vertical_field_of_view_radians,
            near_distance,
            far_distance,
        )
        .map_err(camera_failure)?;
        self.set_camera(camera)
    }

    /// Replaces the active camera with one validated orthographic camera.
    #[wasm_bindgen(js_name = setOrthographicCamera)]
    #[allow(clippy::too_many_arguments)] // The private Wasm ABI carries one explicit camera value.
    pub fn set_orthographic_camera(
        &mut self,
        eye_x: f64,
        eye_y: f64,
        eye_z: f64,
        target_x: f64,
        target_y: f64,
        target_z: f64,
        up_x: f64,
        up_y: f64,
        up_z: f64,
        vertical_world_height: f64,
        near_distance: f32,
        far_distance: f32,
    ) -> Result<String, JsValue> {
        let camera = Camera::orthographic(
            [eye_x, eye_y, eye_z],
            [target_x, target_y, target_z],
            [up_x, up_y, up_z],
            vertical_world_height,
            near_distance,
            far_distance,
        )
        .map_err(camera_failure)?;
        self.set_camera(camera)
    }

    /// Selects one inherited presentation-only display mapping.
    #[wasm_bindgen(js_name = setDisplayMode)]
    pub fn set_display_mode(&mut self, mode: &str) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let mode = mode.parse::<DisplayMode>().map_err(|error| {
            failure(
                FailureCode::DisplayMode,
                error,
                "Choose neutral, elevation, rgb, intensity, or classification and retry.",
            )
        })?;
        let mut next = self.stream.clone();
        let updates = next
            .set_display_mode(mode)
            .map_err(stream_validation_failure)?;
        self.apply_updates(&updates)?;
        self.stream = next;
        self.reset_interaction_facts();
        self.diagnostics()
    }

    /// Replaces the complete presentation-only highlight set.
    #[wasm_bindgen(js_name = setHighlights)]
    pub fn set_highlights(
        &mut self,
        source_identity: &str,
        generation: u64,
        ordinals: &[u64],
    ) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let view_generation = self.require_active_generation(generation)?;
        let source = parse_source_identity(source_identity).map_err(highlight_failure)?;
        if Some(source) != self.active_source() {
            return Err(highlight_failure("highlight Source identity is not active"));
        }
        if ordinals.len() > usize::try_from(MAX_HIGHLIGHT_POINTS).unwrap_or(usize::MAX) {
            return Err(highlight_failure(format!(
                "highlight input contains {} Points above the {MAX_HIGHLIGHT_POINTS}-Point ceiling",
                ordinals.len()
            )));
        }
        if ordinals
            .iter()
            .enumerate()
            .any(|(index, ordinal)| ordinals[..index].contains(ordinal))
        {
            return Err(highlight_failure("highlight Point ordinals must be unique"));
        }
        let point_ids = ordinals
            .iter()
            .map(|ordinal| PointId::new(source, *ordinal))
            .collect::<Vec<_>>();
        self.resources_mut()?
            .apply_update(&RenderUpdate::SetHighlights {
                view_generation,
                point_ids,
            })?;
        self.highlights = HighlightFacts::complete(view_generation, source, ordinals.len())
            .map_err(highlight_failure)?;
        self.diagnostics()
    }

    /// Clears every presentation-only highlight for the active generation.
    #[wasm_bindgen(js_name = clearHighlights)]
    pub fn clear_highlights(&mut self, generation: u64) -> Result<String, JsValue> {
        let source = self
            .active_source()
            .ok_or_else(|| highlight_failure("the active View has no Source identity"))?;
        self.set_highlights(&source.to_string(), generation, &[])
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
        source_min_z: f64,
        source_max_z: f64,
        batch_index: u32,
        payload: &[u8],
    ) -> Result<String, JsValue> {
        self.ensure_source_publication()?;
        let mut next = self.stream.clone();
        let (reset, upsert) = next
            .begin_with_batch(
                source_identity,
                u64::from(expected_points),
                [origin_x, origin_y, origin_z],
                [source_min_z, source_max_z],
                batch_index,
                payload,
            )
            .map_err(stream_validation_failure)?;
        let resources = self.resources_mut()?;
        resources.apply_update(&reset)?;
        resources.apply_update(&upsert)?;
        self.stream = next;
        self.reset_interaction_facts();
        self.highlights = HighlightFacts::empty();
        self.diagnostics()
    }

    /// Publishes one bounded worker-decoded transfer batch.
    #[wasm_bindgen(js_name = publishStreamBatch)]
    pub fn publish_stream_batch(
        &mut self,
        batch_index: u32,
        payload: &[u8],
    ) -> Result<String, JsValue> {
        self.ensure_source_publication()?;
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
        self.ensure_source_publication()?;
        self.stream.complete().map_err(stream_validation_failure)?;
        self.resources_mut()?.cancel_frame_capture();
        self.diagnostics()
    }

    /// Applies one color-only batch weight for the private visual fixture harness.
    #[wasm_bindgen(js_name = setVisualBatchPresentation)]
    pub fn set_visual_batch_presentation(
        &mut self,
        batch_index: u32,
        weight_u8: u8,
    ) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let update = self
            .stream
            .visual_batch_presentation(batch_index, PresentationWeight::new(weight_u8))
            .map_err(stream_validation_failure)?;
        self.resources_mut()?.apply_update(&update)?;
        self.stream
            .commit_visual_batch_presentation(batch_index, PresentationWeight::new(weight_u8))
            .map_err(stream_validation_failure)?;
        self.reset_interaction_facts();
        self.diagnostics()
    }

    /// Conditionally removes one batch for the private visual fixture harness.
    #[wasm_bindgen(js_name = removeVisualBatch)]
    pub fn remove_visual_batch(&mut self, batch_index: u32) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let update = self
            .stream
            .visual_batch_removal(batch_index)
            .map_err(stream_validation_failure)?;
        self.resources_mut()?.apply_update(&update)?;
        self.stream
            .commit_visual_batch_removal(batch_index)
            .map_err(stream_validation_failure)?;
        self.reset_interaction_facts();
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
            schema: "punctra-browser-viewer-v1",
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
            capture_resources: self.resources.as_ref().map_or_else(
                CaptureResourceFacts::released,
                BrowserResources::capture_resource_facts,
            ),
            point_footprint: self.point_footprint_facts()?,
            frame: self.last_frame,
            pick: &self.pick,
            camera: CameraFacts::from_camera(self.camera),
            display_mode: self.stream.display_mode(),
            highlights: self.highlights,
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

    fn ensure_source_publication(&self) -> Result<(), JsValue> {
        self.lifecycle
            .ensure_source_publication()
            .map_err(model_failure)
    }

    fn resources_mut(&mut self) -> Result<&mut BrowserResources, JsValue> {
        self.resources
            .as_mut()
            .ok_or_else(|| model_failure(HostModelError::ViewerShutdown))
    }

    fn scene_frame(&self) -> Result<Frame, JsValue> {
        let viewport = renderer_viewport(self.viewport)?;
        let view_generation = self.stream.view_generation().unwrap_or(VIEW_GENERATION);
        PreparedScene::frame(
            viewport,
            view_generation,
            self.camera,
            self.display_size_physical_pixels(viewport),
        )
        .map_err(|error| failure(FailureCode::FrameValidation, error, RECREATE_ACTION))
    }

    fn display_size_physical_pixels(&self, viewport: Viewport) -> f32 {
        projected_density_display_size(viewport, self.non_retired_resident_point_count())
    }

    fn non_retired_resident_point_count(&self) -> u64 {
        if self.stream.view_generation().is_some() {
            self.stream.non_retired_resident_point_count()
        } else {
            self.scene.facts().point_count
        }
    }

    fn point_footprint_facts(&self) -> Result<PointFootprintFacts, JsValue> {
        let viewport = renderer_viewport(self.viewport)?;
        Ok(PointFootprintFacts::new(
            REQUESTED_POINT_FOOTPRINT,
            self.point_footprint_status,
            NOMINAL_PICK_SIZE_PHYSICAL_PIXELS,
            self.display_size_physical_pixels(viewport),
        ))
    }

    fn reset_interaction_facts(&mut self) {
        self.last_frame = None;
        self.pick = PickFacts::not_requested();
    }

    fn accept_pick(&mut self, hit: PickHit) -> Result<(), JsValue> {
        let active_generation = self.active_view_generation();
        let pick_belongs_to_active_content = self.stream.view_generation().is_some()
            || (hit.batch() == BATCH_KEY
                && hit.version() == BATCH_VERSION
                && hit.point() == centre_point_id());
        let pick_matches_active_view = hit.view_generation() == active_generation
            && Some(hit.point().source()) == self.active_source()
            && pick_belongs_to_active_content;
        if !pick_matches_active_view {
            return Err(failure(
                FailureCode::PickInvariant,
                "the provisional hit did not preserve the active generation, Source, batch, version, and Point identity",
                RECREATE_ACTION,
            ));
        }
        self.pick = PickFacts::hit(hit);
        Ok(())
    }

    fn set_camera(&mut self, camera: Camera) -> Result<String, JsValue> {
        self.ensure_ready()?;
        let viewport = renderer_viewport(self.viewport)?;
        PreparedScene::frame(
            viewport,
            self.active_view_generation(),
            camera,
            self.display_size_physical_pixels(viewport),
        )
        .map_err(camera_failure)?;
        self.camera = camera;
        self.reset_interaction_facts();
        self.resources_mut()?.discard_frame_dependent_state();
        self.diagnostics()
    }

    fn apply_updates(&mut self, updates: &[RenderUpdate]) -> Result<(), JsValue> {
        for update in updates {
            self.resources_mut()?.apply_update(update)?;
        }
        Ok(())
    }

    fn active_view_generation(&self) -> ViewGenerationKey {
        self.stream.view_generation().unwrap_or(VIEW_GENERATION)
    }

    fn active_source(&self) -> Option<render_protocol::SourceId> {
        self.stream
            .source()
            .or_else(|| Some(centre_point_id().source()))
    }

    fn require_active_generation(&self, generation: u64) -> Result<ViewGenerationKey, JsValue> {
        let active = self.active_view_generation();
        if active.generation() == generation {
            Ok(active)
        } else {
            Err(failure(
                FailureCode::StaleGeneration,
                format!(
                    "requested generation {generation} differs from active generation {}",
                    active.generation()
                ),
                "Discard stale interaction state and retry against the current viewer state.",
            ))
        }
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
    frame_capture: CaptureSlot<FrameCaptureTicket>,
    device_loss: DeviceLossState,
}

type DeviceLossState = Arc<Mutex<Option<String>>>;

struct FrameCaptureTicket {
    _target: wgpu::Texture,
    readback: wgpu::Buffer,
    submitted_work_done_receiver: mpsc::Receiver<Duration>,
    readback_mapping_receiver: mpsc::Receiver<(Duration, Result<(), wgpu::BufferAsyncError>)>,
    submitted_work_done: Option<Duration>,
    readback_mapping: Option<(Duration, Result<(), wgpu::BufferAsyncError>)>,
    layout: CaptureLayout,
}

impl FrameCaptureTicket {
    fn poll(&mut self) -> Result<FrameCapturePoll, JsValue> {
        self.poll_submitted_work_done()?;
        self.poll_readback_mapping()?;
        let Some(submitted_work_done) = self.submitted_work_done else {
            return Ok(FrameCapturePoll::Pending);
        };
        let Some((readback_mapping, result)) = self.readback_mapping.take() else {
            return Ok(FrameCapturePoll::Pending);
        };
        match result {
            Ok(()) => self
                .read_mapped_bytes()
                .map(|bytes| FrameCapturePoll::Ready {
                    bytes,
                    completion: CaptureCompletionFacts::new(submitted_work_done, readback_mapping),
                }),
            Err(error) => Err(frame_capture_readback_failure(error)),
        }
    }

    fn poll_submitted_work_done(&mut self) -> Result<(), JsValue> {
        if self.submitted_work_done.is_some() {
            return Ok(());
        }
        match self.submitted_work_done_receiver.try_recv() {
            Ok(completed) => self.submitted_work_done = Some(completed),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(frame_capture_readback_failure(
                    "the submitted-work completion callback ended without a result",
                ));
            }
        }
        Ok(())
    }

    fn poll_readback_mapping(&mut self) -> Result<(), JsValue> {
        if self.readback_mapping.is_some() {
            return Ok(());
        }
        match self.readback_mapping_receiver.try_recv() {
            Ok(completed) => self.readback_mapping = Some(completed),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(frame_capture_readback_failure(
                    "the frame-capture mapping callback ended without a result",
                ));
            }
        }
        Ok(())
    }

    fn read_mapped_bytes(&self) -> Result<Vec<u8>, JsValue> {
        let mapped = match self.readback.get_mapped_range(..) {
            Ok(mapped) => mapped,
            Err(error) => {
                self.readback.unmap();
                return Err(frame_capture_readback_failure(error));
            }
        };
        let rgba = self.layout.canonical_rgba(&mapped);
        drop(mapped);
        self.readback.unmap();
        rgba.map_err(frame_capture_readback_failure)
    }
}

enum FrameCapturePoll {
    Pending,
    Ready {
        bytes: Vec<u8>,
        completion: CaptureCompletionFacts,
    },
}

impl BrowserResources {
    fn capture_resource_facts(&self) -> CaptureResourceFacts {
        CaptureResourceFacts::from_pending(self.frame_capture.is_pending())
    }

    fn point_footprint_status(&self, viewport: Viewport) -> PointFootprintStatus {
        self.renderer.point_footprint_status(viewport)
    }

    fn capture_frame_facts(
        &self,
        frame: &Frame,
        report: FrameReport,
        batches: Vec<crate::streaming::VisualBatchFacts>,
    ) -> CaptureFrameFacts {
        let point_footprint = PointFootprintFacts::new(
            REQUESTED_POINT_FOOTPRINT,
            self.renderer.point_footprint_status(frame.viewport()),
            frame.style().default_size_pixels(),
            frame.style().display_size_pixels(),
        );
        CaptureFrameFacts::new(
            report.view_generation().generation(),
            report.drawn_points(),
            report.draw_calls(),
            report.resident_bytes(),
            report.transient_texture_bytes(),
            point_footprint,
            batches,
        )
    }

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
                frame_capture: CaptureSlot::idle(),
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
        self.discard_frame_dependent_state();
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
        // Capture commands own a separate color target and were submitted
        // before this presentation. Preserve that ticket across same-state
        // rerenders; queue ordering completes its copy before later work.
        self.discard_presented_frame_state();
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
        self.discard_frame_dependent_state();
        self.renderer
            .apply(update)
            .map(|_| ())
            .map_err(|error| failure(FailureCode::StreamPublication, error, RECREATE_ACTION))
    }

    fn begin_pick(&mut self, pixel: [u32; 2]) -> Result<u64, JsValue> {
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
        Ok(self.renderer.transient_texture_bytes())
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

    fn cancel_pick(&mut self) {
        self.pick_ticket = None;
    }

    fn begin_frame_capture(
        &mut self,
        frame: &Frame,
        batches: Vec<crate::streaming::VisualBatchFacts>,
    ) -> Result<String, JsValue> {
        self.ensure_device_available()?;
        if self.frame_capture.is_pending() {
            return Err(failure(
                FailureCode::FrameCapturePending,
                "a frame capture is already pending",
                "Poll the current frame capture to completion before beginning another one.",
            ));
        }
        let started_at = Instant::now();

        let dimensions = frame.viewport().dimensions();
        let layout = CaptureLayout::new(dimensions, self.surface_configuration.format)
            .map_err(frame_capture_validation_failure)?;
        self.validate_frame_capture_resources(layout)?;
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Punctra browser frame capture target"),
            size: capture_extent(dimensions),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: layout.texture_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Punctra browser frame capture readback"),
            size: layout.staging_bytes(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.encoder("Punctra browser frame capture");
        let recorded = self
            .renderer
            .render(&mut encoder, &target_view, frame)
            .map_err(|error| {
                failure(
                    FailureCode::FrameCaptureRecording,
                    error,
                    RETRY_CAPTURE_ACTION,
                )
            })?;
        let report = recorded.report();
        validate_transient_bytes(report.transient_texture_bytes())?;
        let facts = layout
            .pending_facts_json(self.capture_frame_facts(frame, report, batches))
            .map_err(|error| {
                failure(FailureCode::FrameCaptureFacts, error, RETRY_CAPTURE_ACTION)
            })?;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(layout.padded_bytes_per_row()),
                    rows_per_image: Some(dimensions[1]),
                },
            },
            capture_extent(dimensions),
        );
        let (mapping_sender, readback_mapping_receiver) = mpsc::channel();
        let mapping_started_at = started_at;
        encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
            let _ = mapping_sender.send((mapping_started_at.elapsed(), result));
        });
        let (submitted_sender, submitted_work_done_receiver) = mpsc::channel();
        encoder.on_submitted_work_done(move || {
            let _ = submitted_sender.send(started_at.elapsed());
        });
        self.queue.submit([encoder.finish()]);
        let ticket = FrameCaptureTicket {
            _target: target,
            readback,
            submitted_work_done_receiver,
            readback_mapping_receiver,
            submitted_work_done: None,
            readback_mapping: None,
            layout,
        };
        if self.frame_capture.begin(ticket).is_err() {
            return Err(failure(
                FailureCode::FrameCapturePending,
                "a frame capture became pending before ownership could be recorded",
                "Poll the current frame capture to completion before beginning another one.",
            ));
        }
        Ok(facts)
    }

    fn poll_frame_capture(&mut self) -> Result<Option<Vec<u8>>, JsValue> {
        if let Err(error) = self.ensure_device_available() {
            self.cancel_frame_capture();
            return Err(error);
        }
        if !self.frame_capture.is_pending() {
            return Err(failure(
                FailureCode::FrameCaptureNotRequested,
                "no frame capture is pending",
                "Begin a bounded frame capture before polling it.",
            ));
        }
        if let Err(error) = self.device.poll(wgpu::PollType::Poll) {
            self.cancel_frame_capture();
            return Err(failure(FailureCode::DevicePoll, error, RECREATE_ACTION));
        }
        let outcome = self
            .frame_capture
            .pending_mut()
            .expect("the capture ticket was checked above")
            .poll();
        match outcome {
            Ok(FrameCapturePoll::Pending) => Ok(None),
            Ok(FrameCapturePoll::Ready { bytes, completion }) => {
                if !self.frame_capture.complete(completion) {
                    return Err(frame_capture_readback_failure(
                        "the completed frame capture lost ownership before release",
                    ));
                }
                Ok(Some(bytes))
            }
            Err(error) => {
                self.frame_capture.cancel();
                Err(error)
            }
        }
    }

    fn cancel_frame_capture(&mut self) {
        self.frame_capture.cancel();
    }

    fn frame_capture_completion_facts(&self) -> Result<String, JsValue> {
        self.frame_capture
            .completion()
            .ok_or_else(|| {
                failure(
                    FailureCode::FrameCaptureFacts,
                    "no completed frame-capture callback timing is available",
                    "Complete one bounded frame capture before reading its callback timing.",
                )
            })?
            .to_json()
            .map_err(|error| failure(FailureCode::FrameCaptureFacts, error, RETRY_CAPTURE_ACTION))
    }

    fn validate_frame_capture_resources(&self, layout: CaptureLayout) -> Result<(), JsValue> {
        let dimensions = layout.dimensions();
        let limits = self.device.limits();
        if dimensions[0] > limits.max_texture_dimension_2d
            || dimensions[1] > limits.max_texture_dimension_2d
            || layout.staging_bytes() > limits.max_buffer_size
        {
            return Err(frame_capture_validation_failure(format!(
                "capture {:?} with a {}-byte staging buffer exceeds device limits of {} pixels and {} buffer bytes",
                dimensions,
                layout.staging_bytes(),
                limits.max_texture_dimension_2d,
                limits.max_buffer_size,
            )));
        }
        let required_usages =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let allowed_usages = layout
            .texture_format()
            .guaranteed_format_features(self.device.features())
            .allowed_usages;
        if !allowed_usages.contains(required_usages) {
            return Err(frame_capture_validation_failure(format!(
                "capture format {:?} does not guarantee RENDER_ATTACHMENT and COPY_SRC usage",
                layout.texture_format(),
            )));
        }
        Ok(())
    }

    fn discard_presented_frame_state(&mut self) {
        self.cancel_pick();
        self.recorded_frame = None;
    }

    fn discard_frame_dependent_state(&mut self) {
        self.discard_presented_frame_state();
        self.cancel_frame_capture();
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
    let config = RendererConfig::new(format, render_limits())
        .with_point_footprint(REQUESTED_POINT_FOOTPRINT);
    WgpuRenderer::new(device, config).map_err(|error| {
        failure(
            FailureCode::RendererCapability,
            error,
            INITIALIZATION_ACTION,
        )
    })
}

fn renderer_viewport(viewport: PhysicalViewport) -> Result<Viewport, JsValue> {
    let dimensions = viewport.dimensions();
    Viewport::new(dimensions[0], dimensions[1]).map_err(|error| {
        failure(
            FailureCode::ViewportValidation,
            error,
            "Choose a nonzero bounded canvas size.",
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

const fn capture_extent(dimensions: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: dimensions[0],
        height: dimensions[1],
        depth_or_array_layers: 1,
    }
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

fn frame_capture_validation_failure(error: impl std::fmt::Display) -> JsValue {
    failure(
        FailureCode::FrameCaptureValidation,
        error,
        "Keep the last presented frame and retry with the current bounded four-byte canvas format and dimensions.",
    )
}

fn frame_capture_readback_failure(error: impl std::fmt::Display) -> JsValue {
    failure(
        FailureCode::FrameCaptureReadback,
        error,
        RETRY_CAPTURE_ACTION,
    )
}

fn stream_validation_failure(error: crate::streaming::StreamError) -> JsValue {
    failure(
        FailureCode::StreamValidation,
        error,
        "Terminate the current worker operation, keep the last complete frame, and start a new identity-bound stream.",
    )
}

fn camera_failure(error: impl std::fmt::Display) -> JsValue {
    failure(
        FailureCode::CameraValidation,
        error,
        "Keep the current frame, provide one finite nondegenerate camera and valid clipping range, then retry.",
    )
}

fn highlight_failure(error: impl std::fmt::Display) -> JsValue {
    failure(
        FailureCode::HighlightValidation,
        error,
        "Keep the current frame and retry with a unique bounded Point set from the active Source generation.",
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
