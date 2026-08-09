//! Public-interface acceptance tests for adaptive view planning.

use point_view::{
    AvailableNode, AvailableNodes, AxisAlignedBox, NodeKey, NodeStatus, PlanError, PlannerConfig,
    PlanningBudget, ViewPlanner,
};
use render_protocol::{BatchKey, BatchVersion, Camera, RenderUpdate, ViewGenerationKey, ViewId};

const GENEROUS_BUDGET: PlanningBudget = PlanningBudget::new(u64::MAX, u64::MAX, u64::MAX);

#[test]
fn value_constructors_reject_invalid_keys_bounds_config_and_costs() {
    assert_eq!(NodeKey::new(0), Err(PlanError::ZeroNodeKey));
    assert_eq!(
        AxisAlignedBox::new([f64::NAN, 0.0, 0.0], [1.0; 3]),
        Err(PlanError::InvalidBounds { axis: 0 })
    );
    assert_eq!(
        AxisAlignedBox::new([0.0, 2.0, 0.0], [1.0; 3]),
        Err(PlanError::InvalidBounds { axis: 1 })
    );
    assert_eq!(
        PlannerConfig::new(0.0, 0.0),
        Err(PlanError::InvalidPlannerConfig)
    );
    assert_eq!(
        PlannerConfig::new(2.0, 2.0),
        Err(PlanError::InvalidPlannerConfig)
    );

    let key = node_key(1);
    assert_eq!(
        available_node(
            1,
            Some(1),
            box_at(0.0, 0.0, -10.0),
            1.0,
            1,
            1,
            1,
            NodeStatus::Missing
        ),
        Err(PlanError::SelfParent { key })
    );
    assert_eq!(
        available_node(
            1,
            None,
            box_at(0.0, 0.0, -10.0),
            f64::INFINITY,
            1,
            1,
            1,
            NodeStatus::Missing,
        ),
        Err(PlanError::InvalidGeometricError { key })
    );
    assert_eq!(
        available_node(
            1,
            None,
            box_at(0.0, 0.0, -10.0),
            1.0,
            0,
            1,
            1,
            NodeStatus::Missing
        ),
        Err(PlanError::ZeroPointCount { key })
    );
    assert_eq!(
        available_node(
            1,
            None,
            box_at(0.0, 0.0, -10.0),
            1.0,
            1,
            0,
            1,
            NodeStatus::Missing
        ),
        Err(PlanError::ZeroEstimatedBytes { key })
    );
}

#[test]
fn status_updates_preserve_validated_node_metadata() {
    let missing = node(1, None, root_bounds(), 2.0, 3, 4, 5, NodeStatus::Missing);
    let expected = node(1, None, root_bounds(), 2.0, 3, 4, 5, resident(7));

    assert_eq!(missing.with_status(resident(7)), expected);
}

