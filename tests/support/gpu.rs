//! Shared headless GPU setup for workspace acceptance tests.

use std::{
    env,
    sync::{OnceLock, mpsc},
    time::{Duration, Instant},
};

static GPU: OnceLock<Option<GpuContext>> = OnceLock::new();

pub(super) struct GpuContext {
    pub(super) device: wgpu::Device,
    pub(super) queue: wgpu::Queue,
}

#[allow(
    dead_code,
    reason = "the shared path module is compiled by GPU tests that do not all render images"
)]
pub(super) struct Rgba8Target {
    texture: wgpu::Texture,
    pub(super) view: wgpu::TextureView,
    readback: wgpu::Buffer,
    viewport: [u32; 2],
    padded_bytes_per_row: u32,
}

#[allow(
    dead_code,
    reason = "the shared path module is compiled by GPU tests that do not all render images"
)]
impl Rgba8Target {
    pub(super) fn new(
        device: &wgpu::Device,
        viewport: [u32; 2],
        format: wgpu::TextureFormat,
        label: &str,
    ) -> Self {
        let padded_bytes_per_row = padded_bytes_per_row(viewport[0]);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: texture_extent(viewport),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: u64::from(padded_bytes_per_row) * u64::from(viewport[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            texture,
            view,
            readback,
            viewport,
            padded_bytes_per_row,
        }
    }

    pub(super) fn encode_copy(&self, encoder: &mut wgpu::CommandEncoder) {
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
                    rows_per_image: Some(self.viewport[1]),
                },
            },
            texture_extent(self.viewport),
        );
    }

    pub(super) fn map_after_submit(
        &self,
        encoder: &mut wgpu::CommandEncoder,
    ) -> mpsc::Receiver<Result<(), wgpu::BufferAsyncError>> {
        let (sender, receiver) = mpsc::channel();
        encoder.map_buffer_on_submit(&self.readback, wgpu::MapMode::Read, .., move |result| {
            let _ = sender.send(result);
        });
        receiver
    }

    pub(super) fn read(
        self,
        receiver: &mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    ) -> Rgba8Image {
        receiver
            .recv()
            .expect("the mapping callback should run")
            .expect("the RGBA8 readback should map");
        let mapped = self
            .readback
            .get_mapped_range(..)
            .expect("the mapped RGBA8 range should be available");
        let bytes = mapped.to_vec();
        drop(mapped);
        self.readback.unmap();
        Rgba8Image {
            bytes,
            viewport: self.viewport,
            padded_bytes_per_row: self.padded_bytes_per_row,
        }
    }
}

#[allow(
    dead_code,
    reason = "each GPU acceptance suite uses a different subset of image queries"
)]
pub(super) struct Rgba8Image {
    bytes: Vec<u8>,
    viewport: [u32; 2],
    padded_bytes_per_row: u32,
}

#[allow(
    dead_code,
    reason = "each GPU acceptance suite uses a different subset of image queries"
)]
impl Rgba8Image {
    pub(super) fn pixel(&self, pixel: [u32; 2]) -> [u8; 4] {
        assert!(pixel[0] < self.viewport[0] && pixel[1] < self.viewport[1]);
        let offset = usize::try_from(pixel[1] * self.padded_bytes_per_row + pixel[0] * 4)
            .expect("the tiny test image offset fits in usize");
        self.bytes[offset..offset + 4]
            .try_into()
            .expect("an RGBA8 pixel has four bytes")
    }

    pub(super) fn find_pixel(&self, predicate: impl Fn([u8; 4]) -> bool) -> Option<[u32; 2]> {
        for y in 0..self.viewport[1] {
            for x in 0..self.viewport[0] {
                let pixel = [x, y];
                if predicate(self.pixel(pixel)) {
                    return Some(pixel);
                }
            }
        }
        None
    }

    pub(super) fn visible_pixel_count(&self, background: [u8; 4]) -> u64 {
        let mut count = 0;
        for y in 0..self.viewport[1] {
            for x in 0..self.viewport[0] {
                if self.pixel([x, y]) != background {
                    count += 1;
                }
            }
        }
        count
    }

    pub(super) fn longest_background_run(
        &self,
        columns: std::ops::Range<u32>,
        row: u32,
        background: [u8; 4],
    ) -> u32 {
        let mut longest = 0;
        let mut current = 0;
        for x in columns {
            if self.pixel([x, row]) == background {
                current += 1;
                longest = longest.max(current);
            } else {
                current = 0;
            }
        }
        longest
    }

    pub(super) fn first_pixel_where(
        &self,
        other: &Self,
        predicate: impl Fn([u8; 4], [u8; 4]) -> bool,
    ) -> Option<[u32; 2]> {
        assert_eq!(self.viewport, other.viewport);
        for y in 0..self.viewport[1] {
            for x in 0..self.viewport[0] {
                let pixel = [x, y];
                if predicate(self.pixel(pixel), other.pixel(pixel)) {
                    return Some(pixel);
                }
            }
        }
        None
    }
}

impl GpuContext {
    pub(super) fn wait(&self) {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("headless device polling should succeed");
    }

    #[allow(
        dead_code,
        reason = "the shared path module is compiled by GPU tests that only need wait"
    )]
    pub(super) fn wait_for_submission<T>(
        &self,
        submission: &wgpu::SubmissionIndex,
        timeout: Duration,
        label: &str,
        mut poll: impl FnMut() -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(result) = poll() {
                return result;
            }
            let now = Instant::now();
            assert!(
                now < deadline,
                "{label} did not complete within {timeout:?}"
            );
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission.clone()),
                    timeout: Some(deadline.saturating_duration_since(now)),
                })
                .unwrap_or_else(|error| panic!("{label} device polling failed: {error}"));
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

pub(super) fn with_gpu(test: impl FnOnce(&GpuContext)) {
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
            label: Some("punctra GPU acceptance test device"),
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

fn texture_extent(viewport: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: viewport[0],
        height: viewport[1],
        depth_or_array_layers: 1,
    }
}

fn padded_bytes_per_row(width: u32) -> u32 {
    width
        .checked_mul(4)
        .expect("the tiny RGBA8 width should fit in u32")
        .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}
