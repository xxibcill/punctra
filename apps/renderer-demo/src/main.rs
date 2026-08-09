//! Interactive, progressively streamed synthetic point-cloud renderer demo.

mod orbit_camera;
mod synthetic;

use std::{
    error::Error,
    io,
    sync::Arc,
    time::{Duration, Instant},
};

use orbit_camera::OrbitCamera;
use point_view::{AvailableNodes, PlannerConfig, PlanningBudget, ResourceUsage, ViewPlanner};
use render_protocol::{RenderLimits, RenderUpdate, UpdateReport, ViewGenerationKey, ViewId};
use render_wgpu::{Camera, Frame, FrameReport, PointStyle, RendererConfig, WgpuRenderer};
use synthetic::{
    LOGICAL_POINT_COUNT, RESIDENT_BATCH_BUDGET, RESIDENT_BYTE_BUDGET, RESIDENT_POINT_BUDGET,
    SCENE_RADIUS, SCENE_TARGET, SyntheticScene,
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

const BASE_TITLE: &str = "Punctra adaptive View v0.2";
const INITIAL_WIDTH: f64 = 1_280.0;
const INITIAL_HEIGHT: f64 = 800.0;
const TITLE_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const VIEW_GENERATION: ViewGenerationKey = ViewGenerationKey::new(ViewId::new(1), 1);
const PLANNING_BUDGET: PlanningBudget = PlanningBudget::new(
    RESIDENT_POINT_BUDGET,
    RESIDENT_BYTE_BUDGET,
    RESIDENT_BATCH_BUDGET,
);

type DemoResult<T> = Result<T, Box<dyn Error>>;

fn main() -> DemoResult<()> {
    println!(
        "Punctra adaptive View demo (16.7M logical Points, fixed residency)\n\
         Left drag: orbit | Wheel: zoom | R: reset view | H: highlights | \
         Space: pause loads | Escape: quit"
    );

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = DemoApp::default();
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[derive(Default)]
struct DemoApp {
    graphics: Option<Graphics>,
    failed: bool,
}

impl DemoApp {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> DemoResult<()> {
        let attributes = Window::default_attributes()
            .with_title(BASE_TITLE)
            .with_inner_size(LogicalSize::new(INITIAL_WIDTH, INITIAL_HEIGHT))
            .with_visible(false);
        let window = Arc::new(event_loop.create_window(attributes)?);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let graphics = pollster::block_on(Graphics::new(instance, window))?;
        graphics.window.set_visible(true);
        graphics.window.request_redraw();
        self.graphics = Some(graphics);
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: &dyn Error) {
        eprintln!("renderer demo stopped: {error}");
        self.failed = true;
        self.graphics = None;
        event_loop.exit();
    }

    fn handle_redraw(&mut self, event_loop: &ActiveEventLoop) {
        let result = self
            .graphics
            .as_mut()
            .expect("redraw events are filtered to the demo window")
            .redraw();
        if let Err(error) = result {
            self.fail(event_loop, error.as_ref());
            return;
        }

        let graphics = self
            .graphics
            .as_ref()
            .expect("successful redraw keeps graphics state alive");
        if graphics.should_request_redraw() {
            graphics.window.request_redraw();
        }
    }

    fn handle_keyboard_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: &winit::event::KeyEvent,
    ) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        if code == KeyCode::Escape {
            event_loop.exit();
            return;
        }

        let result = self
            .graphics
            .as_mut()
            .expect("keyboard events are filtered to the demo window")
            .handle_key(code);
        if let Err(error) = result {
            self.fail(event_loop, error.as_ref());
        }
    }

    fn is_demo_window(&self, window_id: WindowId) -> bool {
        self.graphics
            .as_ref()
            .is_some_and(|graphics| graphics.window.id() == window_id)
    }
}

