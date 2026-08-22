//! Exact Terrain QA and correction-loop acceptance through public interfaces.

mod support;

use foundation_runtime::ProgressPhase;
use point_contracts::WorldBounds;
use point_terrain::{
    CheckPoint, CheckPointId, ExactTerrainQaRequest, ProfileOutcome, ResidualOutcome,
    StationProfile, SurfaceComparisonLimits, SurfaceReadLimits, TerrainError, TerrainPrepareLimits,
    TerrainQaCurrentState, TerrainQaFreshness, TerrainQaLimits, TerrainRecipe,
    ToleranceDisposition, VerticalTolerance, compare_surfaces, prepare,
};
use point_workspace::{CommitLimits, CommitRequest, PointQuery, PointRowLimits};

use support::{TerrainFixture, committed, derive_with, operation, point_row_limits};

fn tolerance() -> VerticalTolerance {
    VerticalTolerance::new(0.5, 0.5).expect("fixture tolerance is valid")
}

fn check_point(id: u64, position: [f64; 3]) -> CheckPoint {
    CheckPoint::new(
        CheckPointId::new(id).expect("fixture Check Point identity is nonzero"),
        position,
    )
    .expect("fixture Check Point is finite")
}

fn plane_fixture(label: &str) -> TerrainFixture {
    TerrainFixture::new(
        label,
        vec![[0, 0, 0], [10, 0, 10], [10, 10, 20], [0, 10, 10]],
        vec![2; 4],
    )
}

fn complete_request() -> ExactTerrainQaRequest {
    ExactTerrainQaRequest::new(tolerance())
        .source_points(PointQuery::all())
        .check_points(
            vec![
                check_point(1, [5.0, 5.0, 11.0]),
                check_point(2, [5.0, 5.0, 9.5]),
                check_point(3, [20.0, 20.0, 0.0]),
            ]
            .into_boxed_slice(),
        )
        .profile(
            StationProfile::new([0.0, 0.0], [10.0, 10.0], 2).expect("fixture profile is valid"),
        )
}

#[test]
fn exact_qa_reports_profiles_residuals_tolerances_gaps_and_provenance() {
    let fixture = plane_fixture("exact-qa-analytic");
    let snapshot = fixture.snapshot();
    let surface = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .expect("analytic Surface derives");
    let report = surface
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .expect("analytic exact QA succeeds");

    assert_eq!(report.binding().snapshot(), *snapshot.provenance());
    assert_eq!(
        report.binding().artifact_hash(),
        surface.descriptor().artifact_hash()
    );
    assert_eq!(
        report.source_input().expect("Source summary").exact_count(),
        4
    );
    assert_eq!(report.source_points().len(), 4);
    assert!(report.source_points().iter().all(|result| matches!(
        result.outcome(),
        ResidualOutcome::Sampled {
            residual: 0.0,
            tolerance: ToleranceDisposition::Within,
            ..
        }
    )));

    assert!(matches!(
        report.check_points()[0].outcome(),
        ResidualOutcome::Sampled {
            residual: 1.0,
            tolerance: ToleranceDisposition::Above,
            ..
        }
    ));
    assert!(matches!(
        report.check_points()[1].outcome(),
        ResidualOutcome::Sampled {
            residual: -0.5,
            tolerance: ToleranceDisposition::Within,
            ..
        }
    ));
    assert_eq!(report.check_points()[2].outcome(), ResidualOutcome::Gap);

    let stations = report.profile_stations();
    assert_eq!(stations.len(), 3);
    for (station, expected) in stations.iter().zip([0.0, 10.0, 20.0]) {
        let ProfileOutcome::Sampled { surface_z, .. } = station.outcome() else {
            panic!("analytic profile station unexpectedly became a gap");
        };
        assert!((surface_z - expected).abs() < f64::EPSILON);
    }
    assert!(stations[0].station_metres().abs() < f64::EPSILON);
    assert!((stations[2].station_metres() - 200.0_f64.sqrt()).abs() < 1.0e-12);
    assert_eq!(report.profile_gap_count(), 0);
    assert_eq!(report.statistics().covered_count(), 6);
    assert_eq!(report.statistics().gap_count(), 1);
    assert_eq!(report.tolerance_summary().within_count(), 5);
    assert_eq!(report.tolerance_summary().above_count(), 1);
    assert_eq!(report.tolerance_summary().gap_count(), 1);
    assert_eq!(
        report
            .binding()
            .freshness(TerrainQaCurrentState::in_memory(&snapshot, &surface)),
        TerrainQaFreshness::Current
    );
}

