//! GPU acceptance coverage on an available headless adapter.

use std::{
    env,
    fmt::Write as _,
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[path = "../test-support/gpu.rs"]
mod gpu_support;

use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT as POINT_BYTES, PointBatch, PointId,
    PresentationWeight, ProtocolError, RenderLimits, RenderPoint, RenderUpdate, ResidentResource,
    SourceId, UpdateKind, UpdateReport, ViewGenerationKey, ViewId, Viewport,
};
use render_wgpu::{
    Camera, DepthCueStatus, EyeDomeLighting, Frame, FrameReport, PickError, PickHit, PickPoll,
    PickRequest, PickTicket, PointFootprint, PointFootprintStatus, PointStyle, RecordedFrame,
    RendererConfig, RendererError, WgpuRenderer,
};
use sha2::{Digest, Sha256};

use gpu_support::{GpuContext, Rgba8Image as Image, Rgba8Target as ColorTarget, with_gpu};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const VIEWPORT: [u32; 2] = [64, 64];
const FOOTPRINT_QUALITY_VIEWPORT: [u32; 2] = [128, 128];
const FOOTPRINT_QUALITY_PIXELS_PER_WORLD_UNIT: f32 = 16.0;
const IDEAL_DISK_SAMPLES_PER_AXIS: u32 = 16;
const RGBA8_ENDPOINT_QUANTIZATION_TOLERANCE: f64 = 0.5 / 255.0 + f64::EPSILON;
const FOOTPRINT_CENTER_PHASES: [[f32; 2]; 8] = [
    [0.0, 0.0],
    [0.125, 0.375],
    [0.25, 0.75],
    [0.375, 0.125],
    [0.5, 0.5],
    [0.625, 0.875],
    [0.75, 0.25],
    [0.875, 0.625],
];
const FOOTPRINT_BASE_CENTERS: [[f32; 2]; 8] = [
    [20.0, 36.0],
    [48.0, 36.0],
    [76.0, 36.0],
    [104.0, 36.0],
    [20.0, 92.0],
    [48.0, 92.0],
    [76.0, 92.0],
    [104.0, 92.0],
];
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
const MAX_PICK_COMPLETION_TIME: Duration = Duration::from_secs(2);
const LOCAL_EVIDENCE_PRODUCER_COMMAND: &str = "PUNCTRA_REQUIRE_GPU=1 PUNCTRA_POINT_FOOTPRINT_EVIDENCE_PATH=apps/browser-demo/web/fixtures/footprint-v1/local-test-evidence.json cargo test -p render-wgpu --test offscreen write_point_footprint_test_evidence -- --ignored --exact";
const PRIVATE_FACTS_PATH_ENV: &str = "PUNCTRA_PRIVATE_POINT_FOOTPRINT_FACTS_PATH";

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
fn multi_point_cross_fade_preserves_coverage_across_inverted_batch_depth() {
    with_gpu(assert_multi_point_cross_fade_coverage);
}

#[test]
fn display_size_changes_color_coverage_but_not_pick_coverage() {
    with_gpu(assert_display_size_is_color_only);
}

#[test]
fn eye_dome_lighting_has_bounded_active_and_fallback_paths() {
    with_gpu(assert_eye_dome_paths);
}

#[test]
fn transparent_presentation_does_not_leak_through_eye_dome_lighting() {
    with_gpu(assert_transparent_eye_dome_staging);
}

#[test]
fn four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking() {
    with_gpu(assert_multisample_footprint_and_pick);
}

#[test]
fn antialiased_footprint_quality_matrix() {
    with_gpu(assert_antialiased_footprint_quality_matrix);
}

#[test]
fn four_sample_targets_are_bounded_and_released_on_resource_fallback() {
    with_gpu(assert_multisample_resource_fallback);
}

#[test]
fn four_sample_color_composes_with_single_sample_eye_dome_depth() {
    with_gpu(assert_multisample_eye_dome_composition);
}

#[test]
fn eye_dome_visibility_depth_uses_nominal_size_for_single_and_multisample_color() {
    with_gpu(assert_eye_dome_visibility_depth_uses_nominal_size);
}

#[test]
fn resource_fallback_suppresses_eye_dome_and_stays_at_eight_bytes_per_pixel() {
    with_gpu(assert_resource_fallback_suppresses_eye_dome);
}

#[test]
fn exact_high_water_accounts_for_pick_and_eye_dome_targets() {
    with_gpu(|gpu| {
        let pick = measure_multisample_footprint_and_pick(gpu);
        let edl_bytes_per_pixel = measure_multisample_eye_dome_composition(gpu);
        let fallback = measure_multisample_resource_fallback(gpu);
        assert_eq!(pick.transient_bytes_per_pixel, 40);
        assert_eq!(edl_bytes_per_pixel, 48);
        assert_eq!(fallback.bytes_per_pixel, 8);
        let maximum_preferred_transient_bytes = 1_310_720_u64 * edl_bytes_per_pixel;
        assert_eq!(maximum_preferred_transient_bytes, 62_914_560);
        assert!(maximum_preferred_transient_bytes <= 67_108_864);
    });
}