#[test]
fn hierarchy_validation_is_deterministic_and_atomic() {
    let generation = generation(1, 0);
    let valid = node(1, None, root_bounds(), 1.0, 1, 1, 11, NodeStatus::Missing);
    let mut planner = planner(10.0, 1.0);

    assert_eq!(
        planner.plan(
            &camera(),
            [0, 100],
            AvailableNodes::new(generation, &[valid]),
            GENEROUS_BUDGET,
        ),
        Err(PlanError::InvalidViewport)
    );
    assert_eq!(
        plan_error(&mut planner, generation, &[valid, valid]),
        PlanError::DuplicateNodeKey { key: node_key(1) }
    );

    let duplicate_batch = node(2, None, root_bounds(), 1.0, 1, 1, 11, NodeStatus::Missing);
    assert_eq!(
        plan_error(&mut planner, generation, &[duplicate_batch, valid]),
        PlanError::DuplicateBatchKey {
            key: BatchKey::new(11),
        }
    );

    let missing_parent = node(
        2,
        Some(99),
        root_bounds(),
        1.0,
        1,
        1,
        12,
        NodeStatus::Missing,
    );
    assert_eq!(
        plan_error(&mut planner, generation, &[missing_parent]),
        PlanError::MissingParent {
            key: node_key(2),
            parent: node_key(99),
        }
    );

    let cycle_a = node(
        1,
        Some(2),
        root_bounds(),
        1.0,
        1,
        1,
        11,
        NodeStatus::Missing,
    );
    let cycle_b = node(
        2,
        Some(1),
        root_bounds(),
        1.0,
        1,
        1,
        12,
        NodeStatus::Missing,
    );
    assert_eq!(
        plan_error(&mut planner, generation, &[cycle_b, cycle_a]),
        PlanError::ParentCycle { key: node_key(1) }
    );

    let escaped_child = node(
        2,
        Some(1),
        bounds([1.5, -1.0, -11.0], [3.0, 1.0, -9.0]),
        0.0,
        1,
        1,
        12,
        NodeStatus::Missing,
    );
    assert_eq!(
        plan_error(&mut planner, generation, &[escaped_child, valid]),
        PlanError::ChildOutsideParent {
            key: node_key(2),
            parent: node_key(1),
        }
    );

    let empty = planner
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &[]),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert!(empty.requests().is_empty());
    assert!(empty.retained_nodes().is_empty());
    assert!(empty.retirements().is_empty());
}

#[test]
fn perspective_frustum_culls_all_six_planes_and_keeps_intersections() {
    let generation = generation(2, 3);
    let nodes = [
        node(
            1,
            None,
            box_at(0.0, 0.0, -5.0),
            1.0,
            1,
            10,
            1,
            NodeStatus::Missing,
        ),
        node(2, None, box_at(20.0, 0.0, -5.0), 1.0, 1, 10, 2, resident(2)),
        node(
            3,
            None,
            box_at(-20.0, 0.0, -5.0),
            1.0,
            1,
            10,
            3,
            resident(3),
        ),
        node(4, None, box_at(0.0, 20.0, -5.0), 1.0, 1, 10, 4, resident(4)),
        node(
            5,
            None,
            box_at(0.0, -20.0, -5.0),
            1.0,
            1,
            10,
            5,
            resident(5),
        ),
        node(6, None, box_at(0.0, 0.0, 5.0), 1.0, 1, 10, 6, resident(6)),
        node(
            7,
            None,
            box_at(0.0, 0.0, -101.0),
            1.0,
            1,
            10,
            7,
            resident(7),
        ),
        node(
            8,
            None,
            bounds([4.9, -0.1, -5.1], [5.1, 0.1, -4.9]),
            1.0,
            1,
            10,
            8,
            NodeStatus::Missing,
        ),
    ];

    let plan = planner(100.0, 1.0)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &nodes),
            GENEROUS_BUDGET,
        )
        .unwrap();

    assert_eq!(request_keys(&plan), vec![node_key(1), node_key(8)]);
    assert_eq!(
        retirement_batches(&plan),
        vec![2, 3, 4, 5, 6, 7]
            .into_iter()
            .map(BatchKey::new)
            .collect::<Vec<_>>()
    );
}

#[test]
fn projection_uses_f64_world_coordinates_and_reports_pixel_error() {
    let generation = generation(3, 0);
    let world = 1.0e308;
    let camera = Camera::perspective(
        [world, 0.0, 0.0],
        [world, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        100.0,
    )
    .unwrap();
    let node = node(
        1,
        None,
        bounds([world, 0.0, -10.0], [world, 0.0, -10.0]),
        2.0,
        1,
        1,
        1,
        NodeStatus::Missing,
    );

    let plan = planner(20.0, 1.0)
        .plan(
            &camera,
            [100, 100],
            AvailableNodes::new(generation, &[node]),
            GENEROUS_BUDGET,
        )
        .unwrap();

    assert_eq!(plan.requests().len(), 1);
    assert!((plan.requests()[0].screen_space_error_pixels() - 10.0).abs() < 0.001);
}

#[test]
fn projection_handles_extreme_width_without_overflow() {
    let node = node(
        1,
        None,
        bounds([-1.0e308, 0.0, -10.0], [1.0e308, 0.0, -10.0]),
        2.0,
        1,
        1,
        1,
        NodeStatus::Missing,
    );

    let plan = planner(20.0, 1.0)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation(3, 2), &[node]),
            GENEROUS_BUDGET,
        )
        .unwrap();

    assert_eq!(plan.requests().len(), 1);
    assert!((plan.requests()[0].screen_space_error_pixels() - 10.0).abs() < 0.001);
}