#[test]
fn prepared_and_in_memory_surfaces_produce_identical_exact_evidence() {
    let fixture = plane_fixture("exact-qa-prepared");
    let snapshot = fixture.snapshot();
    let bounds = WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap();
    let recipe = TerrainRecipe::new(2).within(bounds);
    let in_memory = derive_with(
        snapshot.clone(),
        recipe,
        point_terrain::TerrainLimits::default(),
    )
    .expect("bounded in-memory Surface derives");
    let prepared = prepare(
        snapshot.clone(),
        fixture.terrain_path("surface.pterr"),
        recipe,
        TerrainPrepareLimits::default(),
    )
    .blocking_wait()
    .expect("bounded Surface prepares");

    let expected = in_memory
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .expect("in-memory QA succeeds");
    let actual = prepared
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .expect("prepared QA succeeds");

    assert_eq!(actual.binding(), expected.binding());
    assert_eq!(actual.input_hash(), expected.input_hash());
    assert_eq!(actual.result_hash(), expected.result_hash());
    assert_eq!(actual.source_points(), expected.source_points());
    assert_eq!(actual.check_points(), expected.check_points());
    assert_eq!(actual.profile_stations(), expected.profile_stations());
    assert_eq!(
        expected
            .binding()
            .freshness(TerrainQaCurrentState::in_memory(&snapshot, &in_memory)),
        TerrainQaFreshness::Current
    );
    assert_eq!(
        actual
            .binding()
            .freshness(TerrainQaCurrentState::prepared(&snapshot, &prepared)),
        TerrainQaFreshness::Current
    );
}