impl ApplicationHandler for DemoApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_some() || self.failed {
            return;
        }
        if let Err(error) = self.initialize(event_loop) {
            self.fail(event_loop, error.as_ref());
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if !self.is_demo_window(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.handle_redraw(event_loop),
            WindowEvent::Resized(size) => {
                self.graphics
                    .as_mut()
                    .expect("resize events are filtered to the demo window")
                    .resize(size);
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let graphics = self
                    .graphics
                    .as_mut()
                    .expect("scale events are filtered to the demo window");
                graphics.resize(graphics.window.inner_size());
            }
            WindowEvent::Occluded(occluded) => {
                let graphics = self
                    .graphics
                    .as_mut()
                    .expect("occlusion events are filtered to the demo window");
                graphics.set_occluded(occluded);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.handle_keyboard_input(event_loop, &event);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                self.graphics
                    .as_mut()
                    .expect("mouse events are filtered to the demo window")
                    .handle_mouse_button(state, button);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.graphics
                    .as_mut()
                    .expect("cursor events are filtered to the demo window")
                    .handle_cursor_moved(position);
            }
            WindowEvent::CursorLeft { .. } => {
                self.graphics
                    .as_mut()
                    .expect("cursor events are filtered to the demo window")
                    .clear_cursor_position();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.graphics
                    .as_mut()
                    .expect("wheel events are filtered to the demo window")
                    .handle_mouse_wheel(delta);
            }
            _ => {}
        }
    }
}

struct Graphics {
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_config: wgpu::SurfaceConfiguration,
    presentation: PresentationState,
    renderer: WgpuRenderer,
    planner: ViewPlanner,
    scene: SyntheticScene,
    camera: OrbitCamera,
    style: PointStyle,
    input: PointerInput,
    loads_paused: bool,
    highlights_enabled: bool,
    metrics: Metrics,
}

impl Graphics {
    async fn new(instance: wgpu::Instance, window: Arc<Window>) -> DemoResult<Self> {
        let surface = instance.create_surface(Arc::clone(&window))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await?;
        let adapter_info = adapter.get_info();
        println!(
            "GPU: {} ({:?}, {:?})",
            adapter_info.name, adapter_info.backend, adapter_info.device_type
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("punctra renderer demo device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await?;

        let size = window.inner_size();
        let surface_configured = has_area(size);
        let surface_config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| io::Error::other("the selected adapter cannot present to the window"))?;
        if surface_configured {
            surface.configure(&device, &surface_config);
        }

        let limits = RenderLimits::new(
            RESIDENT_BYTE_BUDGET,
            RESIDENT_POINT_BUDGET,
            RESIDENT_BATCH_BUDGET,
        );
        let mut renderer =
            WgpuRenderer::new(&device, RendererConfig::new(surface_config.format, limits))?;
        let reset = RenderUpdate::Reset {
            view_generation: VIEW_GENERATION,
        };
        renderer.apply(&reset)?;
        let planner = ViewPlanner::new(PlannerConfig::new(2.0, 0.25)?);
        let scene = SyntheticScene::new(VIEW_GENERATION)?;
        let style = PointStyle::new(2.4, [1.0, 0.24, 0.06], [0.008, 0.012, 0.02, 1.0])?;

        Ok(Self {
            surface,
            window,
            instance,
            device,
            queue,
            surface_config,
            presentation: PresentationState::new(surface_configured),
            renderer,
            planner,
            scene,
            camera: OrbitCamera::new(SCENE_TARGET, SCENE_RADIUS),
            style,
            input: PointerInput::default(),
            loads_paused: false,
            highlights_enabled: false,
            metrics: Metrics::new(),
        })
    }

    fn redraw(&mut self) -> DemoResult<()> {
        if !self.presentation.is_drawable() {
            return Ok(());
        }

        let frame_started = Instant::now();
        let viewport = [self.surface_config.width, self.surface_config.height];
        let camera = self.camera.as_render_camera()?;
        self.stream_next_batch()?;
        self.plan_view(&camera, viewport)?;
        let Some((surface_texture, reconfigure_after_present)) = self.acquire_surface_texture()?
        else {
            return Ok(());
        };
        let target = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("punctra renderer demo frame"),
            });
        let frame = Frame::new(VIEW_GENERATION, camera, viewport)?.with_style(self.style);
        let recorded_frame = self.renderer.render(&mut encoder, &target, &frame)?;
        self.queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        self.queue.present(surface_texture);
        if reconfigure_after_present {
            self.configure_surface();
        }

