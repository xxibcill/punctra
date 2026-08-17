//! Minimal offscreen renderer integration using only public crate APIs.
//!
//! This host owns the complete wgpu lifecycle. Its GPU pick is deliberately
//! reported as provisional; an editing application must confirm the returned
//! identity with `point_review::confirm_pick` and a pinned
//! `point_workspace::Snapshot` before acting on it. This renderer-only example
//! deliberately publishes no exact highlight or Edit from the provisional hit.

use std::error::Error;

use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch, PointId, RenderLimits,
    RenderPoint, RenderUpdate, SourceId, ViewGenerationKey, ViewId, Viewport,
};
use render_wgpu::{
    Camera, Frame, PickHit, PickPoll, PickRequest, PointStyle, RendererConfig, WgpuRenderer,
};

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const TARGET_SIZE: u32 = 64;
const PICK_PIXEL: [u32; 2] = [TARGET_SIZE / 2, TARGET_SIZE / 2];
const VIEW_GENERATION: ViewGenerationKey = ViewGenerationKey::new(ViewId::new(1), 1);
const DISPLAY_POINT: PointId = PointId::new(SourceId::new([0x42; 32]), 7);

fn main() -> Result<(), Box<dyn Error>> {
    pollster::block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let Some(host) = HostGpu::request().await? else {
        if gpu_is_required() {
            return Err("PUNCTRA_REQUIRE_GPU=1 but no headless GPU adapter is available".into());
        }
        eprintln!("no headless GPU adapter is available; the integration example was not run");
        return Ok(());
    };

    let target = HostTarget::new(&host.device);
    let mut renderer = create_bounded_renderer(&host.device)?;
    publish_display_point(&mut renderer)?;

    let frame = inspection_frame()?;
    let mut encoder = host.encoder();
    let recorded = renderer.render(&mut encoder, &target.view, &frame)?;
    let mut pick = renderer.pick(&mut encoder, &recorded, PickRequest::new(PICK_PIXEL))?;

    host.queue.submit([encoder.finish()]);
    host.device.poll(wgpu::PollType::wait_indefinitely())?;

    let PickPoll::Ready(hit) = pick.poll()? else {
        return Err("a fully polled device left the pick pending".into());
    };
    report_provisional_hit(hit)?;
    Ok(())
}

fn create_bounded_renderer(device: &wgpu::Device) -> Result<WgpuRenderer, Box<dyn Error>> {
    let limits =
        RenderLimits::new(ESTIMATED_GPU_BYTES_PER_POINT, 1, 1).with_max_highlight_points(1);
    let config = RendererConfig::new(TARGET_FORMAT, limits);
    Ok(WgpuRenderer::new(device, config)?)
}

fn publish_display_point(renderer: &mut WgpuRenderer) -> Result<(), Box<dyn Error>> {
    renderer.apply(&RenderUpdate::Reset {
        view_generation: VIEW_GENERATION,
    })?;
    let point = RenderPoint::new([0.0; 3], [80, 180, 255, 255], DISPLAY_POINT)?;
    let batch = PointBatch::new(
        VIEW_GENERATION,
        BatchKey::new(1),
        BatchVersion::new(1),
        [0.0; 3],
        vec![point],
    )?;
    renderer.apply(&RenderUpdate::Upsert { batch })?;
    Ok(())
}

fn inspection_frame() -> Result<Frame, Box<dyn Error>> {
    let camera = Camera::perspective(
        [0.0, -5.0, 0.0],
        [0.0; 3],
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_3,
        0.1,
        100.0,
    )?;
    let viewport = Viewport::new(TARGET_SIZE, TARGET_SIZE)?;
    let style = PointStyle::new(16.0, [1.0, 0.8, 0.1], [0.0, 0.0, 0.0, 1.0])?;
    Ok(Frame::new(VIEW_GENERATION, camera, viewport)?.with_style(style))
}

fn report_provisional_hit(hit: Option<PickHit>) -> Result<(), Box<dyn Error>> {
    let hit = hit.ok_or("the center pixel did not contain the display point")?;
    if hit.view_generation() != VIEW_GENERATION || hit.point() != DISPLAY_POINT {
        return Err("the provisional hit does not match the recorded display state".into());
    }

    println!(
        "provisional GPU hit: {:?} (View generation {}, batch {}, version {})",
        hit.point(),
        hit.view_generation().generation(),
        hit.batch().get(),
        hit.version().get(),
    );
    println!(
        "before inspection or Edit: pin the Workspace Snapshot, reject stale View state, \
         and call point_review::confirm_pick with this identity under explicit review limits; \
         publish highlights only after terminal bounded PointSet iteration"
    );
    println!(
        "View generation and Workspace Revision are independent freshness checks; \
         retain caller-owned Operation identity and explicitly resolve indeterminate commits"
    );
    Ok(())
}

fn gpu_is_required() -> bool {
    std::env::var("PUNCTRA_REQUIRE_GPU").is_ok_and(|value| value == "1")
}

struct HostGpu {
    _instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl HostGpu {
    async fn request() -> Result<Option<Self>, Box<dyn Error>> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let Ok(adapter) = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::None,
                force_fallback_adapter: false,
                compatible_surface: None,
                apply_limit_buckets: false,
            })
            .await
        else {
            return Ok(None);
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("third-party Punctra host device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::MemoryUsage,
                trace: wgpu::Trace::Off,
            })
            .await?;
        Ok(Some(Self {
            _instance: instance,
            device,
            queue,
        }))
    }

    fn encoder(&self) -> wgpu::CommandEncoder {
        self.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("third-party Punctra host encoder"),
            })
    }
}

struct HostTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl HostTarget {
    fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("third-party Punctra host target"),
            size: wgpu::Extent3d {
                width: TARGET_SIZE,
                height: TARGET_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}