#[test]
fn correction_marks_old_evidence_stale_changes_the_surface_and_revert_restores_topology() {
    let fixture = seeded_defect_fixture();
    let baseline = fixture.snapshot();
    let recipe = TerrainRecipe::new(2);
    let baseline_surface = derive_with(
        baseline.clone(),
        recipe,
        point_terrain::TerrainLimits::default(),
    )
    .expect("defective baseline derives");
    let qa_request = ExactTerrainQaRequest::new(tolerance())
        .source_points(PointQuery::all())
        .check_points(vec![check_point(1, [1.0, 1.0, 0.0])].into_boxed_slice())
        .profile(StationProfile::new([0.0, 1.0], [2.0, 1.0], 2).unwrap());
    let baseline_qa = baseline_surface
        .exact_qa(
            baseline.clone(),
            qa_request.clone(),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .expect("baseline QA succeeds");
    assert!(matches!(
        baseline_qa.check_points()[0].outcome(),
        ResidualOutcome::Sampled {
            residual: -10.0,
            tolerance: ToleranceDisposition::Below,
            ..
        }
    ));

    let defect = fixture.select_ordinals(&baseline, &[4]);
    let edit = committed(
        fixture
            .workspace()
            .commit(
                CommitRequest::set_classification(operation(41), defect, 1),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("classification correction is definitive"),
    );
    let corrected = fixture
        .workspace()
        .snapshot(edit.revision())
        .expect("corrected Snapshot opens");
    let corrected_surface = derive_with(
        corrected.clone(),
        recipe,
        point_terrain::TerrainLimits::default(),
    )
    .expect("corrected Surface derives");
    let corrected_qa = corrected_surface
        .exact_qa(corrected.clone(), qa_request, TerrainQaLimits::default())
        .blocking_wait()
        .expect("corrected QA succeeds");
    assert!(matches!(
        corrected_qa.check_points()[0].outcome(),
        ResidualOutcome::Sampled {
            residual: 0.0,
            tolerance: ToleranceDisposition::Within,
            ..
        }
    ));
    assert_freshness_states(
        &baseline_qa,
        &corrected_qa,
        &baseline,
        &corrected,
        &baseline_surface,
        &corrected_surface,
    );
    let changed = compare_surfaces(
        &baseline_surface,
        &corrected_surface,
        SurfaceComparisonLimits::default(),
    )
    .blocking_wait()
    .expect("changed Surfaces compare");
    assert_eq!(changed.added_face_count(), 4);
    assert_eq!(changed.removed_face_count(), 6);
    assert_eq!(
        changed.changed_bounds(),
        Some(WorldBounds::new([0.0, 0.0, 0.0], [2.0, 2.0, 10.0]).unwrap())
    );
    assert_eq!(
        compare_surfaces(
            &baseline_surface,
            &corrected_surface,
            SurfaceComparisonLimits::default(),
        )
        .blocking_wait()
        .expect("repeated comparison succeeds"),
        changed
    );

    assert_revert_restores(&fixture, edit.revision(), recipe, &baseline_surface);
}

fn assert_freshness_states(
    baseline_qa: &point_terrain::ExactTerrainQaReport,
    corrected_qa: &point_terrain::ExactTerrainQaReport,
    baseline: &point_workspace::Snapshot,
    corrected: &point_workspace::Snapshot,
    baseline_surface: &point_terrain::TerrainSurface,
    corrected_surface: &point_terrain::TerrainSurface,
) {
    assert_eq!(
        baseline_qa
            .binding()
            .freshness(TerrainQaCurrentState::snapshot(baseline)),
        TerrainQaFreshness::SnapshotOnlyCurrent
    );
    assert_eq!(
        baseline_qa
            .binding()
            .freshness(TerrainQaCurrentState::snapshot(corrected)),
        TerrainQaFreshness::StaleSnapshot
    );
    assert_eq!(
        baseline_qa
            .binding()
            .freshness(TerrainQaCurrentState::in_memory(
                corrected,
                baseline_surface,
            )),
        TerrainQaFreshness::StaleSnapshotAndSurface
    );
    assert_eq!(
        baseline_qa
            .binding()
            .freshness(TerrainQaCurrentState::in_memory(
                baseline,
                corrected_surface,
            )),
        TerrainQaFreshness::StaleSurface
    );
    assert_eq!(
        corrected_qa
            .binding()
            .freshness(TerrainQaCurrentState::in_memory(
                corrected,
                corrected_surface,
            )),
        TerrainQaFreshness::Current
    );
}

fn assert_revert_restores(
    fixture: &TerrainFixture,
    edit: point_workspace::RevisionId,
    recipe: TerrainRecipe,
    baseline_surface: &point_terrain::TerrainSurface,
) {
    let reverted = committed(
        fixture
            .workspace()
            .commit(
                CommitRequest::revert_head(operation(42), edit),
                CommitLimits::default(),
            )
            .blocking_wait()
            .expect("immediate-head Revert is definitive"),
    );
    let restored_surface = derive_with(
        fixture
            .workspace()
            .snapshot(reverted.revision())
            .expect("Revert Snapshot opens"),
        recipe,
        point_terrain::TerrainLimits::default(),
    )
    .expect("restored Surface derives");
    let restored = compare_surfaces(
        baseline_surface,
        &restored_surface,
        SurfaceComparisonLimits::default(),
    )
    .blocking_wait()
    .expect("restored Surface compares");
    assert_eq!(restored.added_face_count(), 0);
    assert_eq!(restored.removed_face_count(), 0);
    assert_eq!(restored.changed_bounds(), None);
    assert_eq!(
        baseline_surface.descriptor().geometry_hash(),
        restored_surface.descriptor().geometry_hash()
    );
    assert_eq!(
        baseline_surface.descriptor().topology_hash(),
        restored_surface.descriptor().topology_hash()
    );
}

#[test]
fn hashes_ignore_source_row_batch_partitioning_and_limits_fail_without_results() {
    let fixture = plane_fixture("exact-qa-batching");
    let snapshot = fixture.snapshot();
    let surface = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let one_row = qa_limits_with_rows(point_row_limits(4, 1));
    let four_rows = qa_limits_with_rows(point_row_limits(4, 4));
    let first = surface
        .exact_qa(snapshot.clone(), complete_request(), one_row)
        .blocking_wait()
        .unwrap();
    let second = surface
        .exact_qa(snapshot.clone(), complete_request(), four_rows)
        .blocking_wait()
        .unwrap();
    assert_eq!(first.input_hash(), second.input_hash());
    assert_eq!(first.result_hash(), second.result_hash());

    let defaults = TerrainQaLimits::default();
    let too_few_source_points = TerrainQaLimits::new(
        defaults.point_rows(),
        defaults.surface_read(),
        3,
        defaults.max_check_points(),
        defaults.max_profile_stations(),
        defaults.max_observations(),
        defaults.max_result_bytes(),
        defaults.max_materialized_surface_bytes(),
        defaults.max_face_tests(),
        defaults.max_working_bytes(),
    );
    let error = surface
        .exact_qa(snapshot, complete_request(), too_few_source_points)
        .blocking_wait()
        .expect_err("fourth Source Point exceeds exact QA count limit");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "exact QA Source Points",
            required: 4,
            allowed: 3,
        }
    ));

    let comparison_error = compare_surfaces(
        &surface,
        &surface,
        SurfaceComparisonLimits::new(3, u64::MAX, u64::MAX, u64::MAX),
    )
    .blocking_wait()
    .expect_err("combined face count is independently bounded");
    assert!(matches!(
        comparison_error,
        TerrainError::ResourceLimit {
            limit: "Surface comparison faces",
            ..
        }
    ));
}

