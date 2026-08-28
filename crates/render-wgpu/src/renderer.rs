use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    sync::{Arc, mpsc},
    time::Duration,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use bytemuck::Zeroable;
use render_protocol::{
    BatchKey, PointBatch, PointId, PresentationWeight, ProtocolError, RenderLimits,
    RenderStateModel, RenderUpdate, UpdateEffect, UpdateReport, ViewGenerationKey, Viewport,
};
use thiserror::Error;
use wgpu::util::DeviceExt;

use crate::{
    Frame,
    footprint::{PointFootprint, PointFootprintPlan, PointFootprintStatus},
    gpu::{BatchUniform, CameraUniform, EdlUniform, GpuPoint},
    pick::{
        PICK_READBACK_ROW_BYTES, PICK_TOKEN_BYTES, PickError, PickRecord, PickRequest, PickTable,
        PickTicket,
    },
    pipeline::{DEPTH_FORMAT, EdlPipeline, PointPipelinePair, PointPipelines},
    targets::{DepthTarget, PickTarget, RenderTargets},
};

/// Immutable construction options for a [`WgpuRenderer`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererConfig {
    color_format: wgpu::TextureFormat,
    limits: RenderLimits,
    eye_dome_lighting: Option<EyeDomeLighting>,
    point_footprint: PointFootprint,
}

impl RendererConfig {
    /// Creates renderer options for a target color format and hard point-residency limits.
    #[must_use]
    pub const fn new(color_format: wgpu::TextureFormat, limits: RenderLimits) -> Self {
        Self {
            color_format,
            limits,
            eye_dome_lighting: None,
            point_footprint: PointFootprint::SingleSample,
        }
    }

    /// Enables bounded eye-dome lighting when the target and device support
    /// the required four-byte color and sampleable depth textures.
    #[must_use]
    pub const fn with_eye_dome_lighting(mut self, config: EyeDomeLighting) -> Self {
        self.eye_dome_lighting = Some(config);
        self
    }

    /// Requests one immutable Point-footprint policy.
    ///
    /// Four-sample coverage falls back explicitly for an unsupported target or
    /// a viewport outside the bounded resource envelope.
    #[must_use]
    pub const fn with_point_footprint(mut self, point_footprint: PointFootprint) -> Self {
        self.point_footprint = point_footprint;
        self
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

    /// Returns the requested Point-footprint policy.
    #[must_use]
    pub const fn point_footprint(self) -> PointFootprint {
        self.point_footprint
    }
}

/// Bounded eye-dome lighting controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeDomeLighting {
    strength: f32,
    radius_pixels: u32,
}

impl Eq for EyeDomeLighting {}

impl EyeDomeLighting {
    /// Creates a depth cue with strength in `(0, 10]` and a radius from one to
    /// eight physical pixels.
    ///
    /// # Errors
    ///
    /// Returns [`DepthCueError`] when either bound is violated.
    pub fn new(strength: f32, radius_pixels: u32) -> Result<Self, DepthCueError> {
        if !strength.is_finite() || strength <= 0.0 || strength > 10.0 {
            return Err(DepthCueError::InvalidStrength);
        }
        if !(1..=8).contains(&radius_pixels) {
            return Err(DepthCueError::InvalidRadius);
        }
        Ok(Self {
            strength,
            radius_pixels,
        })
    }

    /// Returns the bounded depth-discontinuity strength.
    #[must_use]
    pub const fn strength(self) -> f32 {
        self.strength
    }

    /// Returns the physical-pixel neighbor radius.
    #[must_use]
    pub const fn radius_pixels(self) -> u32 {
        self.radius_pixels
    }
}

/// An invalid eye-dome lighting configuration.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DepthCueError {
    /// Strength was non-finite, non-positive, or above ten.
    #[error("eye-dome strength must be finite and inside (0, 10]")]
    InvalidStrength,
    /// Radius was outside one through eight physical pixels.
    #[error("eye-dome radius must be inside 1..=8 physical pixels")]
    InvalidRadius,
}

/// Renderer-wide capability disposition of the optional eye-dome path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthCueStatus {
    /// The caller explicitly left the depth cue disabled.
    Disabled,
    /// Eye-dome lighting is available and used unless a frame enters its
    /// bounded Point-footprint resource fallback.
    Active,
    /// The caller enabled it, but the renderer safely uses the unenhanced path.
    UnsupportedFallback,
}

/// Observable work encoded for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReport {
    view_generation: ViewGenerationKey,
    drawn_points: u64,
    draw_calls: u64,
    resident_bytes: u64,
    encoding_time: Duration,
    transient_texture_bytes: u64,
    eye_dome_lighting_applied: bool,
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

    /// Returns the exact bytes of renderer-owned transient color, depth, and
    /// pick textures retained when this frame was recorded.
    #[must_use]
    pub const fn transient_texture_bytes(self) -> u64 {
        self.transient_texture_bytes
    }

    /// Returns whether eye-dome lighting was actually encoded for this frame.
    ///
    /// This may be false while [`WgpuRenderer::depth_cue_status`] reports
    /// [`DepthCueStatus::Active`] when the frame's Point footprint uses its
    /// bounded [`PointFootprintStatus::ResourceFallback`] path.
    #[must_use]
    pub const fn eye_dome_lighting_applied(self) -> bool {
        self.eye_dome_lighting_applied
    }
}

/// An exact GPU-resource snapshot of one frame recorded by a [`WgpuRenderer`].
///
/// Pass this value back to [`WgpuRenderer::pick`] to pick against the exact
/// batches, versions, camera, and style used by the recorded draw. Retaining a
/// recorded frame also retains any GPU buffers replaced or removed afterward;
/// drop it when exact-frame picking is no longer needed.
pub struct RecordedFrame {
    renderer: Arc<RendererIdentity>,
    frame: Frame,
    batches: Box<[RecordedBatch]>,
    pick_table: Arc<PickTable>,
    report: FrameReport,
}

impl RecordedFrame {
    /// Returns the work report for this recorded frame.
    #[must_use]
    pub const fn report(&self) -> FrameReport {
        self.report
    }
}

struct RendererIdentity;