#[test]
fn projection_normalizes_large_camera_directions_without_overflow() {
    let camera = Camera::perspective(
        [0.0, 0.0, 0.0],
        [1.0e308, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        100.0,
    )
    .unwrap();
    let node = node(
        1,
        None,
        bounds([9.0, -1.0, -1.0], [11.0, 1.0, 1.0]),
        0.0,
        1,
        1,
        1,
        NodeStatus::Missing,
    );

    let plan = planner(2.0, 0.25)
        .plan(
            &camera,
            [100, 100],
            AvailableNodes::new(generation(3, 1), &[node]),
            GENEROUS_BUDGET,
        )
        .unwrap();

    assert_eq!(request_keys(&plan), vec![node_key(1)]);
}

#[test]
fn initial_loading_requests_coarse_coverage_before_refining() {
    let generation = generation(4, 0);
    let mut planner = planner(2.0, 0.25);
    let initial = coverage_tree(
        NodeStatus::Missing,
        NodeStatus::Missing,
        NodeStatus::Missing,
    );

    let coarse = planner
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &initial),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert_eq!(request_keys(&coarse), vec![node_key(1)]);
    assert_eq!(coarse.requests()[0].view_generation(), generation);

    let coarse_resident = coverage_tree(resident(7), NodeStatus::Missing, NodeStatus::Missing);
    let refining = planner
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &coarse_resident),
            PlanningBudget::new(30, 300, 3),
        )
        .unwrap();
    assert_eq!(request_keys(&refining), vec![node_key(2), node_key(3)]);
    assert_eq!(retained_keys(&refining), vec![node_key(1)]);
    assert!(refining.retirements().is_empty());
    assert_eq!(refining.resource_usage().point_count(), 30);
    assert_eq!(refining.resource_usage().estimated_bytes(), 300);
    assert_eq!(refining.resource_usage().batch_count(), 3);
}

#[test]
fn parent_retires_only_after_every_visible_replacement_is_resident() {
    let generation = generation(5, 9);
    let mut planner = planner(2.0, 0.25);
    let partial = coverage_tree(resident(7), resident(8), NodeStatus::Missing);

    let plan = planner
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &partial),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert_eq!(request_keys(&plan), vec![node_key(3)]);
    assert_eq!(retained_keys(&plan), vec![node_key(1), node_key(2)]);
    assert!(plan.retirements().is_empty());

    let complete = coverage_tree(resident(7), resident(8), resident(9));
    let plan = planner
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &complete),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert_eq!(retained_keys(&plan), vec![node_key(2), node_key(3)]);
    assert_eq!(plan.retained_nodes()[0].view_generation(), generation);
    assert_eq!(plan.retirements().len(), 1);
    let retirement = plan.retirements()[0];
    assert_eq!(retirement.view_generation(), generation);
    assert_eq!(retirement.batch_key(), BatchKey::new(101));
    assert_eq!(retirement.expected_version(), BatchVersion::new(7));
    assert_eq!(
        retirement.render_update(),
        RenderUpdate::Remove {
            view_generation: generation,
            key: BatchKey::new(101),
            expected_version: BatchVersion::new(7),
        }
    );
}