#[test]
fn query_semantics_are_hashed_and_an_empty_present_query_is_valid_evidence() {
    let fixture = plane_fixture("exact-qa-query-semantics");
    let snapshot = fixture.snapshot();
    let surface = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let tolerance = tolerance();
    let all = surface
        .exact_qa(
            snapshot.clone(),
            ExactTerrainQaRequest::new(tolerance).source_points(PointQuery::all()),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .unwrap();
    let full_bounds = WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap();
    let bounded = surface
        .exact_qa(
            snapshot.clone(),
            ExactTerrainQaRequest::new(tolerance).source_points(PointQuery::within(full_bounds)),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .unwrap();
    assert_eq!(all.source_points(), bounded.source_points());
    assert_ne!(all.input_hash(), bounded.input_hash());
    assert_ne!(all.result_hash(), bounded.result_hash());

    let empty_bounds = WorldBounds::new([100.0; 3], [101.0; 3]).unwrap();
    let empty = surface
        .exact_qa(
            snapshot,
            ExactTerrainQaRequest::new(tolerance).source_points(PointQuery::within(empty_bounds)),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .expect("a present Query is evidence even when it emits no rows");
    assert_eq!(empty.source_input().unwrap().exact_count(), 0);
    assert!(empty.source_points().is_empty());
    assert_eq!(empty.statistics().covered_count(), 0);
    assert_eq!(empty.statistics().gap_count(), 0);
}

#[test]
fn request_and_binding_validation_fail_closed() {
    assert!(VerticalTolerance::new(-0.1, 0.1).is_err());
    assert!(StationProfile::new([0.0, 0.0], [0.0, 0.0], 1).is_err());
    assert!(StationProfile::new([0.0, 0.0], [1.0, 0.0], 0).is_err());

    let fixture = plane_fixture("exact-qa-validation");
    let baseline = fixture.snapshot();
    let surface = derive_with(
        baseline.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let duplicate = ExactTerrainQaRequest::new(tolerance()).check_points(
        vec![
            check_point(7, [1.0, 1.0, 2.0]),
            check_point(7, [2.0, 2.0, 4.0]),
        ]
        .into_boxed_slice(),
    );
    assert!(matches!(
        surface
            .exact_qa(baseline.clone(), duplicate, TerrainQaLimits::default())
            .blocking_wait(),
        Err(TerrainError::InvalidArgument {
            argument: "detached Check Point identities",
            ..
        })
    ));

    let selected = fixture.select_ordinals(&baseline, &[0]);
    let edit = committed(
        fixture
            .workspace()
            .commit(
                CommitRequest::set_classification(operation(51), selected, 1),
                CommitLimits::default(),
            )
            .blocking_wait()
            .unwrap(),
    );
    let changed = fixture.workspace().snapshot(edit.revision()).unwrap();
    assert!(matches!(
        surface
            .exact_qa(changed, complete_request(), TerrainQaLimits::default())
            .blocking_wait(),
        Err(TerrainError::InvalidArgument {
            argument: "exact QA binding",
            ..
        })
    ));

    let bounded_recipe = TerrainRecipe::new(2)
        .within(WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap());
    let bounded_surface = derive_with(
        baseline,
        bounded_recipe,
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        compare_surfaces(
            &surface,
            &bounded_surface,
            SurfaceComparisonLimits::default(),
        )
        .blocking_wait(),
        Err(TerrainError::InvalidArgument {
            argument: "Surface comparison Recipe",
            ..
        })
    ));
}

#[test]
fn profile_requires_a_finite_representable_planar_length() {
    assert!(StationProfile::new([-1.0e308, 0.0], [1.0e308, 0.0], 1).is_err());
}

#[test]
fn prepared_binding_mismatch_precedes_materialization() {
    let fixture = plane_fixture("exact-qa-prepared-binding");
    let snapshot = fixture.snapshot();
    let bounds = WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap();
    let prepared = prepare(
        snapshot,
        fixture.terrain_path("binding-surface.pterr"),
        TerrainRecipe::new(2).within(bounds),
        TerrainPrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let mismatched_snapshot = plane_fixture("exact-qa-prepared-binding-mismatch").snapshot();

    let error = prepared
        .exact_qa(
            mismatched_snapshot,
            complete_request(),
            TerrainQaLimits::default().with_max_materialized_surface_bytes(0),
        )
        .blocking_wait()
        .expect_err("binding mismatch must fail before prepared Surface materialization");

    assert!(matches!(
        error,
        TerrainError::InvalidArgument {
            argument: "exact QA binding",
            ..
        }
    ));
}

#[test]
fn source_lineage_mismatch_is_rejected_without_partial_evidence() {
    let fixture = plane_fixture("exact-qa-source-baseline");
    let snapshot = fixture.snapshot();
    let surface = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let other_fixture = TerrainFixture::new(
        "exact-qa-source-mismatch",
        vec![[0, 0, 0], [10, 0, 10], [10, 10, 21], [0, 10, 10]],
        vec![2; 4],
    );
    let other_snapshot = other_fixture.snapshot();
    let other_surface = derive_with(
        other_snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    assert_ne!(
        snapshot.provenance().source(),
        other_snapshot.provenance().source()
    );

    assert!(matches!(
        surface
            .exact_qa(
                other_snapshot,
                complete_request(),
                TerrainQaLimits::default(),
            )
            .blocking_wait(),
        Err(TerrainError::InvalidArgument {
            argument: "exact QA binding",
            ..
        })
    ));
    assert!(matches!(
        compare_surfaces(&surface, &other_surface, SurfaceComparisonLimits::default())
            .blocking_wait(),
        Err(TerrainError::InvalidArgument {
            argument: "Surface comparison lineage",
            ..
        })
    ));
}

#[test]
fn qa_resource_ceilings_are_independent_and_inclusive() {
    let fixture = plane_fixture("exact-qa-resources");
    let snapshot = fixture.snapshot();
    let bounds = WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap();
    let recipe = TerrainRecipe::new(2).within(bounds);
    let surface = derive_with(
        snapshot.clone(),
        recipe,
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let successful = surface
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .unwrap();
    let exact = surface
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            exact_qa_limits(&successful, 0),
        )
        .blocking_wait()
        .expect("every in-memory exact QA ceiling is inclusive");
    assert_eq!(exact, successful);
    assert_qa_limit(
        &surface,
        snapshot.clone(),
        TerrainQaLimits::default().with_max_result_bytes(successful.retained_result_bytes() - 1),
        "exact QA retained result bytes",
    );
    assert_qa_limit(
        &surface,
        snapshot.clone(),
        TerrainQaLimits::default().with_max_face_tests(successful.face_tests() - 1),
        "Check Point face tests",
    );
    assert_qa_limit(
        &surface,
        snapshot.clone(),
        TerrainQaLimits::default().with_max_profile_stations(2),
        "profile stations",
    );
    assert_qa_limit(
        &surface,
        snapshot.clone(),
        TerrainQaLimits::default().with_max_check_points(2),
        "exact QA detached Check Points",
    );
    assert_qa_limit(
        &surface,
        snapshot.clone(),
        TerrainQaLimits::default().with_max_observations(9),
        "exact QA observations",
    );
    assert_qa_limit(
        &surface,
        snapshot.clone(),
        TerrainQaLimits::default()
            .with_max_working_bytes(successful.accounted_peak_working_bytes() - 1),
        "exact QA Source input growth overlap",
    );
}

#[test]
fn prepared_qa_resource_ceiling_is_inclusive() {
    let fixture = plane_fixture("exact-qa-prepared-resources");
    let snapshot = fixture.snapshot();
    let bounds = WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap();
    let prepared = prepare(
        snapshot.clone(),
        fixture.terrain_path("resource-surface.pterr"),
        TerrainRecipe::new(2).within(bounds),
        TerrainPrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let materialized_bytes = prepared_surface_bytes(&prepared);
    let prepared_successful = prepared
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .unwrap();
    let prepared_exact = prepared
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            exact_qa_limits(&prepared_successful, materialized_bytes),
        )
        .blocking_wait()
        .expect("every prepared exact QA ceiling is inclusive");
    assert_eq!(prepared_exact, prepared_successful);
    let error = prepared
        .exact_qa(
            snapshot,
            complete_request(),
            TerrainQaLimits::default().with_max_materialized_surface_bytes(materialized_bytes - 1),
        )
        .blocking_wait()
        .expect_err("prepared Surface materialization has an independent ceiling");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "prepared Surface materialization bytes",
            ..
        }
    ));
}

#[test]
fn prepared_qa_shares_one_surface_read_work_ceiling() {
    let fixture = plane_fixture("exact-qa-prepared-read-work");
    let snapshot = fixture.snapshot();
    let bounds = WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap();
    let prepared = prepare(
        snapshot.clone(),
        fixture.terrain_path("read-work-surface.pterr"),
        TerrainRecipe::new(2).within(bounds),
        TerrainPrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();

    let exact = prepared
        .exact_qa(
            snapshot.clone(),
            complete_request(),
            qa_limits_with_surface_read_work(12),
        )
        .blocking_wait()
        .expect("four vertices and two faces consume exactly twelve read-work units");
    assert_eq!(exact.source_points().len(), 4);

    let error = prepared
        .exact_qa(
            snapshot,
            complete_request(),
            qa_limits_with_surface_read_work(8),
        )
        .blocking_wait()
        .expect_err("vertex work must leave only the remaining budget for faces");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "Surface read work units",
            ..
        }
    ));
}

