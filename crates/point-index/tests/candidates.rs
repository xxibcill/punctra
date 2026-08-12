//! Conservative candidate lookup conformance against a sequential oracle.

mod support;

use foundation_runtime::CancellationToken;
use point_contracts::WorldBounds;
use point_index::{CandidateLimits, IndexError, PrepareLimits, prepare};

use support::{
    BLOCK_POINTS, TemporaryTarget, bounds_around, clustered_ticks, open_source, ordinal_is_covered,
    point_is_inside, transform,
};

#[test]
fn candidate_traversal_observes_cancellation() {
    let source = open_source(clustered_ticks(BLOCK_POINTS * 2 + 37));
    let target = TemporaryTarget::new("candidate-cancellation");
    let index = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let bounds = source.metadata().world_bounds().unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    assert!(matches!(
        index.candidates_with_cancellation(bounds, CandidateLimits::default(), &cancellation),
        Err(IndexError::Runtime(
            foundation_runtime::RuntimeError::Cancelled
        ))
    ));
}

#[test]
fn candidate_spans_are_sorted_disjoint_and_have_no_false_negatives() {
    let point_count = BLOCK_POINTS * 2 + 37;
    let ticks = clustered_ticks(point_count);
    let source = open_source(ticks.clone());
    let target = TemporaryTarget::new("candidates");
    let index = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let source_bounds = source.metadata().world_bounds().unwrap();

    let all = index
        .candidates(source_bounds, CandidateLimits::default())
        .unwrap();
    assert_eq!(all.spans().len(), 1);
    assert_eq!(all.spans()[0].first_ordinal(), 0);
    assert_eq!(
        all.spans()[0].point_count(),
        u64::try_from(point_count).unwrap()
    );
    assert_eq!(
        all.candidate_point_count(),
        u64::try_from(point_count).unwrap()
    );

    let outside = WorldBounds::new(
        [source_bounds.max()[0] + 1.0; 3],
        [source_bounds.max()[0] + 2.0; 3],
    )
    .unwrap();
    let none = index
        .candidates(outside, CandidateLimits::default())
        .unwrap();
    assert!(none.spans().is_empty());
    assert_eq!(none.candidate_point_count(), 0);

    let nonadjacent = world_bounds_from_tick_bounds([-200, -200, -100], [200, 100_200, 30_100]);
    let two_spans = index
        .candidates(nonadjacent, CandidateLimits::default())
        .unwrap();
    assert_eq!(two_spans.spans().len(), 2);
    assert_eq!(two_spans.spans()[0].first_ordinal(), 0);
    assert_eq!(
        two_spans.spans()[0].point_count(),
        u64::try_from(BLOCK_POINTS).unwrap()
    );
    assert_eq!(
        two_spans.spans()[1].first_ordinal(),
        u64::try_from(BLOCK_POINTS * 2).unwrap()
    );
    assert_eq!(two_spans.spans()[1].point_count(), 37);

    let mut generator = 0x5eed_1234_89ab_cdef_u64;
    let mut requests = vec![source_bounds, nonadjacent];
    for _ in 0..64 {
        generator = next_random(generator);
        let ordinal = usize::try_from(generator % u64::try_from(point_count).unwrap()).unwrap();
        generator = next_random(generator);
        let radii = [
            f64::from(u16::try_from(generator & 0x1ff).unwrap()) * 0.25,
            f64::from(u16::try_from((generator >> 9) & 0x1ff).unwrap()) * 0.5,
            f64::from(u16::try_from((generator >> 18) & 0xff).unwrap()) * 2.0,
        ];
        requests.push(bounds_around(transform().world_f64(ticks[ordinal]), radii));
    }
    for ordinal in [0, BLOCK_POINTS - 1, BLOCK_POINTS, point_count - 1] {
        let world = transform().world_f64(ticks[ordinal]);
        requests.push(WorldBounds::new(world, world).unwrap());
    }

    for request in requests {
        let plan = index
            .candidates(request, CandidateLimits::default())
            .unwrap();
        assert_plan_is_normalized(&plan);
        let oracle = ticks
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(ordinal, ticks)| {
                point_is_inside(transform().world_f64(ticks), request)
                    .then_some(u64::try_from(ordinal).unwrap())
            })
            .collect::<Vec<_>>();
        for ordinal in oracle {
            assert!(
                ordinal_is_covered(ordinal, plan.spans()),
                "oracle ordinal {ordinal} was absent from {plan:?} for {request:?}"
            );
        }
    }
}