#[test]
fn nested_refinement_retires_already_replaced_ancestors() {
    let nodes = [
        node(1, None, root_bounds(), 10.0, 1, 1, 101, resident(1)),
        node(2, Some(1), root_bounds(), 10.0, 1, 1, 102, resident(1)),
        node(
            3,
            Some(2),
            root_bounds(),
            0.0,
            1,
            1,
            103,
            NodeStatus::Missing,
        ),
    ];

    let plan = planner(2.0, 0.25)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation(5, 1), &nodes),
            PlanningBudget::new(2, 2, 2),
        )
        .unwrap();

    assert_eq!(request_keys(&plan), vec![node_key(3)]);
    assert_eq!(retained_keys(&plan), vec![node_key(2)]);
    assert_eq!(retirement_batches(&plan), vec![BatchKey::new(101)]);
    assert_eq!(plan.resource_usage().batch_count(), 2);
}

#[test]
fn coarsening_keeps_only_the_nearest_resident_fallback() {
    let nodes = [
        node(1, None, root_bounds(), 0.0, 1, 1, 101, NodeStatus::Missing),
        node(2, Some(1), root_bounds(), 0.0, 1, 1, 102, resident(1)),
        node(3, Some(2), root_bounds(), 0.0, 1, 1, 103, resident(1)),
    ];

    let plan = planner(2.0, 0.25)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation(5, 2), &nodes),
            PlanningBudget::new(2, 2, 2),
        )
        .unwrap();

    assert_eq!(request_keys(&plan), vec![node_key(1)]);
    assert_eq!(retained_keys(&plan), vec![node_key(2)]);
    assert_eq!(retirement_batches(&plan), vec![BatchKey::new(103)]);
    assert_eq!(plan.resource_usage().point_count(), 2);
    assert_eq!(plan.resource_usage().estimated_bytes(), 2);
    assert_eq!(plan.resource_usage().batch_count(), 2);
}

