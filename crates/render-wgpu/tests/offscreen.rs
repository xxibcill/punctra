//! GPU acceptance coverage on an available headless adapter.

use std::sync::mpsc;

#[path = "../../../tests/support/gpu.rs"]
mod gpu_support;

use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT as POINT_BYTES, PointBatch, PointId,
    PresentationWeight, ProtocolError, RenderLimits, RenderPoint, RenderUpdate, ResidentResource,
    SourceId, UpdateKind, UpdateReport, ViewGenerationKey, ViewId, Viewport,
};
use render_wgpu::{
    Camera, DepthCueStatus, EyeDomeLighting, Frame, FrameReport, PickError, PickHit, PickPoll,
    PickRequest, PickTicket, PointStyle, RecordedFrame, RendererConfig, RendererError,
    WgpuRenderer,
};

use gpu_support::{GpuContext, with_gpu};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const VIEWPORT: [u32; 2] = [64, 64];
const RESIZED_VIEWPORT: [u32; 2] = [96, 80];
const CENTER: [u32; 2] = [32, 32];
const WORLD_ORIGIN: [f64; 3] = [1_000_000_000.125, 1_000_000_000.25, 1_000_000_000.5];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const CYAN: [u8; 4] = [0, 255, 255, 255];
const TRANSPARENT: [u8; 4] = [255, 255, 255, 0];
const TWO_POINT_BYTES: u64 = 2 * POINT_BYTES;
const TEST_SOURCE: SourceId = SourceId::new([0x55; 32]);

#[test]
fn lifecycle_updates_are_atomic_in_gpu_state() {
    with_gpu(assert_lifecycle_updates_are_atomic);
}

#[test]
fn depth_highlights_circular_splats_and_pick_identity_are_preserved() {
    with_gpu(assert_raster_and_pick_semantics);
}

#[test]
fn highlights_preserve_source_alpha() {
    with_gpu(assert_highlight_alpha_preservation);
}

#[test]
fn millimeter_separation_survives_a_billion_unit_world_origin() {
    with_gpu(assert_large_world_precision);
}

#[test]
fn orthographic_screen_position_is_independent_of_depth() {
    with_gpu(assert_orthographic_depth_independent_position);
}

#[test]
fn orthographic_depth_and_pick_identity_survive_a_billion_unit_origin() {
    with_gpu(assert_orthographic_depth_and_pick);
}

#[test]
fn asynchronous_pick_tickets_survive_reset_and_resize() {
    with_gpu(assert_async_ticket_stability);
}

#[test]
fn frames_recorded_before_one_submit_keep_their_exact_cameras() {
    with_gpu(assert_deferred_frame_camera_stability);
}

#[test]
fn recorded_frames_keep_replaced_batch_data_and_identity() {
    with_gpu(assert_recorded_frame_replacement_stability);
}

#[test]
fn recorded_frames_are_bound_to_the_renderer_that_created_them() {
    with_gpu(assert_foreign_recorded_frame_rejected);
}

#[test]
fn presentation_weight_changes_color_but_not_pick_coverage() {
    with_gpu(assert_presentation_weight_is_color_only);
}

#[test]
fn eye_dome_lighting_has_bounded_active_and_fallback_paths() {
    with_gpu(assert_eye_dome_paths);
}

