//! Detached Check Point acceptance through the public terrain interface.

mod support;

use std::{
    mem,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::Duration,
};

use foundation_runtime::ProgressPhase;
use point_contracts::PositionTransform;
use point_terrain::{
    CheckPoint, CheckPointId, CheckPointLimits, CheckPointOutcome, SurfaceFaceId, SurfaceVertexId,
    TerrainError, TerrainSurface,
};

use support::{TerrainFixture, derive_surface};

fn planar_surface(label: &str) -> (TerrainFixture, TerrainSurface) {
    let fixture = TerrainFixture::new(
        label,
        vec![[0, 0, 0], [10, 0, 10], [10, 10, 30], [0, 10, 20]],
        vec![2; 4],
    );
    let surface = derive_surface(fixture.snapshot(), 2);
    (fixture, surface)
}

fn check_point(id: u64, position: [f64; 3]) -> CheckPoint {
    CheckPoint::new(
        CheckPointId::new(id).expect("identity is nonzero"),
        position,
    )
    .expect("fixture Check Point is finite")
}

fn world_vertex(surface: &TerrainSurface, id: SurfaceVertexId) -> [f64; 3] {
    let vertex = surface
        .vertices()
        .iter()
        .find(|vertex| vertex.id() == id)
        .expect("fixture face references an existing vertex");
    surface
        .descriptor()
        .position_transform()
        .world_f64(vertex.ticks())
}

fn shared_edge(surface: &TerrainSurface) -> ([SurfaceVertexId; 2], SurfaceFaceId) {
    let faces = surface.faces();
    assert_eq!(faces.len(), 2, "planar square has two canonical faces");
    let first = faces[0].vertices();
    let second = faces[1].vertices();
    let common = first
        .into_iter()
        .filter(|vertex| second.contains(vertex))
        .collect::<Vec<_>>();
    assert_eq!(common.len(), 2, "the two triangles share one edge");
    ([common[0], common[1]], faces[0].id().min(faces[1].id()))
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-12,
        "expected {expected}, found {actual}"
    );
}

#[test]
fn closed_boundaries_choose_the_lowest_face_and_residuals_preserve_caller_order() {
    let (_fixture, surface) = planar_surface("qa-analytic");
    let surface = &surface;
    let (edge, lowest_shared_face) = shared_edge(surface);
    let edge_a = world_vertex(surface, edge[0]);
    let edge_b = world_vertex(surface, edge[1]);
    let shared_position = [
        edge_a[0].midpoint(edge_b[0]),
        edge_a[1].midpoint(edge_b[1]),
        edge_a[2].midpoint(edge_b[2]) + 2.0,
    ];

    let first_face = surface.faces()[0];
    let face_positions = first_face.vertices().map(|id| world_vertex(surface, id));
    let vertex_position = [
        face_positions[0][0],
        face_positions[0][1],
        face_positions[0][2],
    ];
    let below_centroid = [
        (face_positions[0][0] + face_positions[1][0] + face_positions[2][0]) / 3.0,
        (face_positions[0][1] + face_positions[1][1] + face_positions[2][1]) / 3.0,
        (face_positions[0][2] + face_positions[1][2] + face_positions[2][2]) / 3.0 - 3.0,
    ];
    let supplied = [
        check_point(40, [-5.0, -5.0, 11.0]),
        check_point(20, shared_position),
        check_point(10, vertex_position),
        check_point(30, below_centroid),
    ];

    let report = surface
        .check_points(supplied, CheckPointLimits::default())
        .blocking_wait()
        .expect("analytic Check Point evaluation succeeds");

    assert_eq!(
        report
            .results()
            .iter()
            .map(|result| result.check_point().id().get())
            .collect::<Vec<_>>(),
        [40, 20, 10, 30]
    );
    assert_eq!(report.results()[0].outcome(), CheckPointOutcome::Gap);
    let CheckPointOutcome::Sampled {
        face,
        surface_z,
        residual,
    } = report.results()[1].outcome()
    else {
        panic!("shared-edge Check Point must be covered");
    };
    assert_eq!(face, lowest_shared_face);
    assert_close(surface_z, shared_position[2] - 2.0);
    assert_close(residual, 2.0);

    let CheckPointOutcome::Sampled { face, residual, .. } = report.results()[2].outcome() else {
        panic!("Surface vertex Check Point must be covered");
    };
    assert_eq!(face, first_face.id());
    assert_eq!(residual.to_bits(), 0.0_f64.to_bits());

    let CheckPointOutcome::Sampled { face, residual, .. } = report.results()[3].outcome() else {
        panic!("face-centroid Check Point must be covered");
    };
    assert_eq!(face, first_face.id());
    assert_close(residual, -3.0);

    let statistics = report.statistics();
    assert_eq!(statistics.covered_count(), 3);
    assert_eq!(statistics.gap_count(), 1);
    assert_close(statistics.minimum().expect("covered minimum"), -3.0);
    assert_close(statistics.maximum().expect("covered maximum"), 2.0);
    assert_close(statistics.mean().expect("covered mean"), -1.0 / 3.0);
    assert_close(
        statistics.root_mean_square().expect("covered RMS"),
        (13.0_f64 / 3.0).sqrt(),
    );
    assert_eq!(report.face_tests(), 5);
    assert!(report.accounted_peak_working_bytes() > 0);
}