#[test]
#[ignore = "writes an explicitly requested local qualification artifact"]
fn write_point_footprint_test_evidence() {
    assert_eq!(
        env::var("PUNCTRA_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "the evidence producer requires PUNCTRA_REQUIRE_GPU=1"
    );
    with_gpu(write_point_footprint_test_evidence_with_gpu);
}

#[test]
#[ignore = "re-runs every GPU measurement without writing the guarded release artifact"]
fn point_footprint_test_evidence_composes_from_bound_gpu_results() {
    assert_eq!(
        env::var("PUNCTRA_REQUIRE_GPU").as_deref(),
        Ok("1"),
        "the evidence composition test requires PUNCTRA_REQUIRE_GPU=1"
    );
    with_gpu(|gpu| {
        let environment = LocalEvidenceEnvironment::new(gpu);
        let measurements = collect_local_evidence_measurements(gpu, &environment);
        let cases = local_evidence_cases(&measurements);
        let expected = [
            "single_sample_request_never_becomes_a_fallback",
            "capability_fallback_precedes_the_viewport_resource_check",
            "antialiased_footprint_quality_matrix",
            "four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking",
            "exact_high_water_accounts_for_pick_and_eye_dome_targets",
        ];
        assert_eq!(cases.len(), expected.len());
        for (case, expected_id) in cases.iter().zip(expected) {
            let object = case
                .as_object()
                .expect("a local evidence case is an object");
            assert_eq!(object.len(), 4);
            assert_eq!(case["id"], expected_id);
            assert_eq!(case["source_test"], expected_id);
            assert_eq!(case["passed"], true);
            assert!(case["facts"].is_object());
        }
        assert_eq!(measurements.unsupported_facts["physical_width"], 4_096);
        assert_eq!(measurements.unsupported_facts["physical_height"], 2_048);
        assert_eq!(
            measurements.resource_facts["resource_fallback"]["nominal_pick_identity"]["matched"],
            true
        );
    });
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
    let near_identity = point_id(901);
    let far_identity = point_id(902);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![point([0.0, -1.0, 0.0], RED, near_identity.ordinal())],
        ),
    });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            2,
            1,
            WORLD_ORIGIN,
            vec![point([0.0, 1.0, 0.0], BLUE, far_identity.ordinal())],
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
    assert_pixel(rendered.image.pixel(CENTER), BLUE);
    let hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("transparent presentation must preserve source pick coverage");
    assert_hit(hit, view_generation, 1, 1, near_identity);
}

fn assert_multi_point_cross_fade_coverage(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(10), 1);
    let near_identity = point_id(1_001);
    subject.apply(&RenderUpdate::Reset { view_generation });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            1,
            1,
            WORLD_ORIGIN,
            vec![
                point([0.0, -1.0, 0.0], RED, near_identity.ordinal()),
                point([20.0, 4.0, 0.0], RED, 1_002),
            ],
        ),
    });
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(
            view_generation,
            2,
            1,
            WORLD_ORIGIN,
            vec![
                point([0.0, 1.0, 0.0], RED, 1_003),
                point([-20.0, -4.0, 0.0], RED, 1_004),
            ],
        ),
    });
    for key in [BatchKey::new(1), BatchKey::new(2)] {
        subject.apply(&RenderUpdate::SetBatchPresentation {
            view_generation,
            key,
            expected_version: BatchVersion::new(1),
            weight: PresentationWeight::new(128),
        });
    }

    let frame = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);
    let rendered = subject.render(&frame);
    let center = rendered.image.pixel(CENTER);
    assert!(
        center[0] >= 180 && center[1] <= 1 && center[2] <= 1,
        "the multi-point cross-fade exposed too much background: {center:?}"
    );
    assert_eq!(rendered.report.draw_calls(), 2);
    let hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("the cross-fade must preserve fixed pick coverage");
    assert_hit(hit, view_generation, 1, 1, near_identity);
}

fn assert_display_size_is_color_only(gpu: &GpuContext) {
    let mut subject = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(10), 1);
    let identity = point_id(1_001);
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

    let style = PointStyle::new(2.4, [1.0; 3], [0.0, 0.0, 0.0, 1.0])
        .unwrap()
        .with_display_size_pixels(18.0)
        .unwrap();
    let frame = frame_with_style(view_generation, VIEWPORT, style);
    let rendered = subject.render(&frame);
    let visual_only_pixel = [CENTER[0] + 5, CENTER[1]];
    assert_pixel(rendered.image.pixel(visual_only_pixel), RED);
    assert_eq!(
        subject.pick_and_wait(&rendered.recorded_frame, visual_only_pixel),
        None
    );
    let center_hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("the nominal pick footprint should still cover the point center");
    assert_hit(center_hit, view_generation, 1, 1, identity);
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
    let frame = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);
    let rendered = subject.render(&frame);
    assert_eq!(
        rendered.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 8
    );
    let edge = rendered.image.pixel([CENTER[0] + 8, CENTER[1]]);
    assert!(
        edge[0] < 200 && edge[0] > 0,
        "eye-dome edge should be visibly but tolerantly darkened: {edge:?}"
    );
    let repeated = subject.render(&frame);
    assert_eq!(repeated.image.pixel(CENTER), rendered.image.pixel(CENTER));
    assert!(
        subject
            .pick_and_wait(&repeated.recorded_frame, CENTER)
            .is_some()
    );
    let after_pick = subject.render(&frame);
    assert_eq!(
        after_pick.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 12
    );

    let resized_viewport = [80, 64];
    let resized = subject.render(&standard_frame(
        view_generation,
        resized_viewport,
        18.0,
        GREEN,
    ));
    assert_eq!(
        resized.report.transient_texture_bytes(),
        u64::from(resized_viewport[0]) * u64::from(resized_viewport[1]) * 8
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

fn assert_transparent_eye_dome_staging(gpu: &GpuContext) {
    let cue = EyeDomeLighting::new(1.25, 1).unwrap();
    let config = RendererConfig::new(FORMAT, roomy_limits()).with_eye_dome_lighting(cue);
    let mut subject = OffscreenRenderer::with_config(gpu, config);
    let view_generation = ViewGenerationKey::new(ViewId::new(11), 1);
    subject.apply(&RenderUpdate::Reset { view_generation });
    let frame = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);
    let edge = [CENTER[0] + 8, CENTER[1]];
    let empty_background = subject.render(&frame).image.pixel(edge);

    let identity = point_id(1_101);
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

    let staged = subject.render(&frame);
    assert_eq!(staged.image.pixel(edge), empty_background);
    let hit = subject
        .pick_and_wait(&staged.recorded_frame, CENTER)
        .expect("transparent EDL staging must preserve nominal pick coverage");
    assert_hit(hit, view_generation, 1, 1, identity);
}

fn assert_multisample_footprint_and_pick(gpu: &GpuContext) {
    let _ = measure_multisample_footprint_and_pick(gpu);
}

#[derive(Clone, Copy)]
struct PickIndependenceMeasurements {
    display_diameter_physical_pixels: f64,
    nominal_pick_diameter_physical_pixels: f64,
    visual_only_probe_offset_physical_pixels: [u32; 2],
    visual_only_probe_missed: bool,
    nominal_probe_matched: bool,
    transient_bytes_per_pixel: u64,
}