fn assert_lifecycle_updates_are_atomic(gpu: &GpuContext) {
    let limits = RenderLimits::new(POINT_BYTES, 1, 1);
    let mut subject = OffscreenRenderer::new(gpu, limits);
    let generation_one = ViewGenerationKey::new(ViewId::new(1), 1);
    let generation_two = ViewGenerationKey::new(ViewId::new(1), 2);

    let reset = subject.apply(&RenderUpdate::Reset {
        view_generation: generation_one,
    });
    assert_eq!(reset.kind(), UpdateKind::Reset);

    let red_batch = batch(
        generation_one,
        1,
        1,
        WORLD_ORIGIN,
        vec![point([0.0; 3], RED, 10)],
    );
    let inserted = subject.apply(&RenderUpdate::Upsert { batch: red_batch });
    assert_eq!(inserted.kind(), UpdateKind::BatchInserted);
    assert_eq!(inserted.resident().point_count(), 1);

    let oversized_replacement = batch(
        generation_one,
        1,
        2,
        WORLD_ORIGIN,
        vec![point([0.0; 3], BLUE, 20), point([0.1, 0.0, 0.0], BLUE, 21)],
    );
    let error = subject
        .try_apply(&RenderUpdate::Upsert {
            batch: oversized_replacement,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        RendererError::Protocol(ProtocolError::ResidentLimitExceeded {
            resource: ResidentResource::EstimatedGpuBytes,
            limit: POINT_BYTES,
            attempted: TWO_POINT_BYTES,
        })
    ));

    let frame_one = standard_frame(generation_one, VIEWPORT, 14.0, GREEN);
    let after_rejection = subject.render(&frame_one);
    assert_eq!(after_rejection.report.drawn_points(), 1);
    assert_pixel(after_rejection.image.pixel(CENTER), RED);

    let blue_batch = batch(
        generation_one,
        1,
        2,
        WORLD_ORIGIN,
        vec![point([0.0; 3], BLUE, 20)],
    );
    let replaced = subject.apply(&RenderUpdate::Upsert { batch: blue_batch });
    assert_eq!(replaced.kind(), UpdateKind::BatchReplaced);
    assert_eq!(replaced.removed_points(), 1);
    assert_pixel(subject.render(&frame_one).image.pixel(CENTER), BLUE);

    let wrong_remove = RenderUpdate::Remove {
        view_generation: generation_one,
        key: BatchKey::new(1),
        expected_version: BatchVersion::new(1),
    };
    assert!(matches!(
        subject.try_apply(&wrong_remove),
        Err(RendererError::Protocol(
            ProtocolError::BatchVersionMismatch { .. }
        ))
    ));
    let removed = subject.apply(&RenderUpdate::Remove {
        view_generation: generation_one,
        key: BatchKey::new(1),
        expected_version: BatchVersion::new(2),
    });
    assert_eq!(removed.kind(), UpdateKind::BatchRemoved);
    let empty = subject.render(&frame_one);
    assert_eq!(empty.report.drawn_points(), 0);
    assert_eq!(empty.report.draw_calls(), 0);
    assert_pixel(empty.image.pixel(CENTER), BLACK);

    subject.apply(&RenderUpdate::Reset {
        view_generation: generation_two,
    });
    assert!(matches!(
        subject.try_apply(&RenderUpdate::Reset {
            view_generation: generation_one,
        }),
        Err(RendererError::Protocol(
            ProtocolError::StaleGeneration { .. }
        ))
    ));
    assert!(matches!(
        subject.try_render(&frame_one),
        Err(RendererError::ViewGenerationMismatch { .. })
    ));
    let frame_two = standard_frame(generation_two, VIEWPORT, 14.0, GREEN);
    assert_eq!(subject.render(&frame_two).report.drawn_points(), 0);
}

