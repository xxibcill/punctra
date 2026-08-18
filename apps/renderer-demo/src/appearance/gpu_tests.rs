use std::{
    collections::BTreeMap,
    sync::mpsc,
    time::{Duration, Instant},
};

#[path = "../../../../tests/support/gpu.rs"]
mod gpu_support;

use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT as POINT_BYTES, PointBatch, PointId,
    PresentationWeight, RenderLimits, RenderPoint, RenderUpdate, SourceId, ViewGenerationKey,
    ViewId, Viewport,
};
use render_wgpu::{
    Camera, Frame, FrameReport, PickHit, PickPoll, PickRequest, PointStyle, RecordedFrame,
    RendererConfig, WgpuRenderer,
};

use super::{
    CROSS_FADE_PRESENTED_FRAMES, REFERENCE_POINT_SIZE_PIXELS, projected_spacing_point_size,
    weight_for_step,
};
use gpu_support::{GpuContext, with_gpu};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const VIEWPORT: [u32; 2] = [96, 96];
const CENTER: [u32; 2] = [48, 48];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const SOURCE: SourceId = SourceId::new([0xa7; 32]);
const GENERATION: ViewGenerationKey = ViewGenerationKey::new(ViewId::new(93), 1);
const MAX_FIXED_VIEW_ENCODING_TIME: Duration = Duration::from_secs(1);
const MAX_FIXED_VIEW_FRAME_TIME: Duration = Duration::from_secs(2);

#[test]
fn fixed_view_adaptive_sizing_improves_coverage_without_hiding_a_source_hole() {
    with_gpu(assert_adaptive_sizing_image);
}

#[test]
fn fixed_view_cross_fade_has_no_hole_or_conspicuous_frame_discontinuity() {
    with_gpu(assert_cross_fade_images);
}

fn assert_adaptive_sizing_image(gpu: &GpuContext) {
    let (points, known_point) = grid_with_center_hole();
    let point_count = u64::try_from(points.len()).unwrap();
    let mut subject = ImageHarness::new(gpu, point_count, 1);
    subject.upsert(1, 1, points);

    let reference_style = point_style(REFERENCE_POINT_SIZE_PIXELS);
    let adaptive_size = projected_spacing_point_size(viewport(), point_count);
    assert_eq!(adaptive_size.to_bits(), 4.0_f32.to_bits());
    let adaptive_style = reference_style
        .with_display_size_pixels(adaptive_size)
        .unwrap();
    for projection in FixedProjection::ALL {
        let reference = subject.render(reference_style, projection);
        let adaptive = subject.render(adaptive_style, projection);

        let reference_coverage = reference.image.visible_pixel_count();
        let adaptive_coverage = adaptive.image.visible_pixel_count();
        assert!(
            adaptive_coverage > reference_coverage + reference_coverage / 2,
            "{projection:?} adaptive coverage {adaptive_coverage} should materially exceed the 2.4 px reference {reference_coverage}"
        );
        assert_eq!(reference.image.pixel(CENTER), BLACK);
        assert_eq!(adaptive.image.pixel(CENTER), BLACK);

        let stable_pick_pixel = [24, CENTER[1]];
        assert_eq!(
            subject
                .pick(&reference.recorded, stable_pick_pixel)
                .map(PickHit::point),
            Some(known_point)
        );
        assert_eq!(
            subject
                .pick(&adaptive.recorded, stable_pick_pixel)
                .map(PickHit::point),
            Some(known_point)
        );
        let visual_only_pixel = reference
            .image
            .first_pixel_where(&adaptive.image, |reference, adaptive| {
                reference == BLACK && adaptive != BLACK
            })
            .expect("adaptive sizing should add visible coverage around a fixed sample");
        assert_eq!(subject.pick(&reference.recorded, visual_only_pixel), None);
        assert_eq!(subject.pick(&adaptive.recorded, visual_only_pixel), None);

        assert_frame_ceiling(&reference, point_count, 1);
        assert_frame_ceiling(&adaptive, point_count, 1);
    }
}