#[test]
fn coarsening_keeps_resident_descendants_until_parent_arrives() {
    let generation = generation(6, 0);
    let mut planner = planner(10.0, 2.0);
    let fine = hysteresis_tree(resident(1), resident(2), resident(3));

    let refined = planner
        .plan(
            &camera(),
            [130, 130],
            AvailableNodes::new(generation, &fine),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert_eq!(retained_keys(&refined), vec![node_key(2), node_key(3)]);

    let parent_missing = hysteresis_tree(NodeStatus::Missing, resident(2), resident(3));
    let coarsening = planner
        .plan(
            &camera(),
            [70, 70],
            AvailableNodes::new(generation, &parent_missing),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert_eq!(request_keys(&coarsening), vec![node_key(1)]);
    assert_eq!(retained_keys(&coarsening), vec![node_key(2), node_key(3)]);
    assert!(coarsening.retirements().is_empty());

    let parent_arrived = hysteresis_tree(resident(4), resident(2), resident(3));
    let coarse = planner
        .plan(
            &camera(),
            [70, 70],
            AvailableNodes::new(generation, &parent_arrived),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert_eq!(retained_keys(&coarse), vec![node_key(1)]);
    assert_eq!(
        retirement_batches(&coarse),
        vec![BatchKey::new(102), BatchKey::new(103)]
    );
}

#[test]
fn hysteresis_prevents_oscillation_and_resets_on_generation_change() {
    let first_generation = generation(7, 1);
    let second_generation = generation(7, 2);
    let nodes = hysteresis_tree(resident(1), resident(2), resident(3));
    let mut planner = planner(10.0, 2.0);

    let above_upper = plan_at_height(&mut planner, first_generation, &nodes, 130);
    assert_eq!(retained_keys(&above_upper), vec![node_key(2), node_key(3)]);

    let dead_band = plan_at_height(&mut planner, first_generation, &nodes, 90);
    assert_eq!(retained_keys(&dead_band), vec![node_key(2), node_key(3)]);

    let below_lower = plan_at_height(&mut planner, first_generation, &nodes, 70);
    assert_eq!(retained_keys(&below_lower), vec![node_key(1)]);

    let above_upper_again = plan_at_height(&mut planner, first_generation, &nodes, 130);
    assert_eq!(
        retained_keys(&above_upper_again),
        vec![node_key(2), node_key(3)]
    );

    let reset_in_dead_band = plan_at_height(&mut planner, second_generation, &nodes, 90);
    assert_eq!(retained_keys(&reset_in_dead_band), vec![node_key(1)]);
}

#[test]
fn frustum_culling_preserves_same_generation_hysteresis_history() {
    let generation = generation(7, 3);
    let nodes = hysteresis_tree(resident(1), resident(2), resident(3));
    let mut planner = planner(10.0, 2.0);

    let refined = plan_at_height(&mut planner, generation, &nodes, 130);
    assert_eq!(retained_keys(&refined), vec![node_key(2), node_key(3)]);

    let looking_away = Camera::perspective(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        100.0,
    )
    .unwrap();
    let culled = planner
        .plan(
            &looking_away,
            [90, 90],
            AvailableNodes::new(generation, &nodes),
            GENEROUS_BUDGET,
        )
        .unwrap();
    assert!(culled.retained_nodes().is_empty());

    let reentered_dead_band = plan_at_height(&mut planner, generation, &nodes, 90);
    assert_eq!(
        retained_keys(&reentered_dead_band),
        vec![node_key(2), node_key(3)]
    );
}

#[test]
fn each_budget_dimension_blocks_an_unaffordable_refinement() {
    for budget in [
        PlanningBudget::new(29, 300, 3),
        PlanningBudget::new(30, 299, 3),
        PlanningBudget::new(30, 300, 2),
    ] {
        let generation = generation(8, budget.max_points());
        let nodes = coverage_tree(resident(1), NodeStatus::Missing, NodeStatus::Missing);
        let plan = planner(2.0, 0.25)
            .plan(
                &camera(),
                [100, 100],
                AvailableNodes::new(generation, &nodes),
                budget,
            )
            .unwrap();

        assert!(plan.requests().is_empty());
        assert_eq!(retained_keys(&plan), vec![node_key(1)]);
        assert!(plan.retirements().is_empty());
    }
}

#[test]
fn refinement_budget_is_spent_on_the_highest_screen_error_first() {
    let generation = generation(13, 0);
    let high_bounds = bounds([-2.0, -1.0, -11.0], [0.0, 1.0, -9.0]);
    let low_bounds = bounds([0.0, -1.0, -11.0], [2.0, 1.0, -9.0]);
    let nodes = [
        node(1, None, low_bounds, 5.0, 1, 1, 1, resident(1)),
        node(2, Some(1), low_bounds, 0.0, 1, 1, 2, NodeStatus::Missing),
        node(3, Some(1), low_bounds, 0.0, 1, 1, 3, NodeStatus::Missing),
        node(4, None, high_bounds, 10.0, 1, 1, 4, resident(1)),
        node(5, Some(4), high_bounds, 0.0, 1, 1, 5, NodeStatus::Missing),
        node(6, Some(4), high_bounds, 0.0, 1, 1, 6, NodeStatus::Missing),
    ];

    let complete_plan = planner(2.0, 0.25)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &nodes),
            GENEROUS_BUDGET,
        )
        .unwrap();

    assert_eq!(
        request_keys(&complete_plan),
        vec![node_key(5), node_key(6), node_key(2), node_key(3)]
    );
    let request_errors = complete_plan
        .requests()
        .iter()
        .map(|request| request.screen_space_error_pixels())
        .collect::<Vec<_>>();
    assert_eq!(request_errors[0].to_bits(), request_errors[1].to_bits());
    assert!(request_errors[1] > request_errors[2]);
    assert_eq!(request_errors[2].to_bits(), request_errors[3].to_bits());

    let plan = planner(2.0, 0.25)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &nodes),
            PlanningBudget::new(4, 4, 4),
        )
        .unwrap();

    assert_eq!(request_keys(&plan), vec![node_key(5), node_key(6)]);
    assert_eq!(retained_keys(&plan), vec![node_key(1), node_key(4)]);
}