#[test]
fn inclusive_degenerate_and_extreme_coordinate_queries_have_no_false_negatives() {
    let ticks = vec![
        [i64::MIN + 4_096, -1, 7],
        [-9_000_000_000_000, 0, -7],
        [9_000_000_000_000, 1, 0],
        [i64::MAX - 4_096, 2, 11],
    ];
    let source = open_source(ticks.clone());
    let target = TemporaryTarget::new("candidate-extremes");
    let index = prepare(source, target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();

    for (ordinal, ticks) in ticks.into_iter().enumerate() {
        let world = transform().world_f64(ticks);
        let request = WorldBounds::new(world, world).unwrap();
        let plan = index
            .candidates(request, CandidateLimits::default())
            .unwrap();
        assert_plan_is_normalized(&plan);
        assert!(ordinal_is_covered(
            u64::try_from(ordinal).unwrap(),
            plan.spans()
        ));
    }
}

#[test]
fn candidate_planning_fails_instead_of_returning_over_budget_partial_output() {
    let point_count = BLOCK_POINTS * 2 + 37;
    let source = open_source(clustered_ticks(point_count));
    let target = TemporaryTarget::new("candidate-limits");
    let index = prepare(source.clone(), target.path(), PrepareLimits::default())
        .blocking_wait()
        .unwrap();
    let all = source.metadata().world_bounds().unwrap();
    let nonadjacent = world_bounds_from_tick_bounds([-200, -200, -100], [200, 100_200, 30_100]);

    assert_resource_error(
        &index.candidates(all, CandidateLimits::new(0, u64::MAX, u64::MAX, u64::MAX)),
    );
    assert_resource_error(&index.candidates(
        all,
        CandidateLimits::new(
            u64::MAX,
            u64::MAX,
            u64::try_from(point_count - 1).unwrap(),
            u64::MAX,
        ),
    ));
    assert_resource_error(
        &index.candidates(all, CandidateLimits::new(u64::MAX, u64::MAX, u64::MAX, 0)),
    );
    let one_merged_span = index
        .candidates(all, CandidateLimits::new(u64::MAX, 1, u64::MAX, u64::MAX))
        .unwrap();
    assert_eq!(one_merged_span.spans().len(), 1);
    assert_resource_error(&index.candidates(
        nonadjacent,
        CandidateLimits::new(u64::MAX, 1, u64::MAX, u64::MAX),
    ));
}

fn assert_plan_is_normalized(plan: &point_index::CandidatePlan) {
    assert!(plan.spans().iter().all(|span| span.point_count() > 0));
    assert!(
        plan.spans()
            .windows(2)
            .all(|pair| pair[0].end_ordinal() < pair[1].first_ordinal())
    );
    assert_eq!(
        plan.candidate_point_count(),
        plan.spans().iter().map(|span| span.point_count()).sum()
    );
    if plan.spans().is_empty() {
        assert_eq!(plan.candidate_point_count(), 0);
    }
}

fn world_bounds_from_tick_bounds(minimum: [i64; 3], maximum: [i64; 3]) -> WorldBounds {
    WorldBounds::new(
        transform().world_f64(minimum),
        transform().world_f64(maximum),
    )
    .unwrap()
}

fn next_random(value: u64) -> u64 {
    value
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407)
}

fn assert_resource_error(result: &Result<point_index::CandidatePlan, IndexError>) {
    assert!(matches!(result, Err(IndexError::ResourceLimit { .. })));
}