fn assert_cross_fade_images(gpu: &GpuContext) {
    let parent = point_id(201);
    let child = point_id(202);
    for projection in FixedProjection::ALL {
        let mut subject = ImageHarness::new(gpu, 3, 3);
        subject.upsert(1, 1, vec![point([0.0; 3], RED, parent)]);
        subject.upsert(2, 1, vec![point([0.02, 0.0, 0.0], RED, child)]);
        subject.upsert(3, 1, vec![point([0.0, 1.0, 0.0], BLUE, point_id(203))]);
        subject.present(2, PresentationWeight::TRANSPARENT);
        let mut previous_red = None;
        for step in 0..=CROSS_FADE_PRESENTED_FRAMES {
            if step > 0 {
                subject.present(2, weight_for_step(step));
                subject.present(
                    1,
                    weight_for_step(CROSS_FADE_PRESENTED_FRAMES.saturating_sub(step)),
                );
            }
            if step == CROSS_FADE_PRESENTED_FRAMES {
                subject.remove(1, 1);
            }

            let point_size = projected_spacing_point_size(viewport(), subject.resident_points());
            let style = point_style(REFERENCE_POINT_SIZE_PIXELS)
                .with_display_size_pixels(point_size)
                .unwrap();
            let rendered = subject.render(style, projection);
            let center = rendered.image.pixel(CENTER);
            assert!(
                center[0] >= 180 && center[1] <= 1 && center[2] <= 1,
                "{projection:?} cross-fade frame {step} produced a hole or exposed the farther blue point: {center:?}"
            );
            if let Some(previous) = previous_red {
                assert!(
                    center[0].abs_diff(previous) <= 40,
                    "{projection:?} cross-fade frame {step} changed red intensity from {previous} to {}",
                    center[0]
                );
            }
            previous_red = Some(center[0]);
            let hit = subject
                .pick(&rendered.recorded, CENTER)
                .expect("the fixed center coverage should remain pickable");
            assert_eq!(hit.point(), child);
            assert_eq!(hit.batch(), BatchKey::new(2));

            let expected_points = if step == CROSS_FADE_PRESENTED_FRAMES {
                2
            } else {
                3
            };
            assert_frame_ceiling(&rendered, expected_points, expected_points);
        }
    }
}

fn grid_with_center_hole() -> (Vec<RenderPoint>, PointId) {
    let mut points = Vec::new();
    let mut known_point = None;
    for z in -3..=3 {
        for x in -3..=3 {
            if x == 0 && z == 0 {
                continue;
            }
            let identity = point_id(u64::try_from(points.len()).unwrap().saturating_add(1));
            if x == -2 && z == 0 {
                known_point = Some(identity);
            }
            #[allow(clippy::cast_precision_loss)]
            let position = [x as f32 * 0.5, 0.0, z as f32 * 0.5];
            points.push(point(position, GREEN, identity));
        }
    }
    (
        points,
        known_point.expect("the fixed grid should contain the known pick sample"),
    )
}

fn point(position: [f32; 3], color: [u8; 4], identity: PointId) -> RenderPoint {
    RenderPoint::new(position, color, identity).unwrap()
}

const fn point_id(ordinal: u64) -> PointId {
    PointId::new(SOURCE, ordinal)
}

fn point_style(size_pixels: f32) -> PointStyle {
    PointStyle::new(size_pixels, [1.0; 3], [0.0, 0.0, 0.0, 1.0]).unwrap()
}

fn viewport() -> Viewport {
    Viewport::new(VIEWPORT[0], VIEWPORT[1]).unwrap()
}

fn assert_frame_ceiling(rendered: &RenderedImage, points: u64, batches: u64) {
    let report = rendered.report;
    assert_eq!(report.drawn_points(), points);
    assert_eq!(report.draw_calls(), batches);
    assert_eq!(report.resident_bytes(), points * POINT_BYTES);
    assert!(
        report.transient_texture_bytes() <= 8 * u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1])
    );
    assert!(report.encoding_time() <= MAX_FIXED_VIEW_ENCODING_TIME);
    assert!(rendered.frame_time <= MAX_FIXED_VIEW_FRAME_TIME);
}

struct ImageHarness<'gpu> {
    gpu: &'gpu GpuContext,
    renderer: WgpuRenderer,
    batch_points: BTreeMap<BatchKey, u64>,
}

impl<'gpu> ImageHarness<'gpu> {
    fn new(gpu: &'gpu GpuContext, point_limit: u64, batch_limit: u64) -> Self {
        let limits = RenderLimits::new(point_limit * POINT_BYTES, point_limit, batch_limit);
        let mut renderer = WgpuRenderer::new(&gpu.device, RendererConfig::new(FORMAT, limits))
            .expect("the fixed-view renderer should initialize");
        renderer
            .apply(&RenderUpdate::Reset {
                view_generation: GENERATION,
            })
            .unwrap();
        Self {
            gpu,
            renderer,
            batch_points: BTreeMap::new(),
        }
    }

    fn upsert(&mut self, key: u64, version: u64, points: Vec<RenderPoint>) {
        let point_count = u64::try_from(points.len()).unwrap();
        let key = BatchKey::new(key);
        let batch = PointBatch::new(
            GENERATION,
            key,
            BatchVersion::new(version),
            [0.0; 3],
            points,
        )
        .unwrap();
        self.renderer
            .apply(&RenderUpdate::Upsert { batch })
            .unwrap();
        self.batch_points.insert(key, point_count);
    }

    fn present(&mut self, key: u64, weight: PresentationWeight) {
        self.renderer
            .apply(&RenderUpdate::SetBatchPresentation {
                view_generation: GENERATION,
                key: BatchKey::new(key),
                expected_version: BatchVersion::new(1),
                weight,
            })
            .unwrap();
    }

    fn remove(&mut self, key: u64, version: u64) {
        let key = BatchKey::new(key);
        self.renderer
            .apply(&RenderUpdate::Remove {
                view_generation: GENERATION,
                key,
                expected_version: BatchVersion::new(version),
            })
            .unwrap();
        self.batch_points.remove(&key);
    }