fn assert_raster_and_pick_semantics(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(2), 1);
    subject.apply(&RenderUpdate::Reset { view_generation });
    let near_id = point_id(101);
    let far_id = point_id(202);
    let transparent_id = point_id(303);
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0, -1.0, 0.0], RED, near_id.ordinal())],
        ),
    });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            2,
            1,
            WORLD_ORIGIN,
            vec![point([0.0, 1.0, 0.0], BLUE, far_id.ordinal())],
        ),
    });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            3,
            1,
            WORLD_ORIGIN,
            vec![point(
                [0.0, -2.0, 0.0],
                TRANSPARENT,
                transparent_id.ordinal(),
            )],
        ),
    });

    let frame = standard_frame(view_generation, VIEWPORT, 24.0, GREEN);
    let depth_result = subject.render(&frame);
    assert_eq!(depth_result.report.draw_calls(), 3);
    assert_pixel(depth_result.image.pixel(CENTER), RED);
    let discarded_corner = [CENTER[0] + 10, CENTER[1] + 10];
    assert_pixel(depth_result.image.pixel(discarded_corner), BLACK);

    subject.apply(&RenderUpdate::SetHighlights {
        view_generation,
        point_ids: vec![near_id],
    });
    assert_eq!(subject.renderer.resident_highlight_points(), 1);
    let highlighted = subject.render(&frame);
    assert_pixel(highlighted.image.pixel(CENTER), GREEN);

    let hit = subject
        .pick_and_wait(&highlighted.recorded_frame, CENTER)
        .expect("the center point should be picked");
    assert_hit(hit, view_generation, 1, 1, near_id);
    assert_eq!(
        subject.pick_and_wait(&highlighted.recorded_frame, discarded_corner),
        None
    );

    subject.apply(&RenderUpdate::SetHighlights {
        view_generation,
        point_ids: vec![transparent_id],
    });
    assert_eq!(subject.renderer.resident_highlight_points(), 1);
    let transparent_highlighted = subject.render(&frame);
    assert_pixel(transparent_highlighted.image.pixel(CENTER), RED);
    let visible_hit = subject
        .pick_and_wait(&transparent_highlighted.recorded_frame, CENTER)
        .expect("highlighting a transparent point must not hide the visible point behind it");
    assert_hit(visible_hit, view_generation, 1, 1, near_id);
}

fn assert_highlight_alpha_preservation(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(8), 1);
    let point_id = point_id(801);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0; 3], [255, 0, 0, 128], point_id.ordinal())],
        ),
    });
    subject.apply(&RenderUpdate::SetHighlights {
        view_generation,
        point_ids: vec![point_id],
    });

    let style = PointStyle::new(18.0, [0.0, 1.0, 0.0], [0.0; 4])
        .expect("the alpha-preservation style should be valid");
    let frame = frame_with_style(view_generation, VIEWPORT, style);
    let rendered = subject.render(&frame);
    assert_pixel(rendered.image.pixel(CENTER), [0, 128, 0, 128]);
    let hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("highlighting must preserve a source-visible point for picking");
    assert_hit(hit, view_generation, 1, 1, point_id);
}

fn assert_presentation_weight_is_color_only(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(9), 1);
    let identity = point_id(901);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0; 3], RED, identity.ordinal())],
        ),
    });
    subject.apply(&RenderUpdate::SetBatchPresentation {
        view_generation,
        key: BatchKey::new(1),
        expected_version: BatchVersion::new(1),
        weight: PresentationWeight::TRANSPARENT,
    });

    let frame = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);
    let rendered = subject.render(&frame);
    assert_pixel(rendered.image.pixel(CENTER), BLACK);
    let hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("transparent presentation must preserve source pick coverage");
    assert_hit(hit, view_generation, 1, 1, identity);
}

fn assert_eye_dome_paths(gpu: &GpuContext) {
    let cue = EyeDomeLighting::new(1.25, 1).unwrap();
    let config = RendererConfig::new(FORMAT, roomy_limits()).with_eye_dome_lighting(cue);
    let mut subject = OffscreenRenderer::with_config(gpu, config);
    assert_eq!(subject.renderer.depth_cue_status(), DepthCueStatus::Active);

    let view_generation = ViewGenerationKey::new(ViewId::new(10), 1);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0; 3], RED, 1_001)],
        ),
    });
    let rendered = subject.render(&standard_frame(view_generation, VIEWPORT, 18.0, GREEN));
    assert_eq!(
        rendered.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 8
    );
    let edge = rendered.image.pixel([CENTER[0] + 8, CENTER[1]]);
    assert!(
        edge[0] < 200 && edge[0] > 0,
        "eye-dome edge should be visibly but tolerantly darkened: {edge:?}"
    );

    let fallback = WgpuRenderer::new(
        &gpu.device,
        RendererConfig::new(wgpu::TextureFormat::Rgba16Float, roomy_limits())
            .with_eye_dome_lighting(cue),
    )
    .expect("the blendable fallback format should remain a valid renderer target");
    assert_eq!(
        fallback.depth_cue_status(),
        DepthCueStatus::UnsupportedFallback
    );
}