fn measure_multisample_footprint_and_pick(gpu: &GpuContext) -> PickIndependenceMeasurements {
    let config = RendererConfig::new(FORMAT, roomy_limits())
        .with_point_footprint(PointFootprint::Antialiased);
    let mut subject = OffscreenRenderer::with_config(gpu, config);
    let view_generation = ViewGenerationKey::new(ViewId::new(12), 1);
    let identity = point_id(1_201);
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
    let style = PointStyle::new(2.4, [1.0; 3], [0.0, 0.0, 0.0, 1.0])
        .unwrap()
        .with_display_size_pixels(18.0)
        .unwrap();
    let frame = frame_with_style(view_generation, VIEWPORT, style);

    assert_eq!(
        subject.renderer.point_footprint_status(frame.viewport()),
        PointFootprintStatus::Multisample4x
    );
    let rendered = subject.render(&frame);
    assert_eq!(
        rendered.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 32
    );
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        rendered.report.transient_texture_bytes()
    );
    assert!(
        rendered.image.find_pixel(is_partial_red).is_some(),
        "the resolved circular edge should contain partial four-sample coverage"
    );
    assert_pixel(rendered.image.pixel([CENTER[0] + 8, CENTER[1] + 8]), BLACK);

    let visual_only_pixel = [CENTER[0] + 5, CENTER[1]];
    assert_pixel(rendered.image.pixel(visual_only_pixel), RED);
    let visual_only_probe = subject.pick_and_wait(&rendered.recorded_frame, visual_only_pixel);
    assert_eq!(visual_only_probe, None);
    let hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("four-sample color must retain nominal single-sample picking");
    let nominal_probe_matched = hit.point() == identity;
    assert_hit(hit, view_generation, 1, 1, identity);
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 40
    );
    let after_pick = subject.render(&frame);
    assert_eq!(
        after_pick.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 40
    );
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        after_pick.report.transient_texture_bytes()
    );
    PickIndependenceMeasurements {
        display_diameter_physical_pixels: physical_pixel_tenth(frame.style().display_size_pixels()),
        nominal_pick_diameter_physical_pixels: physical_pixel_tenth(
            frame.style().default_size_pixels(),
        ),
        visual_only_probe_offset_physical_pixels: [
            visual_only_pixel[0] - CENTER[0],
            visual_only_pixel[1] - CENTER[1],
        ],
        visual_only_probe_missed: visual_only_probe.is_none(),
        nominal_probe_matched,
        transient_bytes_per_pixel: after_pick.report.transient_texture_bytes()
            / (u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1])),
    }
}

fn physical_pixel_tenth(value: f32) -> f64 {
    (f64::from(value) * 10.0).round() / 10.0
}

fn assert_antialiased_footprint_quality_matrix(gpu: &GpuContext) {
    let _ = measure_antialiased_footprint_quality_matrix(gpu);
}

#[derive(Clone, Copy)]
struct FootprintQualityMeasurements {
    maximum_coverage_rmse: f64,
    coverage_rmse_at_preferred_worst_case: f64,
    maximum_exact_distance_outer_leakage_pixels: u32,
    all_centers_foreground: bool,
    all_quad_corners_clear: bool,
}

fn measure_antialiased_footprint_quality_matrix(gpu: &GpuContext) -> FootprintQualityMeasurements {
    let antialiased_config = RendererConfig::new(FORMAT, roomy_limits())
        .with_point_footprint(PointFootprint::Antialiased);
    let mut antialiased = OffscreenRenderer::with_config(gpu, antialiased_config);
    let mut single_sample = OffscreenRenderer::new(gpu, roomy_limits());
    let antialiased_generation = ViewGenerationKey::new(ViewId::new(15), 1);
    let single_sample_generation = ViewGenerationKey::new(ViewId::new(16), 1);
    populate_footprint_quality_fixture(&mut antialiased, antialiased_generation);
    populate_footprint_quality_fixture(&mut single_sample, single_sample_generation);
    let mut measurements = FootprintQualityMeasurements {
        maximum_coverage_rmse: 0.0,
        coverage_rmse_at_preferred_worst_case: 0.0,
        maximum_exact_distance_outer_leakage_pixels: 0,
        all_centers_foreground: true,
        all_quad_corners_clear: true,
    };

    for diameter in [2.0_f32, 3.0, 4.0, 5.0, 6.0] {
        let antialiased_frame = footprint_quality_frame(antialiased_generation, diameter);
        let single_sample_frame = footprint_quality_frame(single_sample_generation, diameter);
        assert_eq!(
            antialiased
                .renderer
                .point_footprint_status(antialiased_frame.viewport()),
            PointFootprintStatus::Multisample4x
        );

        let antialiased_image = antialiased.render(&antialiased_frame).image;
        let single_sample_image = single_sample.render(&single_sample_frame).image;
        let mut antialiased_error = CoverageError::default();
        let mut single_sample_error = CoverageError::default();

        for center in footprint_quality_centers() {
            let candidate = footprint_coverage_error(&antialiased_image, center, diameter);
            let predecessor = footprint_coverage_error(&single_sample_image, center, diameter);
            let candidate_rmse = candidate.root_mean_square_error();
            let predecessor_rmse = predecessor.root_mean_square_error();
            assert!(
                candidate_rmse <= 0.18,
                "{diameter}px antialiased footprint at {center:?} has coverage RMSE {candidate_rmse:.6}, exceeding 0.18"
            );
            assert!(
                candidate_rmse <= predecessor_rmse * 0.8,
                "{diameter}px antialiased footprint at {center:?} has coverage RMSE {candidate_rmse:.6}, not at least 20% below same-size single-sample RMSE {predecessor_rmse:.6}"
            );
            if candidate_rmse > measurements.maximum_coverage_rmse {
                measurements.maximum_coverage_rmse = candidate_rmse;
                measurements.coverage_rmse_at_preferred_worst_case = predecessor_rmse;
            }
            antialiased_error.add(candidate);
            single_sample_error.add(predecessor);
            let shape = footprint_shape_measurements(&antialiased_image, center, diameter);
            measurements.maximum_exact_distance_outer_leakage_pixels = measurements
                .maximum_exact_distance_outer_leakage_pixels
                .max(shape.exact_distance_outer_leakage_pixels);
            measurements.all_centers_foreground &= shape.center_foreground;
            measurements.all_quad_corners_clear &= shape.all_quad_corners_clear;
            assert_footprint_shape_gates(shape, center, diameter);
        }

        let candidate_rmse = antialiased_error.root_mean_square_error();
        let single_sample_rmse = single_sample_error.root_mean_square_error();
        assert!(
            candidate_rmse <= 0.18,
            "{diameter}px antialiased coverage RMSE {candidate_rmse:.6} exceeds 0.18"
        );
        assert!(
            candidate_rmse <= single_sample_rmse * 0.8,
            "{diameter}px antialiased coverage RMSE {candidate_rmse:.6} is not at least 20% below same-size single-sample RMSE {single_sample_rmse:.6}"
        );
    }
    measurements
}

