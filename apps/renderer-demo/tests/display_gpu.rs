//! GPU acceptance for every exact CPU display mapping used by renderer-demo.

use std::sync::mpsc;

#[path = "../../../tests/support/gpu.rs"]
mod gpu_support;

use point_contracts::WorldBounds;
use render_protocol::{
    BatchKey, BatchVersion, PointBatch, PointId, RenderLimits, RenderPoint, RenderUpdate, SourceId,
    ViewGenerationKey, ViewId, Viewport,
};
use render_wgpu::{Camera, Frame, PointStyle, RendererConfig, WgpuRenderer};
use renderer_demo::display::{
    DisplayMode, NEUTRAL_COLOR, PointColorizer, classification_color, intensity_color, rgb_color,
};

use gpu_support::{GpuContext, with_gpu};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const VIEWPORT: [u32; 2] = [32, 32];
const CENTER: [u32; 2] = [16, 16];
const SOURCE: SourceId = SourceId::new([0x91; 32]);
const GENERATION: ViewGenerationKey = ViewGenerationKey::new(ViewId::new(92), 1);
const POINT_ID: PointId = PointId::new(SOURCE, 7);

#[test]
fn accepted_display_mappings_survive_gpu_upload_and_render() {
    with_gpu(assert_display_mappings);
}

fn assert_display_mappings(gpu: &GpuContext) {
    let limits = RenderLimits::new(24, 1, 1);
    let mut renderer = WgpuRenderer::new(&gpu.device, RendererConfig::new(FORMAT, limits))
        .expect("the display test renderer should initialize");
    renderer
        .apply(&RenderUpdate::Reset {
            view_generation: GENERATION,
        })
        .expect("the display generation should begin");

    for (index, expected) in mapped_colors().into_iter().enumerate() {
        let version = u64::try_from(index).unwrap().saturating_add(1);
        let point = RenderPoint::new([0.0; 3], expected, POINT_ID).unwrap();
        assert!(
            point
                .relative_position()
                .map(f32::to_bits)
                .iter()
                .all(|bits| *bits == 0.0_f32.to_bits())
        );
        assert_eq!(point.point_id(), POINT_ID);
        let batch = PointBatch::new(
            GENERATION,
            BatchKey::new(1),
            BatchVersion::new(version),
            [0.0; 3],
            vec![point],
        )
        .unwrap();
        renderer
            .apply(&RenderUpdate::Upsert { batch })
            .expect("each mapped color should replace the same point atomically");
        let actual = render_center(gpu, &mut renderer);
        assert_pixel(actual, expected);
    }
}

fn mapped_colors() -> [[u8; 4]; 5] {
    let bounds = WorldBounds::new([0.0, 0.0, 0.0], [100.0; 3]).unwrap();
    [
        NEUTRAL_COLOR,
        PointColorizer::for_source(DisplayMode::Elevation, Some(bounds)).color(75.0, None),
        rgb_color([0, 32_768, u16::MAX]),
        intensity_color(32_768),
        classification_color(2),
    ]
}

fn render_center(gpu: &GpuContext, renderer: &mut WgpuRenderer) -> [u8; 4] {
    let padded_bytes_per_row = padded_bytes_per_row(VIEWPORT[0]);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("renderer-demo display mapping target"),
        size: extent(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("renderer-demo display mapping readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(VIEWPORT[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("renderer-demo display mapping encoder"),
        });
    renderer
        .render(&mut encoder, &target, &frame())
        .expect("the mapped point should render");
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(VIEWPORT[1]),
            },
        },
        extent(),
    );
    let (sender, receiver) = mpsc::channel();
    encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
        let _ = sender.send(result);
    });
    gpu.queue.submit([encoder.finish()]);
    gpu.wait();
    receiver
        .recv()
        .expect("the mapping callback should run")
        .expect("the display readback should map");
    let mapped = readback
        .get_mapped_range(..)
        .expect("the mapped display range should be available");
    let offset = usize::try_from(CENTER[1] * padded_bytes_per_row + CENTER[0] * 4).unwrap();
    let pixel = mapped[offset..offset + 4].try_into().unwrap();
    drop(mapped);
    readback.unmap();
    pixel
}

fn frame() -> Frame {
    let camera = Camera::perspective(
        [0.0, -5.0, 0.0],
        [0.0; 3],
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_3,
        0.1,
        100.0,
    )
    .unwrap();
    let style = PointStyle::new(18.0, [1.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap();
    Frame::new(
        GENERATION,
        camera,
        Viewport::new(VIEWPORT[0], VIEWPORT[1]).unwrap(),
    )
    .unwrap()
    .with_style(style)
}

fn extent() -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: VIEWPORT[0],
        height: VIEWPORT[1],
        depth_or_array_layers: 1,
    }
}

fn padded_bytes_per_row(width: u32) -> u32 {
    width
        .checked_mul(4)
        .unwrap()
        .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

fn assert_pixel(actual: [u8; 4], expected: [u8; 4]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!(
            actual.abs_diff(expected) <= 1,
            "expected {expected}, got {actual}"
        );
    }
}
