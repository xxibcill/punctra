//! Demo-level planner-to-renderer GPU acceptance on an available headless adapter.

use std::{env, sync::OnceLock};

use point_view::{
    AvailableNode, AvailableNodes, AxisAlignedBox, NodeKey, NodeRequest, NodeStatus, PlannerConfig,
    PlanningBudget, ViewPlan, ViewPlanner,
};
use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT as POINT_BYTES, PointBatch, PointId,
    ProtocolError, RenderLimits, RenderPoint, RenderUpdate, UpdateKind, ViewGenerationKey, ViewId,
};
use render_wgpu::{Camera, Frame, FrameReport, RendererConfig, RendererError, WgpuRenderer};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const VIEWPORT: [u32; 2] = [64, 64];
const WORLD_ORIGIN: [f64; 3] = [0.0; 3];
const PARENT_KEY: u64 = 1;
const LEFT_CHILD_KEY: u64 = 2;
const RIGHT_CHILD_KEY: u64 = 3;
const TRANSITION_POINTS: u64 = 3;
const TRANSITION_BYTES: u64 = TRANSITION_POINTS * POINT_BYTES;
const TRANSITION_BATCHES: u64 = 3;

static GPU: OnceLock<Option<GpuContext>> = OnceLock::new();

#[test]
fn adaptive_plan_keeps_coverage_and_retires_exact_gpu_batches() {
    with_gpu(assert_adaptive_plan_transition);
}

fn assert_adaptive_plan_transition(gpu: &GpuContext) {
    let mut subject = PlannerRenderer::new(gpu);
    subject.start_with_parent();
    let initial_nodes = hierarchy(
        NodeStatus::Resident {
            version: BatchVersion::new(1),
        },
        NodeStatus::Missing,
        NodeStatus::Missing,
    );
    let loading_plan = subject.plan(&initial_nodes);
    assert_loading_plan(&loading_plan, subject.budget);
    subject.materialize_requests(loading_plan.requests());
    let resident_nodes = hierarchy(
        NodeStatus::Resident {
            version: BatchVersion::new(1),
        },
        NodeStatus::Resident {
            version: BatchVersion::new(1),
        },
        NodeStatus::Resident {
            version: BatchVersion::new(1),
        },
    );
    let parent_removal =
        exact_parent_removal(&subject.plan(&resident_nodes), subject.view_generation);
    subject.apply_parent_retirement(&parent_removal);
    assert_children_only(subject.render());
    subject.assert_newer_parent_survives(&parent_removal);
}

fn assert_loading_plan(plan: &ViewPlan, budget: PlanningBudget) {
    assert_eq!(request_keys(plan), vec![LEFT_CHILD_KEY, RIGHT_CHILD_KEY]);
    assert_eq!(retained_keys(plan), vec![PARENT_KEY]);
    assert!(plan.retirements().is_empty());
    assert_eq!(plan.resource_usage().point_count(), TRANSITION_POINTS);
    assert_eq!(plan.resource_usage().estimated_bytes(), TRANSITION_BYTES);
    assert_eq!(plan.resource_usage().batch_count(), TRANSITION_BATCHES);
    assert!(plan.resource_usage().fits_within(budget));
}

fn exact_parent_removal(plan: &ViewPlan, view_generation: ViewGenerationKey) -> RenderUpdate {
    assert!(plan.requests().is_empty());
    assert_eq!(retained_keys(plan), vec![LEFT_CHILD_KEY, RIGHT_CHILD_KEY]);
    let [parent_retirement] = plan.retirements() else {
        panic!("the covered parent should have one exact retirement");
    };
    assert_eq!(parent_retirement.view_generation(), view_generation);
    assert_eq!(parent_retirement.batch_key(), BatchKey::new(PARENT_KEY));
    assert_eq!(parent_retirement.expected_version(), BatchVersion::new(1));
    parent_retirement.render_update()
}

fn assert_children_only(report: FrameReport) {
    assert_eq!(report.drawn_points(), 2);
    assert_eq!(report.draw_calls(), 2);
    assert_eq!(report.resident_bytes(), 2 * POINT_BYTES);
}

struct PlannerRenderer<'gpu> {
    gpu: &'gpu GpuContext,
    view_generation: ViewGenerationKey,
    budget: PlanningBudget,
    camera: Camera,
    planner: ViewPlanner,
    renderer: WgpuRenderer,
}