fn populate_footprint_quality_fixture(
    subject: &mut OffscreenRenderer<'_>,
    view_generation: ViewGenerationKey,
) {
    subject.apply(&RenderUpdate::Reset { view_generation });
    let points = footprint_quality_centers()
        .into_iter()
        .enumerate()
        .map(|(index, center)| {
            let position = [
                (center[0] - 64.0) / FOOTPRINT_QUALITY_PIXELS_PER_WORLD_UNIT,
                0.0,
                (64.0 - center[1]) / FOOTPRINT_QUALITY_PIXELS_PER_WORLD_UNIT,
            ];
            point(
                position,
                RED,
                1_500 + u64::try_from(index).expect("the eight fixture indexes fit in u64"),
            )
        })
        .collect();
    subject.apply(&RenderUpdate::Upsert {
        batch: batch(view_generation, 1, 1, WORLD_ORIGIN, points),
    });
}

fn footprint_quality_centers() -> [[f32; 2]; 8] {
    std::array::from_fn(|index| {
        [
            FOOTPRINT_BASE_CENTERS[index][0] + FOOTPRINT_CENTER_PHASES[index][0],
            FOOTPRINT_BASE_CENTERS[index][1] + FOOTPRINT_CENTER_PHASES[index][1],
        ]
    })
}

#[derive(Clone, Copy, Default)]
struct CoverageError {
    squared_error: f64,
    samples: u32,
}

impl CoverageError {
    fn add(&mut self, other: Self) {
        self.squared_error += other.squared_error;
        self.samples += other.samples;
    }

    fn root_mean_square_error(&self) -> f64 {
        (self.squared_error / f64::from(self.samples)).sqrt()
    }
}

fn footprint_coverage_error(image: &Image, center: [f32; 2], diameter: f32) -> CoverageError {
    let radius = diameter / 2.0;
    let rectangle = footprint_metric_rectangle(center, radius);
    let mut result = CoverageError::default();
    for y in rectangle[1]..rectangle[3] {
        for x in rectangle[0]..rectangle[2] {
            let observed = coverage(image, [x, y]);
            let ideal = ideal_disk_coverage([x, y], center, radius);
            result.squared_error += (observed - ideal).powi(2);
            result.samples += 1;
        }
    }
    result
}

fn footprint_metric_rectangle(center: [f32; 2], radius: f32) -> [u32; 4] {
    let minimum_x = fixture_pixel_coordinate((center[0] - radius - 3.0).floor().max(0.0));
    let minimum_y = fixture_pixel_coordinate((center[1] - radius - 3.0).floor().max(0.0));
    let maximum_x = fixture_pixel_coordinate((center[0] + radius + 3.0).ceil().min(128.0));
    let maximum_y = fixture_pixel_coordinate((center[1] + radius + 3.0).ceil().min(128.0));
    [minimum_x, minimum_y, maximum_x, maximum_y]
}

fn ideal_disk_coverage(pixel: [u32; 2], center: [f32; 2], radius: f32) -> f64 {
    let mut inside = 0_u32;
    let center = center.map(f64::from);
    let radius = f64::from(radius);
    for sample_y in 0..IDEAL_DISK_SAMPLES_PER_AXIS {
        for sample_x in 0..IDEAL_DISK_SAMPLES_PER_AXIS {
            let x = f64::from(pixel[0])
                + (f64::from(sample_x) + 0.5) / f64::from(IDEAL_DISK_SAMPLES_PER_AXIS);
            let y = f64::from(pixel[1])
                + (f64::from(sample_y) + 0.5) / f64::from(IDEAL_DISK_SAMPLES_PER_AXIS);
            let delta_x = x - center[0];
            let delta_y = y - center[1];
            if delta_x.mul_add(delta_x, delta_y * delta_y) <= radius * radius {
                inside += 1;
            }
        }
    }
    f64::from(inside) / f64::from(IDEAL_DISK_SAMPLES_PER_AXIS.pow(2))
}

#[derive(Clone, Copy)]
struct FootprintShapeMeasurements {
    center_foreground: bool,
    exact_distance_outer_leakage_pixels: u32,
    all_quad_corners_clear: bool,
}

fn footprint_shape_measurements(
    image: &Image,
    center: [f32; 2],
    diameter: f32,
) -> FootprintShapeMeasurements {
    let radius = diameter / 2.0;
    let center_pixel = [
        fixture_pixel_coordinate(center[0].floor()),
        fixture_pixel_coordinate(center[1].floor()),
    ];
    let center_foreground = coverage(image, center_pixel) > RGBA8_ENDPOINT_QUANTIZATION_TOLERANCE;
    let mut exact_distance_outer_leakage_pixels = 0;
    let rectangle = footprint_metric_rectangle(center, radius);
    for y in rectangle[1]..rectangle[3] {
        for x in rectangle[0]..rectangle[2] {
            let distance = ((f64::from(x) + 0.5 - f64::from(center[0])).powi(2)
                + (f64::from(y) + 0.5 - f64::from(center[1])).powi(2))
            .sqrt();
            if distance > f64::from(radius) + 0.75
                && coverage(image, [x, y]) > RGBA8_ENDPOINT_QUANTIZATION_TOLERANCE
            {
                exact_distance_outer_leakage_pixels += 1;
            }
        }
    }

    let mut all_quad_corners_clear = true;
    for [sign_x, sign_y] in [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]] {
        let corner_pixel = [
            quad_corner_pixel(center[0], radius, sign_x),
            quad_corner_pixel(center[1], radius, sign_y),
        ];
        all_quad_corners_clear &=
            coverage(image, corner_pixel) < 1.0 - RGBA8_ENDPOINT_QUANTIZATION_TOLERANCE;
    }
    FootprintShapeMeasurements {
        center_foreground,
        exact_distance_outer_leakage_pixels,
        all_quad_corners_clear,
    }
}

