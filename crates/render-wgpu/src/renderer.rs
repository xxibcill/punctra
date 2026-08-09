use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use bytemuck::Zeroable;
use render_protocol::{
    BatchKey, PointBatch, PointId, ProtocolError, RenderLimits, RenderStateModel, RenderUpdate,
    UpdateReport, ViewGenerationKey,
};
use thiserror::Error;
use wgpu::util::DeviceExt;

use crate::{
    Frame,
    depth::DepthTarget,
    gpu::{BatchUniform, CameraUniform, GpuPoint},
    pick::{
        PICK_READBACK_ROW_BYTES, PICK_TOKEN_BYTES, PickError, PickRecord, PickRequest, PickTable,
        PickTarget, PickTicket,
    },
    pipeline::PointPipelines,
};

/// Immutable construction options for a [`WgpuRenderer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererConfig {
    color_format: wgpu::TextureFormat,
    limits: RenderLimits,
}

impl RendererConfig {
    /// Creates renderer options for a target color format and hard point-residency limits.
    #[must_use]
    pub const fn new(color_format: wgpu::TextureFormat, limits: RenderLimits) -> Self {
        Self {
            color_format,
            limits,
        }
    }

    /// Returns the target color format used to compile the point pipeline.
    #[must_use]
    pub const fn color_format(self) -> wgpu::TextureFormat {
        self.color_format
    }

    /// Returns the hard point-residency limits.
    #[must_use]
    pub const fn limits(self) -> RenderLimits {
        self.limits
    }
}

/// Observable work encoded for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReport {
    view_generation: ViewGenerationKey,
    drawn_points: u64,
    draw_calls: u64,
    resident_bytes: u64,
    encoding_time: Duration,
}

impl FrameReport {
    /// Returns the View generation that was drawn.
    #[must_use]
    pub const fn view_generation(self) -> ViewGenerationKey {
        self.view_generation
    }

    /// Returns the number of point instances encoded for drawing.
    #[must_use]
    pub const fn drawn_points(self) -> u64 {
        self.drawn_points
    }

    /// Returns the number of point-batch draw calls.
    #[must_use]
    pub const fn draw_calls(self) -> u64 {
        self.draw_calls
    }

    /// Returns resident point bytes under the protocol accounting model.
    #[must_use]
    pub const fn resident_bytes(self) -> u64 {
        self.resident_bytes
    }

    /// Returns CPU time spent preparing uniforms and encoding the render pass.
    #[must_use]
    pub const fn encoding_time(self) -> Duration {
        self.encoding_time
    }
}

/// A bounded wgpu representation of one active progressive View.
pub struct WgpuRenderer {
    device: wgpu::Device,
    state: RenderStateModel,
    pipelines: PointPipelines,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    batches: BTreeMap<BatchKey, GpuBatch>,
    depth: Option<DepthTarget>,
    pick_target: Option<PickTarget>,
    pick_table: Option<Arc<PickTable>>,
}