#[test]
fn prepared_profile_report_includes_the_surface_read_peak() {
    let fixture = plane_fixture("exact-qa-prepared-read-peak");
    let snapshot = fixture.snapshot();
    let bounds = WorldBounds::new([0.0, 0.0, 0.0], [10.0, 10.0, 20.0]).unwrap();
    let prepared = prepare(
        snapshot.clone(),
        fixture.terrain_path("read-peak-surface.pterr"),
        TerrainRecipe::new(2).within(bounds),
        TerrainPrepareLimits::default(),
    )
    .blocking_wait()
    .unwrap();
    let request = ExactTerrainQaRequest::new(tolerance())
        .profile(StationProfile::new([0.0, 0.0], [10.0, 10.0], 2).unwrap());

    let report = prepared
        .exact_qa(
            snapshot.clone(),
            request.clone(),
            TerrainQaLimits::default(),
        )
        .blocking_wait()
        .unwrap();
    let materialization_peak = prepared_surface_bytes(&prepared).saturating_add(
        TerrainQaLimits::default()
            .surface_read()
            .max_working_bytes(),
    );
    assert!(report.accounted_peak_working_bytes() >= materialization_peak);

    let exact = prepared
        .exact_qa(
            snapshot,
            request,
            TerrainQaLimits::default()
                .with_max_working_bytes(report.accounted_peak_working_bytes()),
        )
        .blocking_wait()
        .expect("the reported prepared-QA peak must be an inclusive rerun ceiling");
    assert_eq!(
        exact.accounted_peak_working_bytes(),
        report.accounted_peak_working_bytes()
    );
}