#[test]
fn unaffordable_missing_root_does_not_block_an_affordable_refinement() {
    let nodes = [
        node(
            1,
            None,
            root_bounds(),
            30.0,
            23,
            230,
            1,
            NodeStatus::Missing,
        ),
        node(2, None, root_bounds(), 0.0, 2, 20, 2, NodeStatus::Requested),
        node(3, None, root_bounds(), 10.0, 10, 100, 3, resident(1)),
        node(
            4,
            Some(3),
            root_bounds(),
            2.0,
            4,
            40,
            4,
            NodeStatus::Missing,
        ),
        node(
            5,
            Some(3),
            root_bounds(),
            1.0,
            6,
            60,
            5,
            NodeStatus::Missing,
        ),
    ];

    let plan = planner(2.0, 0.25)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation(14, 0), &nodes),
            PlanningBudget::new(22, 220, 4),
        )
        .unwrap();

    assert_eq!(request_keys(&plan), vec![node_key(4), node_key(5)]);
    assert_eq!(retained_keys(&plan), vec![node_key(3)]);
    assert!(plan.retirements().is_empty());
    assert_eq!(plan.resource_usage().point_count(), 22);
    assert_eq!(plan.resource_usage().estimated_bytes(), 220);
    assert_eq!(plan.resource_usage().batch_count(), 4);
}

#[test]
fn in_flight_work_is_reserved_and_never_requested_twice() {
    let generation = generation(9, 0);
    let nodes = [
        node(
            1,
            None,
            box_at(0.0, 0.0, -10.0),
            1.0,
            5,
            50,
            1,
            NodeStatus::Requested,
        ),
        node(
            2,
            None,
            box_at(1.0, 0.0, -10.0),
            1.0,
            6,
            60,
            2,
            NodeStatus::Missing,
        ),
    ];

    let exact = planner(20.0, 1.0)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &nodes),
            PlanningBudget::new(11, 110, 2),
        )
        .unwrap();
    assert_eq!(request_keys(&exact), vec![node_key(2)]);
    assert_eq!(exact.resource_usage().point_count(), 11);

    let constrained = planner(20.0, 1.0)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &nodes),
            PlanningBudget::new(10, 110, 2),
        )
        .unwrap();
    assert!(constrained.requests().is_empty());
    assert_eq!(constrained.resource_usage().point_count(), 5);
}

#[test]
fn over_budget_visible_coverage_is_not_retired_to_force_compliance() {
    let generation = generation(10, 0);
    let resident_root = node(1, None, root_bounds(), 1.0, 100, 1_000, 1, resident(3));

    let plan = planner(20.0, 1.0)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &[resident_root]),
            PlanningBudget::new(10, 100, 0),
        )
        .unwrap();

    assert!(plan.requests().is_empty());
    assert_eq!(retained_keys(&plan), vec![node_key(1)]);
    assert!(plan.retirements().is_empty());
    assert!(
        !plan
            .resource_usage()
            .fits_within(PlanningBudget::new(10, 100, 0))
    );
}

