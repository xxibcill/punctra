//! Bounded wgpu rendering for progressive point-cloud Views.
//!
//! The host owns the wgpu device, queue, command encoder, target texture, and
//! submission schedule. The renderer owns generation-safe point residency and
//! records draw and pick commands into the host's encoder.
//!
//! # Example
//!
//! ```no_run
//! use render_protocol::{
//!     BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch, PointId,
//!     RenderLimits, RenderPoint, RenderUpdate, SourceId, ViewGenerationKey, ViewId, Viewport,
//! };
//! use render_wgpu::{Camera, Frame, RendererConfig, WgpuRenderer};
//!
//! # fn record(
//! #     device: &wgpu::Device,
//! #     encoder: &mut wgpu::CommandEncoder,
//! #     target: &wgpu::TextureView,
//! #     target_format: wgpu::TextureFormat,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let view_generation = ViewGenerationKey::new(ViewId::new(7), 1);
//! let limits = RenderLimits::new(
//!     ESTIMATED_GPU_BYTES_PER_POINT * 1_000_000,
//!     1_000_000,
//!     256,
//! );
//! let mut renderer = WgpuRenderer::new(device, RendererConfig::new(target_format, limits))?;
//! renderer.apply(&RenderUpdate::Reset { view_generation })?;
//!
//! let source = SourceId::new([0x42; 32]);
//! let point = RenderPoint::new(
//!     [0.0, 0.0, 0.0],
//!     [80, 180, 255, 255],
//!     PointId::new(source, 42),
//! )?;
//! let batch = PointBatch::new(
//!     view_generation,
//!     BatchKey::new(3),
//!     BatchVersion::new(1),
//!     [500_000.0, 6_000_000.0, 120.0],
//!     vec![point],
//! )?;
//! renderer.apply(&RenderUpdate::Upsert { batch })?;
//!
//! let camera = Camera::perspective(
//!     [500_000.0, 5_999_995.0, 120.0],
//!     [500_000.0, 6_000_000.0, 120.0],
//!     [0.0, 0.0, 1.0],
//!     std::f32::consts::FRAC_PI_3,
//!     0.1,
//!     1_000.0,
//! )?;
//! let viewport = Viewport::new(1280, 720)?;
//! let frame = Frame::new(view_generation, camera, viewport)?;
//! let recorded_frame = renderer.render(encoder, target, &frame)?;
//! assert_eq!(recorded_frame.report().drawn_points(), 1);
//! # Ok(())
//! # }
//! ```

mod frame;
mod gpu;
mod pick;
mod pipeline;
mod renderer;
mod targets;

pub use frame::{Frame, FrameError, PointStyle};
pub use pick::{PickError, PickHit, PickPoll, PickRequest, PickTicket};
pub use render_protocol::{Camera, CameraBasis, CameraError, Viewport, ViewportError};
pub use renderer::{FrameReport, RecordedFrame, RendererConfig, RendererError, WgpuRenderer};