fn assert_footprint_shape_gates(
    measurements: FootprintShapeMeasurements,
    center: [f32; 2],
    diameter: f32,
) {
    assert!(
        measurements.center_foreground,
        "{diameter}px footprint at {center:?} lost its ideal center"
    );
    assert_eq!(
        measurements.exact_distance_outer_leakage_pixels, 0,
        "{diameter}px footprint at {center:?} leaked beyond its ideal radius plus 0.75px"
    );
    assert!(
        measurements.all_quad_corners_clear,
        "{diameter}px footprint at {center:?} filled a quad-corner region"
    );
}

fn quad_corner_pixel(center: f32, radius: f32, sign: f32) -> u32 {
    fixture_pixel_coordinate((center + sign * radius).floor())
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the fixed fixture coordinates are finite integers inside the 128-pixel target"
)]
fn fixture_pixel_coordinate(value: f32) -> u32 {
    debug_assert!(value.is_finite() && (0.0..=128.0).contains(&value));
    value as u32
}

fn coverage(image: &Image, pixel: [u32; 2]) -> f64 {
    f64::from(image.pixel(pixel)[0]) / 255.0
}

fn foreground_mask(image: &Image, viewport: [u32; 2], background: [u8; 4]) -> Vec<u8> {
    let mut mask = Vec::with_capacity(
        usize::try_from(u64::from(viewport[0]) * u64::from(viewport[1])).unwrap(),
    );
    for y in 0..viewport[1] {
        for x in 0..viewport[0] {
            mask.push(u8::from(image.pixel([x, y]) != background));
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

fn pick_identity_fixture(
    view_generation: ViewGenerationKey,
    batch_key: u64,
    batch_version: u64,
    point: PointId,
) -> serde_json::Value {
    serde_json::json!({
        "generation": view_generation.generation(),
        "source_identity": point.source().to_string(),
        "batch_key": batch_key,
        "batch_version": batch_version,
        "point_ordinal": point.ordinal(),
    })
}

fn pick_identity(hit: PickHit) -> serde_json::Value {
    serde_json::json!({
        "generation": hit.view_generation().generation(),
        "source_identity": hit.point().source().to_string(),
        "batch_key": hit.batch().get(),
        "batch_version": hit.version().get(),
        "point_ordinal": hit.point().ordinal(),
    })
}

fn assert_multisample_resource_fallback(gpu: &GpuContext) {
    let _ = measure_multisample_resource_fallback(gpu);
}

struct ResourceFallbackMeasurements {
    bytes_per_pixel: u64,
    proof: serde_json::Value,
}

fn measure_multisample_resource_fallback(gpu: &GpuContext) -> ResourceFallbackMeasurements {
    let config = RendererConfig::new(FORMAT, roomy_limits())
        .with_point_footprint(PointFootprint::Antialiased);
    let mut subject = OffscreenRenderer::with_config(gpu, config);
    let mut reference = OffscreenRenderer::new(gpu, roomy_limits());
    let view_generation = ViewGenerationKey::new(ViewId::new(13), 1);
    let identity = point_id(1_301);
    subject.apply(&RenderUpdate::Reset { view_generation });
    reference.apply(&RenderUpdate::Reset { view_generation });
    for renderer in [&mut subject, &mut reference] {
        renderer.apply(&RenderUpdate::Upsert {
            batch: batch(
                view_generation,
                1,
                1,
                WORLD_ORIGIN,
                vec![point([0.0; 3], RED, identity.ordinal())],
            ),
        });
    }

    let preferred = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);
    let preferred_result = subject.render(&preferred);
    assert_eq!(
        preferred_result.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 32
    );
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        preferred_result.report.transient_texture_bytes()
    );

    let fallback_viewport = [1_281, 1_024];
    let fallback_style = PointStyle::new(7.0, [0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0])
        .unwrap()
        .with_display_size_pixels(18.0)
        .unwrap();
    let fallback = frame_with_style(view_generation, fallback_viewport, fallback_style);
    assert_eq!(
        subject.renderer.point_footprint_status(fallback.viewport()),
        PointFootprintStatus::ResourceFallback
    );
    let reference_result = reference.render(&fallback);
    let fallback_result = subject.render(&fallback);
    assert_eq!(
        fallback_result.report.transient_texture_bytes(),
        u64::from(fallback_viewport[0]) * u64::from(fallback_viewport[1]) * 4
    );
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        fallback_result.report.transient_texture_bytes()
    );
    let fallback_center = [fallback_viewport[0] / 2, fallback_viewport[1] / 2];
    let expected_identity = pick_identity_fixture(view_generation, 1, 1, identity);
    let reference_hit = reference
        .pick_and_wait(&reference_result.recorded_frame, fallback_center)
        .expect("the SingleSample fallback reference must preserve nominal pick identity");
    assert_eq!(pick_identity(reference_hit), expected_identity);
    let fallback_hit = subject
        .pick_and_wait(&fallback_result.recorded_frame, fallback_center)
        .expect("ResourceFallback must preserve nominal pick identity");
    let observed_identity = pick_identity(fallback_hit);
    assert_eq!(observed_identity, expected_identity);
    let after_pick = subject.render(&fallback);
    let fallback_pixels = u64::from(fallback_viewport[0]) * u64::from(fallback_viewport[1]);
    assert_eq!(
        after_pick.report.transient_texture_bytes(),
        fallback_pixels * 8
    );
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        after_pick.report.transient_texture_bytes()
    );
    let reference_mask = foreground_mask(&reference_result.image, fallback_viewport, BLACK);
    let observed_mask = foreground_mask(&fallback_result.image, fallback_viewport, BLACK);
    let reference_sha256 = sha256_hex(&reference_mask);
    let observed_sha256 = sha256_hex(&observed_mask);
    assert_eq!(reference_mask, observed_mask);
    ResourceFallbackMeasurements {
        bytes_per_pixel: after_pick.report.transient_texture_bytes() / fallback_pixels,
        proof: serde_json::json!({
            "hard_circle_mask": {
                "width": fallback_viewport[0],
                "height": fallback_viewport[1],
                "byte_length": reference_mask.len(),
                "reference_sha256": reference_sha256,
                "observed_sha256": observed_sha256,
                "equivalent": true,
            },
            "nominal_pick_identity": {
                "expected": expected_identity,
                "observed": observed_identity,
                "matched": true,
            },
        }),
    }
}