fn assert_large_world_precision(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(3), 1);
    subject.apply(&RenderUpdate::Reset { view_generation });
    let red_id = point_id(301);
    let cyan_id = point_id(302);
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![
                point([-0.000_5, 0.0, 0.0], RED, red_id.ordinal()),
                point([0.000_5, 0.0, 0.0], CYAN, cyan_id.ordinal()),
            ],
        ),
    });

    let frame = precision_frame(view_generation);
    let rendered = subject.render(&frame);
    let red_pixel = rendered
        .image
        .find_pixel(is_red)
        .expect("the negative half-millimeter point should render");
    let cyan_pixel = rendered
        .image
        .find_pixel(is_cyan)
        .expect("the positive half-millimeter point should render");
    assert!(
        red_pixel[0].abs_diff(cyan_pixel[0]) >= 40,
        "millimeter-separated points projected too closely: {red_pixel:?}, {cyan_pixel:?}"
    );

    let red_hit = subject
        .pick_and_wait(&rendered.recorded_frame, red_pixel)
        .expect("the red precision point should be pickable");
    let cyan_hit = subject
        .pick_and_wait(&rendered.recorded_frame, cyan_pixel)
        .expect("the cyan precision point should be pickable");
    assert_eq!(red_hit.point(), red_id);
    assert_eq!(cyan_hit.point(), cyan_id);
}

fn assert_orthographic_depth_independent_position(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(9), 1);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([1.0, -1.0, 0.0], RED, 901)],
        ),
    });
    let frame = orthographic_frame(view_generation, 8.0);
    let near = subject.render(&frame);
    let near_pixel = near
        .image
        .find_pixel(is_red)
        .expect("the near orthographic point should render");

    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            2,
            WORLD_ORIGIN,
            vec![point([1.0, 1.0, 0.0], CYAN, 902)],
        ),
    });
    let far = subject.render(&frame);
    let far_pixel = far
        .image
        .find_pixel(is_cyan)
        .expect("the far orthographic point should render");

    assert_eq!(near_pixel, far_pixel);
}

fn assert_orthographic_depth_and_pick(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(10), 1);
    let near_id = point_id(1_001);
    let far_id = point_id(1_002);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0, -1.0, 0.0], RED, near_id.ordinal())],
        ),
    });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            2,
            1,
            WORLD_ORIGIN,
            vec![point([0.0, 1.0, 0.0], BLUE, far_id.ordinal())],
        ),
    });

    let frame = orthographic_frame(view_generation, 18.0);
    let rendered = subject.render(&frame);
    assert_pixel(rendered.image.pixel(CENTER), RED);
    let hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("the nearer orthographic point should be picked");
    assert_hit(hit, view_generation, 1, 1, near_id);
}