#[test]
fn requests_and_output_lists_are_input_order_independent() {
    let generation = generation(11, 4);
    let low_key_tie = node(
        1,
        None,
        box_at(-1.0, 0.0, -10.0),
        2.0,
        1,
        1,
        30,
        NodeStatus::Missing,
    );
    let high_key_tie = node(
        2,
        None,
        box_at(1.0, 0.0, -10.0),
        2.0,
        1,
        1,
        20,
        NodeStatus::Missing,
    );
    let highest_error = node(
        3,
        None,
        box_at(0.0, 0.0, -8.0),
        3.0,
        1,
        1,
        10,
        NodeStatus::Missing,
    );
    let resident_visible = node(4, None, box_at(0.0, 1.0, -10.0), 1.0, 1, 1, 40, resident(4));
    let resident_hidden = node(5, None, box_at(30.0, 0.0, -10.0), 1.0, 1, 1, 5, resident(5));
    let ordered = [
        low_key_tie,
        high_key_tie,
        highest_error,
        resident_visible,
        resident_hidden,
    ];
    let shuffled = [
        resident_hidden,
        highest_error,
        resident_visible,
        high_key_tie,
        low_key_tie,
    ];

    let first = planner(100.0, 1.0)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &ordered),
            GENEROUS_BUDGET,
        )
        .unwrap();
    let second = planner(100.0, 1.0)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &shuffled),
            GENEROUS_BUDGET,
        )
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(
        request_keys(&first),
        vec![node_key(3), node_key(1), node_key(2)]
    );
    assert_eq!(retained_keys(&first), vec![node_key(4)]);
    assert_eq!(retirement_batches(&first), vec![BatchKey::new(5)]);
}

#[test]
fn accounting_overflow_is_reported_without_updating_hysteresis() {
    let generation = generation(12, 0);
    let valid = hysteresis_tree(resident(1), resident(2), resident(3));
    let mut planner = planner(10.0, 2.0);
    let refined = plan_at_height(&mut planner, generation, &valid, 130);
    assert_eq!(retained_keys(&refined), vec![node_key(2), node_key(3)]);

    let overflowing = [
        node(
            1,
            None,
            box_at(0.0, 0.0, -10.0),
            1.0,
            u64::MAX,
            1,
            1,
            NodeStatus::Requested,
        ),
        node(
            2,
            None,
            box_at(1.0, 0.0, -10.0),
            1.0,
            1,
            1,
            2,
            NodeStatus::Requested,
        ),
    ];
    assert_eq!(
        planner.plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, &overflowing),
            GENEROUS_BUDGET,
        ),
        Err(PlanError::ResourceUsageOverflow)
    );

    let plan = plan_at_height(&mut planner, generation, &valid, 90);
    assert_eq!(retained_keys(&plan), vec![node_key(2), node_key(3)]);
}

#[test]
fn deep_reverse_key_hierarchies_validate_in_linear_time() {
    const DEPTH: u64 = 25_000;
    let mut nodes = Vec::with_capacity(usize::try_from(DEPTH).unwrap());
    for key in 1..=DEPTH {
        nodes.push(node(
            key,
            (key < DEPTH).then_some(key + 1),
            box_at(0.0, 0.0, -10.0),
            0.0,
            1,
            1,
            key,
            if key == 1 {
                resident(1)
            } else {
                NodeStatus::Missing
            },
        ));
    }

    let plan = planner(2.0, 0.25)
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation(14, 0), &nodes),
            GENEROUS_BUDGET,
        )
        .unwrap();

    assert_eq!(request_keys(&plan), vec![node_key(DEPTH)]);
    assert_eq!(retained_keys(&plan), vec![node_key(1)]);
}

fn plan_error(
    planner: &mut ViewPlanner,
    generation: ViewGenerationKey,
    nodes: &[AvailableNode],
) -> PlanError {
    planner
        .plan(
            &camera(),
            [100, 100],
            AvailableNodes::new(generation, nodes),
            GENEROUS_BUDGET,
        )
        .unwrap_err()
}

fn plan_at_height(
    planner: &mut ViewPlanner,
    generation: ViewGenerationKey,
    nodes: &[AvailableNode],
    height: u32,
) -> point_view::ViewPlan {
    planner
        .plan(
            &camera(),
            [height, height],
            AvailableNodes::new(generation, nodes),
            GENEROUS_BUDGET,
        )
        .unwrap()
}