impl<'gpu> PlannerRenderer<'gpu> {
    fn new(gpu: &'gpu GpuContext) -> Self {
        let limits = RenderLimits::new(TRANSITION_BYTES, TRANSITION_POINTS, TRANSITION_BATCHES);
        let config =
            PlannerConfig::new(2.0, 0.25).expect("the acceptance thresholds should be valid");
        let renderer = WgpuRenderer::new(&gpu.device, RendererConfig::new(FORMAT, limits))
            .expect("the renderer should attach to the test device");
        Self {
            gpu,
            view_generation: ViewGenerationKey::new(ViewId::new(91), 1),
            budget: PlanningBudget::new(TRANSITION_POINTS, TRANSITION_BYTES, TRANSITION_BATCHES),
            camera: fixture_camera(),
            planner: ViewPlanner::new(config),
            renderer,
        }
    }

    fn start_with_parent(&mut self) {
        self.renderer
            .apply(&RenderUpdate::Reset {
                view_generation: self.view_generation,
            })
            .expect("the test generation should begin");
        self.renderer
            .apply(&RenderUpdate::Upsert {
                batch: point_batch(self.view_generation, PARENT_KEY, 1, [0.0; 3], 1),
            })
            .expect("the coarse parent should become resident");
    }

    fn plan(&mut self, nodes: &[AvailableNode]) -> ViewPlan {
        self.planner
            .plan(
                &self.camera,
                VIEWPORT,
                AvailableNodes::new(self.view_generation, nodes),
                self.budget,
            )
            .expect("the acceptance plan should succeed")
    }

    fn materialize_requests(&mut self, requests: &[NodeRequest]) {
        for request in requests {
            let key = request.node().get();
            self.renderer
                .apply(&RenderUpdate::Upsert {
                    batch: point_batch(
                        self.view_generation,
                        request.batch_key().get(),
                        1,
                        child_position(key),
                        key,
                    ),
                })
                .expect("each planned child should fit the matching renderer limits");
        }
    }

    fn apply_parent_retirement(&mut self, parent_removal: &RenderUpdate) {
        let removed = self
            .renderer
            .apply(parent_removal)
            .expect("the exact parent retirement should be accepted");
        assert_eq!(removed.kind(), UpdateKind::BatchRemoved);
        assert_eq!(removed.resident().point_count(), 2);
        assert_eq!(removed.resident().estimated_gpu_bytes(), 2 * POINT_BYTES);
        assert_eq!(removed.resident().batch_count(), 2);
    }

    fn render(&mut self) -> FrameReport {
        let frame = Frame::new(self.view_generation, self.camera, VIEWPORT)
            .expect("the shared planner camera should create a renderer frame");
        render_and_submit(self.gpu, &mut self.renderer, &frame)
    }

    fn assert_newer_parent_survives(&mut self, stale_removal: &RenderUpdate) {
        self.renderer
            .apply(&RenderUpdate::Upsert {
                batch: point_batch(self.view_generation, PARENT_KEY, 2, [0.0; 3], 4),
            })
            .expect("a newer parent replacement should fit the transition budget");
        assert!(matches!(
            self.renderer.apply(stale_removal),
            Err(RendererError::Protocol(
                ProtocolError::BatchVersionMismatch {
                    key,
                    resident,
                    expected,
                }
            )) if key == BatchKey::new(PARENT_KEY)
                && resident == BatchVersion::new(2)
                && expected == BatchVersion::new(1)
        ));

        let newer_parent_preserved = self.render();
        assert_eq!(newer_parent_preserved.drawn_points(), TRANSITION_POINTS);
        assert_eq!(newer_parent_preserved.draw_calls(), TRANSITION_BATCHES);
        assert_eq!(newer_parent_preserved.resident_bytes(), TRANSITION_BYTES);
    }
}

fn child_position(key: u64) -> [f32; 3] {
    match key {
        LEFT_CHILD_KEY => [-0.5, 0.0, 0.0],
        RIGHT_CHILD_KEY => [0.5, 0.0, 0.0],
        _ => panic!("only fixture children should be requested"),
    }
}