#[test]
fn boxed_result_conversion_is_included_in_the_working_ceiling() {
    let fixture = plane_fixture("exact-qa-boxed-result-overlap");
    let snapshot = fixture.snapshot();
    let surface = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let request = ExactTerrainQaRequest::new(tolerance())
        .check_points(vec![check_point(1, [1.0, 1.0, 2.0])].into_boxed_slice());
    let retained_result_bytes =
        u64::try_from(std::mem::size_of::<point_terrain::CheckPointResidual>()).unwrap();

    let error = surface
        .exact_qa(
            snapshot,
            request,
            TerrainQaLimits::default().with_max_working_bytes(retained_result_bytes),
        )
        .blocking_wait()
        .expect_err("boxed-result conversion overlap must respect the working-byte ceiling");

    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "exact QA boxed result conversion working bytes",
            ..
        }
    ));
}

#[test]
fn qa_cancellation_and_comparison_work_limits_publish_no_partial_result() {
    let fixture = plane_fixture("exact-qa-cancel");
    let snapshot = fixture.snapshot();
    let surface = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let long_profile = StationProfile::new([0.0, 0.0], [10.0, 10.0], 100_000).unwrap();
    let job = surface.exact_qa(
        snapshot,
        ExactTerrainQaRequest::new(tolerance()).profile(long_profile),
        TerrainQaLimits::default(),
    );
    job.handle().cancel();
    assert!(matches!(job.blocking_wait(), Err(TerrainError::Cancelled)));

    let successful = compare_surfaces(&surface, &surface, SurfaceComparisonLimits::default())
        .blocking_wait()
        .unwrap();
    let exact_faces = surface.descriptor().face_count().saturating_mul(2);
    let exact = compare_surfaces(
        &surface,
        &surface,
        SurfaceComparisonLimits::new(
            exact_faces,
            successful.retained_record_bytes(),
            successful.accounted_peak_working_bytes(),
            successful.work_units(),
        ),
    )
    .blocking_wait()
    .expect("every Surface comparison ceiling is inclusive");
    assert_eq!(exact, successful);
    let record_error = compare_surfaces(
        &surface,
        &surface,
        SurfaceComparisonLimits::new(
            u64::MAX,
            successful.retained_record_bytes() - 1,
            u64::MAX,
            u64::MAX,
        ),
    )
    .blocking_wait()
    .expect_err("one-under comparison record-byte ceiling fails");
    assert!(matches!(
        record_error,
        TerrainError::ResourceLimit {
            limit: "Surface comparison record bytes",
            ..
        }
    ));
    let working_error = compare_surfaces(
        &surface,
        &surface,
        SurfaceComparisonLimits::new(
            u64::MAX,
            u64::MAX,
            successful.accounted_peak_working_bytes() - 1,
            u64::MAX,
        ),
    )
    .blocking_wait()
    .expect_err("one-under comparison working-byte ceiling fails");
    assert!(matches!(
        working_error,
        TerrainError::ResourceLimit {
            limit: "Surface comparison working bytes",
            ..
        }
    ));
    let error = compare_surfaces(
        &surface,
        &surface,
        SurfaceComparisonLimits::new(
            u64::MAX,
            u64::MAX,
            u64::MAX,
            successful.work_units().saturating_sub(1),
        ),
    )
    .blocking_wait()
    .expect_err("one-under comparison work ceiling fails");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "Surface comparison work units",
            ..
        }
    ));
}