fn assert_async_ticket_stability(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let old_view_generation = ViewGenerationKey::new(ViewId::new(4), 1);
    let new_view_generation = ViewGenerationKey::new(ViewId::new(4), 2);
    let old_id = point_id(401);
    let new_id = point_id(402);
    subject.apply(&RenderUpdate::Reset {
        view_generation: old_view_generation,
    });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            old_view_generation,
            7,
            3,
            WORLD_ORIGIN,
            vec![point([0.0; 3], RED, old_id.ordinal())],
        ),
    });
    let old_frame = standard_frame(old_view_generation, VIEWPORT, 18.0, GREEN);
    let old_render = subject.render(&old_frame);
    let (mut old_ticket, old_commands) = subject.encode_pick(&old_render.recorded_frame, CENTER);
    assert_eq!(old_ticket.poll().unwrap(), PickPoll::Pending);
    gpu.queue.submit([old_commands]);

    subject.apply(&RenderUpdate::Reset {
        view_generation: new_view_generation,
    });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            new_view_generation,
            8,
            4,
            WORLD_ORIGIN,
            vec![point([0.0; 3], BLUE, new_id.ordinal())],
        ),
    });
    let new_center = [RESIZED_VIEWPORT[0] / 2, RESIZED_VIEWPORT[1] / 2];
    let new_frame = standard_frame(new_view_generation, RESIZED_VIEWPORT, 18.0, GREEN);
    let new_render = subject.render(&new_frame);
    let (mut new_ticket, new_commands) =
        subject.encode_pick(&new_render.recorded_frame, new_center);
    let (mut no_hit_ticket, no_hit_commands) =
        subject.encode_pick(&new_render.recorded_frame, [0, 0]);
    assert_eq!(new_ticket.poll().unwrap(), PickPoll::Pending);
    assert_eq!(no_hit_ticket.poll().unwrap(), PickPoll::Pending);
    gpu.queue.submit([new_commands, no_hit_commands]);
    gpu.wait();

    let PickPoll::Ready(Some(old_hit)) = old_ticket.poll().unwrap() else {
        panic!("the old ticket should retain its submitted generation metadata");
    };
    assert_hit(old_hit, old_view_generation, 7, 3, old_id);
    let PickPoll::Ready(Some(new_hit)) = new_ticket.poll().unwrap() else {
        panic!("the resized target should return the new point");
    };
    assert_hit(new_hit, new_view_generation, 8, 4, new_id);
    assert_eq!(no_hit_ticket.poll().unwrap(), PickPoll::Ready(None));
    let retained_old_hit = subject
        .pick_and_wait(&old_render.recorded_frame, CENTER)
        .expect("switching targets back should preserve the retained frame");
    assert_hit(retained_old_hit, old_view_generation, 7, 3, old_id);
    assert!(matches!(
        old_ticket.poll(),
        Err(PickError::AlreadyCompleted)
    ));
}

fn assert_deferred_frame_camera_stability(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(5), 1);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0; 3], RED, 501)],
        ),
    });
    let centered = standard_frame(view_generation, VIEWPORT, 14.0, GREEN);
    let translated = translated_frame(view_generation, 1.5);

    let (centered_result, translated_result) =
        subject.render_pair_before_submit(&centered, &translated);

    assert_pixel(centered_result.image.pixel(CENTER), RED);
    assert_pixel(translated_result.image.pixel(CENTER), BLACK);
    let translated_point = translated_result
        .image
        .find_pixel(is_red)
        .expect("the translated camera should keep the point inside the viewport");
    assert!(
        translated_point[0] < CENTER[0] - 8,
        "the translated point should move left, got {translated_point:?}"
    );
}

fn assert_recorded_frame_replacement_stability(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(6), 1);
    let batch_key = 19;
    let first_id = point_id(101);
    let replacement_id = point_id(202);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            batch_key,
            1,
            WORLD_ORIGIN,
            vec![point([0.0; 3], RED, first_id.ordinal())],
        ),
    });
    let frame = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);
    let retained = subject.render(&frame);
    assert_pixel(retained.image.pixel(CENTER), RED);

    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            batch_key,
            2,
            WORLD_ORIGIN,
            vec![point([0.0; 3], BLUE, replacement_id.ordinal())],
        ),
    });
    let current = subject.render(&frame);
    assert_pixel(current.image.pixel(CENTER), BLUE);

    let retained_hit = subject
        .pick_and_wait(&retained.recorded_frame, CENTER)
        .expect("the retained frame should pick its original point");
    assert_hit(retained_hit, view_generation, batch_key, 1, first_id);
}

fn assert_foreign_recorded_frame_rejected(gpu: &GpuContext) {
    let view_generation = ViewGenerationKey::new(ViewId::new(7), 1);
    let mut owner = OffscreenRenderer::new(gpu, roomy_limits());
    owner.apply(&RenderUpdate::Reset { view_generation });
    owner.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0; 3], RED, 701)],
        ),
    });
    let frame = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);
    let recorded = owner.render(&frame);

    let mut foreign = OffscreenRenderer::new(gpu, roomy_limits());
    foreign.apply(&RenderUpdate::Reset { view_generation });
    let mut encoder = foreign.encoder("punctra foreign recorded frame encoder");
    assert!(matches!(
        foreign.renderer.pick(
            &mut encoder,
            &recorded.recorded_frame,
            PickRequest::new(CENTER),
        ),
        Err(RendererError::ForeignRecordedFrame)
    ));
}

