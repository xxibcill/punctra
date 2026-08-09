//! CPU benchmark for a complete six-level synthetic quadtree.

#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use point_view::{
    AvailableNode, AvailableNodes, AxisAlignedBox, NodeKey, NodeStatus, PlannerConfig,
    PlanningBudget, ViewPlanner,
};
use render_protocol::{
    BatchKey, BatchVersion, Camera, ESTIMATED_GPU_BYTES_PER_POINT, ViewGenerationKey, ViewId,
};

const LEAF_LEVEL: u32 = 6;
const NODE_COUNT: usize = 5_461;
const POINTS_PER_LEAF: u64 = 4_096;
const LOGICAL_LEAF_POINTS: u64 = 16_777_216;

fn planner_benchmark(criterion: &mut Criterion) {
    let nodes = quadtree();
    let camera = Camera::perspective(
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -200.0],
        [0.0, 1.0, 0.0],
        std::f32::consts::FRAC_PI_2,
        1.0,
        1_000.0,
    )
    .unwrap();
    let generation = ViewGenerationKey::new(ViewId::new(1), 0);
    let budget = PlanningBudget::new(
        LOGICAL_LEAF_POINTS,
        LOGICAL_LEAF_POINTS * ESTIMATED_GPU_BYTES_PER_POINT,
        u64::from(4_u32.pow(LEAF_LEVEL)),
    );
    let mut planner = ViewPlanner::new(PlannerConfig::new(2.0, 0.25).unwrap());

    criterion.bench_function("plan_5461_node_quadtree_16m_logical_points", |bencher| {
        bencher.iter(|| {
            planner
                .plan(
                    &camera,
                    [1_920, 1_080],
                    AvailableNodes::new(generation, &nodes),
                    budget,
                )
                .unwrap()
        });
    });
}

fn quadtree() -> Vec<AvailableNode> {
    let mut nodes = Vec::with_capacity(NODE_COUNT);
    for level in 0..=LEAF_LEVEL {
        let side = 1_u32 << level;
        for row in 0..side {
            for column in 0..side {
                nodes.push(quadtree_node(level, column, row));
            }
        }
    }
    assert_eq!(nodes.len(), NODE_COUNT);
    assert_eq!(
        u64::from(4_u32.pow(LEAF_LEVEL)) * POINTS_PER_LEAF,
        LOGICAL_LEAF_POINTS
    );
    nodes
}

fn quadtree_node(level: u32, column: u32, row: u32) -> AvailableNode {
    let side = 1_u32 << level;
    let key = key_for(level, column, row);
    let parent = (level > 0).then(|| key_for(level - 1, column / 2, row / 2));
    let width = 128.0 / f64::from(side);
    let min_x = -64.0 + f64::from(column) * width;
    let min_y = -64.0 + f64::from(row) * width;
    let bounds = AxisAlignedBox::new(
        [min_x, min_y, -201.0],
        [min_x + width, min_y + width, -199.0],
    )
    .unwrap();
    let geometric_error = if level == LEAF_LEVEL {
        0.0
    } else {
        256.0 / f64::from(side)
    };
    AvailableNode::new(
        key,
        parent,
        bounds,
        geometric_error,
        POINTS_PER_LEAF,
        POINTS_PER_LEAF * ESTIMATED_GPU_BYTES_PER_POINT,
        BatchKey::new(key.get()),
        NodeStatus::Resident {
            version: BatchVersion::new(1),
        },
    )
    .unwrap()
}

fn key_for(level: u32, column: u32, row: u32) -> NodeKey {
    let level_offset = (u64::from(4_u32.pow(level)) - 1) / 3;
    let side = 1_u32 << level;
    let index = u64::from(row * side + column);
    NodeKey::new(level_offset + index + 1).unwrap()
}

criterion_group!(benches, planner_benchmark);
criterion_main!(benches);