#[test]
fn large_profile_keeps_progress_monotonic_through_result_hashing() {
    let fixture = plane_fixture("exact-qa-progress");
    let snapshot = fixture.snapshot();
    let surface = derive_with(
        snapshot.clone(),
        TerrainRecipe::new(2),
        point_terrain::TerrainLimits::default(),
    )
    .unwrap();
    let profile = StationProfile::new([0.0, 0.0], [10.0, 10.0], 2_048).unwrap();
    let job = surface.exact_qa(
        snapshot,
        ExactTerrainQaRequest::new(tolerance()).profile(profile),
        TerrainQaLimits::default(),
    );
    let handle = job.handle();

    let report = job
        .blocking_wait()
        .expect("large exact profile completes without progress regression");

    assert_eq!(report.profile_stations().len(), 2_049);
    assert_eq!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

fn assert_qa_limit(
    surface: &point_terrain::TerrainSurface,
    snapshot: point_workspace::Snapshot,
    limits: TerrainQaLimits,
    expected: &'static str,
) {
    let error = surface
        .exact_qa(snapshot, complete_request(), limits)
        .blocking_wait()
        .expect_err("one-under exact QA ceiling fails");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit { limit, .. } if limit == expected
    ));
}

fn exact_qa_limits(
    report: &point_terrain::ExactTerrainQaReport,
    max_materialized_surface_bytes: u64,
) -> TerrainQaLimits {
    let source_points = u64::try_from(report.source_points().len()).unwrap();
    let check_points = u64::try_from(report.check_points().len()).unwrap();
    let profile_stations = u64::try_from(report.profile_stations().len()).unwrap();
    let observations = source_points
        .saturating_add(check_points)
        .saturating_add(profile_stations);
    let defaults = TerrainQaLimits::default();
    TerrainQaLimits::new(
        point_row_limits(source_points, source_points),
        defaults.surface_read(),
        source_points,
        check_points,
        profile_stations,
        observations,
        report.retained_result_bytes(),
        max_materialized_surface_bytes,
        report.face_tests(),
        report.accounted_peak_working_bytes(),
    )
}