    fn resident_points(&self) -> u64 {
        self.batch_points.values().copied().sum()
    }

    fn render(&mut self, style: PointStyle, projection: FixedProjection) -> RenderedImage {
        let frame_started = Instant::now();
        let target = ColorTarget::new(&self.gpu.device);
        let mut encoder = self.encoder("renderer-demo fixed-view image encoder");
        let recorded = self
            .renderer
            .render(&mut encoder, &target.view, &frame(style, projection))
            .unwrap();
        let report = recorded.report();
        target.encode_copy(&mut encoder);
        let receiver = target.map_after_submit(&mut encoder);
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.wait();
        let image = target.read(&receiver);
        RenderedImage {
            image,
            recorded,
            report,
            frame_time: frame_started.elapsed(),
        }
    }

    fn pick(&mut self, recorded: &RecordedFrame, pixel: [u32; 2]) -> Option<PickHit> {
        let mut encoder = self.encoder("renderer-demo fixed-view pick encoder");
        let mut ticket = self
            .renderer
            .pick(&mut encoder, recorded, PickRequest::new(pixel))
            .unwrap();
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.wait();
        let PickPoll::Ready(hit) = ticket.poll().unwrap() else {
            panic!("a fully polled fixed-view pick should be ready");
        };
        hit
    }

    fn encoder(&self, label: &'static str) -> wgpu::CommandEncoder {
        self.gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) })
    }
}

struct RenderedImage {
    image: Image,
    recorded: RecordedFrame,
    report: FrameReport,
    frame_time: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixedProjection {
    Perspective,
    Orthographic,
}

impl FixedProjection {
    const ALL: [Self; 2] = [Self::Perspective, Self::Orthographic];

    fn camera(self) -> Camera {
        match self {
            Self::Perspective => Camera::perspective(
                [0.0, -5.0, 0.0],
                [0.0; 3],
                [0.0, 0.0, 1.0],
                0.761_012_73,
                0.1,
                100.0,
            ),
            Self::Orthographic => {
                Camera::orthographic([0.0, -5.0, 0.0], [0.0; 3], [0.0, 0.0, 1.0], 4.0, 0.1, 100.0)
            }
        }
        .unwrap()
    }
}

fn frame(style: PointStyle, projection: FixedProjection) -> Frame {
    let camera = projection.camera();
    Frame::new(GENERATION, camera, viewport())
        .unwrap()
        .with_style(style)
}

struct ColorTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

impl ColorTarget {
    fn new(device: &wgpu::Device) -> Self {
        let padded_bytes_per_row = padded_bytes_per_row();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("renderer-demo fixed-view color target"),
            size: extent(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("renderer-demo fixed-view color readback"),
            size: u64::from(padded_bytes_per_row) * u64::from(VIEWPORT[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            view,
            readback,
            padded_bytes_per_row,
        }
    }

    fn encode_copy(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(VIEWPORT[1]),
                },
            },
            extent(),
        );
    }

    fn map_after_submit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
    ) -> mpsc::Receiver<Result<(), wgpu::BufferAsyncError>> {
        let (sender, receiver) = mpsc::channel();
        encoder.map_buffer_on_submit(&self.readback, wgpu::MapMode::Read, .., move |result| {
            let _ = sender.send(result);
        });
        receiver
    }

    fn read(self, receiver: &mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>) -> Image {
        receiver.recv().unwrap().unwrap();
        let mapped = self.readback.get_mapped_range(..).unwrap();
        let bytes = mapped.to_vec();
        drop(mapped);
        self.readback.unmap();
        Image {
            bytes,
            padded_bytes_per_row: self.padded_bytes_per_row,
        }
    }
}

struct Image {
    bytes: Vec<u8>,
    padded_bytes_per_row: u32,
}

impl Image {
    fn pixel(&self, pixel: [u32; 2]) -> [u8; 4] {
        let offset = usize::try_from(pixel[1] * self.padded_bytes_per_row + pixel[0] * 4).unwrap();
        self.bytes[offset..offset + 4].try_into().unwrap()
    }

    fn visible_pixel_count(&self) -> u64 {
        let mut count = 0;
        for y in 0..VIEWPORT[1] {
            for x in 0..VIEWPORT[0] {
                if self.pixel([x, y]) != BLACK {
                    count += 1;
                }
            }
        }
        count
    }

    fn first_pixel_where(
        &self,
        other: &Self,
        predicate: impl Fn([u8; 4], [u8; 4]) -> bool,
    ) -> Option<[u32; 2]> {
        for y in 0..VIEWPORT[1] {
            for x in 0..VIEWPORT[0] {
                let pixel = [x, y];
                if predicate(self.pixel(pixel), other.pixel(pixel)) {
                    return Some(pixel);
                }
            }
        }
        None
    }
}

fn extent() -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: VIEWPORT[0],
        height: VIEWPORT[1],
        depth_or_array_layers: 1,
    }
}

fn padded_bytes_per_row() -> u32 {
    VIEWPORT[0]
        .checked_mul(4)
        .unwrap()
        .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}