struct OffscreenRenderer<'gpu> {
    gpu: &'gpu GpuContext,
    renderer: WgpuRenderer,
}

impl<'gpu> OffscreenRenderer<'gpu> {
    fn new(gpu: &'gpu GpuContext, limits: RenderLimits) -> Self {
        Self::with_config(gpu, RendererConfig::new(FORMAT, limits))
    }

    fn with_config(gpu: &'gpu GpuContext, config: RendererConfig) -> Self {
        let renderer = WgpuRenderer::new(&gpu.device, config)
            .expect("the renderer should attach to the test device");
        Self { gpu, renderer }
    }

    fn apply(&mut self, update: &RenderUpdate) -> UpdateReport {
        self.try_apply(update)
            .expect("the update should be accepted")
    }

    fn try_apply(&mut self, update: &RenderUpdate) -> Result<UpdateReport, RendererError> {
        self.renderer.apply(update)
    }

    fn render(&mut self, frame: &Frame) -> RenderedFrame {
        self.try_render(frame)
            .expect("the offscreen frame should render")
    }

    fn try_render(&mut self, frame: &Frame) -> Result<RenderedFrame, RendererError> {
        let target = ColorTarget::new(&self.gpu.device, frame.viewport().dimensions());
        let mut encoder = self.encoder("punctra acceptance render encoder");
        let recorded_frame = self.renderer.render(&mut encoder, &target.view, frame)?;
        let report = recorded_frame.report();
        target.encode_copy(&mut encoder);
        let receiver = target.map_after_submit(&mut encoder);
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.wait();
        let image = target.read(&receiver);
        Ok(RenderedFrame {
            recorded_frame,
            report,
            image,
        })
    }

    fn render_pair_before_submit(
        &mut self,
        first_frame: &Frame,
        second_frame: &Frame,
    ) -> (RenderedFrame, RenderedFrame) {
        let first_target = ColorTarget::new(&self.gpu.device, first_frame.viewport().dimensions());
        let second_target =
            ColorTarget::new(&self.gpu.device, second_frame.viewport().dimensions());
        let mut encoder = self.encoder("punctra deferred frame acceptance encoder");
        let first_recorded_frame = self
            .renderer
            .render(&mut encoder, &first_target.view, first_frame)
            .expect("the first deferred frame should encode");
        let second_recorded_frame = self
            .renderer
            .render(&mut encoder, &second_target.view, second_frame)
            .expect("the second deferred frame should encode");
        let first_report = first_recorded_frame.report();
        let second_report = second_recorded_frame.report();
        first_target.encode_copy(&mut encoder);
        second_target.encode_copy(&mut encoder);
        let first_receiver = first_target.map_after_submit(&mut encoder);
        let second_receiver = second_target.map_after_submit(&mut encoder);
        self.gpu.queue.submit([encoder.finish()]);
        self.gpu.wait();
        let first_image = first_target.read(&first_receiver);
        let second_image = second_target.read(&second_receiver);
        (
            RenderedFrame {
                recorded_frame: first_recorded_frame,
                report: first_report,
                image: first_image,
            },
            RenderedFrame {
                recorded_frame: second_recorded_frame,
                report: second_report,
                image: second_image,
            },
        )
    }

    fn encode_pick(
        &mut self,
        recorded_frame: &RecordedFrame,
        pixel: [u32; 2],
    ) -> (PickTicket, wgpu::CommandBuffer) {
        let mut encoder = self.encoder("punctra acceptance pick encoder");
        let ticket = self
            .renderer
            .pick(&mut encoder, recorded_frame, PickRequest::new(pixel))
            .expect("the pick request should encode");
        (ticket, encoder.finish())
    }