impl WgpuRenderer {
    /// Attaches a renderer to a caller-owned wgpu device.
    ///
    /// The handle is cheaply cloned. The caller retains its device and queue
    /// handles for command-encoder creation, submission, and device polling.
    ///
    /// # Errors
    ///
    /// Returns [`RendererError::InvalidColorFormat`] unless the format has
    /// WebGPU-guaranteed blendable render-attachment support on this device.
    pub fn new(device: &wgpu::Device, config: RendererConfig) -> Result<Self, RendererError> {
        let format_features = config
            .color_format
            .guaranteed_format_features(device.features());
        if !format_features
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            || !format_features
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
        {
            return Err(RendererError::InvalidColorFormat(config.color_format));
        }

        let pipelines = PointPipelines::new(device, config.color_format);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("punctra camera uniform"),
            contents: bytemuck::bytes_of(&CameraUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group = uniform_bind_group(
            device,
            &pipelines.camera_layout,
            &camera_buffer,
            "punctra camera bind group",
        );

        Ok(Self {
            device: device.clone(),
            state: RenderStateModel::new(config.limits),
            pipelines,
            camera_buffer,
            camera_bind_group,
            batches: BTreeMap::new(),
            depth: None,
            pick_target: None,
            pick_table: None,
        })
    }

    /// Applies one complete renderer-neutral update atomically.
    ///
    /// Point data is copied before this method returns. The caller may release
    /// the update immediately.
    ///
    /// # Errors
    ///
    /// Returns a protocol or renderer resource error without changing the
    /// renderer's active logical state.
    pub fn apply(&mut self, update: &RenderUpdate) -> Result<UpdateReport, RendererError> {
        let mut next_state = self.state.clone();
        let report = next_state.apply(update)?;

        match update {
            RenderUpdate::Reset { view_generation } => {
                self.batches.clear();
                self.pick_table = Some(Arc::new(PickTable::new(*view_generation)));
            }
            RenderUpdate::Upsert { batch } => {
                let highlights = highlights(&next_state);
                let pick_table = self
                    .pick_table
                    .as_ref()
                    .ok_or(RendererError::PickMetadataUnavailable)?;
                let gpu_batch = GpuBatch::new(
                    &self.device,
                    &self.pipelines.batch_layout,
                    batch,
                    &highlights,
                    pick_table,
                )?;
                self.batches.insert(batch.key(), gpu_batch);
            }
            RenderUpdate::Remove { key, .. } => {
                self.batches.remove(key);
            }
            RenderUpdate::SetHighlights { .. } => {
                let highlights = highlights(&next_state);
                for batch in self.batches.values_mut() {
                    batch.apply_highlights(&self.device, &highlights);
                }
            }
        }

        self.state = next_state;
        Ok(report)
    }

    /// Records one point-cloud frame into a caller-owned command encoder.
    ///
    /// The method never performs file I/O, finishes the encoder, or submits the
    /// queue. Uniform uploads are encoded into this encoder, so multiple frames
    /// may be recorded before submission without sharing later camera values.
    /// `target` must be a non-multisampled 2D view in the configured color
    /// format whose extent matches [`Frame::viewport`]. wgpu reports violations
    /// of those texture-view requirements through its normal validation path.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested generation is not active or when a
    /// batch origin cannot be represented relative to the 64-bit camera.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &Frame,
    ) -> Result<FrameReport, RendererError> {
        let started_at = Instant::now();
        let snapshot = self.state.snapshot();
        let active_view_generation =
            self.require_active_view_generation(frame.view_generation())?;

        let viewport = frame.viewport();
        self.ensure_depth(viewport);
        self.record_frame_uniforms(encoder, frame)?;

        let clear = frame.style().clear_color();
        let color_attachment = Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: clear[0],
                    g: clear[1],
                    b: clear[2],
                    a: clear[3],
                }),
                store: wgpu::StoreOp::Store,
            },
        });
        let color_attachments = [color_attachment];
        let depth = self
            .depth
            .as_ref()
            .ok_or(RendererError::DepthTargetUnavailable)?;

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("punctra point pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth.view(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines.draw);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            for batch in self.batches.values() {
                pass.set_bind_group(1, &batch.bind_group, &[]);
                pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));
                pass.draw(0..6, 0..batch.point_count);
            }
        }

        Ok(FrameReport {
            view_generation: active_view_generation,
            drawn_points: snapshot.resident().point_count(),
            draw_calls: snapshot.resident().batch_count(),
            resident_bytes: snapshot.resident().estimated_gpu_bytes(),
            encoding_time: started_at.elapsed(),
        })
    }

    /// Records a provisional point-ID pass and asynchronous one-pixel readback.
    ///
    /// Submit the containing command encoder, drive normal wgpu device polling,
    /// and then poll the returned ticket. Picking never confirms exact Point Set
    /// membership; it only identifies the resident display point at one pixel.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale frame, an out-of-bounds pixel, or unavailable
    /// generation metadata.
    pub fn pick(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        request: PickRequest,
    ) -> Result<PickTicket, RendererError> {
        let active_view_generation =
            self.require_active_view_generation(frame.view_generation())?;
        let viewport = frame.viewport();
        let pixel = request.pixel();
        if pixel[0] >= viewport[0] || pixel[1] >= viewport[1] {
            return Err(RendererError::PickOutsideViewport { pixel, viewport });
        }

        self.ensure_depth(viewport);
        self.ensure_pick_target(viewport);
        self.record_frame_uniforms(encoder, frame)?;
        let pick_target = self
            .pick_target
            .as_ref()
            .ok_or(RendererError::PickTargetUnavailable)?;
        let depth = self
            .depth
            .as_ref()
            .ok_or(RendererError::DepthTargetUnavailable)?;
        self.record_pick_pass(encoder, pick_target, depth);
        let (readback, receiver) = self.record_pick_readback(encoder, pick_target, pixel);
        let table = Arc::clone(
            self.pick_table
                .as_ref()
                .ok_or(RendererError::PickMetadataUnavailable)?,
        );
        Ok(PickTicket::new(
            active_view_generation,
            readback,
            receiver,
            table,
        ))
    }

    fn require_active_view_generation(
        &self,
        requested: ViewGenerationKey,
    ) -> Result<ViewGenerationKey, RendererError> {
        let active = self
            .state
            .snapshot()
            .active_view_generation()
            .ok_or(RendererError::NoActiveViewGeneration)?;
        if active == requested {
            Ok(active)
        } else {
            Err(RendererError::ViewGenerationMismatch { active, requested })
        }
    }

    fn record_pick_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &PickTarget,
        depth: &DepthTarget,
    ) {
        let pick_attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &target.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("punctra pick pass"),
            color_attachments: &pick_attachments,
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth.view(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipelines.pick);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        for batch in self.batches.values() {
            pass.set_bind_group(1, &batch.bind_group, &[]);
            pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));
            pass.draw(0..6, 0..batch.point_count);
        }
    }

    fn record_pick_readback(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &PickTarget,
        pixel: [u32; 2],
    ) -> (
        wgpu::Buffer,
        mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    ) {
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("punctra pick readback"),
            size: PICK_READBACK_ROW_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: pixel[0],
                    y: pixel[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let (sender, receiver) = mpsc::channel();
        encoder.map_buffer_on_submit(
            &readback,
            wgpu::MapMode::Read,
            0..PICK_TOKEN_BYTES,
            move |result| {
                let _ = sender.send(result);
            },
        );
        (readback, receiver)
    }

    fn ensure_depth(&mut self, viewport: [u32; 2]) {
        let matches = self
            .depth
            .as_ref()
            .is_some_and(|depth| depth.viewport() == viewport);
        if !matches {
            self.depth = Some(DepthTarget::new(&self.device, viewport));
        }
    }

    fn ensure_pick_target(&mut self, viewport: [u32; 2]) {
        let matches = self
            .pick_target
            .as_ref()
            .is_some_and(|target| target.viewport == viewport);
        if !matches {
            self.pick_target = Some(PickTarget::new(&self.device, viewport));
        }
    }

    fn record_frame_uniforms(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
    ) -> Result<(), RendererError> {
        let viewport = frame.viewport();
        let viewport_f32 = viewport_as_f32(viewport);
        let aspect_ratio = viewport_f32[0] / viewport_f32[1];
        let style = frame.style();
        let camera = frame.camera();
        let camera_uniform = CameraUniform {
            view_projection: camera.view_projection(aspect_ratio).to_cols_array_2d(),
            viewport_size: viewport_f32,
            default_point_size: style.default_size_pixels(),
            _padding: 0.0,
            highlight_color: style.highlight_color(),
        };
        let eye = camera.eye();
        let camera_bytes = bytemuck::bytes_of(&camera_uniform);
        let batch_uniform_size = size_of::<BatchUniform>() as wgpu::BufferAddress;
        let mut upload_bytes =
            Vec::with_capacity(camera_bytes.len() + self.batches.len() * size_of::<BatchUniform>());
        upload_bytes.extend_from_slice(camera_bytes);
        let mut copies = Vec::with_capacity(self.batches.len());
        for (key, batch) in &self.batches {
            let mut offset = [0.0_f32; 3];
            for axis in 0..3 {
                offset[axis] =
                    camera_relative_axis(batch.world_origin[axis], eye[axis], *key, axis)?;
            }
            let uniform = BatchUniform {
                origin_from_camera: [offset[0], offset[1], offset[2], 0.0],
            };
            let source_offset = u64::try_from(upload_bytes.len())
                .expect("frame uniform staging fits in wgpu's buffer address space");
            upload_bytes.extend_from_slice(bytemuck::bytes_of(&uniform));
            copies.push((batch, source_offset));
        }

        let upload = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("punctra frame uniform upload"),
                contents: &upload_bytes,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
        encoder.copy_buffer_to_buffer(
            &upload,
            0,
            &self.camera_buffer,
            0,
            camera_bytes.len() as wgpu::BufferAddress,
        );
        for (batch, source_offset) in copies {
            encoder.copy_buffer_to_buffer(
                &upload,
                source_offset,
                &batch.uniform_buffer,
                0,
                batch_uniform_size,
            );
        }
        Ok(())
    }
}