#[test]
fn all_gaps_have_explicit_outcomes_and_absent_numeric_statistics() {
    let (_fixture, surface) = planar_surface("qa-gaps");
    let report = surface
        .check_points(
            [
                check_point(1, [-1.0, -1.0, 0.0]),
                check_point(2, [11.0, 11.0, 0.0]),
            ],
            CheckPointLimits::default(),
        )
        .blocking_wait()
        .expect("outside Check Points produce a successful report");

    assert!(
        report
            .results()
            .iter()
            .all(|result| result.outcome() == CheckPointOutcome::Gap)
    );
    let statistics = report.statistics();
    assert_eq!(statistics.covered_count(), 0);
    assert_eq!(statistics.gap_count(), 2);
    assert_eq!(statistics.minimum(), None);
    assert_eq!(statistics.maximum(), None);
    assert_eq!(statistics.mean(), None);
    assert_eq!(statistics.root_mean_square(), None);
}

#[test]
fn compensated_statistics_retain_small_residuals_across_large_cancellation() {
    let (_fixture, surface) = planar_surface("qa-compensated");
    let report = surface
        .check_points(
            [
                check_point(1, [0.0, 0.0, 1.0e16]),
                check_point(2, [0.0, 0.0, 1.0]),
                check_point(3, [0.0, 0.0, -1.0e16]),
            ],
            CheckPointLimits::default(),
        )
        .blocking_wait()
        .expect("large finite residuals remain statistically representable");

    let statistics = report.statistics();
    assert_eq!(statistics.covered_count(), 3);
    assert_eq!(statistics.gap_count(), 0);
    assert_eq!(statistics.mean(), Some(1.0 / 3.0));
    assert_close(
        statistics.root_mean_square().expect("covered RMS"),
        (2.0e32_f64 / 3.0).sqrt(),
    );
}

#[test]
fn root_mean_square_accepts_large_finite_residuals() {
    let fixture = TerrainFixture::new(
        "qa-large-rms",
        vec![[0, 0, 0], [10, 0, 0], [0, 10, 0]],
        vec![2; 3],
    );
    let surface = derive_surface(fixture.snapshot(), 2);

    let report = surface
        .check_points(
            [check_point(1, [0.0, 0.0, 1.0e200])],
            CheckPointLimits::default(),
        )
        .blocking_wait()
        .expect("a representable RMS must not overflow during accumulation");

    let statistics = report.statistics();
    assert_eq!(statistics.mean(), Some(1.0e200));
    assert_eq!(statistics.root_mean_square(), Some(1.0e200));
}

#[test]
fn mean_accepts_repeated_large_finite_residuals() {
    let fixture = TerrainFixture::new(
        "qa-large-mean",
        vec![[0, 0, 0], [10, 0, 0], [0, 10, 0]],
        vec![2; 3],
    );
    let surface = derive_surface(fixture.snapshot(), 2);

    let report = surface
        .check_points(
            [
                check_point(1, [0.0, 0.0, 1.0e308]),
                check_point(2, [0.0, 0.0, 1.0e308]),
            ],
            CheckPointLimits::default(),
        )
        .blocking_wait()
        .expect("a representable mean must not overflow during accumulation");

    let statistics = report.statistics();
    assert_eq!(statistics.mean(), Some(1.0e308));
    assert_eq!(statistics.root_mean_square(), Some(1.0e308));
}