        self.metrics
            .record_frame(recorded_frame.report(), frame_started.elapsed());
        self.metrics.update_title(
            &self.window,
            self.scene.resident_batches(),
            self.scene.pending_batches(),
            self.loads_paused,
            self.highlights_enabled,
        );
        Ok(())
    }

    fn acquire_surface_texture(&mut self) -> DemoResult<Option<(wgpu::SurfaceTexture, bool)>> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => Ok(Some((texture, false))),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok(Some((texture, true))),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                Ok(None)
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.configure_surface();
                Ok(None)
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface()?;
                Ok(None)
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                Err(io::Error::other("surface acquisition failed validation").into())
            }
        }
    }

    fn stream_next_batch(&mut self) -> DemoResult<()> {
        if self.loads_paused {
            return Ok(());
        }
        let Some(batch) = self.scene.next_batch()? else {
            return Ok(());
        };

        let batch_key = batch.key();
        let batch_version = batch.version();
        let update = RenderUpdate::Upsert { batch };
        let upload_started = Instant::now();
        let report = self.renderer.apply(&update)?;
        self.scene.mark_resident(batch_key, batch_version);
        self.metrics.record_upload(report, upload_started.elapsed());
        Ok(())
    }

    fn plan_view(&mut self, camera: &Camera, viewport: [u32; 2]) -> DemoResult<()> {
        let planning_nodes = self.scene.planning_nodes();
        let plan = self.planner.plan(
            camera,
            viewport,
            AvailableNodes::new(VIEW_GENERATION, &planning_nodes),
            PLANNING_BUDGET,
        )?;

        for retirement in plan.retirements().iter().copied() {
            let update = retirement.render_update();
            self.renderer.apply(&update)?;
            self.scene
                .mark_retired(retirement.batch_key(), retirement.expected_version());
        }
        if !self.loads_paused {
            self.scene.enqueue_requests(plan.requests());
        }
        self.metrics.record_plan(
            u64::try_from(plan.requests().len()).expect("the request count fits in u64"),
            plan.resource_usage(),
        );
        Ok(())
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.presentation.configured = has_area(size);
        if !self.presentation.configured {
            return;
        }

        self.surface_config.width = size.width;
        self.surface_config.height = size.height;
        self.configure_surface();
        self.window.request_redraw();
    }

    fn configure_surface(&self) {
        if self.presentation.configured {
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    fn recreate_surface(&mut self) -> DemoResult<()> {
        let surface = self.instance.create_surface(Arc::clone(&self.window))?;
        if self.presentation.configured {
            surface.configure(&self.device, &self.surface_config);
        }
        self.surface = surface;
        Ok(())
    }

    fn set_occluded(&mut self, occluded: bool) {
        self.presentation.occluded = occluded;
        if !occluded {
            self.window.request_redraw();
        }
    }

    fn should_request_redraw(&self) -> bool {
        self.presentation.is_drawable()
    }

    fn handle_key(&mut self, code: KeyCode) -> DemoResult<()> {
        match code {
            KeyCode::KeyR => self.camera.reset(SCENE_RADIUS),
            KeyCode::KeyH => self.toggle_highlights()?,
            KeyCode::Space => self.loads_paused = !self.loads_paused,
            _ => return Ok(()),
        }
        self.window.request_redraw();
        Ok(())
    }

    fn toggle_highlights(&mut self) -> DemoResult<()> {
        self.highlights_enabled = !self.highlights_enabled;
        let point_ids = if self.highlights_enabled {
            SyntheticScene::highlight_ids()
        } else {
            Vec::new()
        };
        let update = RenderUpdate::SetHighlights {
            view_generation: VIEW_GENERATION,
            point_ids,
        };
        self.renderer.apply(&update)?;
        Ok(())
    }

    fn handle_mouse_button(&mut self, state: ElementState, button: MouseButton) {
        if button != MouseButton::Left {
            return;
        }
        self.input.dragging = state == ElementState::Pressed;
        self.input.last_cursor = None;
    }

    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if self.input.dragging {
            if let Some(previous) = self.input.last_cursor {
                self.camera
                    .orbit(position.x - previous.x, position.y - previous.y);
                self.window.request_redraw();
            }
            self.input.last_cursor = Some(position);
        }
    }

    fn clear_cursor_position(&mut self) {
        self.input.last_cursor = None;
    }

    fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, vertical) => f64::from(vertical),
            MouseScrollDelta::PixelDelta(position) => position.y / 80.0,
        };
        self.camera.zoom(lines);
        self.window.request_redraw();
    }
}

#[derive(Default)]
struct PointerInput {
    dragging: bool,
    last_cursor: Option<PhysicalPosition<f64>>,
}

struct PresentationState {
    configured: bool,
    occluded: bool,
}

impl PresentationState {
    const fn new(configured: bool) -> Self {
        Self {
            configured,
            occluded: false,
        }
    }