struct GpuBatch {
    world_origin: [f64; 3],
    point_count: u32,
    point_ids: Vec<PointId>,
    gpu_points: Vec<GpuPoint>,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl GpuBatch {
    fn new(
        device: &wgpu::Device,
        batch_layout: &wgpu::BindGroupLayout,
        batch: &PointBatch,
        highlights: &BTreeSet<PointId>,
        pick_table: &PickTable,
    ) -> Result<Self, RendererError> {
        let point_count =
            u32::try_from(batch.points().len()).map_err(|_| RendererError::BatchTooLarge {
                key: batch.key(),
                point_count: batch.point_count(),
            })?;
        validate_batch_buffer_size(
            batch.key(),
            batch.estimated_gpu_bytes(),
            device.limits().max_buffer_size,
        )?;
        let mut point_ids = Vec::with_capacity(batch.points().len());
        let mut gpu_points = Vec::with_capacity(batch.points().len());
        let records = batch.points().iter().map(|point| PickRecord {
            batch: batch.key(),
            version: batch.version(),
            point: point.point_id(),
        });
        let pick_tokens = pick_table.append(records)?;

        for (point, pick_token) in batch.points().iter().zip(pick_tokens) {
            let point_id = point.point_id();
            let mut gpu_point = GpuPoint {
                position: point.relative_position(),
                point_size: 0.0,
                color: point.color(),
                flags: 0,
                pick_token,
                _padding: 0,
            };
            gpu_point.set_highlighted(highlights.contains(&point_id));
            point_ids.push(point_id);
            gpu_points.push(gpu_point);
        }

        let vertex_buffer = point_buffer(device, &gpu_points);
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("punctra batch uniform"),
            contents: bytemuck::bytes_of(&BatchUniform::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = uniform_bind_group(
            device,
            batch_layout,
            &uniform_buffer,
            "punctra batch bind group",
        );

        Ok(Self {
            world_origin: batch.world_origin(),
            point_count,
            point_ids,
            gpu_points,
            vertex_buffer,
            uniform_buffer,
            bind_group,
        })
    }

    fn apply_highlights(&mut self, device: &wgpu::Device, highlights: &BTreeSet<PointId>) {
        let mut changed = false;
        for (point_id, gpu_point) in self.point_ids.iter().zip(&mut self.gpu_points) {
            let before = gpu_point.flags;
            gpu_point.set_highlighted(highlights.contains(point_id));
            changed |= before != gpu_point.flags;
        }
        if changed {
            self.vertex_buffer = point_buffer(device, &self.gpu_points);
        }
    }
}

fn point_buffer(device: &wgpu::Device, points: &[GpuPoint]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("punctra point batch"),
        contents: bytemuck::cast_slice(points),
        usage: wgpu::BufferUsages::VERTEX,
    })
}