    fn pick_and_wait(
        &mut self,
        recorded_frame: &RecordedFrame,
        pixel: [u32; 2],
    ) -> Option<PickHit> {
        let (mut ticket, commands) = self.encode_pick(recorded_frame, pixel);
        self.gpu.queue.submit([commands]);
        self.gpu.wait();
        let PickPoll::Ready(hit) = ticket.poll().expect("the submitted pick should resolve") else {
            panic!("a fully polled pick should not remain pending");
        };
        hit
    }

    fn encoder(&self, label: &'static str) -> wgpu::CommandEncoder {
        self.gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) })
    }
}

struct RenderedFrame {
    recorded_frame: RecordedFrame,
    report: FrameReport,
    image: Image,
}

struct ColorTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
    viewport: [u32; 2],
    padded_bytes_per_row: u32,
}

impl ColorTarget {
    fn new(device: &wgpu::Device, viewport: [u32; 2]) -> Self {
        let padded_bytes_per_row = padded_bytes_per_row(viewport[0]);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("punctra acceptance color target"),
            size: extent(viewport),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("punctra acceptance color readback"),
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
                    rows_per_image: Some(self.viewport[1]),
                },
            },
            extent(self.viewport),
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
        receiver
            .recv()
            .expect("the mapping callback should run")
            .expect("the color readback should map");
        let mapped = self
            .readback
            .get_mapped_range(..)
            .expect("the mapped color range should be available");
        let bytes = mapped.to_vec();
        drop(mapped);
        self.readback.unmap();
        Image {
            bytes,
            viewport: self.viewport,
            padded_bytes_per_row: self.padded_bytes_per_row,
        }
    }
}

struct Image {
    bytes: Vec<u8>,
    viewport: [u32; 2],
    padded_bytes_per_row: u32,
}

impl Image {
    fn pixel(&self, pixel: [u32; 2]) -> [u8; 4] {
        assert!(pixel[0] < self.viewport[0] && pixel[1] < self.viewport[1]);
        let offset = usize::try_from(pixel[1] * self.padded_bytes_per_row + pixel[0] * 4)
            .expect("the tiny test image offset fits in usize");
        self.bytes[offset..offset + 4]
            .try_into()
            .expect("an RGBA pixel has four bytes")
    }

