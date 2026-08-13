//! Public-interface coverage for exact CPU screen review and pick confirmation.

use std::{fs, io, mem, path::Path};

use point_contracts::{
    AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
    AttributeSchema, AttributeValues, CoordinateReference, PointId, PositionTransform, SourceId,
    SourceMetadata, WorldBounds,
};
use point_index::{PrepareLimits, prepare};
use point_review::{
    ProjectionStage, ReviewError, ReviewResource, ScreenRect, ScreenReviewLimits, ScreenSelection,
    confirm_pick, screen_through,
};
use point_workspace::{
    CommitLimits, CommitOutcome, CommitRequest, OpenLimits, OperationId, PointIdReadLimits,
    PointRowLimits, PointSetLimits, WorkspaceSchema, create,
};
use render_protocol::{Camera, CameraProjection, Viewport};
use source_memory::{MemoryFaultControl, MemorySource};

const CLASSIFICATION: u32 = 101;

#[test]
fn perspective_screen_through_is_inclusive_exact_and_revision_pinned() {
    let fixture = Fixture::new("perspective");
    let snapshot = fixture.workspace.head();
    let selection = ScreenSelection::new(
        ScreenRect::new([70.0, 70.0], [30.0, 30.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap()
    .classification_is(2);

    for row_batch_points in [1, 3, 64] {
        let limits = review_limits_with_row_batches(row_batch_points);
        let inspection = screen_through(&snapshot, selection, limits)
            .blocking_wait()
            .unwrap();

        assert_eq!(ordinals(inspection.points()), [0, 1, 3, 4, 5, 6]);
        let summary = *inspection.summary();
        assert_eq!(summary.provenance(), *snapshot.provenance());
        assert_eq!(summary.selection(), selection);
        assert_eq!(summary.candidate_point_count(), 11);
        assert_eq!(summary.examined_point_count(), 11);
        assert_eq!(summary.exact_count(), 6);
        assert_eq!(
            summary.point_id_hash(),
            inspection.points().metadata().point_id_hash()
        );
        assert!(summary.accounted_peak_working_bytes() > 0);
        assert!(summary.accounted_peak_working_bytes() <= limits.max_working_bytes());
    }
}

#[test]
fn independent_projection_oracle_matches_varied_cameras_rectangles_filters_and_batches() {
    let transform = PositionTransform::new([12_000.0, -8_000.0, 400.0], [0.25, 0.5, 0.25]).unwrap();
    let ticks = vec![
        [-24, -8, -8],
        [-12, 4, -16],
        [0, 0, -20],
        [8, 6, -24],
        [20, -10, -32],
        [40, 0, -20],
        [0, 20, -4],
        [0, 0, 8],
        [-6, 2, -80],
    ];
    let classes = vec![1, 2, 2, 3, 2, 2, 1, 2, 2];
    let fixture = Fixture::from_rows(
        "projection-oracle",
        transform,
        ticks.clone(),
        classes.clone(),
    );
    let snapshot = fixture.workspace.head();
    let viewport = Viewport::new(320, 180).unwrap();
    let perspective = Camera::perspective(
        [12_000.0, -8_000.0, 400.0],
        [12_000.0, -8_000.0, 399.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_3,
        1.0,
        30.0,
    )
    .unwrap();
    let orthographic = Camera::orthographic(
        [12_000.0, -8_000.0, 400.0],
        [12_000.0, -8_000.0, 399.0],
        [0.0, 1.0, 0.0],
        12.0,
        1.0,
        30.0,
    )
    .unwrap();
    let requests = [
        (perspective, [0.0, 0.0], [320.0, 180.0], None),
        (perspective, [55.0, 30.0], [245.0, 155.0], Some(2)),
        (orthographic, [0.0, 0.0], [320.0, 180.0], None),
        (orthographic, [80.0, 40.0], [160.0, 90.0], Some(2)),
        (orthographic, [160.0, 90.0], [160.0, 90.0], None),
        (perspective, [0.0, 0.0], [1.0, 1.0], None),
    ];

    for (camera, first, second, filter) in requests {
        let mut selection =
            ScreenSelection::new(ScreenRect::new(first, second).unwrap(), camera, viewport)
                .unwrap();
        if let Some(filter) = filter {
            selection = selection.classification_is(filter);
        }
        let expected = projection_oracle(transform, &ticks, &classes, selection);
        for row_batch_points in [1, 2, 5, 64] {
            let inspection = screen_through(
                &snapshot,
                selection,
                review_limits_with_row_batches(row_batch_points),
            )
            .blocking_wait()
            .unwrap();
            assert_eq!(ordinals(inspection.points()), expected);
            assert_eq!(inspection.summary().examined_point_count(), 9);
        }
    }
}

#[test]
fn non_square_orthographic_viewport_includes_all_four_normalized_boundaries() {
    let fixture = Fixture::from_rows(
        "non-square",
        identity_transform(),
        vec![
            [-10, 0, -5],
            [10, 0, -5],
            [0, 5, -5],
            [0, -5, -5],
            [11, 0, -5],
            [0, 0, -5],
        ],
        vec![2; 6],
    );
    let snapshot = fixture.workspace.head();
    let camera = Camera::orthographic(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        10.0,
        1.0,
        10.0,
    )
    .unwrap();
    let viewport = Viewport::new(200, 100).unwrap();
    let full = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [200.0, 100.0]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();

    for row_batch_points in [1, 4, 64] {
        let inspection = screen_through(
            &snapshot,
            full,
            review_limits_with_row_batches(row_batch_points),
        )
        .blocking_wait()
        .unwrap();
        assert_eq!(ordinals(inspection.points()), [0, 1, 2, 3, 5]);
    }

    let one_pixel_boundary = ScreenSelection::new(
        ScreenRect::new([100.0, 50.0], [100.0, 50.0]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();
    let inspection = screen_through(&snapshot, one_pixel_boundary, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(ordinals(inspection.points()), [5]);

    let empty = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [1.0, 1.0]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();
    let inspection = screen_through(&snapshot, empty, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert!(ordinals(inspection.points()).is_empty());
    assert_eq!(inspection.summary().exact_count(), 0);
}

#[test]
fn one_pixel_viewport_uses_continuous_coordinates_and_top_left_y() {
    let fixture = Fixture::from_rows(
        "one-pixel-top-left",
        identity_transform(),
        vec![[0, 1, -5], [0, -1, -5], [0, 0, -5]],
        vec![2; 3],
    );
    let snapshot = fixture.workspace.head();
    let camera = perspective_camera();
    let viewport = Viewport::new(1, 1).unwrap();

    let center = ScreenSelection::new(
        ScreenRect::new([0.5, 0.5], [0.5, 0.5]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();
    let inspection = screen_through(&snapshot, center, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(ordinals(inspection.points()), [2]);

    let top_half = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [1.0, 0.5]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();
    let inspection = screen_through(&snapshot, top_half, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(ordinals(inspection.points()), [0, 2]);
}

#[test]
fn inclusive_clip_side_and_rectangle_edges_exclude_adjacent_exact_ticks() {
    let transform = PositionTransform::new([0.0; 3], [0.001; 3]).unwrap();
    let ticks = vec![
        [0, 0, -1000],
        [0, 0, -1001],
        [0, 0, -999],
        [0, 0, -10_000],
        [0, 0, -9_999],
        [0, 0, -10_001],
        [5_000, 0, -5_000],
        [4_999, 0, -5_000],
        [5_001, 0, -5_000],
        [-5_000, 0, -5_000],
        [-4_999, 0, -5_000],
        [-5_001, 0, -5_000],
        [0, 5_000, -5_000],
        [0, -5_000, -5_000],
        [0, 5_001, -5_000],
        [0, -5_001, -5_000],
    ];
    let fixture = Fixture::from_rows("adjacent-boundaries", transform, ticks, vec![2; 16]);
    let snapshot = fixture.workspace.head();
    let camera = Camera::orthographic(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        10.0,
        1.0,
        10.0,
    )
    .unwrap();
    let viewport = Viewport::new(100, 100).unwrap();
    let full = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();
    let inspection = screen_through(&snapshot, full, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(
        ordinals(inspection.points()),
        [0, 1, 3, 4, 6, 7, 9, 10, 12, 13]
    );

    let rectangle = ScreenSelection::new(
        ScreenRect::new([49.99, 49.99], [50.01, 50.01]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();
    let inspection = screen_through(&snapshot, rectangle, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(ordinals(inspection.points()), [0, 1, 3, 4]);

    let rectangle_boundary_fixture = Fixture::from_rows(
        "rectangle-boundaries",
        identity_transform(),
        vec![[0, -1, -5], [0, 0, -5], [0, -2, -5]],
        vec![2; 3],
    );
    let rectangle_boundary = ScreenSelection::new(
        ScreenRect::new([50.0, 60.0], [50.0, 60.0]).unwrap(),
        camera,
        viewport,
    )
    .unwrap();
    let inspection = screen_through(
        &rectangle_boundary_fixture.workspace.head(),
        rectangle_boundary,
        ScreenReviewLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(ordinals(inspection.points()), [0]);

    let signed_zero = ScreenRect::new([-0.0, 0.0], [0.0, -0.0]).unwrap();
    assert!(signed_zero.min()[0] == 0.0 && signed_zero.max()[1] == 0.0);
    ScreenSelection::new(signed_zero, camera, viewport).unwrap();
}

#[test]
fn nondegenerate_rectangle_includes_each_edge_and_excludes_one_tick_outside() {
    let transform = PositionTransform::new([0.0; 3], [0.001; 3]).unwrap();
    let fixture = Fixture::from_rows(
        "nondegenerate-rectangle-edges",
        transform,
        vec![
            [-2_000, 0, -5_000],
            [2_000, 0, -5_000],
            [0, 3_000, -5_000],
            [0, -3_000, -5_000],
            [0, 0, -5_000],
            [-2_001, 0, -5_000],
            [2_001, 0, -5_000],
            [0, 3_001, -5_000],
            [0, -3_001, -5_000],
        ],
        vec![2; 9],
    );
    let camera = Camera::orthographic(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        10.0,
        1.0,
        10.0,
    )
    .unwrap();
    let selection = ScreenSelection::new(
        ScreenRect::new([30.0, 20.0], [70.0, 80.0]).unwrap(),
        camera,
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();

    let inspection = screen_through(
        &fixture.workspace.head(),
        selection,
        ScreenReviewLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    assert_eq!(ordinals(inspection.points()), [0, 1, 2, 3, 4]);
}

#[test]
fn exact_screen_through_is_independent_of_display_visibility_policy() {
    let fixture = Fixture::from_rows(
        "visibility-independent",
        identity_transform(),
        vec![[0, 0, -2], [0, 0, -5], [0, 0, -9]],
        vec![2, 2, 2],
    );
    let snapshot = fixture.workspace.head();
    let selection = ScreenSelection::new(
        ScreenRect::new([50.0, 50.0], [50.0, 50.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();
    let inspection = screen_through(&snapshot, selection, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();

    // These coincident centers can be mutually occluded, transparent, or absent
    // from a progressive display. The review API has no renderer input, so all
    // three exact Snapshot rows remain members.
    assert_eq!(ordinals(inspection.points()), [0, 1, 2]);
    assert_eq!(inspection.summary().examined_point_count(), 3);
}

#[test]
fn perspective_projection_preserves_large_world_origin_precision() {
    let origin = 1_000_000_000_000_000.0;
    let transform = PositionTransform::new([origin; 3], [1.0; 3]).unwrap();
    let fixture = Fixture::from_rows(
        "large-origin",
        transform,
        vec![[0, 0, -5], [2, 0, -5]],
        vec![2, 2],
    );
    let snapshot = fixture.workspace.head();
    let camera = Camera::perspective(
        [origin; 3],
        [origin, origin, origin - 1.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        10.0,
    )
    .unwrap();
    let selection = ScreenSelection::new(
        ScreenRect::new([50.0, 50.0], [50.0, 50.0]).unwrap(),
        camera,
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();

    let inspection = screen_through(&snapshot, selection, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(ordinals(inspection.points()), [0]);
}

#[test]
fn classification_mismatch_does_not_skip_non_finite_projection() {
    let transform = PositionTransform::new([1.0e308, 0.0, 0.0], [1.0; 3]).unwrap();
    let fixture = Fixture::from_rows(
        "mismatching-non-finite",
        transform,
        vec![[0, 0, -5]],
        vec![1],
    );
    let snapshot = fixture.workspace.head();
    let camera = Camera::perspective(
        [-1.0e308, 0.0, 0.0],
        [-1.0e308, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        10.0,
    )
    .unwrap();
    let selection = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        camera,
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap()
    .classification_is(2);

    let error = screen_through(&snapshot, selection, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::NonFiniteProjection {
            point,
            stage: ProjectionStage::WorldMinusEye,
        }
            if point == PointId::new(fixture.source, 0)
    ));
}

#[test]
fn non_finite_projection_reports_each_constructible_stable_stage() {
    assert_projection_stage(
        "world-minus-eye",
        PositionTransform::new([1.0e308, 0.0, 0.0], [1.0; 3]).unwrap(),
        [0, 0, -5],
        Camera::perspective(
            [-1.0e308, 0.0, 0.0],
            [-1.0e308, 0.0, -1.0],
            [0.0, 1.0, 0.0],
            std::f32::consts::FRAC_PI_2,
            1.0,
            10.0,
        )
        .unwrap(),
        ProjectionStage::WorldMinusEye,
    );

    // With validated finite world bounds and an orthonormal Camera basis, a
    // finite relative vector cannot overflow a three-term dot product: each
    // basis component is at most one and the relative vector's Euclidean norm
    // is finite. Valid f32 clip planes reject depth before perspective-scale
    // overflow, validated orthographic height cannot overflow its half-scale,
    // and a nonzero u32 Viewport cannot overflow pixel mapping. Those stable
    // stages remain defensive diagnostics but are not independently reachable
    // through accepted public inputs.
}

#[test]
fn exact_pick_confirmation_returns_only_snapshot_values() {
    let fixture = Fixture::new("pick");
    let snapshot = fixture.workspace.head();
    let picked = PointId::new(fixture.source, 2);

    let confirmed = confirm_pick(&snapshot, picked, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();

    assert_eq!(confirmed.point_id(), picked);
    assert_eq!(confirmed.ticks(), [2, 0, -5]);
    assert_eq!(confirmed.position_transform(), identity_transform());
    assert!(
        confirmed
            .world_position()
            .into_iter()
            .zip([2.0, 0.0, -5.0])
            .all(|(actual, expected)| (actual - expected).abs() <= f64::EPSILON)
    );
    assert_eq!(confirmed.effective_classification(), 1);
    assert_eq!(confirmed.provenance(), *snapshot.provenance());
    assert_eq!(ordinals(confirmed.points()), [2]);

    let foreign = PointId::new(SourceId::new([0x99; 32]), 2);
    let error = confirm_pick(&snapshot, foreign, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(error, ReviewError::Workspace(_)));

    let impossible = PointId::new(fixture.source, 11);
    let error = confirm_pick(&snapshot, impossible, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::Workspace(point_workspace::WorkspaceError::InvalidArgument { .. })
    ));

    let defaults = ScreenReviewLimits::default();
    let no_composition_working = ScreenReviewLimits::new(
        defaults.point_row_limits(),
        defaults.point_set_limits(),
        defaults.max_screen_matches(),
        0,
    );
    let error = confirm_pick(&snapshot, picked, no_composition_working)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::ResourceLimit {
            resource: ReviewResource::WorkingBytes,
            allowed: 0,
            ..
        }
    ));
}

#[test]
fn pick_confirmation_fails_if_the_verified_source_changes() {
    let (fixture, faults) = Fixture::controlled("changed-source");
    let snapshot = fixture.workspace.head();
    faults.mark_changed();

    let error = confirm_pick(
        &snapshot,
        PointId::new(fixture.source, 0),
        ScreenReviewLimits::default(),
    )
    .blocking_wait()
    .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::Workspace(point_workspace::WorkspaceError::Source {
            source: point_source::SourceError::SourceChanged { .. },
            ..
        })
    ));
}

#[test]
fn effective_overlay_values_drive_filtering_and_confirmation() {
    let fixture = Fixture::new("overlay");
    let root = fixture.workspace.head();
    let edited_point = PointId::new(fixture.source, 2);
    let target = root
        .select_point_ids([edited_point], PointSetLimits::default())
        .blocking_wait()
        .unwrap();
    let outcome = fixture
        .workspace
        .commit(
            CommitRequest::set_classification(
                OperationId::from_bytes([0x44; 16]).unwrap(),
                target,
                42,
            ),
            CommitLimits::default(),
        )
        .blocking_wait()
        .unwrap();
    assert!(matches!(outcome, CommitOutcome::Committed(_)));
    let edited = fixture.workspace.head();
    let selection = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap()
    .classification_is(42);

    let inspection = screen_through(&edited, selection, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(ordinals(inspection.points()), [2]);
    assert_eq!(inspection.summary().provenance(), *edited.provenance());

    let confirmed = confirm_pick(&edited, edited_point, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(confirmed.effective_classification(), 42);
    assert_eq!(confirmed.provenance(), *edited.provenance());

    let root_class_one = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap()
    .classification_is(1);
    let historical = screen_through(&root, root_class_one, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert_eq!(ordinals(historical.points()), [2]);
    assert_eq!(historical.summary().provenance(), *root.provenance());

    let root_class_42 = root_class_one.classification_is(42);
    let historical = screen_through(&root, root_class_42, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    assert!(ordinals(historical.points()).is_empty());
    assert_eq!(historical.summary().provenance(), *root.provenance());
}

#[test]
fn forced_spill_point_set_remains_exact_and_repeatable() {
    let fixture = Fixture::new("forced-spill");
    let snapshot = fixture.workspace.head();
    let selection = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();
    let baseline = fixture.stable_owned_file_footprint();
    let resident = screen_through(&snapshot, selection, ScreenReviewLimits::default())
        .blocking_wait()
        .unwrap();
    let with_resident = fixture.stable_owned_file_footprint();
    assert_eq!(with_resident, baseline);
    let limits = forced_spill_review_limits();

    let spilled = screen_through(&snapshot, selection, limits)
        .blocking_wait()
        .unwrap();
    let with_spill = fixture.stable_owned_file_footprint();
    let spill_delta = with_spill
        .checked_delta(with_resident)
        .expect("retained forced spill only grows owned fixture storage");
    assert!(spill_delta.files > 0);
    assert!(spill_delta.bytes > 0);
    let expected = [0, 1, 2, 3, 4, 5, 6, 8];
    assert_eq!(ordinals(resident.points()), expected);
    assert_eq!(ordinals(spilled.points()), expected);
    assert_eq!(resident.points().metadata(), spilled.points().metadata());
    assert_eq!(entries(resident.points()), entries(spilled.points()));
    assert_eq!(entries(spilled.points()), entries(spilled.points()));
    assert!(
        resident.summary().accounted_peak_working_bytes()
            <= ScreenReviewLimits::default().max_working_bytes()
    );
    assert!(spilled.summary().accounted_peak_working_bytes() <= limits.max_working_bytes());

    drop(spilled);
    let after_spill_release = fixture.stable_owned_file_footprint();
    assert_eq!(after_spill_release.bytes, with_resident.bytes);
}

#[test]
fn review_working_limit_combines_scan_growth_and_handoff_peaks() {
    let fixture = Fixture::new("combined-working");
    let snapshot = fixture.workspace.head();
    let selection = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();
    let defaults = ScreenReviewLimits::default();
    let row_peak = defaults.point_row_limits().max_working_bytes();
    let inspection = screen_through(&snapshot, selection, defaults)
        .blocking_wait()
        .unwrap();
    let minimum_first_growth = row_peak.saturating_add(
        u64::try_from(1_024_usize.saturating_mul(mem::size_of::<PointId>())).unwrap(),
    );
    assert!(inspection.summary().accounted_peak_working_bytes() >= minimum_first_growth);
    assert!(inspection.summary().accounted_peak_working_bytes() <= defaults.max_working_bytes());
    let row_only = ScreenReviewLimits::new(
        defaults.point_row_limits(),
        defaults.point_set_limits(),
        defaults.max_screen_matches(),
        row_peak,
    );

    let error = screen_through(&snapshot, selection, row_only)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::ResourceLimit {
            resource: ReviewResource::WorkingBytes,
            required,
            allowed,
        } if required > row_peak && allowed == row_peak
    ));

    let point_set = defaults.point_set_limits();
    let point_set_peak = row_peak.saturating_add(64 * 1024 * 1024);
    let larger_point_set_peak = PointSetLimits::new(
        point_set.candidate_limits(),
        point_set.source_read_budget(),
        point_set.max_input_point_ids(),
        point_set.max_output_points(),
        point_set.max_overlay_segments(),
        point_set.max_overlay_bytes(),
        point_set_peak,
        point_set.max_resident_bytes(),
        point_set.max_temporary_bytes(),
    );
    let handoff_complete = ScreenReviewLimits::new(
        defaults.point_row_limits(),
        larger_point_set_peak,
        defaults.max_screen_matches(),
        defaults.max_working_bytes(),
    );
    let inspection = screen_through(&snapshot, selection, handoff_complete)
        .blocking_wait()
        .unwrap();
    assert!(inspection.summary().accounted_peak_working_bytes() > point_set_peak);
    assert!(
        inspection.summary().accounted_peak_working_bytes() <= handoff_complete.max_working_bytes()
    );
    let allowed = row_peak.saturating_add(8 * 1024 * 1024);
    let handoff_too_small = ScreenReviewLimits::new(
        defaults.point_row_limits(),
        larger_point_set_peak,
        defaults.max_screen_matches(),
        allowed,
    );
    let error = screen_through(&snapshot, selection, handoff_too_small)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::ResourceLimit {
            resource: ReviewResource::WorkingBytes,
            required,
            allowed: actual_allowed,
        } if required > point_set_peak && actual_allowed == allowed
    ));
}

#[test]
fn invalid_rectangles_and_review_limits_fail_without_partial_inspection() {
    assert!(matches!(
        ScreenRect::new([f64::NAN, 0.0], [1.0, 1.0]),
        Err(ReviewError::NonFiniteScreenCoordinate {
            endpoint: "first",
            axis: 0
        })
    ));

    let viewport = Viewport::new(100, 50).unwrap();
    assert!(matches!(
        ScreenSelection::new(
            ScreenRect::new([-0.25, 0.0], [10.0, 10.0]).unwrap(),
            perspective_camera(),
            viewport
        ),
        Err(ReviewError::ScreenCoordinateOutsideViewport {
            boundary: "minimum",
            axis: 0,
            ..
        })
    ));
    assert!(matches!(
        ScreenSelection::new(
            ScreenRect::new([0.0, 0.0], [100.25, 50.0]).unwrap(),
            perspective_camera(),
            viewport
        ),
        Err(ReviewError::ScreenCoordinateOutsideViewport {
            boundary: "maximum",
            axis: 0,
            ..
        })
    ));
    assert!(matches!(
        ScreenSelection::new(
            ScreenRect::new([0.0, 0.0], [100.0, 50.25]).unwrap(),
            perspective_camera(),
            viewport
        ),
        Err(ReviewError::ScreenCoordinateOutsideViewport {
            boundary: "maximum",
            axis: 1,
            ..
        })
    ));
    ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 50.0]).unwrap(),
        perspective_camera(),
        viewport,
    )
    .unwrap();

    let fixture = Fixture::new("limits");
    let snapshot = fixture.workspace.head();
    let selection = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();
    let defaults = ScreenReviewLimits::default();
    let no_matches = ScreenReviewLimits::new(
        defaults.point_row_limits(),
        defaults.point_set_limits(),
        0,
        defaults.max_working_bytes(),
    );
    let error = screen_through(&snapshot, selection, no_matches)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::ResourceLimit {
            resource: ReviewResource::ScreenMatches,
            required: 1,
            allowed: 0
        }
    ));

    let no_memory = ScreenReviewLimits::new(
        defaults.point_row_limits(),
        defaults.point_set_limits(),
        100,
        0,
    );
    let error = screen_through(&snapshot, selection, no_memory)
        .blocking_wait()
        .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::ResourceLimit {
            resource: ReviewResource::WorkingBytes,
            allowed: 0,
            ..
        }
    ));
}

#[test]
fn cancellation_prevents_terminal_publication() {
    let fixture = Fixture::new("cancel");
    let snapshot = fixture.workspace.head();
    let selection = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        perspective_camera(),
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();
    let job = screen_through(&snapshot, selection, ScreenReviewLimits::default());
    job.handle().cancel();

    let error = job.blocking_wait().unwrap_err();
    assert!(matches!(
        error,
        ReviewError::Runtime(foundation_runtime::RuntimeError::Cancelled)
            | ReviewError::Workspace(point_workspace::WorkspaceError::Cancelled)
    ));
}

fn perspective_camera() -> Camera {
    Camera::perspective(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        10.0,
    )
    .unwrap()
}

fn identity_transform() -> PositionTransform {
    PositionTransform::new([0.0; 3], [1.0; 3]).unwrap()
}

fn forced_spill_review_limits() -> ScreenReviewLimits {
    let defaults = ScreenReviewLimits::default();
    let point_set = defaults.point_set_limits();
    let forced_spill = PointSetLimits::new(
        point_set.candidate_limits(),
        point_set.source_read_budget(),
        point_set.max_input_point_ids(),
        point_set.max_output_points(),
        point_set.max_overlay_segments(),
        point_set.max_overlay_bytes(),
        point_set.max_working_bytes(),
        0,
        point_set.max_temporary_bytes(),
    );
    ScreenReviewLimits::new(
        defaults.point_row_limits(),
        forced_spill,
        defaults.max_screen_matches(),
        defaults.max_working_bytes(),
    )
}

fn review_limits_with_row_batches(max_batch_points: u64) -> ScreenReviewLimits {
    const ROW_PAYLOAD_BYTES: u64 = 8 + 24 + 1;

    let defaults = ScreenReviewLimits::default();
    let rows = defaults.point_row_limits();
    let rows = PointRowLimits::new(
        rows.candidate_limits(),
        rows.source_read_budget(),
        rows.max_overlay_segments(),
        rows.max_overlay_bytes(),
        rows.max_output_points(),
        max_batch_points,
        max_batch_points.saturating_mul(ROW_PAYLOAD_BYTES),
        rows.max_working_bytes(),
    );
    ScreenReviewLimits::new(
        rows,
        defaults.point_set_limits(),
        defaults.max_screen_matches(),
        defaults.max_working_bytes(),
    )
}

fn projection_oracle(
    transform: PositionTransform,
    ticks: &[[i64; 3]],
    classes: &[u8],
    selection: ScreenSelection,
) -> Vec<u64> {
    let camera = selection.camera();
    let basis = camera.world_basis();
    let viewport = selection.viewport();
    let aspect = f64::from(viewport.width()) / f64::from(viewport.height());
    ticks
        .iter()
        .zip(classes)
        .enumerate()
        .filter_map(|(ordinal, (&ticks, &classification))| {
            let world = transform.world_f64(ticks);
            let relative = [
                world[0] - camera.eye()[0],
                world[1] - camera.eye()[1],
                world[2] - camera.eye()[2],
            ];
            let dot = |axis: [f64; 3]| {
                axis[0] * relative[0] + axis[1] * relative[1] + axis[2] * relative[2]
            };
            let depth = dot(basis.forward());
            if depth < f64::from(camera.near_distance()) || depth > f64::from(camera.far_distance())
            {
                return None;
            }
            let horizontal = dot(basis.right());
            let vertical = dot(basis.up());
            let [ndc_x, ndc_y] = match camera.projection() {
                CameraProjection::Perspective {
                    vertical_field_of_view_radians,
                } => {
                    if depth <= 0.0 {
                        return None;
                    }
                    let half_height =
                        depth * (f64::from(vertical_field_of_view_radians) * 0.5).tan();
                    [horizontal / (half_height * aspect), vertical / half_height]
                }
                CameraProjection::Orthographic {
                    vertical_world_height,
                } => {
                    let half_height = vertical_world_height / 2.0;
                    [horizontal / (half_height * aspect), vertical / half_height]
                }
            };
            if !(-1.0..=1.0).contains(&ndc_x) || !(-1.0..=1.0).contains(&ndc_y) {
                return None;
            }
            let pixel = [
                (ndc_x + 1.0) * f64::from(viewport.width()) / 2.0,
                (1.0 - ndc_y) * f64::from(viewport.height()) / 2.0,
            ];
            let rect = selection.rect();
            let inside = pixel[0] >= rect.min()[0]
                && pixel[0] <= rect.max()[0]
                && pixel[1] >= rect.min()[1]
                && pixel[1] <= rect.max()[1];
            let class_matches = selection
                .classification()
                .is_none_or(|expected| expected == classification);
            (inside && class_matches).then(|| u64::try_from(ordinal).unwrap())
        })
        .collect()
}

fn assert_projection_stage(
    label: &str,
    transform: PositionTransform,
    ticks: [i64; 3],
    camera: Camera,
    expected: ProjectionStage,
) {
    let fixture = Fixture::from_rows(label, transform, vec![ticks], vec![2]);
    let selection = ScreenSelection::new(
        ScreenRect::new([0.0, 0.0], [100.0, 100.0]).unwrap(),
        camera,
        Viewport::new(100, 100).unwrap(),
    )
    .unwrap();
    let error = screen_through(
        &fixture.workspace.head(),
        selection,
        ScreenReviewLimits::default(),
    )
    .blocking_wait()
    .unwrap_err();
    assert!(matches!(
        error,
        ReviewError::NonFiniteProjection { stage, .. } if stage == expected
    ));
}

fn ordinals(points: &point_workspace::PointSet) -> Vec<u64> {
    let point_bytes = u64::try_from(mem::size_of::<PointId>()).unwrap();
    let mut batches = points
        .ids(PointIdReadLimits::new(
            points.metadata().exact_count(),
            64,
            64 * point_bytes,
            64 * point_bytes,
            128 * point_bytes,
        ))
        .unwrap();
    let mut ordinals = Vec::new();
    while let Some(batch) = batches.next().unwrap() {
        ordinals.extend(batch.ids().iter().map(|point| point.ordinal()));
    }
    ordinals
}

fn entries(points: &point_workspace::PointSet) -> Vec<(PointId, u8)> {
    let entry_bytes = u64::try_from(mem::size_of::<point_workspace::PointSetEntry>()).unwrap();
    let mut batches = points
        .entries(PointIdReadLimits::new(
            points.metadata().exact_count(),
            64,
            64 * entry_bytes,
            1024 * 1024,
            2 * 1024 * 1024,
        ))
        .unwrap();
    let mut entries = Vec::new();
    while let Some(batch) = batches.next().unwrap() {
        entries.extend(
            batch
                .entries()
                .iter()
                .map(|entry| (entry.point_id(), entry.effective_classification())),
        );
    }
    entries
}

struct Fixture {
    workspace: point_workspace::Workspace,
    directory: tempfile::TempDir,
    source: SourceId,
}

impl Fixture {
    fn new(label: &str) -> Self {
        Self::from_rows(
            label,
            identity_transform(),
            vec![
                [0, 0, -5],
                [-2, 0, -5],
                [2, 0, -5],
                [0, 2, -5],
                [0, -2, -5],
                [0, 0, -1],
                [0, 0, -10],
                [0, 0, 1],
                [5, 0, -5],
                [0, 0, -11],
                [6, 0, -5],
            ],
            vec![2, 2, 1, 2, 2, 2, 2, 2, 2, 2, 2],
        )
    }

    fn from_rows(
        label: &str,
        transform: PositionTransform,
        ticks: Vec<[i64; 3]>,
        classes: Vec<u8>,
    ) -> Self {
        assert_eq!(ticks.len(), classes.len());
        let directory = tempfile::Builder::new()
            .prefix(&format!("punctra-point-review-{label}-"))
            .tempdir()
            .unwrap();
        let definition = AttributeDefinition::new(
            AttributeId::new(CLASSIFICATION).unwrap(),
            "classification",
            AttributeDataType::U8,
        )
        .unwrap();
        let column = AttributeColumn::new(definition, AttributeValues::u8(classes)).unwrap();
        let columns = AttributeColumns::new(vec![column], ticks.len()).unwrap();
        let memory =
            MemorySource::from_columns(transform, CoordinateReference::Unknown, ticks, columns)
                .unwrap();
        let source = source_memory::open(memory).blocking_wait().unwrap();
        let source_id = source.identity();
        let index = prepare(
            source,
            directory.path().join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        let workspace = create(
            directory.path().join("fixture.pcw"),
            index,
            WorkspaceSchema::new(AttributeId::new(CLASSIFICATION).unwrap()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        Self {
            workspace,
            directory,
            source: source_id,
        }
    }

    fn controlled(label: &str) -> (Self, MemoryFaultControl) {
        let ticks = vec![[0, 0, -5], [1, 0, -5]];
        let classes = vec![2, 2];
        let definition = AttributeDefinition::new(
            AttributeId::new(CLASSIFICATION).unwrap(),
            "classification",
            AttributeDataType::U8,
        )
        .unwrap();
        let column =
            AttributeColumn::new(definition.clone(), AttributeValues::u8(classes)).unwrap();
        let columns = AttributeColumns::new(vec![column], ticks.len()).unwrap();
        let transform = identity_transform();
        let schema = AttributeSchema::new(vec![definition]).unwrap();
        let worlds = ticks
            .iter()
            .copied()
            .map(|ticks| transform.world_f64(ticks))
            .collect::<Vec<_>>();
        let bounds = WorldBounds::new(worlds[0], worlds[1]).unwrap();
        let metadata = SourceMetadata::new(
            2,
            transform,
            CoordinateReference::Unknown,
            schema,
            Some(bounds),
            "memory",
            Vec::new(),
        )
        .unwrap();
        let (memory, faults) = MemorySource::with_fault_control(metadata, ticks, columns).unwrap();
        let source = source_memory::open(memory).blocking_wait().unwrap();
        let source_id = source.identity();
        let directory = tempfile::Builder::new()
            .prefix(&format!("punctra-point-review-{label}-"))
            .tempdir()
            .unwrap();
        let index = prepare(
            source,
            directory.path().join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        let workspace = create(
            directory.path().join("fixture.pcw"),
            index,
            WorkspaceSchema::new(AttributeId::new(CLASSIFICATION).unwrap()),
            OpenLimits::default(),
        )
        .blocking_wait()
        .unwrap();
        (
            Self {
                workspace,
                directory,
                source: source_id,
            },
            faults,
        )
    }

    fn stable_owned_file_footprint(&self) -> OwnedFileFootprint {
        let first = owned_file_footprint(self.directory.path())
            .expect("test reads its complete owned fixture footprint");
        let second = owned_file_footprint(self.directory.path())
            .expect("test repeats its complete owned fixture footprint");
        assert_eq!(first, second, "owned fixture footprint must be stable");
        first
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OwnedFileFootprint {
    files: u64,
    bytes: u64,
}

impl OwnedFileFootprint {
    fn checked_delta(self, baseline: Self) -> Option<Self> {
        Some(Self {
            files: self.files.checked_sub(baseline.files)?,
            bytes: self.bytes.checked_sub(baseline.bytes)?,
        })
    }
}

fn owned_file_footprint(root: &Path) -> io::Result<OwnedFileFootprint> {
    let mut footprint = OwnedFileFootprint::default();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let child = owned_file_footprint(&entry.path())?;
            footprint.files = footprint
                .files
                .checked_add(child.files)
                .ok_or_else(|| io::Error::other("owned fixture file-count footprint overflowed"))?;
            footprint.bytes = footprint
                .bytes
                .checked_add(child.bytes)
                .ok_or_else(|| io::Error::other("owned fixture byte footprint overflowed"))?;
        } else if file_type.is_file() {
            footprint.files = footprint
                .files
                .checked_add(1)
                .ok_or_else(|| io::Error::other("owned fixture file-count footprint overflowed"))?;
            footprint.bytes = footprint
                .bytes
                .checked_add(entry.metadata()?.len())
                .ok_or_else(|| io::Error::other("owned fixture byte footprint overflowed"))?;
        } else {
            return Err(io::Error::other(
                "owned fixture contains an unsupported non-file entry",
            ));
        }
    }
    Ok(footprint)
}