/// An error returned by renderer construction, update, or frame recording.
#[derive(Debug, Error)]
pub enum RendererError {
    /// A renderer-neutral transition was invalid.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// Picking metadata or asynchronous readback setup failed.
    #[error(transparent)]
    Pick(#[from] PickError),
    /// The color format is not a guaranteed blendable render attachment.
    #[error("renderer color target must be a blendable render-attachment format, got {0:?}")]
    InvalidColorFormat(wgpu::TextureFormat),
    /// A batch cannot be indexed by wgpu's 32-bit instance range.
    #[error("batch {key:?} has {point_count} points, exceeding the renderer's per-batch limit")]
    BatchTooLarge {
        /// The rejected batch.
        key: BatchKey,
        /// Its point count.
        point_count: u64,
    },
    /// A batch's fixed point vertex buffer exceeds the device limit.
    #[error(
        "batch {key:?} needs a {requested_bytes}-byte point buffer, exceeding the device limit {max_buffer_size}"
    )]
    BatchBufferTooLarge {
        /// The rejected batch.
        key: BatchKey,
        /// Exact bytes required by its fixed point vertices.
        requested_bytes: u64,
        /// The attached device's maximum buffer size.
        max_buffer_size: u64,
    },
    /// Rendering was requested before a reset began a View generation.
    #[error("rendering requires an active View generation")]
    NoActiveViewGeneration,
    /// A frame requested a View generation other than the active generation.
    #[error("requested View generation {requested:?} does not match active generation {active:?}")]
    ViewGenerationMismatch {
        /// The active View generation.
        active: ViewGenerationKey,
        /// The requested View generation.
        requested: ViewGenerationKey,
    },
    /// A requested physical pixel lies outside the frame viewport.
    #[error("pick pixel {pixel:?} is outside viewport {viewport:?}")]
    PickOutsideViewport {
        /// The rejected pixel coordinate.
        pixel: [u32; 2],
        /// The current physical viewport.
        viewport: [u32; 2],
    },
    /// Internal generation pick metadata was unexpectedly absent.
    #[error("active View generation has no pick metadata")]
    PickMetadataUnavailable,
    /// A lazily created depth target was unexpectedly absent.
    #[error("renderer depth target is unavailable")]
    DepthTargetUnavailable,
    /// A lazily created pick target was unexpectedly absent.
    #[error("renderer pick target is unavailable")]
    PickTargetUnavailable,
    /// A batch origin is too far from the camera to fit the display model.
    #[error("batch {key:?} origin axis {axis} is outside finite f32 camera-relative range")]
    BatchOriginOutOfRange {
        /// The affected batch.
        key: BatchKey,
        /// Zero-based world axis.
        axis: usize,
    },
}