fn coverage_tree(
    root_status: NodeStatus,
    left_status: NodeStatus,
    right_status: NodeStatus,
) -> [AvailableNode; 3] {
    [
        node(1, None, root_bounds(), 10.0, 10, 100, 101, root_status),
        node(
            2,
            Some(1),
            bounds([-2.0, -1.0, -11.0], [0.0, 1.0, -9.0]),
            0.0,
            10,
            100,
            102,
            left_status,
        ),
        node(
            3,
            Some(1),
            bounds([0.0, -1.0, -11.0], [2.0, 1.0, -9.0]),
            0.0,
            10,
            100,
            103,
            right_status,
        ),
    ]
}

fn hysteresis_tree(
    root_status: NodeStatus,
    left_status: NodeStatus,
    right_status: NodeStatus,
) -> [AvailableNode; 3] {
    [
        node(
            1,
            None,
            box_at(0.0, 0.0, -10.0),
            2.0,
            1,
            1,
            101,
            root_status,
        ),
        node(
            2,
            Some(1),
            box_at(0.0, 0.0, -10.0),
            0.0,
            1,
            1,
            102,
            left_status,
        ),
        node(
            3,
            Some(1),
            box_at(0.0, 0.0, -10.0),
            0.0,
            1,
            1,
            103,
            right_status,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn node(
    key: u64,
    parent: Option<u64>,
    bounds: AxisAlignedBox,
    geometric_error: f64,
    point_count: u64,
    estimated_bytes: u64,
    batch_key: u64,
    status: NodeStatus,
) -> AvailableNode {
    available_node(
        key,
        parent,
        bounds,
        geometric_error,
        point_count,
        estimated_bytes,
        batch_key,
        status,
    )
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn available_node(
    key: u64,
    parent: Option<u64>,
    bounds: AxisAlignedBox,
    geometric_error: f64,
    point_count: u64,
    estimated_bytes: u64,
    batch_key: u64,
    status: NodeStatus,
) -> Result<AvailableNode, PlanError> {
    AvailableNode::new(
        node_key(key),
        parent.map(node_key),
        bounds,
        geometric_error,
        point_count,
        estimated_bytes,
        BatchKey::new(batch_key),
        status,
    )
}

fn node_key(value: u64) -> NodeKey {
    NodeKey::new(value).unwrap()
}

fn resident(version: u64) -> NodeStatus {
    NodeStatus::Resident {
        version: BatchVersion::new(version),
    }
}

fn root_bounds() -> AxisAlignedBox {
    bounds([-2.0, -1.0, -11.0], [2.0, 1.0, -9.0])
}

fn box_at(x: f64, y: f64, z: f64) -> AxisAlignedBox {
    bounds([x - 0.1, y - 0.1, z - 0.1], [x + 0.1, y + 0.1, z + 0.1])
}

fn bounds(min: [f64; 3], max: [f64; 3]) -> AxisAlignedBox {
    AxisAlignedBox::new(min, max).unwrap()
}

fn planner(max_error: f64, hysteresis: f64) -> ViewPlanner {
    ViewPlanner::new(PlannerConfig::new(max_error, hysteresis).unwrap())
}

fn camera() -> Camera {
    Camera::perspective(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        100.0,
    )
    .unwrap()
}

fn generation(view: u64, generation: u64) -> ViewGenerationKey {
    ViewGenerationKey::new(ViewId::new(view), generation)
}

fn request_keys(plan: &point_view::ViewPlan) -> Vec<NodeKey> {
    plan.requests()
        .iter()
        .map(|request| request.node())
        .collect()
}

fn retained_keys(plan: &point_view::ViewPlan) -> Vec<NodeKey> {
    plan.retained_nodes()
        .iter()
        .map(|node| node.node_key())
        .collect()
}

fn retirement_batches(plan: &point_view::ViewPlan) -> Vec<BatchKey> {
    plan.retirements()
        .iter()
        .map(|retirement| retirement.batch_key())
        .collect()
}