/// A bounded wgpu representation of one active progressive View.
pub struct WgpuRenderer {
    identity: Arc<RendererIdentity>,
    device: wgpu::Device,
    state: RenderStateModel,
    pipelines: PointPipelines,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    batches: BTreeMap<BatchKey, GpuBatch>,
    targets: RenderTargets,
    pick_table: Option<Arc<PickTable>>,
    eye_dome: EyeDomeState,
    point_footprint: PointFootprintPlan,
}

enum EyeDomeState {
    Inactive(DepthCueStatus),
    Active {
        pipeline: EdlPipeline,
        uniform_buffer: wgpu::Buffer,
    },
}

impl EyeDomeState {
    const fn status(&self) -> DepthCueStatus {
        match self {
            Self::Inactive(status) => *status,
            Self::Active { .. } => DepthCueStatus::Active,
        }
    }

    const fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }
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

        let eye_dome = match (depth_cue_status(device, config), config.eye_dome_lighting) {
            (DepthCueStatus::Active, Some(cue)) => EyeDomeState::Active {
                pipeline: EdlPipeline::new(device, config.color_format),
                uniform_buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("punctra eye-dome uniform"),
                    contents: bytemuck::bytes_of(&EdlUniform {
                        strength: cue.strength(),
                        radius_pixels: cue.radius_pixels(),
                        _padding: [0; 2],
                    }),
                    usage: wgpu::BufferUsages::UNIFORM,
                }),
            },
            (DepthCueStatus::Disabled, _) => EyeDomeState::Inactive(DepthCueStatus::Disabled),
            (DepthCueStatus::UnsupportedFallback, _) | (DepthCueStatus::Active, None) => {
                EyeDomeState::Inactive(DepthCueStatus::UnsupportedFallback)
            }
        };
        let edl_active = eye_dome.is_active();
        let point_footprint = PointFootprintPlan::new(
            device,
            config.color_format,
            config.point_footprint,
            edl_active,
        );
        let pipelines = PointPipelines::new(
            device,
            config.color_format,
            edl_active,
            point_footprint.creates_multisample_pipelines(),
        );
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
            identity: Arc::new(RendererIdentity),
            device: device.clone(),
            state: RenderStateModel::new(config.limits),
            pipelines,
            camera_buffer,
            camera_bind_group,
            batches: BTreeMap::new(),
            targets: RenderTargets::new(
                config.color_format,
                edl_active.then_some(config.color_format),
            ),
            pick_table: None,
            eye_dome,
            point_footprint,
        })
    }

    /// Returns whether the requested eye-dome path is disabled, active, or
    /// using its explicit capability fallback.
    ///
    /// This is renderer-wide capability status. A frame whose Point footprint
    /// cannot retain its complete eye-dome and pick target set within the exact
    /// transient ceiling suppresses eye-dome staging for that frame; this method
    /// continues to report [`DepthCueStatus::Active`].
    #[must_use]
    pub const fn depth_cue_status(&self) -> DepthCueStatus {
        self.eye_dome.status()
    }

    /// Returns the Point-footprint path selected for one physical viewport.
    ///
    /// Selection is a pure preflight result: it does not allocate targets and
    /// remains unchanged before and after rendering the same viewport.
    #[must_use]
    pub fn point_footprint_status(&self, viewport: Viewport) -> PointFootprintStatus {
        self.point_footprint.status(viewport)
    }

    /// Returns the exact bytes retained by the renderer's current transient
    /// color, depth, and pick targets.
    ///
    /// The value changes lazily as rendering and picking allocate targets for
    /// the current viewport, and returns zero before any target is allocated.
    #[must_use]
    pub fn transient_texture_bytes(&self) -> u64 {
        self.targets.transient_texture_bytes()
    }

    /// Returns the number of resident Points currently carrying the highlight
    /// locator flag. This is presentation state only and does not imply exact
    /// selection completeness.
    #[must_use]
    pub fn resident_highlight_points(&self) -> u64 {
        self.batches
            .values()
            .map(|batch| {
                u64::try_from(
                    batch
                        .gpu_points
                        .iter()
                        .filter(|point| point.flags & crate::gpu::HIGHLIGHTED_FLAG != 0)
                        .count(),
                )
                .unwrap_or(u64::MAX)
            })
            .fold(0_u64, u64::saturating_add)
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
        let applied = next_state.apply(update)?;
        let report = applied.report();

        match applied.effect() {
            UpdateEffect::GenerationReset => {
                self.batches.clear();
                self.pick_table = Some(Arc::new(PickTable::new(report.view_generation())));
            }
            UpdateEffect::BatchUpserted { batch } => {
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
            UpdateEffect::BatchRemoved { key } => {
                self.batches.remove(&key);
            }
            UpdateEffect::BatchPresentationSet { key, weight } => {
                let batch = self
                    .batches
                    .get_mut(&key)
                    .ok_or(ProtocolError::BatchNotResident { key })?;
                batch.presentation_weight = weight;
            }
            UpdateEffect::HighlightsSet => {
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
    /// Returns an error when the requested generation is not active, generation
    /// pick metadata is unavailable, or a batch origin cannot be represented
    /// relative to the 64-bit camera.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &Frame,
    ) -> Result<RecordedFrame, RendererError> {
        let started_at = Instant::now();
        let snapshot = self.state.snapshot();
        let active_view_generation =
            self.require_active_view_generation(frame.view_generation())?;
        let batches = self.recorded_batches(frame.camera());
        let pick_table = Arc::clone(
            self.pick_table
                .as_ref()
                .ok_or(RendererError::PickMetadataUnavailable)?,
        );

        let viewport = frame.viewport();
        let point_footprint_status = self.point_footprint_status(viewport);
        let eye_dome_lighting_applied =
            self.eye_dome.is_active() && self.point_footprint.allows_eye_dome(viewport);
        self.record_frame_uniforms(
            encoder,
            frame,
            frame.style().display_size_pixels(),
            &batches,
        )?;
        let nominal_camera_upload = eye_dome_lighting_applied.then(|| {
            self.camera_uniform_upload(
                frame,
                frame.style().default_size_pixels(),
                "punctra nominal visibility camera upload",
            )
        });
        self.record_frame_passes(
            encoder,
            target,
            frame,
            &batches,
            point_footprint_status,
            nominal_camera_upload.as_ref(),
        );
        let transient_texture_bytes = self.targets.transient_texture_bytes();

        let report = FrameReport {
            view_generation: active_view_generation,
            drawn_points: snapshot.resident().point_count(),
            draw_calls: snapshot.resident().batch_count(),
            resident_bytes: snapshot.resident().estimated_gpu_bytes(),
            encoding_time: started_at.elapsed(),
            transient_texture_bytes,
            eye_dome_lighting_applied,
        };
        Ok(RecordedFrame {
            renderer: Arc::clone(&self.identity),
            frame: *frame,
            batches,
            pick_table,
            report,
        })
    }

    fn record_frame_passes(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame: &Frame,
        batches: &[RecordedBatch],
        point_footprint_status: PointFootprintStatus,
        nominal_camera_upload: Option<&wgpu::Buffer>,
    ) {
        let viewport = frame.viewport();
        let multisample = (point_footprint_status == PointFootprintStatus::Multisample4x)
            .then_some(self.pipelines.multisample.as_ref())
            .flatten();
        let mut pass = FramePass {
            encoder,
            target,
            clear: frame.style().clear_color(),
            camera_buffer: &self.camera_buffer,
            camera_bind_group: &self.camera_bind_group,
            batches,
        };
        if point_footprint_status == PointFootprintStatus::ResourceFallback {
            let depth = self.targets.single_sample_depth(&self.device, viewport);
            pass.record_points(target, None, depth, &self.pipelines.single_sample);
            return;
        }
        match (
            &self.eye_dome,
            self.pipelines.eye_dome_depth.as_ref(),
            multisample,
            nominal_camera_upload,
        ) {
            (
                EyeDomeState::Active {
                    pipeline,
                    uniform_buffer,
                },
                Some(depth_pipeline),
                Some(point_pipelines),
                Some(nominal_camera_upload),
            ) => {
                let (color, depth, visibility_depth, resolved_color, bind_group) = self
                    .targets
                    .multisample_eye_dome(&self.device, viewport, &pipeline.layout, uniform_buffer);
                pass.record_points(
                    color.view(),
                    Some(resolved_color.view()),
                    depth,
                    point_pipelines,
                );
                pass.stage_camera_uniform(nominal_camera_upload);
                pass.record_eye_dome_depth(visibility_depth, depth_pipeline);
                pass.record_eye_dome(pipeline, bind_group);
            }
            (
                EyeDomeState::Active {
                    pipeline,
                    uniform_buffer,
                },
                Some(depth_pipeline),
                None,
                Some(nominal_camera_upload),
            ) => {
                let (depth, color, bind_group) =
                    self.targets
                        .eye_dome(&self.device, viewport, &pipeline.layout, uniform_buffer);
                pass.record_points(color.view(), None, depth, &self.pipelines.single_sample);
                pass.stage_camera_uniform(nominal_camera_upload);
                pass.record_eye_dome_depth(depth, depth_pipeline);
                pass.record_eye_dome(pipeline, bind_group);
            }
            (_, _, Some(point_pipelines), _) => {
                let (color, depth) = self.targets.multisample(&self.device, viewport);
                pass.record_points(color.view(), Some(target), depth, point_pipelines);
            }
            (_, _, None, _) => {
                let depth = self.targets.single_sample_depth(&self.device, viewport);
                pass.record_points(target, None, depth, &self.pipelines.single_sample);
            }
        }
    }

    /// Records a provisional point-ID pass and asynchronous one-pixel readback.
    ///
    /// Submit the containing command encoder, drive normal wgpu device polling,
    /// and then poll the returned ticket. Picking never confirms exact Point Set
    /// membership; it only identifies the resident display point at one pixel.
    ///
    /// # Errors
    ///
    /// Returns an error when the frame belongs to another renderer or the pixel
    /// is outside its viewport.
    pub fn pick(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        recorded_frame: &RecordedFrame,
        request: PickRequest,
    ) -> Result<PickTicket, RendererError> {
        if !Arc::ptr_eq(&self.identity, &recorded_frame.renderer) {
            return Err(RendererError::ForeignRecordedFrame);
        }
        let frame = recorded_frame.frame;
        let viewport = frame.viewport();
        let pixel = request.pixel();
        if pixel[0] >= viewport.width() || pixel[1] >= viewport.height() {
            return Err(RendererError::PickOutsideViewport {
                pixel,
                viewport: viewport.dimensions(),
            });
        }

        self.record_frame_uniforms(
            encoder,
            &frame,
            frame.style().default_size_pixels(),
            &recorded_frame.batches,
        )?;
        let separate_pick_depth =
            self.point_footprint_status(viewport) == PointFootprintStatus::Multisample4x;
        let (depth, pick_target) =
            self.targets
                .depth_and_pick(&self.device, viewport, separate_pick_depth);
        Self::record_pick_pass(
            encoder,
            pick_target,
            depth,
            &recorded_frame.batches,
            &self.pipelines.pick,
            &self.camera_bind_group,
        );
        let (readback, receiver) =
            Self::record_pick_readback(&self.device, encoder, pick_target, pixel);
        Ok(PickTicket::new(
            frame.view_generation(),
            readback,
            receiver,
            Arc::clone(&recorded_frame.pick_table),
        ))
    }

    fn recorded_batches(&self, camera: render_protocol::Camera) -> Box<[RecordedBatch]> {
        let mut batches = self
            .batches
            .iter()
            .map(|(key, batch)| RecordedBatch::new(*key, batch))
            .collect::<Vec<_>>();
        batches.sort_by(|left, right| {
            view_depth(right.world_center, camera)
                .total_cmp(&view_depth(left.world_center, camera))
                .then_with(|| left.key.cmp(&right.key))
        });
        batches.into_boxed_slice()
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
        encoder: &mut wgpu::CommandEncoder,
        target: &PickTarget,
        depth: &DepthTarget,
        batches: &[RecordedBatch],
        pipeline: &wgpu::RenderPipeline,
        camera_bind_group: &wgpu::BindGroup,
    ) {
        let pick_attachments = [Some(wgpu::RenderPassColorAttachment {
            view: target.view(),
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
        record_point_batches(&mut pass, pipeline, camera_bind_group, batches);
    }

    fn record_pick_readback(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &PickTarget,
        pixel: [u32; 2],
    ) -> (
        wgpu::Buffer,
        mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    ) {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("punctra pick readback"),
            size: PICK_READBACK_ROW_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target.texture(),
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

    fn record_frame_uniforms(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        point_size_pixels: f32,
        batches: &[RecordedBatch],
    ) -> Result<(), RendererError> {
        let staging =
            preflight_frame_uniform_staging(batches.len(), self.device.limits().max_buffer_size)?;
        let camera = frame.camera();
        let camera_uniform = frame_camera_uniform(frame, point_size_pixels);
        let eye = camera.eye();
        let camera_bytes = bytemuck::bytes_of(&camera_uniform);
        let mut upload_bytes = Vec::with_capacity(staging.allocation_capacity);
        upload_bytes.extend_from_slice(camera_bytes);
        let mut copies = Vec::with_capacity(batches.len());
        for batch in batches {
            let mut offset = [0.0_f32; 3];
            for axis in 0..3 {
                offset[axis] =
                    camera_relative_axis(batch.world_origin[axis], eye[axis], batch.key, axis)?;
            }
            let uniform = BatchUniform {
                origin_from_camera: [offset[0], offset[1], offset[2], 0.0],
                presentation_weight: normalized_presentation_weight(batch.presentation_weight),
                _presentation_padding: [0.0; 3],
            };
            let source_offset =
                wgpu::BufferAddress::try_from(upload_bytes.len()).map_err(|_| {
                    RendererError::FrameUniformStagingSizeOverflow {
                        batch_count: batches.len(),
                    }
                })?;
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
        encoder.copy_buffer_to_buffer(&upload, 0, &self.camera_buffer, 0, staging.camera_copy_size);
        for (batch, source_offset) in copies {
            encoder.copy_buffer_to_buffer(
                &upload,
                source_offset,
                &batch.uniform_buffer,
                0,
                staging.batch_copy_size,
            );
        }
        Ok(())
    }

    fn camera_uniform_upload(
        &self,
        frame: &Frame,
        point_size_pixels: f32,
        label: &'static str,
    ) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::bytes_of(&frame_camera_uniform(frame, point_size_pixels)),
                usage: wgpu::BufferUsages::COPY_SRC,
            })
    }
}

struct RecordedBatch {
    key: BatchKey,
    world_origin: [f64; 3],
    world_center: [f64; 3],
    point_count: u32,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    presentation_weight: PresentationWeight,
}

impl RecordedBatch {
    fn new(key: BatchKey, batch: &GpuBatch) -> Self {
        Self {
            key,
            world_origin: batch.world_origin,
            world_center: batch.world_center,
            point_count: batch.point_count,
            vertex_buffer: batch.vertex_buffer.clone(),
            uniform_buffer: batch.uniform_buffer.clone(),
            bind_group: batch.bind_group.clone(),
            presentation_weight: batch.presentation_weight,
        }
    }
}

struct GpuBatch {
    world_origin: [f64; 3],
    world_center: [f64; 3],
    point_count: u32,
    point_ids: Vec<PointId>,
    gpu_points: Vec<GpuPoint>,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    presentation_weight: PresentationWeight,
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
                color: point.color(),
                flags: 0,
                pick_token,
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

        let world_origin = batch.world_origin();
        Ok(Self {
            world_origin,
            world_center: batch_world_center(world_origin, &gpu_points),
            point_count,
            point_ids,
            gpu_points,
            vertex_buffer,
            uniform_buffer,
            bind_group,
            presentation_weight: PresentationWeight::OPAQUE,
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

struct FramePass<'a> {
    encoder: &'a mut wgpu::CommandEncoder,
    target: &'a wgpu::TextureView,
    clear: [f64; 4],
    camera_buffer: &'a wgpu::Buffer,
    camera_bind_group: &'a wgpu::BindGroup,
    batches: &'a [RecordedBatch],
}

impl FramePass<'_> {
    fn stage_camera_uniform(&mut self, upload: &wgpu::Buffer) {
        self.encoder.copy_buffer_to_buffer(
            upload,
            0,
            self.camera_buffer,
            0,
            u64::try_from(size_of::<CameraUniform>())
                .expect("CameraUniform size always fits a wgpu buffer address"),
        );
    }

    fn record_points(
        &mut self,
        target: &wgpu::TextureView,
        resolve_target: Option<&wgpu::TextureView>,
        depth: &DepthTarget,
        pipelines: &PointPipelinePair,
    ) {
        record_point_pass(
            self.encoder,
            PointPassDescriptor {
                target,
                resolve_target,
                depth,
                clear: self.clear,
                pipelines,
                camera_bind_group: self.camera_bind_group,
                batches: self.batches,
            },
        );
    }

    fn record_eye_dome_depth(&mut self, depth: &DepthTarget, pipeline: &wgpu::RenderPipeline) {
        record_eye_dome_depth_pass(
            self.encoder,
            depth,
            pipeline,
            self.camera_bind_group,
            self.batches,
        );
    }

    fn record_eye_dome(
        &mut self,
        pipeline: &crate::pipeline::EdlPipeline,
        bind_group: &wgpu::BindGroup,
    ) {
        record_eye_dome_pass(self.encoder, self.target, pipeline, bind_group);
    }
}

fn batch_world_center(world_origin: [f64; 3], points: &[GpuPoint]) -> [f64; 3] {
    let mut minimum = points[0].position;
    let mut maximum = points[0].position;
    for point in &points[1..] {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(point.position[axis]);
            maximum[axis] = maximum[axis].max(point.position[axis]);
        }
    }
    std::array::from_fn(|axis| {
        world_origin[axis] + (f64::from(minimum[axis]) + f64::from(maximum[axis])) * 0.5
    })
}

fn view_depth(world_position: [f64; 3], camera: render_protocol::Camera) -> f64 {
    let eye = camera.eye();
    let forward = camera.world_basis().forward();
    (0..3)
        .map(|axis| (world_position[axis] - eye[axis]) * forward[axis])
        .sum()
}

fn normalized_presentation_weight(weight: PresentationWeight) -> f32 {
    f32::from(weight.get()) / f32::from(u8::MAX)
}

fn record_point_batches<'pass>(
    pass: &mut wgpu::RenderPass<'pass>,
    pipeline: &'pass wgpu::RenderPipeline,
    camera_bind_group: &'pass wgpu::BindGroup,
    batches: impl IntoIterator<Item = &'pass RecordedBatch>,
) {
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, camera_bind_group, &[]);
    for batch in batches {
        pass.set_bind_group(1, &batch.bind_group, &[]);
        pass.set_vertex_buffer(0, batch.vertex_buffer.slice(..));
        pass.draw(0..6, 0..batch.point_count);
    }
}

#[derive(Clone, Copy)]
struct PointPassDescriptor<'a> {
    target: &'a wgpu::TextureView,
    resolve_target: Option<&'a wgpu::TextureView>,
    depth: &'a DepthTarget,
    clear: [f64; 4],
    pipelines: &'a PointPipelinePair,
    camera_bind_group: &'a wgpu::BindGroup,
    batches: &'a [RecordedBatch],
}

fn record_point_pass(encoder: &mut wgpu::CommandEncoder, descriptor: PointPassDescriptor<'_>) {
    let color_attachments = [Some(wgpu::RenderPassColorAttachment {
        view: descriptor.target,
        depth_slice: None,
        resolve_target: descriptor.resolve_target,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color {
                r: descriptor.clear[0],
                g: descriptor.clear[1],
                b: descriptor.clear[2],
                a: descriptor.clear[3],
            }),
            store: wgpu::StoreOp::Store,
        },
    })];
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("punctra point pass"),
        color_attachments: &color_attachments,
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: descriptor.depth.view(),
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
    record_point_batches(
        &mut pass,
        &descriptor.pipelines.opaque,
        descriptor.camera_bind_group,
        descriptor
            .batches
            .iter()
            .filter(|batch| batch.presentation_weight == PresentationWeight::OPAQUE),
    );
    record_point_batches(
        &mut pass,
        &descriptor.pipelines.translucent,
        descriptor.camera_bind_group,
        descriptor
            .batches
            .iter()
            .filter(|batch| batch.presentation_weight != PresentationWeight::OPAQUE),
    );
}

