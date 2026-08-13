//! Bounded wgpu rendering for progressive point-cloud Views.
//!
//! The host owns the wgpu device, queue, command encoder, target texture, and
//! submission schedule. The renderer owns generation-safe point residency and
//! records draw and pick commands into the host's encoder.
//!
//! # Host ownership
//!
//! A host creates and retains the [`wgpu::Instance`], [`wgpu::Device`], and
//! [`wgpu::Queue`]. It also creates every command encoder and render target,
//! chooses when command buffers are submitted, drives device polling, and owns
//! device-loss policy. [`WgpuRenderer`] accepts the device by reference, retains
//! a device handle, and records work into host-provided encoders; it never
//! submits the queue.
//!
//! The caller also selects hard logical residency limits through
//! [`render_protocol::RenderLimits`]. Those limits cover resident point vertex
//! bytes, points, batches, and complete highlight-update input. Render targets,
//! command encoders, staging owned by the host, allocator padding, and other
//! wgpu resources are outside that accounting and require separate host policy.
//!
//! # Provisional picking and exact confirmation
//!
//! [`WgpuRenderer::pick`] identifies a resident display sample from one
//! [`RecordedFrame`]. A [`PickHit`] preserves the producing View generation,
//! batch, batch version, and canonical Point identity, but remains a GPU hint.
//! It does not prove that the Point still belongs to a current selection or
//! report its effective classification. A miss is likewise not proof that an
//! exact Query would find no Point because progressive display Coverage may be
//! incomplete.
//!
//! A Workspace host must track View generation and Workspace Revision as
//! separate freshness dimensions. At interaction capture it pins the intended
//! `point_workspace::Snapshot` and records both identities. Before inspection
//! or Edit it rejects a hit when the active View generation, host interaction
//! generation, or Workspace head Revision changed. It then passes only
//! `PickHit::point()` to `point_review::confirm_pick` under caller-selected
//! `point_review::ScreenReviewLimits`. Renderer position, depth, color, and
//! classification remain non-authoritative. This renderer crate intentionally
//! has no Workspace/review dependency; the compiled `no_run` example in the
//! `point-review` crate documents the exact composition. In pseudocode:
//!
//! ```text
//! capture = (active_view_generation, workspace.head())
//! hit = render_wgpu_pick(captured_frame)
//! require(hit.view_generation == capture.view_generation)
//! require(active_view_generation and workspace.head_revision still match capture)
//! confirmed = point_review.confirm_pick(capture.snapshot, hit.point, review_limits)
//! require(confirmed.provenance == capture.snapshot.provenance)
//! ```
//!
//! Exact multi-Point highlights require another complete-only handoff. Iterate
//! the terminal `point_workspace::PointSet` with explicit
//! `point_workspace::PointIdReadLimits`, retain the complete identity vector
//! under a host byte ceiling, and apply one [`render_protocol::RenderUpdate::SetHighlights`]
//! only after the iterator returns terminal success. A read or limit failure
//! must leave the previous renderer state untouched. The
//! [`render_protocol::RenderLimits`] highlight ceiling bounds accepted input
//! count; host vector bytes and Point-Set read/working bytes are separate
//! caller policy.
//!
//! For mutation recovery, retain the caller-owned Operation identity. A
//! rejected commit is terminal, an indeterminate commit requires explicit
//! close/reopen plus resolution of that same Operation, and a committed receipt
//! remains committed even if later audit reporting fails. A host may choose the
//! stricter policy of withholding dependent mutation until its audit is
//! available; it must not retry the committed Operation as if it had failed.
//!
//! The standalone `third_party_host` example demonstrates the renderer
//! lifecycle without depending on private `renderer-demo` state:
//!
//! ```text
//! cargo run -p render-wgpu --example third_party_host
//! ```
//!
//! # Interface classification
//!
//! The documented renderer configuration, frame, update, picking, report, and
//! error APIs are a **v1-candidate foundation surface**. Shader layout,
//! allocation strategy, and submission policy are not interfaces. This records
//! v0.9 review intent, not a `1.0.0` or production-support claim.
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
pub use render_protocol::{
    Camera, CameraBasis, CameraError, CameraProjection, Viewport, ViewportError,
};
pub use renderer::{FrameReport, RecordedFrame, RendererConfig, RendererError, WgpuRenderer};