fn uniform_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    label: &'static str,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

fn highlights(state: &RenderStateModel) -> BTreeSet<PointId> {
    state.snapshot().highlights().iter().copied().collect()
}

#[allow(clippy::cast_precision_loss)]
fn viewport_as_f32(viewport: [u32; 2]) -> [f32; 2] {
    [viewport[0] as f32, viewport[1] as f32]
}

fn camera_relative_axis(
    world_origin: f64,
    camera_eye: f64,
    key: BatchKey,
    axis: usize,
) -> Result<f32, RendererError> {
    let relative = world_origin - camera_eye;
    if !relative.is_finite() || relative.abs() > f64::from(f32::MAX) {
        return Err(RendererError::BatchOriginOutOfRange { key, axis });
    }

    #[allow(clippy::cast_possible_truncation)]
    let relative = relative as f32;
    Ok(relative)
}

fn validate_batch_buffer_size(
    key: BatchKey,
    requested_bytes: u64,
    max_buffer_size: u64,
) -> Result<(), RendererError> {
    if requested_bytes > max_buffer_size {
        Err(RendererError::BatchBufferTooLarge {
            key,
            requested_bytes,
            max_buffer_size,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_buffer_limit_is_checked_before_allocation() {
        let key = BatchKey::new(9);

        assert!(validate_batch_buffer_size(key, 256, 256).is_ok());
        assert!(matches!(
            validate_batch_buffer_size(key, 288, 256),
            Err(RendererError::BatchBufferTooLarge {
                key: rejected,
                requested_bytes: 288,
                max_buffer_size: 256,
            }) if rejected == key
        ));
    }
}