fn assert_multisample_eye_dome_composition(gpu: &GpuContext) {
    let _ = measure_multisample_eye_dome_composition(gpu);
}

fn measure_multisample_eye_dome_composition(gpu: &GpuContext) -> u64 {
    let cue = EyeDomeLighting::new(1.25, 1).unwrap();
    let config = RendererConfig::new(FORMAT, roomy_limits())
        .with_eye_dome_lighting(cue)
        .with_point_footprint(PointFootprint::Antialiased);
    let mut subject = OffscreenRenderer::with_config(gpu, config);
    let view_generation = ViewGenerationKey::new(ViewId::new(14), 1);
    let identity = point_id(1_401);
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
    let frame = standard_frame(view_generation, VIEWPORT, 18.0, GREEN);

    let rendered = subject.render(&frame);
    assert!(rendered.report.eye_dome_lighting_applied());
    assert_eq!(
        rendered.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 40
    );
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        rendered.report.transient_texture_bytes()
    );
    assert!(rendered.image.find_pixel(is_partial_red).is_some());
    let hit = subject
        .pick_and_wait(&rendered.recorded_frame, CENTER)
        .expect("EDL composition must retain nominal pick identity");
    assert_hit(hit, view_generation, 1, 1, identity);
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 48
    );

    let after_pick = subject.render(&frame);
    assert_eq!(
        after_pick.report.transient_texture_bytes(),
        u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]) * 48
    );
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        after_pick.report.transient_texture_bytes()
    );
    after_pick.report.transient_texture_bytes() / (u64::from(VIEWPORT[0]) * u64::from(VIEWPORT[1]))
}

fn assert_eye_dome_visibility_depth_uses_nominal_size(gpu: &GpuContext) {
    for (index, footprint) in [PointFootprint::SingleSample, PointFootprint::Antialiased]
        .into_iter()
        .enumerate()
    {
        let cue = EyeDomeLighting::new(1.25, 1).unwrap();
        let base_config =
            RendererConfig::new(FORMAT, roomy_limits()).with_point_footprint(footprint);
        let mut reference = OffscreenRenderer::with_config(gpu, base_config);
        let mut subject =
            OffscreenRenderer::with_config(gpu, base_config.with_eye_dome_lighting(cue));
        let view_generation =
            ViewGenerationKey::new(ViewId::new(30 + u64::try_from(index).unwrap()), 1);
        let identity = point_id(3_000 + u64::try_from(index).unwrap());
        for renderer in [&mut reference, &mut subject] {
            renderer.apply(&RenderUpdate::Reset { view_generation });
            renderer.apply(&RenderUpdate::Upsert {
                batch: batch(
                    view_generation,
                    1,
                    1,
                    WORLD_ORIGIN,
                    vec![point([0.0; 3], RED, identity.ordinal())],
                ),
            });
        }
        let style = PointStyle::new(7.0, [1.0; 3], [0.0, 0.0, 0.0, 1.0])
            .unwrap()
            .with_display_size_pixels(18.0)
            .unwrap();
        let frame = frame_with_style(view_generation, VIEWPORT, style);
        let reference = reference.render(&frame);
        let rendered = subject.render(&frame);
        assert!(rendered.report.eye_dome_lighting_applied());

        let nominal_outer_radius = f64::from(style.default_size_pixels()) / 2.0 + 0.75;
        let mut outer_display_pixels = 0_u64;
        let mut shaded_nominal_pixels = 0_u64;
        for y in 0..VIEWPORT[1] {
            for x in 0..VIEWPORT[0] {
                let reference_pixel = reference.image.pixel([x, y]);
                if reference_pixel == BLACK {
                    continue;
                }
                let dx = f64::from(x) + 0.5 - f64::from(CENTER[0]);
                let dy = f64::from(y) + 0.5 - f64::from(CENTER[1]);
                if dx.mul_add(dx, dy * dy) > nominal_outer_radius * nominal_outer_radius {
                    outer_display_pixels += 1;
                    assert_eq!(
                        rendered.image.pixel([x, y]),
                        reference_pixel,
                        "{footprint:?} EDL visibility depth leaked into display-only pixel [{x}, {y}]"
                    );
                } else if rendered.image.pixel([x, y]) != reference_pixel {
                    shaded_nominal_pixels += 1;
                }
            }
        }
        assert!(outer_display_pixels > 0);
        assert!(
            shaded_nominal_pixels > 0,
            "{footprint:?} EDL fixture did not shade"
        );
    }
}

fn assert_resource_fallback_suppresses_eye_dome(gpu: &GpuContext) {
    let cue = EyeDomeLighting::new(1.25, 1).unwrap();
    let config = RendererConfig::new(FORMAT, roomy_limits())
        .with_eye_dome_lighting(cue)
        .with_point_footprint(PointFootprint::Antialiased);
    let mut subject = OffscreenRenderer::with_config(gpu, config);
    let view_generation = ViewGenerationKey::new(ViewId::new(15), 1);
    subject.apply(&RenderUpdate::Reset { view_generation });

    let fallback_viewport = [1_281, 1_024];
    let frame = standard_frame(view_generation, fallback_viewport, 18.0, GREEN);
    assert_eq!(subject.renderer.depth_cue_status(), DepthCueStatus::Active);
    assert_eq!(
        subject.renderer.point_footprint_status(frame.viewport()),
        PointFootprintStatus::ResourceFallback
    );

    let rendered = subject.render(&frame);
    let pixels = u64::from(fallback_viewport[0]) * u64::from(fallback_viewport[1]);
    assert!(!rendered.report.eye_dome_lighting_applied());
    assert_eq!(rendered.report.transient_texture_bytes(), pixels * 4);
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        rendered.report.transient_texture_bytes()
    );

    assert_eq!(
        subject.pick_and_wait(&rendered.recorded_frame, [0, 0]),
        None
    );
    assert_eq!(subject.renderer.transient_texture_bytes(), pixels * 8);
    let after_pick = subject.render(&frame);
    assert!(!after_pick.report.eye_dome_lighting_applied());
    assert_eq!(after_pick.report.transient_texture_bytes(), pixels * 8);
    assert_eq!(
        subject.renderer.transient_texture_bytes(),
        after_pick.report.transient_texture_bytes()
    );
}