fn hierarchy(
    parent_status: NodeStatus,
    left_status: NodeStatus,
    right_status: NodeStatus,
) -> [AvailableNode; 3] {
    [
        available_node(
            RIGHT_CHILD_KEY,
            Some(PARENT_KEY),
            [0.0, -0.25, -1.0],
            [1.0, 0.25, 1.0],
            0.0,
            right_status,
        ),
        available_node(
            PARENT_KEY,
            None,
            [-1.0, -0.25, -1.0],
            [1.0, 0.25, 1.0],
            2.0,
            parent_status,
        ),
        available_node(
            LEFT_CHILD_KEY,
            Some(PARENT_KEY),
            [-1.0, -0.25, -1.0],
            [0.0, 0.25, 1.0],
            0.0,
            left_status,
        ),
    ]
}

fn available_node(
    key: u64,
    parent: Option<u64>,
    min: [f64; 3],
    max: [f64; 3],
    geometric_error: f64,
    status: NodeStatus,
) -> AvailableNode {
    AvailableNode::new(
        node_key(key),
        parent.map(node_key),
        AxisAlignedBox::new(min, max).expect("the acceptance bounds should be valid"),
        geometric_error,
        1,
        POINT_BYTES,
        BatchKey::new(key),
        status,
    )
    .expect("the acceptance hierarchy node should be valid")
}

fn node_key(value: u64) -> NodeKey {
    NodeKey::new(value).expect("acceptance node keys should be nonzero")
}

fn request_keys(plan: &ViewPlan) -> Vec<u64> {
    plan.requests()
        .iter()
        .map(|request| request.node().get())
        .collect()
}

fn retained_keys(plan: &ViewPlan) -> Vec<u64> {
    plan.retained_nodes()
        .iter()
        .map(|retained| retained.node_key().get())
        .collect()
}

fn fixture_camera() -> Camera {
    Camera::perspective(
        [0.0, -5.0, 0.0],
        WORLD_ORIGIN,
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_3,
        0.1,
        100.0,
    )
    .expect("the acceptance camera should be valid")
}

fn point_batch(
    view_generation: ViewGenerationKey,
    key: u64,
    version: u64,
    position: [f32; 3],
    point_id: u64,
) -> PointBatch {
    let point = RenderPoint::new(position, [80, 180, 255, 255], PointId::new(point_id))
        .expect("the acceptance point should be valid");
    PointBatch::new(
        view_generation,
        BatchKey::new(key),
        BatchVersion::new(version),
        WORLD_ORIGIN,
        vec![point],
    )
    .expect("the acceptance batch should be valid")
}

fn render_and_submit(gpu: &GpuContext, renderer: &mut WgpuRenderer, frame: &Frame) -> FrameReport {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("punctra planner acceptance target"),
        size: wgpu::Extent3d {
            width: VIEWPORT[0],
            height: VIEWPORT[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("punctra planner acceptance encoder"),
        });
    let recorded = renderer
        .render(&mut encoder, &target, frame)
        .expect("the planned child batches should encode");
    let report = recorded.report();
    gpu.queue.submit([encoder.finish()]);
    gpu.wait();
    report
}

struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl GpuContext {
    fn wait(&self) {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("headless device polling should succeed");
    }
}

fn with_gpu(test: impl FnOnce(&GpuContext)) {
    if let Some(gpu) = GPU.get_or_init(initialize_gpu).as_ref() {
        test(gpu);
    }
}

fn initialize_gpu() -> Option<GpuContext> {
    pollster::block_on(request_gpu())
}

async fn request_gpu() -> Option<GpuContext> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::None,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await;
    let adapter = match adapter {
        Ok(adapter) => adapter,
        Err(error) if gpu_is_required() => {
            panic!("PUNCTRA_REQUIRE_GPU=1 but no headless adapter is available: {error}");
        }
        Err(error) => {
            eprintln!(
                "skipping GPU acceptance tests because no headless adapter is available: {error}"
            );
            return None;
        }
    };
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("punctra planner acceptance test device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })
        .await
        .expect("a discovered headless adapter should provide a baseline device");
    Some(GpuContext { device, queue })
}

fn gpu_is_required() -> bool {
    env::var("PUNCTRA_REQUIRE_GPU").is_ok_and(|value| value == "1")
}
