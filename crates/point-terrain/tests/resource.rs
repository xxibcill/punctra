//! Invalid Ground Input, hard-limit, and cancellation evidence.

mod support;

use foundation_runtime::ProgressPhase;
use point_terrain::{TerrainError, TerrainLimits, TerrainRecipe};
use point_workspace::WorkspaceError;

use support::{TerrainFixture, derive_with, point_row_limits};

#[test]
fn fewer_than_three_ground_points_report_the_exact_count() {
    for count in 0..3_usize {
        let ticks = (0..count)
            .map(|ordinal| [i64::try_from(ordinal).unwrap(), 0, 0])
            .collect::<Vec<_>>();
        let fixture = TerrainFixture::new(&format!("insufficient-{count}"), ticks, vec![2; count]);
        assert!(matches!(
            derive_with(
                fixture.snapshot(),
                TerrainRecipe::new(2),
                TerrainLimits::default()
            ),
            Err(TerrainError::InsufficientGroundInput { actual })
                if actual == u64::try_from(count).unwrap()
        ));
    }
}

#[test]
fn duplicate_xy_distinguishes_equal_and_conflicting_elevations() {
    let same = TerrainFixture::new(
        "duplicate-xy-same-z",
        vec![[0, 0, 1], [0, 0, 1], [1, 0, 2], [0, 1, 3]],
        vec![2; 4],
    );
    assert!(matches!(
        derive_with(
            same.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::default()
        ),
        Err(TerrainError::DuplicateHorizontalPosition {
            first,
            second,
            conflicting_elevation: false,
        }) if first == same.point(0) && second == same.point(1)
    ));

    let conflicting = TerrainFixture::new(
        "duplicate-xy-different-z",
        vec![[0, 0, 1], [0, 0, 2], [1, 0, 2], [0, 1, 3]],
        vec![2; 4],
    );
    assert!(matches!(
        derive_with(
            conflicting.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::default()
        ),
        Err(TerrainError::DuplicateHorizontalPosition {
            first,
            second,
            conflicting_elevation: true,
        }) if first == conflicting.point(0) && second == conflicting.point(1)
    ));
}

#[test]
fn collinear_and_unsupported_numeric_ranges_are_explicit() {
    let collinear = TerrainFixture::new(
        "collinear",
        vec![[-2, -2, 0], [0, 0, 1], [3, 3, 2], [8, 8, 3]],
        vec![2; 4],
    );
    assert!(matches!(
        derive_with(
            collinear.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::default()
        ),
        Err(TerrainError::CollinearGroundInput)
    ));

    let beyond_exact_f64 = (1_i64 << 53) + 1;
    let numeric = TerrainFixture::new(
        "numeric-range",
        vec![[0, 0, 0], [beyond_exact_f64, 0, 1], [0, 1, 2]],
        vec![2; 3],
    );
    assert!(matches!(
        derive_with(
            numeric.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::default()
        ),
        Err(TerrainError::UnsupportedNumericRange { reason })
            if reason.as_str().contains("exact normalized f64 integer range")
    ));
}

#[test]
fn input_face_and_row_output_limits_fail_before_surface_publication() {
    let fixture = limit_fixture("output-limits");
    let defaults = TerrainLimits::default();
    assert_terrain_resource(
        derive_with(
            fixture.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::new(
                defaults.point_rows(),
                3,
                defaults.max_vertices(),
                defaults.max_faces(),
                defaults.max_working_bytes(),
                defaults.max_surface_bytes(),
                defaults.max_work_units(),
            ),
        ),
        "Ground Input Points",
        3,
    );
    assert_terrain_resource(
        derive_with(
            fixture.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::new(
                defaults.point_rows(),
                defaults.max_input_points(),
                defaults.max_vertices(),
                1,
                defaults.max_working_bytes(),
                defaults.max_surface_bytes(),
                defaults.max_work_units(),
            ),
        ),
        "Terrain faces",
        1,
    );
    let row_limited = TerrainLimits::new(
        point_row_limits(3, 2),
        defaults.max_input_points(),
        defaults.max_vertices(),
        defaults.max_faces(),
        defaults.max_working_bytes(),
        defaults.max_surface_bytes(),
        defaults.max_work_units(),
    );
    match derive_with(fixture.snapshot(), TerrainRecipe::new(2), row_limited) {
        Err(TerrainError::Workspace { source, .. }) => assert!(matches!(
            source.as_ref(),
            WorkspaceError::ResourceLimit {
                limit: "emitted Snapshot Points",
                required: 4,
                allowed: 3,
            }
        )),
        result => panic!("expected Point-row output limit, got {result:?}"),
    }
}

#[test]
fn working_and_retained_memory_limits_fail_before_surface_publication() {
    let fixture = limit_fixture("memory-limits");
    let defaults = TerrainLimits::default();
    assert_terrain_resource(
        derive_with(
            fixture.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::new(
                defaults.point_rows(),
                defaults.max_input_points(),
                defaults.max_vertices(),
                defaults.max_faces(),
                defaults.max_working_bytes(),
                0,
                defaults.max_work_units(),
            ),
        ),
        "retained Terrain Surface bytes",
        0,
    );
    assert_terrain_resource(
        derive_with(
            fixture.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::new(
                defaults.point_rows(),
                defaults.max_input_points(),
                defaults.max_vertices(),
                defaults.max_faces(),
                0,
                defaults.max_surface_bytes(),
                defaults.max_work_units(),
            ),
        ),
        "Ground Input allocation bytes",
        0,
    );
}

#[test]
fn work_limit_fails_before_surface_publication() {
    let fixture = limit_fixture("work-limit");
    let defaults = TerrainLimits::default();
    assert_terrain_resource(
        derive_with(
            fixture.snapshot(),
            TerrainRecipe::new(2),
            TerrainLimits::new(
                defaults.point_rows(),
                defaults.max_input_points(),
                defaults.max_vertices(),
                defaults.max_faces(),
                defaults.max_working_bytes(),
                defaults.max_surface_bytes(),
                0,
            ),
        ),
        "Terrain Derivation work units",
        0,
    );
}

#[test]
fn cancellation_returns_no_publishable_surface() {
    let mut ticks = Vec::new();
    for y in 0..128_i64 {
        for x in 0..128_i64 {
            ticks.push([x, y, (x * 3 + y * 5).rem_euclid(17)]);
        }
    }
    let fixture = TerrainFixture::new(
        "cancel-before-publication",
        ticks.clone(),
        vec![2; ticks.len()],
    );
    let job = point_terrain::derive(
        fixture.snapshot(),
        TerrainRecipe::new(2),
        TerrainLimits::default(),
    );
    let handle = job.handle();
    handle.cancel();

    match job.blocking_wait() {
        Err(TerrainError::Cancelled) => {}
        Err(error) => panic!("cancelled Derivation returned another error: {error:?}"),
        Ok(surface) => panic!("cancelled Derivation published a Surface: {surface:?}"),
    }
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

fn limit_fixture(label: &str) -> TerrainFixture {
    TerrainFixture::new(
        label,
        vec![[0, 0, 0], [10, 0, 1], [10, 10, 2], [0, 10, 3]],
        vec![2; 4],
    )
}

fn assert_terrain_resource(
    result: Result<point_terrain::TerrainSurface, TerrainError>,
    expected_limit: &'static str,
    expected_allowed: u64,
) {
    match result {
        Err(TerrainError::ResourceLimit { limit, allowed, .. }) => {
            assert_eq!(limit, expected_limit);
            assert_eq!(allowed, expected_allowed);
        }
        result => panic!("expected {expected_limit} resource failure, got {result:?}"),
    }
}