fn record_eye_dome_pass(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &crate::pipeline::EdlPipeline,
    bind_group: &wgpu::BindGroup,
) {
    let color_attachments = [Some(wgpu::RenderPassColorAttachment {
        view: target,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    })];
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("punctra eye-dome pass"),
        color_attachments: &color_attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&pipeline.pipeline);
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

fn record_eye_dome_depth_pass(
    encoder: &mut wgpu::CommandEncoder,
    depth: &DepthTarget,
    pipeline: &wgpu::RenderPipeline,
    camera_bind_group: &wgpu::BindGroup,
    batches: &[RecordedBatch],
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("punctra eye-dome visibility depth pass"),
        color_attachments: &[],
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
    record_point_batches(&mut pass, pipeline, camera_bind_group, batches);
}

fn point_buffer(device: &wgpu::Device, points: &[GpuPoint]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("punctra point batch"),
        contents: bytemuck::cast_slice(points),
        usage: wgpu::BufferUsages::VERTEX,
    })
}

fn depth_cue_status(device: &wgpu::Device, config: RendererConfig) -> DepthCueStatus {
    if config.eye_dome_lighting.is_none() {
        return DepthCueStatus::Disabled;
    }
    let color_is_four_bytes = matches!(
        config.color_format,
        wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb
    );
    let color_sampleable = config
        .color_format
        .guaranteed_format_features(device.features())
        .allowed_usages
        .contains(wgpu::TextureUsages::TEXTURE_BINDING);
    let depth_sampleable = DEPTH_FORMAT
        .guaranteed_format_features(device.features())
        .allowed_usages
        .contains(wgpu::TextureUsages::TEXTURE_BINDING);
    if color_is_four_bytes && color_sampleable && depth_sampleable {
        DepthCueStatus::Active
    } else {
        DepthCueStatus::UnsupportedFallback
    }
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
    /// The combined frame-uniform staging size cannot be represented safely.
    #[error("frame uniform staging size cannot be represented for {batch_count} batches")]
    FrameUniformStagingSizeOverflow {
        /// The number of batch uniforms requested alongside the camera uniform.
        batch_count: usize,
    },
    /// The combined frame-uniform staging buffer exceeds the device limit.
    #[error(
        "frame uniforms for {batch_count} batches need a {requested_bytes}-byte staging buffer, exceeding the device limit {max_buffer_size}"
    )]
    FrameUniformStagingBufferTooLarge {
        /// The number of batch uniforms requested alongside the camera uniform.
        batch_count: usize,
        /// Exact combined bytes required by the camera and batch uniforms.
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
    /// A recorded frame was passed to a renderer other than the one that made it.
    #[error("recorded frame belongs to another renderer")]
    ForeignRecordedFrame,
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
fn viewport_as_f32(viewport: Viewport) -> [f32; 2] {
    [viewport.width() as f32, viewport.height() as f32]
}

fn frame_camera_uniform(frame: &Frame, point_size_pixels: f32) -> CameraUniform {
    CameraUniform {
        view_projection: frame.view_projection().to_cols_array_2d(),
        viewport_size: viewport_as_f32(frame.viewport()),
        default_point_size: point_size_pixels,
        _padding: 0.0,
        highlight_color: frame.style().highlight_color(),
        _highlight_padding: 0.0,
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FrameUniformStagingLayout {
    camera_copy_size: wgpu::BufferAddress,
    batch_copy_size: wgpu::BufferAddress,
    allocation_capacity: usize,
}

fn preflight_frame_uniform_staging(
    batch_count: usize,
    max_buffer_size: wgpu::BufferAddress,
) -> Result<FrameUniformStagingLayout, RendererError> {
    let size_overflow = || RendererError::FrameUniformStagingSizeOverflow { batch_count };
    let camera_bytes =
        wgpu::BufferAddress::try_from(size_of::<CameraUniform>()).map_err(|_| size_overflow())?;
    let batch_bytes =
        wgpu::BufferAddress::try_from(size_of::<BatchUniform>()).map_err(|_| size_overflow())?;
    let batch_count_address =
        wgpu::BufferAddress::try_from(batch_count).map_err(|_| size_overflow())?;
    let total_bytes = batch_count_address
        .checked_mul(batch_bytes)
        .and_then(|bytes| camera_bytes.checked_add(bytes))
        .ok_or_else(size_overflow)?;

    if total_bytes > max_buffer_size {
        return Err(RendererError::FrameUniformStagingBufferTooLarge {
            batch_count,
            requested_bytes: total_bytes,
            max_buffer_size,
        });
    }

    let allocation_bytes = usize::try_from(total_bytes).map_err(|_| size_overflow())?;
    Ok(FrameUniformStagingLayout {
        camera_copy_size: camera_bytes,
        batch_copy_size: batch_bytes,
        allocation_capacity: allocation_bytes,
    })
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
pub(crate) mod point_footprint_test_support {
    use std::{fmt::Write as _, time::Duration};

    use render_protocol::{BatchVersion, PointId, RenderPoint, SourceId, ViewId};
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    use super::*;
    use crate::{
        PickHit, PickPoll, PointStyle,
        footprint::MAX_TRANSIENT_TEXTURE_BYTES,
        gpu_support::{GpuContext, Rgba8Image, Rgba8Target, with_gpu},
    };

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    const SINGLE_SAMPLE_VIEWPORT: [u32; 2] = [64, 64];
    const CLEAR: [u8; 4] = [0, 0, 0, 255];
    const SOURCE: SourceId = SourceId::new([0x21; 32]);
    const POINT_ORDINALS: [u64; 2] = [1_866, 2_005];
    const BATCH_KEY: u64 = 4;
    const BATCH_VERSION: u64 = 2;
    const VIEW_GENERATION: u64 = 1;

    #[derive(Clone, Copy)]
    pub(crate) enum TestFootprintPath {
        SingleSample,
        UnsupportedFallback,
    }

    pub(crate) struct TestFootprintMeasurement {
        pub(crate) environment: Value,
        pub(crate) facts: Value,
    }

    pub(crate) fn measure(path: TestFootprintPath) -> TestFootprintMeasurement {
        let mut proof = None;
        with_gpu(|gpu| proof = Some(measure_with_gpu(gpu, path)));
        proof.expect("private Point-footprint evidence requires a GPU adapter")
    }

    pub(crate) fn assert_resource_bounded_single_sample_eye_dome() {
        with_gpu(|gpu| {
            let limits = RenderLimits::new(1_024 * 1_024, 1_024, 16);
            let eye_dome = EyeDomeLighting::new(1.0, 1).unwrap();
            let config = RendererConfig::new(FORMAT, limits).with_eye_dome_lighting(eye_dome);
            let mut renderer = configured_fixture_renderer(gpu, config);
            renderer.point_footprint = PointFootprintPlan::forced_for_test(
                PointFootprint::SingleSample,
                true,
                48,
                MAX_TRANSIENT_TEXTURE_BYTES,
            );
            let viewport = SINGLE_SAMPLE_VIEWPORT;
            let pixels = u64::from(viewport[0]) * u64::from(viewport[1]);
            let frame = fixture_frame(viewport);

            let (recorded, image) = render(gpu, &mut renderer, &frame, viewport);
            assert_eq!(
                renderer.point_footprint_status(frame.viewport()),
                PointFootprintStatus::SingleSample
            );
            assert!(!recorded.report().eye_dome_lighting_applied());
            assert_eq!(recorded.report().transient_texture_bytes(), pixels * 4);

            let probe_pixels = fixture_probe_pixels(&image);
            for (&point_ordinal, &pixel) in POINT_ORDINALS.iter().zip(&probe_pixels) {
                let hit = pick(gpu, &mut renderer, &recorded, pixel)
                    .expect("the resource-bounded SingleSample path should remain pickable");
                assert_eq!(identity_json(&hit), fixture_identity(point_ordinal));
            }
            assert_eq!(renderer.transient_texture_bytes(), pixels * 8);
        });
    }

    fn measure_with_gpu(gpu: &GpuContext, path: TestFootprintPath) -> TestFootprintMeasurement {
        let mut reference = fixture_renderer(gpu, TestFootprintPath::SingleSample);
        let mut observed = fixture_renderer(gpu, path);
        let viewport = SINGLE_SAMPLE_VIEWPORT;
        let frame = fixture_frame(viewport);
        let expected_status = match path {
            TestFootprintPath::SingleSample => PointFootprintStatus::SingleSample,
            TestFootprintPath::UnsupportedFallback => PointFootprintStatus::UnsupportedFallback,
        };
        assert_eq!(
            observed.point_footprint_status(frame.viewport()),
            expected_status
        );
        assert!(observed.pipelines.multisample.is_none());

        let (reference_frame, reference_image) = render(gpu, &mut reference, &frame, viewport);
        let (observed_frame, observed_image) = render(gpu, &mut observed, &frame, viewport);
        let pixels = u64::from(viewport[0]) * u64::from(viewport[1]);
        assert_eq!(
            observed_frame.report().transient_texture_bytes(),
            pixels * 4
        );
        let reference_mask = foreground_mask(&reference_image, viewport);
        let observed_mask = foreground_mask(&observed_image, viewport);
        let reference_sha256 = sha256_hex(&reference_mask);
        let observed_sha256 = sha256_hex(&observed_mask);
        assert_eq!(reference_mask, observed_mask);

        let probe_pixels = fixture_probe_pixels(&reference_image);
        let mut observed_identities = Vec::with_capacity(POINT_ORDINALS.len());
        let mut observed_pick_probes = Vec::with_capacity(POINT_ORDINALS.len());
        for (&point_ordinal, &pixel) in POINT_ORDINALS.iter().zip(&probe_pixels) {
            let expected_hit = fixture_identity(point_ordinal);
            let reference_hit = pick(gpu, &mut reference, &reference_frame, pixel)
                .expect("the SingleSample reference must preserve every preferred pick identity");
            assert_eq!(identity_json(&reference_hit), expected_hit);
            let observed_hit = pick(gpu, &mut observed, &observed_frame, pixel)
                .expect("the selected fallback path must preserve every preferred pick identity");
            let observed_identity = identity_json(&observed_hit);
            let observed_pick_probe = pick_probe_json(&observed_hit);
            assert_eq!(observed_identity, expected_hit);
            assert_eq!(observed_pick_probe, fixture_pick_probe(point_ordinal));
            observed_identities.push(observed_identity);
            observed_pick_probes.push(observed_pick_probe);
        }
        assert_eq!(observed.transient_texture_bytes(), pixels * 8);

        let adapter_info = gpu.device.adapter_info();
        let adapter_name = if adapter_info.name.trim().is_empty() {
            "local wgpu adapter".to_owned()
        } else {
            adapter_info.name
        };
        TestFootprintMeasurement {
            environment: serde_json::json!({
                "operating_system": std::env::consts::OS,
                "adapter_name": adapter_name,
                "backend": format!("{:?}", adapter_info.backend),
            }),
            facts: serde_json::json!({
                "hard_circle_mask": {
                    "width": viewport[0],
                    "height": viewport[1],
                    "byte_length": reference_mask.len(),
                    "reference_sha256": reference_sha256,
                    "observed_sha256": observed_sha256,
                    "equivalent": true,
                },
                "pick_probes": observed_pick_probes,
                "nominal_pick_identity": {
                    "expected": fixture_identity(POINT_ORDINALS[0]),
                    "observed": observed_identities[0],
                    "matched": true,
                },
            }),
        }
    }

    fn fixture_renderer(gpu: &GpuContext, path: TestFootprintPath) -> WgpuRenderer {
        let limits = RenderLimits::new(1_024 * 1_024, 1_024, 16);
        let config = RendererConfig::new(FORMAT, limits);
        let mut renderer = configured_fixture_renderer(gpu, config);
        if matches!(path, TestFootprintPath::UnsupportedFallback) {
            renderer.point_footprint =
                PointFootprintPlan::forced_for_test(PointFootprint::Antialiased, false, 40, 12);
        }
        renderer
    }

    fn configured_fixture_renderer(gpu: &GpuContext, config: RendererConfig) -> WgpuRenderer {
        let mut renderer = WgpuRenderer::new(&gpu.device, config)
            .expect("the private fallback fixture renderer should attach");
        let view_generation = fixture_view_generation();
        renderer
            .apply(&RenderUpdate::Reset { view_generation })
            .expect("the private fallback fixture reset should apply");
        let points = vec![
            RenderPoint::new(
                [-1.0, 0.0, 0.0],
                [255, 0, 0, 255],
                PointId::new(SOURCE, POINT_ORDINALS[0]),
            )
            .expect("the first private fallback fixture point should be valid"),
            RenderPoint::new(
                [1.0, 0.0, 0.0],
                [0, 255, 255, 255],
                PointId::new(SOURCE, POINT_ORDINALS[1]),
            )
            .expect("the second private fallback fixture point should be valid"),
        ];
        let batch = PointBatch::new(
            view_generation,
            BatchKey::new(BATCH_KEY),
            BatchVersion::new(BATCH_VERSION),
            [0.0; 3],
            points,
        )
        .expect("the private fallback fixture batch should be valid");
        renderer
            .apply(&RenderUpdate::Upsert { batch })
            .expect("the private fallback fixture batch should apply");
        renderer
    }

    fn fixture_frame(viewport: [u32; 2]) -> Frame {
        let camera = render_protocol::Camera::perspective(
            [0.0, -5.0, 0.0],
            [0.0; 3],
            [0.0, 0.0, 1.0],
            std::f32::consts::FRAC_PI_3,
            0.1,
            100.0,
        )
        .expect("the private fallback fixture camera should be valid");
        let style = PointStyle::new(7.0, [1.0; 3], [0.0, 0.0, 0.0, 1.0])
            .expect("the private fallback fixture style should be valid")
            .with_display_size_pixels(18.0)
            .expect("the private fallback display size should be valid");
        Frame::new(
            fixture_view_generation(),
            camera,
            Viewport::new(viewport[0], viewport[1]).unwrap(),
        )
        .expect("the private fallback fixture frame should be valid")
        .with_style(style)
    }

    fn render(
        gpu: &GpuContext,
        renderer: &mut WgpuRenderer,
        frame: &Frame,
        viewport: [u32; 2],
    ) -> (RecordedFrame, Rgba8Image) {
        let target = Rgba8Target::new(
            &gpu.device,
            viewport,
            FORMAT,
            "punctra private fallback evidence target",
        );
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("punctra private fallback evidence encoder"),
            });
        let recorded = renderer
            .render(&mut encoder, &target.view, frame)
            .expect("the private fallback fixture should render");
        target.encode_copy(&mut encoder);
        let receiver = target.map_after_submit(&mut encoder);
        gpu.queue.submit([encoder.finish()]);
        gpu.wait();
        (recorded, target.read(&receiver))
    }

    fn pick(
        gpu: &GpuContext,
        renderer: &mut WgpuRenderer,
        frame: &RecordedFrame,
        pixel: [u32; 2],
    ) -> Option<PickHit> {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("punctra private fallback pick encoder"),
            });
        let mut ticket = renderer
            .pick(&mut encoder, frame, PickRequest::new(pixel))
            .expect("the private fallback fixture pick should encode");
        let submission = gpu.queue.submit([encoder.finish()]);
        gpu.wait_for_submission(
            &submission,
            Duration::from_secs(2),
            "private fallback pick",
            || match ticket
                .poll()
                .expect("the private fallback pick should resolve")
            {
                PickPoll::Ready(hit) => Some(hit),
                PickPoll::Pending => None,
            },
        )
    }

    fn foreground_mask(image: &Rgba8Image, viewport: [u32; 2]) -> Vec<u8> {
        let mut mask = Vec::with_capacity(
            usize::try_from(u64::from(viewport[0]) * u64::from(viewport[1])).unwrap(),
        );
        for y in 0..viewport[1] {
            for x in 0..viewport[0] {
                mask.push(u8::from(image.pixel([x, y]) != CLEAR));
            }
        }
        mask
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut encoded = String::with_capacity(64);
        for byte in digest {
            write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }
        encoded
    }

    fn fixture_view_generation() -> ViewGenerationKey {
        ViewGenerationKey::new(ViewId::new(22), VIEW_GENERATION)
    }

    fn fixture_probe_pixels(image: &Rgba8Image) -> [[u32; 2]; 2] {
        [
            color_centroid_pixel(
                image,
                |pixel| pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40,
                "the preferred ordinal 1866 should render red",
            ),
            color_centroid_pixel(
                image,
                |pixel| pixel[0] < 40 && pixel[1] > 200 && pixel[2] > 200,
                "the preferred ordinal 2005 should render cyan",
            ),
        ]
    }

    fn color_centroid_pixel(
        image: &Rgba8Image,
        predicate: impl Fn([u8; 4]) -> bool,
        missing_message: &str,
    ) -> [u32; 2] {
        let mut sum = [0_u64; 2];
        let mut count = 0_u64;
        for y in 0..SINGLE_SAMPLE_VIEWPORT[1] {
            for x in 0..SINGLE_SAMPLE_VIEWPORT[0] {
                if predicate(image.pixel([x, y])) {
                    sum[0] += u64::from(x);
                    sum[1] += u64::from(y);
                    count += 1;
                }
            }
        }
        assert!(count > 0, "{missing_message}");
        [
            u32::try_from((sum[0] + count / 2) / count).unwrap(),
            u32::try_from((sum[1] + count / 2) / count).unwrap(),
        ]
    }

    fn fixture_identity(point_ordinal: u64) -> Value {
        serde_json::json!({
            "generation": VIEW_GENERATION,
            "source_identity": SOURCE.to_string(),
            "batch_key": BATCH_KEY,
            "batch_version": BATCH_VERSION,
            "point_ordinal": point_ordinal,
        })
    }

    fn fixture_pick_probe(point_ordinal: u64) -> Value {
        serde_json::json!({
            "ordinal": point_ordinal,
            "generation": VIEW_GENERATION,
            "source_identity": SOURCE.to_string(),
            "batch_key": BATCH_KEY,
            "batch_version": BATCH_VERSION,
            "point_ordinal": point_ordinal.to_string(),
        })
    }

    fn identity_json(hit: &PickHit) -> Value {
        serde_json::json!({
            "generation": hit.view_generation().generation(),
            "source_identity": hit.point().source().to_string(),
            "batch_key": hit.batch().get(),
            "batch_version": hit.version().get(),
            "point_ordinal": hit.point().ordinal(),
        })
    }

    fn pick_probe_json(hit: &PickHit) -> Value {
        serde_json::json!({
            "ordinal": hit.point().ordinal(),
            "generation": hit.view_generation().generation(),
            "source_identity": hit.point().source().to_string(),
            "batch_key": hit.batch().get(),
            "batch_version": hit.version().get(),
            "point_ordinal": hit.point().ordinal().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_eq_implementation<T: Eq>() {}

    #[test]
    fn renderer_config_preserves_exact_equality() {
        assert_eq_implementation::<RendererConfig>();
    }

    #[test]
    fn resource_bounded_single_sample_suppresses_eye_dome_targets() {
        point_footprint_test_support::assert_resource_bounded_single_sample_eye_dome();
    }

    #[test]
    fn eye_dome_lighting_accepts_exact_bounds() {
        let minimum = EyeDomeLighting::new(f32::MIN_POSITIVE, 1).unwrap();
        assert_eq!(minimum.strength().to_bits(), f32::MIN_POSITIVE.to_bits());
        assert_eq!(minimum.radius_pixels(), 1);

        let maximum = EyeDomeLighting::new(10.0, 8).unwrap();
        assert_eq!(maximum.strength().to_bits(), 10.0_f32.to_bits());
        assert_eq!(maximum.radius_pixels(), 8);
    }

    #[test]
    fn eye_dome_lighting_rejects_invalid_strengths() {
        for strength in [f32::NAN, f32::NEG_INFINITY, f32::INFINITY, -1.0, 0.0, 10.1] {
            assert_eq!(
                EyeDomeLighting::new(strength, 1),
                Err(DepthCueError::InvalidStrength)
            );
        }
    }

    #[test]
    fn eye_dome_lighting_rejects_invalid_radii() {
        for radius_pixels in [0, 9, u32::MAX] {
            assert_eq!(
                EyeDomeLighting::new(1.0, radius_pixels),
                Err(DepthCueError::InvalidRadius)
            );
        }
    }

    #[test]
    fn frame_uniform_staging_accepts_the_exact_device_limit() {
        let batch_count = 3;
        let camera_bytes = wgpu::BufferAddress::try_from(size_of::<CameraUniform>()).unwrap();
        let batch_bytes = wgpu::BufferAddress::try_from(size_of::<BatchUniform>()).unwrap();
        let exact_limit = camera_bytes + 3 * batch_bytes;

        let layout = preflight_frame_uniform_staging(batch_count, exact_limit).unwrap();

        assert_eq!(
            layout,
            FrameUniformStagingLayout {
                camera_copy_size: camera_bytes,
                batch_copy_size: batch_bytes,
                allocation_capacity: usize::try_from(exact_limit).unwrap(),
            }
        );
    }

    #[test]
    fn frame_uniform_staging_rejects_a_device_limit_one_byte_too_small() {
        let batch_count = 3;
        let camera_bytes = wgpu::BufferAddress::try_from(size_of::<CameraUniform>()).unwrap();
        let batch_bytes = wgpu::BufferAddress::try_from(size_of::<BatchUniform>()).unwrap();
        let requested_bytes = camera_bytes + 3 * batch_bytes;
        let max_buffer_size = requested_bytes - 1;

        assert!(matches!(
            preflight_frame_uniform_staging(batch_count, max_buffer_size),
            Err(RendererError::FrameUniformStagingBufferTooLarge {
                batch_count: rejected_count,
                requested_bytes: rejected_bytes,
                max_buffer_size: rejected_limit,
            }) if rejected_count == batch_count
                && rejected_bytes == requested_bytes
                && rejected_limit == max_buffer_size
        ));
    }

    #[test]
    fn frame_uniform_staging_rejects_arithmetic_or_allocation_size_overflow() {
        assert!(matches!(
            preflight_frame_uniform_staging(usize::MAX, wgpu::BufferAddress::MAX),
            Err(RendererError::FrameUniformStagingSizeOverflow {
                batch_count: usize::MAX,
            })
        ));
    }

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