#[test]
fn finite_extreme_world_coordinates_remain_sampleable() {
    let transform = PositionTransform::new([0.0; 3], [1.0e200, 1.0e200, 1.0])
        .expect("finite extreme transform is valid");
    let fixture = TerrainFixture::with_transform(
        "qa-extreme-world",
        transform,
        vec![[0, 0, 0], [1, 0, 0], [0, 1, 0]],
        vec![2; 3],
    );
    let surface = derive_surface(fixture.snapshot(), 2);

    let report = surface
        .check_points(
            [check_point(1, [2.0e199, 2.0e199, 0.0])],
            CheckPointLimits::default(),
        )
        .blocking_wait()
        .expect("finite world geometry is supported");

    let CheckPointOutcome::Sampled {
        surface_z,
        residual,
        ..
    } = report.results()[0].outcome()
    else {
        panic!("interior Check Point must not become a gap at large world magnitudes");
    };
    assert_eq!(surface_z.to_bits(), 0.0_f64.to_bits());
    assert_eq!(residual.to_bits(), 0.0_f64.to_bits());
}

#[test]
fn large_world_offsets_do_not_collapse_a_valid_surface_face() {
    let transform = PositionTransform::new([1.0e12, 1.0e15, 0.0], [0.000_122_070_312_5, 0.25, 1.0])
        .expect("finite offset transform is valid");
    let fixture = TerrainFixture::with_transform(
        "qa-large-offset",
        transform,
        vec![[0, 0, 0], [1, 0, 0], [0, 1, 0]],
        vec![2; 3],
    );
    let surface = derive_surface(fixture.snapshot(), 2);
    let vertex = transform.world_f64([0, 0, 0]);

    let report = surface
        .check_points([check_point(1, vertex)], CheckPointLimits::default())
        .blocking_wait()
        .expect("a Surface vertex remains sampleable after normalization");

    assert!(matches!(
        report.results()[0].outcome(),
        CheckPointOutcome::Sampled {
            surface_z: 0.0,
            residual: 0.0,
            ..
        }
    ));
}

#[test]
fn duplicate_identities_and_every_qa_resource_family_fail_without_a_report() {
    let (_fixture, surface) = planar_surface("qa-limits");
    let surface = &surface;
    let duplicate = CheckPointId::new(9).expect("identity is nonzero");
    let error = surface
        .check_points(
            [
                CheckPoint::new(duplicate, [1.0, 1.0, 0.0]).unwrap(),
                CheckPoint::new(duplicate, [2.0, 2.0, 0.0]).unwrap(),
            ],
            CheckPointLimits::default(),
        )
        .blocking_wait()
        .expect_err("duplicate identities are rejected");
    assert!(matches!(error, TerrainError::InvalidArgument { .. }));

    assert_resource_limit(
        surface,
        [check_point(1, [1.0, 1.0, 0.0])],
        CheckPointLimits::new(1, 0, u64::MAX, u64::MAX),
        "Check Point result bytes",
    );
    assert_resource_limit(
        surface,
        [check_point(1, [-1.0, -1.0, 0.0])],
        CheckPointLimits::new(1, u64::MAX, 1, u64::MAX),
        "Check Point face tests",
    );
    assert_resource_limit(
        surface,
        [check_point(1, [1.0, 1.0, 0.0])],
        CheckPointLimits::new(1, u64::MAX, u64::MAX, 0),
        "Check Point input growth overlap",
    );
}

#[test]
fn count_limit_collects_only_max_plus_one_in_the_worker() {
    let (_fixture, surface) = planar_surface("qa-count");
    let pulls = Arc::new(AtomicUsize::new(0));
    let input = UnhintedCheckPoints {
        next_id: 1,
        remaining: 100,
        pulls: Arc::clone(&pulls),
    };

    let job = surface.check_points(
        input,
        CheckPointLimits::new(2, u64::MAX, u64::MAX, u64::MAX),
    );
    let error = job
        .blocking_wait()
        .expect_err("third Check Point exceeds the count limit");
    assert_eq!(pulls.load(Ordering::Relaxed), 3);
    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "detached Check Points",
            required: 3,
            allowed: 2,
        }
    ));
}