fn write_point_footprint_test_evidence_with_gpu(gpu: &GpuContext) {
    let output_path = point_footprint_evidence_output_path();
    let implementation_commit = implementation_head();
    let environment = LocalEvidenceEnvironment::new(gpu);
    let measurements = collect_local_evidence_measurements(gpu, &environment);
    let cases = local_evidence_cases(&measurements);
    let document = serde_json::json!({
        "schema": "punctra-render-wgpu-point-footprint-test-evidence-v1",
        "implementation_commit": implementation_commit,
        "producer_command": LOCAL_EVIDENCE_PRODUCER_COMMAND,
        "environment": environment.as_json(),
        "cases": cases,
    });
    let mut bytes = serde_json::to_vec_pretty(&document)
        .expect("the bounded local test evidence should serialize");
    bytes.push(b'\n');
    fs::write(&output_path, bytes).unwrap_or_else(|error| {
        panic!(
            "failed to write local Point-footprint evidence to {}: {error}",
            output_path.display()
        )
    });
}

struct LocalEvidenceEnvironment {
    operating_system: &'static str,
    adapter_name: String,
    backend: String,
}

impl LocalEvidenceEnvironment {
    fn new(gpu: &GpuContext) -> Self {
        let adapter_info = gpu.device.adapter_info();
        let adapter_name = if adapter_info.name.trim().is_empty() {
            "local wgpu adapter".to_owned()
        } else {
            adapter_info.name
        };
        Self {
            operating_system: env::consts::OS,
            adapter_name,
            backend: format!("{:?}", adapter_info.backend),
        }
    }

    fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "operating_system": self.operating_system,
            "adapter_name": self.adapter_name,
            "backend": self.backend,
        })
    }
}

struct LocalEvidenceMeasurements {
    single_facts: serde_json::Value,
    unsupported_facts: serde_json::Value,
    quality: FootprintQualityMeasurements,
    pick: PickIndependenceMeasurements,
    resource_facts: serde_json::Value,
}

fn collect_local_evidence_measurements(
    gpu: &GpuContext,
    environment: &LocalEvidenceEnvironment,
) -> LocalEvidenceMeasurements {
    let single_facts = collect_private_unit_test_facts(
        "single_sample_request_never_becomes_a_fallback",
        Some(environment),
    );
    let unsupported_facts = collect_private_unit_test_facts(
        "capability_fallback_precedes_the_viewport_resource_check",
        Some(environment),
    );
    let resource_contract_facts = collect_private_unit_test_facts(
        "exact_high_water_accounts_for_pick_and_eye_dome_targets",
        None,
    );

    let quality = measure_antialiased_footprint_quality_matrix(gpu);
    let pick = measure_multisample_footprint_and_pick(gpu);
    let edl_bytes_per_pixel = measure_multisample_eye_dome_composition(gpu);
    let fallback = measure_multisample_resource_fallback(gpu);
    let measured_resource_facts = serde_json::json!({
        "transient_bounds": {
            "preferred_non_edl_bytes_per_pixel": pick.transient_bytes_per_pixel,
            "preferred_edl_bytes_per_pixel": edl_bytes_per_pixel,
            "fallback_bytes_per_pixel": fallback.bytes_per_pixel,
            "maximum_preferred_physical_pixels": 1_310_720_u64,
            "maximum_preferred_transient_bytes": 62_914_560_u64,
            "renderer_transient_byte_ceiling": 67_108_864_u64,
        },
    });
    assert_eq!(
        measured_resource_facts, resource_contract_facts,
        "GPU-measured transient costs must match the private resource-bound unit test"
    );
    let mut resource_facts = resource_contract_facts;
    resource_facts
        .as_object_mut()
        .expect("private resource facts are an object")
        .insert("resource_fallback".to_owned(), fallback.proof);
    LocalEvidenceMeasurements {
        single_facts,
        unsupported_facts,
        quality,
        pick,
        resource_facts,
    }
}

fn local_evidence_cases(measurements: &LocalEvidenceMeasurements) -> Vec<serde_json::Value> {
    vec![
        selection_evidence_case(
            "single_sample_request_never_becomes_a_fallback",
            &measurements.single_facts,
        ),
        selection_evidence_case(
            "capability_fallback_precedes_the_viewport_resource_check",
            &measurements.unsupported_facts,
        ),
        quality_evidence_case(measurements.quality),
        pick_evidence_case(measurements.pick),
        resource_evidence_case(measurements),
    ]
}

fn selection_evidence_case(id: &'static str, facts: &serde_json::Value) -> serde_json::Value {
    evidence_case(id, facts)
}

fn quality_evidence_case(quality: FootprintQualityMeasurements) -> serde_json::Value {
    let facts = serde_json::json!({
        "diameters_physical_pixels": [2, 3, 4, 5, 6],
        "subpixel_center_phases": FOOTPRINT_CENTER_PHASES,
        "preferred": {
            "maximum_coverage_rmse": quality.maximum_coverage_rmse,
            "maximum_exact_distance_outer_leakage_pixels":
                quality.maximum_exact_distance_outer_leakage_pixels,
            "all_centers_foreground": quality.all_centers_foreground,
            "all_quad_corners_clear": quality.all_quad_corners_clear,
        },
        "single_sample": {
            "coverage_rmse_at_preferred_worst_case":
                quality.coverage_rmse_at_preferred_worst_case,
        },
    });
    evidence_case("antialiased_footprint_quality_matrix", &facts)
}

fn pick_evidence_case(pick: PickIndependenceMeasurements) -> serde_json::Value {
    let facts = serde_json::json!({
        "pick_independence": {
            "display_diameter_physical_pixels": pick.display_diameter_physical_pixels,
            "nominal_pick_diameter_physical_pixels": pick.nominal_pick_diameter_physical_pixels,
            "visual_only_probe_offset_physical_pixels":
                pick.visual_only_probe_offset_physical_pixels,
            "visual_only_probe_result": if pick.visual_only_probe_missed {
                "miss"
            } else {
                "unexpected_hit"
            },
            "nominal_probe_result": if pick.nominal_probe_matched {
                "expected_identity"
            } else {
                "unexpected_identity"
            },
        },
    });
    evidence_case(
        "four_sample_edges_resolve_partial_coverage_and_keep_nominal_picking",
        &facts,
    )
}