    const fn is_drawable(&self) -> bool {
        self.configured && !self.occluded
    }
}

struct Metrics {
    interval_started: Instant,
    interval_frames: u32,
    total_uploaded_bytes: u64,
    latest_upload_time: Duration,
    latest_frame_time: Duration,
    latest_report: Option<FrameReport>,
    latest_plan_requests: u64,
    latest_plan_usage: ResourceUsage,
}

impl Metrics {
    fn new() -> Self {
        Self {
            interval_started: Instant::now(),
            interval_frames: 0,
            total_uploaded_bytes: 0,
            latest_upload_time: Duration::ZERO,
            latest_frame_time: Duration::ZERO,
            latest_report: None,
            latest_plan_requests: 0,
            latest_plan_usage: ResourceUsage::default(),
        }
    }

    fn record_upload(&mut self, report: UpdateReport, elapsed: Duration) {
        self.total_uploaded_bytes = self
            .total_uploaded_bytes
            .saturating_add(report.uploaded_bytes());
        self.latest_upload_time = elapsed;
    }

    fn record_frame(&mut self, report: FrameReport, elapsed: Duration) {
        self.interval_frames = self.interval_frames.saturating_add(1);
        self.latest_frame_time = elapsed;
        self.latest_report = Some(report);
    }

    fn record_plan(&mut self, requests: u64, usage: ResourceUsage) {
        self.latest_plan_requests = requests;
        self.latest_plan_usage = usage;
    }

    fn update_title(
        &mut self,
        window: &Window,
        resident_batches: u64,
        pending_batches: u64,
        loads_paused: bool,
        highlights_enabled: bool,
    ) {
        let interval = self.interval_started.elapsed();
        if interval < TITLE_REFRESH_INTERVAL {
            return;
        }
        let Some(report) = self.latest_report else {
            return;
        };

        let frames_per_second = f64::from(self.interval_frames) / interval.as_secs_f64();
        let stream_state = if loads_paused {
            "loads-paused"
        } else if pending_batches > 0 || self.latest_plan_requests > 0 {
            "streaming"
        } else {
            "steady"
        };
        let highlight_state = if highlights_enabled { "on" } else { "off" };
        let title = format!(
            "{BASE_TITLE} | {} logical | {} / {} pts | {} MiB resident | {} MiB uploaded | \
             {} draws | \
             {:.0} fps | frame {:.2} ms | encode {:.2} ms | upload {:.2} ms | \
             {} req | {} planned | {pending_batches} queued | \
             {resident_batches} batches {stream_state} | H:{highlight_state}",
            compact_count(LOGICAL_POINT_COUNT),
            compact_count(report.drawn_points()),
            compact_count(RESIDENT_POINT_BUDGET),
            mebibytes(report.resident_bytes()),
            mebibytes(self.total_uploaded_bytes),
            report.draw_calls(),
            frames_per_second,
            duration_milliseconds(self.latest_frame_time),
            duration_milliseconds(report.encoding_time()),
            duration_milliseconds(self.latest_upload_time),
            self.latest_plan_requests,
            self.latest_plan_usage.batch_count(),
        );
        window.set_title(&title);
        self.interval_started = Instant::now();
        self.interval_frames = 0;
    }
}

fn has_area(size: PhysicalSize<u32>) -> bool {
    size.width > 0 && size.height > 0
}

fn duration_milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn mebibytes(bytes: u64) -> String {
    const MEBIBYTE: u64 = 1_024 * 1_024;
    let tenths = bytes.saturating_mul(10) / MEBIBYTE;
    format!("{}.{:01}", tenths / 10, tenths % 10)
}

fn compact_count(count: u64) -> String {
    const MILLION: u64 = 1_000_000;
    const THOUSAND: u64 = 1_000;
    if count >= MILLION {
        let tenths = count.saturating_mul(10) / MILLION;
        format!("{}.{:01}M", tenths / 10, tenths % 10)
    } else if count >= THOUSAND {
        let tenths = count.saturating_mul(10) / THOUSAND;
        format!("{}.{:01}K", tenths / 10, tenths % 10)
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_format_counts_without_losing_integer_precision() {
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(12_345), "12.3K");
        assert_eq!(compact_count(1_234_567), "1.2M");
        assert_eq!(mebibytes(3 * 1_024 * 1_024 + 512 * 1_024), "3.5");
    }
}