#[test]
fn cancellation_during_input_collection_returns_no_report() {
    let (_fixture, surface) = planar_surface("qa-collection-cancel");
    let (started_sender, started_receiver) = sync_channel(0);
    let (release_sender, release_receiver) = sync_channel(0);
    let input = GatedCheckPoints {
        started: Some(started_sender),
        release: release_receiver,
        yielded: false,
    };
    let job = surface.check_points(input, CheckPointLimits::default());
    let handle = job.handle();
    started_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("worker begins Check Point input collection");
    handle.cancel();
    release_sender
        .send(())
        .expect("release the worker's in-flight iterator pull");

    assert!(matches!(job.blocking_wait(), Err(TerrainError::Cancelled)));
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn boxed_result_overlap_failure_never_publishes_terminal_progress() {
    let (_fixture, surface) = planar_surface("qa-boxed-overlap");
    let input_bytes = u64::try_from(mem::size_of::<CheckPoint>()).unwrap();
    let result_bytes = u64::try_from(mem::size_of::<point_terrain::CheckPointResult>()).unwrap();
    let identity_bytes = u64::try_from(mem::size_of::<CheckPointId>()).unwrap();
    let earlier_peak = input_bytes
        .saturating_add(identity_bytes)
        .max(input_bytes.saturating_add(result_bytes));
    let boxed_overlap = result_bytes.saturating_mul(2);
    assert!(boxed_overlap > earlier_peak);
    let job = surface.check_points(
        [check_point(1, [1.0, 1.0, 3.0])],
        CheckPointLimits::new(1, result_bytes, u64::MAX, boxed_overlap.saturating_sub(1)),
    );
    let handle = job.handle();

    let error = job
        .blocking_wait()
        .expect_err("boxed result overlap exceeds the final working ceiling");

    assert!(matches!(
        error,
        TerrainError::ResourceLimit {
            limit: "Check Point boxed-result conversion working bytes",
            ..
        }
    ));
    assert_ne!(handle.progress().phase(), ProgressPhase::COMPLETE);
}

#[test]
fn cancellation_publishes_no_partial_check_point_report() {
    let (_fixture, surface) = planar_surface("qa-cancel");
    let input = (1_u64..=50_000).map(|id| check_point(id, [-1.0, -1.0, 0.0]));
    let job = surface.check_points(input, CheckPointLimits::default());
    job.handle().cancel();
    assert!(matches!(job.blocking_wait(), Err(TerrainError::Cancelled)));
}

fn assert_resource_limit<const N: usize>(
    surface: &TerrainSurface,
    input: [CheckPoint; N],
    limits: CheckPointLimits,
    expected_limit: &'static str,
) {
    let error = surface
        .check_points(input, limits)
        .blocking_wait()
        .expect_err("resource ceiling rejects the complete report");
    assert!(matches!(
        error,
        TerrainError::ResourceLimit { limit, .. } if limit == expected_limit
    ));
}

struct UnhintedCheckPoints {
    next_id: u64,
    remaining: usize,
    pulls: Arc<AtomicUsize>,
}

struct GatedCheckPoints {
    started: Option<SyncSender<()>>,
    release: Receiver<()>,
    yielded: bool,
}

impl Iterator for GatedCheckPoints {
    type Item = CheckPoint;

    fn next(&mut self) -> Option<Self::Item> {
        if self.yielded {
            return None;
        }
        self.started
            .take()
            .expect("fixture iterator starts once")
            .send(())
            .expect("test waits for collection to start");
        self.release
            .recv()
            .expect("test releases the blocked iterator");
        self.yielded = true;
        Some(check_point(1, [1.0, 1.0, 0.0]))
    }
}

impl Iterator for UnhintedCheckPoints {
    type Item = CheckPoint;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.pulls.fetch_add(1, Ordering::Relaxed);
        let point = check_point(self.next_id, [1.0, 1.0, 0.0]);
        self.next_id += 1;
        Some(point)
    }
}