fn resource_evidence_case(measurements: &LocalEvidenceMeasurements) -> serde_json::Value {
    evidence_case(
        "exact_high_water_accounts_for_pick_and_eye_dome_targets",
        &measurements.resource_facts,
    )
}

fn evidence_case(id: &'static str, facts: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "source_test": id,
        "passed": true,
        "facts": facts,
    })
}

fn point_footprint_evidence_output_path() -> PathBuf {
    let requested = PathBuf::from(
        env::var_os("PUNCTRA_POINT_FOOTPRINT_EVIDENCE_PATH")
            .expect("PUNCTRA_POINT_FOOTPRINT_EVIDENCE_PATH must name the JSON output"),
    );
    if requested.is_absolute() {
        requested
    } else {
        repository_root().join(requested)
    }
}

fn implementation_head() -> String {
    assert_repository_is_clean();
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repository_root())
        .output()
        .expect("git must be available to bind local test evidence to HEAD");
    assert!(output.status.success(), "git rev-parse HEAD failed");
    let head = String::from_utf8(output.stdout)
        .expect("Git HEAD must be UTF-8")
        .trim()
        .to_owned();
    assert!(
        head.len() == 40
            && head
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "Git HEAD must be one full lowercase object id"
    );
    if let Ok(supplied) = env::var("PUNCTRA_IMPLEMENTATION_COMMIT") {
        assert_eq!(
            supplied, head,
            "PUNCTRA_IMPLEMENTATION_COMMIT must equal the full current Git HEAD"
        );
    }
    head
}

fn assert_repository_is_clean() {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .current_dir(repository_root())
        .output()
        .expect("git must be available to verify local test evidence source bytes");
    assert!(output.status.success(), "git status --porcelain failed");
    assert!(
        output.stdout.is_empty(),
        "local test evidence requires a clean implementation tree; Git reported:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

fn collect_private_unit_test_facts(
    source_test: &str,
    expected_environment: Option<&LocalEvidenceEnvironment>,
) -> serde_json::Value {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the host clock must be after the Unix epoch")
        .as_nanos();
    let facts_path = env::temp_dir().join(format!(
        "punctra-point-footprint-{}-{nonce}-{source_test}.json",
        std::process::id()
    ));
    let full_test_name = format!("footprint::tests::{source_test}");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["test", "-p", "render-wgpu", "--lib", &full_test_name])
        .args(["--", "--exact"])
        .env(PRIVATE_FACTS_PATH_ENV, &facts_path)
        .current_dir(repository_root())
        .output()
        .expect("cargo must be available to run private Point-footprint unit evidence");
    assert!(
        output.status.success(),
        "private source test {source_test} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(&facts_path).unwrap_or_else(|error| {
        panic!(
            "private source test {source_test} did not write {}: {error}",
            facts_path.display()
        )
    });
    fs::remove_file(&facts_path).unwrap_or_else(|error| {
        panic!(
            "failed to remove private source-test facts {}: {error}",
            facts_path.display()
        )
    });
    let output: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!("private source test {source_test} wrote invalid JSON: {error}")
    });
    let Some(expected_environment) = expected_environment else {
        return output;
    };
    let object = output
        .as_object()
        .expect("GPU-backed private source-test output is an object");
    assert_eq!(
        object.len(),
        2,
        "GPU-backed private source-test output has only environment and facts"
    );
    assert_eq!(
        object.get("environment"),
        Some(&expected_environment.as_json()),
        "private source test {source_test} ran on a different GPU environment"
    );
    object
        .get("facts")
        .expect("GPU-backed private source-test output includes facts")
        .clone()
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
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
        let target = ColorTarget::new(
            &self.gpu.device,
            frame.viewport().dimensions(),
            FORMAT,
            "punctra acceptance color target",
        );
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
        let first_target = ColorTarget::new(
            &self.gpu.device,
            first_frame.viewport().dimensions(),
            FORMAT,
            "punctra first deferred color target",
        );
        let second_target = ColorTarget::new(
            &self.gpu.device,
            second_frame.viewport().dimensions(),
            FORMAT,
            "punctra second deferred color target",
        );
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
        let submission = self.gpu.queue.submit([commands]);
        self.gpu.wait_for_submission(
            &submission,
            MAX_PICK_COMPLETION_TIME,
            "offscreen pick",
            || match ticket.poll().expect("the submitted pick should resolve") {
                PickPoll::Ready(hit) => Some(hit),
                PickPoll::Pending => None,
            },
        )
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

fn footprint_quality_frame(view_generation: ViewGenerationKey, point_size: f32) -> Frame {
    let vertical_world_size = f64::from(FOOTPRINT_QUALITY_VIEWPORT[1])
        / f64::from(FOOTPRINT_QUALITY_PIXELS_PER_WORLD_UNIT);
    let camera = Camera::orthographic(
        [WORLD_ORIGIN[0], WORLD_ORIGIN[1] - 5.0, WORLD_ORIGIN[2]],
        WORLD_ORIGIN,
        [0.0, 0.0, 1.0],
        vertical_world_size,
        0.1,
        100.0,
    )
    .expect("the footprint quality camera should be valid");
    let style = PointStyle::new(point_size, [1.0; 3], [0.0, 0.0, 0.0, 1.0])
        .expect("the footprint quality style should be valid");
    Frame::new(
        view_generation,
        camera,
        Viewport::new(FOOTPRINT_QUALITY_VIEWPORT[0], FOOTPRINT_QUALITY_VIEWPORT[1]).unwrap(),
    )
    .expect("the footprint quality frame should be valid")
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

fn is_partial_red(pixel: [u8; 4]) -> bool {
    (1..u8::MAX).contains(&pixel[0]) && pixel[1] <= 1 && pixel[2] <= 1
}

fn is_cyan(pixel: [u8; 4]) -> bool {
    pixel[0] < 40 && pixel[1] > 200 && pixel[2] > 200
}