fn seeded_defect_fixture() -> TerrainFixture {
    let mut ticks = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            let z = if x == 1 && y == 1 { 10 } else { 0 };
            ticks.push([x, y, z]);
        }
    }
    TerrainFixture::new("exact-qa-correction", ticks, vec![2; 9])
}

fn qa_limits_with_rows(point_rows: PointRowLimits) -> TerrainQaLimits {
    let defaults = TerrainQaLimits::default();
    TerrainQaLimits::new(
        point_rows,
        defaults.surface_read(),
        defaults.max_source_points(),
        defaults.max_check_points(),
        defaults.max_profile_stations(),
        defaults.max_observations(),
        defaults.max_result_bytes(),
        defaults.max_materialized_surface_bytes(),
        defaults.max_face_tests(),
        defaults.max_working_bytes(),
    )
}

fn qa_limits_with_surface_read_work(max_work_units: u64) -> TerrainQaLimits {
    let defaults = TerrainQaLimits::default();
    let read = defaults.surface_read();
    TerrainQaLimits::new(
        defaults.point_rows(),
        SurfaceReadLimits::new(
            read.max_batch_records(),
            read.max_batch_payload_bytes(),
            read.max_verify_buffer_bytes(),
            read.max_working_bytes(),
            max_work_units,
        ),
        defaults.max_source_points(),
        defaults.max_check_points(),
        defaults.max_profile_stations(),
        defaults.max_observations(),
        defaults.max_result_bytes(),
        defaults.max_materialized_surface_bytes(),
        defaults.max_face_tests(),
        defaults.max_working_bytes(),
    )
}

fn prepared_surface_bytes(surface: &point_terrain::PreparedTerrainSurface) -> u64 {
    u64::try_from(std::mem::size_of::<point_terrain::SurfaceVertex>())
        .unwrap()
        .saturating_mul(surface.descriptor().vertex_count())
        .saturating_add(
            u64::try_from(std::mem::size_of::<point_terrain::SurfaceFace>())
                .unwrap()
                .saturating_mul(surface.descriptor().face_count()),
        )
}