    fn find_pixel(&self, predicate: fn([u8; 4]) -> bool) -> Option<[u32; 2]> {
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
}

fn roomy_limits() -> RenderLimits {
    RenderLimits::new(1024 * 1024, 1024, 16)
}

fn batch(
    view_generation: ViewGenerationKey,
    key: u64,
    version: u64,
    origin: [f64; 3],
    points: Vec<RenderPoint>,
) -> PointBatch {
    PointBatch::new(
        view_generation,
        BatchKey::new(key),
        BatchVersion::new(version),
        origin,
        points,
    )
    .expect("the acceptance fixture batch should be valid")
}

fn point(position: [f32; 3], color: [u8; 4], id: u64) -> RenderPoint {
    RenderPoint::new(position, color, point_id(id))
        .expect("the acceptance fixture point should be valid")
}

const fn point_id(ordinal: u64) -> PointId {
    PointId::new(TEST_SOURCE, ordinal)
}

fn standard_frame(
    view_generation: ViewGenerationKey,
    viewport: [u32; 2],
    point_size: f32,
    highlight_color: [u8; 4],
) -> Frame {
    let highlight = rgba8_to_linear_rgb(highlight_color);
    let style = PointStyle::new(point_size, highlight, [0.0, 0.0, 0.0, 1.0])
        .expect("the acceptance point style should be valid");
    frame_with_style(view_generation, viewport, style)
}

fn frame_with_style(
    view_generation: ViewGenerationKey,
    viewport: [u32; 2],
    style: PointStyle,
) -> Frame {
    let camera = Camera::perspective(
        [WORLD_ORIGIN[0], WORLD_ORIGIN[1] - 5.0, WORLD_ORIGIN[2]],
        WORLD_ORIGIN,
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_3,
        0.1,
        100.0,
    )
    .expect("the standard acceptance camera should be valid");
    Frame::new(
        view_generation,
        camera,
        Viewport::new(viewport[0], viewport[1]).unwrap(),
    )
    .expect("the acceptance frame should be valid")
    .with_style(style)
}

fn precision_frame(view_generation: ViewGenerationKey) -> Frame {
    let camera = Camera::perspective(
        [WORLD_ORIGIN[0], WORLD_ORIGIN[1] - 0.02, WORLD_ORIGIN[2]],
        WORLD_ORIGIN,
        [0.0, 0.0, 1.0],
        0.1,
        0.001,
        1.0,
    )
    .expect("the precision camera should be valid");
    let style = PointStyle::new(5.0, [1.0; 3], [0.0, 0.0, 0.0, 1.0])
        .expect("the precision style should be valid");
    Frame::new(view_generation, camera, Viewport::new(128, 128).unwrap())
        .expect("the precision frame should be valid")
        .with_style(style)
}

fn orthographic_frame(view_generation: ViewGenerationKey, point_size: f32) -> Frame {
    let camera = Camera::orthographic(
        [WORLD_ORIGIN[0], WORLD_ORIGIN[1] - 5.0, WORLD_ORIGIN[2]],
        WORLD_ORIGIN,
        [0.0, 0.0, 1.0],
        4.0,
        0.1,
        100.0,
    )
    .expect("the orthographic acceptance camera should be valid");
    let style = PointStyle::new(point_size, [1.0; 3], [0.0, 0.0, 0.0, 1.0])
        .expect("the orthographic point style should be valid");
    Frame::new(
        view_generation,
        camera,
        Viewport::new(VIEWPORT[0], VIEWPORT[1]).unwrap(),
    )
    .expect("the orthographic acceptance frame should be valid")
    .with_style(style)
}

fn translated_frame(view_generation: ViewGenerationKey, horizontal_offset: f64) -> Frame {
    let target = [
        WORLD_ORIGIN[0] + horizontal_offset,
        WORLD_ORIGIN[1],
        WORLD_ORIGIN[2],
    ];
    let camera = Camera::perspective(
        [target[0], target[1] - 5.0, target[2]],
        target,
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_3,
        0.1,
        100.0,
    )
    .expect("the translated camera should be valid");
    let style = PointStyle::new(14.0, rgba8_to_linear_rgb(GREEN), [0.0, 0.0, 0.0, 1.0])
        .expect("the translated frame style should be valid");
    Frame::new(
        view_generation,
        camera,
        Viewport::new(VIEWPORT[0], VIEWPORT[1]).unwrap(),
    )
    .expect("the translated frame should be valid")
    .with_style(style)
}

fn rgba8_to_linear_rgb(color: [u8; 4]) -> [f32; 3] {
    let maximum = f32::from(u8::MAX);
    [
        f32::from(color[0]) / maximum,
        f32::from(color[1]) / maximum,
        f32::from(color[2]) / maximum,
    ]
}

fn extent(viewport: [u32; 2]) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: viewport[0],
        height: viewport[1],
        depth_or_array_layers: 1,
    }
}

fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width
        .checked_mul(4)
        .expect("test row byte count should fit");
    unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

fn assert_pixel(actual: [u8; 4], expected: [u8; 4]) {
    for (actual_channel, expected_channel) in actual.into_iter().zip(expected) {
        assert!(
            actual_channel.abs_diff(expected_channel) <= 1,
            "expected pixel {expected:?}, got {actual:?}"
        );
    }
}

fn assert_hit(
    hit: PickHit,
    view_generation: ViewGenerationKey,
    batch_key: u64,
    version: u64,
    point_id: PointId,
) {
    assert_eq!(hit.view_generation(), view_generation);
    assert_eq!(hit.batch(), BatchKey::new(batch_key));
    assert_eq!(hit.version(), BatchVersion::new(version));
    assert_eq!(hit.point(), point_id);
}

fn is_red(pixel: [u8; 4]) -> bool {
    pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40
}

fn is_cyan(pixel: [u8; 4]) -> bool {
    pixel[0] < 40 && pixel[1] > 200 && pixel[2] > 200
}
