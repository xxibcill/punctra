use std::{cmp::Ordering, mem};

use foundation_runtime::OperationControl;
use point_contracts::WorldBounds;
use point_source::SourceSpan;

use crate::{IndexError, IndexLimit, PrepareLimits, limits::require};

pub(crate) const BLOCK_POINTS: u64 = 65_536;
pub(crate) const MAX_NODE_SAMPLES: u64 = 4_096;
pub(crate) const SAMPLE_BYTES: u64 = 32;

#[derive(Clone, Copy, Debug)]
pub(crate) struct LeafRecord {
    pub(crate) span: SourceSpan,
    pub(crate) bounds: WorldBounds,
    pub(crate) sample_offset: u64,
    pub(crate) sample_count: u64,
    pub(crate) sample_checksum: [u8; 32],
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PlannedNode {
    pub(crate) parent: Option<usize>,
    pub(crate) children: Option<[usize; 2]>,
    pub(crate) bounds: WorldBounds,
    pub(crate) covered_point_count: u64,
    pub(crate) display_point_count: u64,
    pub(crate) geometric_error: f64,
    pub(crate) leaf: Option<LeafRecord>,
}

#[derive(Debug)]
pub(crate) struct TreePlan {
    pub(crate) nodes: Vec<PlannedNode>,
    pub(crate) leaf_count: u64,
}

#[derive(Clone, Copy)]
struct TemporaryNode {
    children: Option<[usize; 2]>,
    bounds: WorldBounds,
    covered_point_count: u64,
    leaf: Option<LeafRecord>,
}

pub(crate) fn plan(
    leaves: &[LeafRecord],
    limits: PrepareLimits,
    control: &OperationControl,
) -> Result<TreePlan, IndexError> {
    control.check_cancelled()?;
    if leaves.is_empty() {
        return Ok(TreePlan {
            nodes: Vec::new(),
            leaf_count: 0,
        });
    }
    let leaf_count = u64::try_from(leaves.len()).unwrap_or(u64::MAX);
    let node_count = leaf_count
        .checked_mul(2)
        .and_then(|value| value.checked_sub(1))
        .unwrap_or(u64::MAX);
    require(
        node_count,
        limits.max_hierarchy_nodes(),
        IndexLimit::HierarchyNodes,
    )?;
    preflight_memory(leaf_count, node_count, limits.max_build_working_bytes())?;

    let node_capacity = usize::try_from(node_count).map_err(|_| IndexError::ResourceLimit {
        limit: IndexLimit::AddressableHierarchyNodes,
        required: node_count,
        allowed: usize::MAX as u64,
    })?;
    let mut leaf_order = reserved_vec(leaves.len(), limits.max_build_working_bytes())?;
    leaf_order.extend(0..leaves.len());
    let mut temporary = reserved_vec(node_capacity, limits.max_build_working_bytes())?;
    let root = build_range(&mut leaf_order, leaves, &mut temporary, control)?;
    debug_assert_eq!(temporary.len(), node_capacity);

    let mut breadth_first = reserved_vec(node_capacity, limits.max_build_working_bytes())?;
    breadth_first.push(root);
    let mut cursor = 0;
    while cursor < breadth_first.len() {
        if cursor % 4_096 == 0 {
            control.check_cancelled()?;
        }
        let temporary_index = breadth_first[cursor];
        if let Some([left, right]) = temporary[temporary_index].children {
            breadth_first.push(left);
            breadth_first.push(right);
        }
        cursor += 1;
    }

    let mut stable_index_by_temporary =
        reserved_vec(node_capacity, limits.max_build_working_bytes())?;
    stable_index_by_temporary.resize(node_capacity, usize::MAX);
    for (stable, &temporary_index) in breadth_first.iter().enumerate() {
        if stable % 4_096 == 0 {
            control.check_cancelled()?;
        }
        stable_index_by_temporary[temporary_index] = stable;
    }

    let mut parent_by_stable = reserved_vec(node_capacity, limits.max_build_working_bytes())?;
    parent_by_stable.resize(node_capacity, None);
    for (position, &temporary_index) in breadth_first.iter().enumerate() {
        if position % 4_096 == 0 {
            control.check_cancelled()?;
        }
        let parent = stable_index_by_temporary[temporary_index];
        if let Some(children) = temporary[temporary_index].children {
            for child in children {
                parent_by_stable[stable_index_by_temporary[child]] = Some(parent);
            }
        }
    }

    let mut nodes = reserved_vec(node_capacity, limits.max_build_working_bytes())?;
    for (position, &temporary_index) in breadth_first.iter().enumerate() {
        if position % 4_096 == 0 {
            control.check_cancelled()?;
        }
        let temporary_node = temporary[temporary_index];
        let stable = stable_index_by_temporary[temporary_index];
        let children = temporary_node.children.map(|[left, right]| {
            [
                stable_index_by_temporary[left],
                stable_index_by_temporary[right],
            ]
        });
        let display_point_count = temporary_node.leaf.map_or_else(
            || temporary_node.covered_point_count.min(MAX_NODE_SAMPLES),
            |leaf| leaf.span.point_count(),
        );
        let geometric_error = if temporary_node.leaf.is_some() {
            0.0
        } else {
            finite_diagonal(temporary_node.bounds)
        };
        nodes.push(PlannedNode {
            parent: parent_by_stable[stable],
            children,
            bounds: temporary_node.bounds,
            covered_point_count: temporary_node.covered_point_count,
            display_point_count,
            geometric_error,
            leaf: temporary_node.leaf,
        });
    }

    Ok(TreePlan { nodes, leaf_count })
}

fn build_range(
    order: &mut [usize],
    leaves: &[LeafRecord],
    nodes: &mut Vec<TemporaryNode>,
    control: &OperationControl,
) -> Result<usize, IndexError> {
    control.check_cancelled()?;
    if let [leaf_index] = order {
        let leaf = leaves[*leaf_index];
        let node = TemporaryNode {
            children: None,
            bounds: leaf.bounds,
            covered_point_count: leaf.span.point_count(),
            leaf: Some(leaf),
        };
        nodes.push(node);
        return Ok(nodes.len() - 1);
    }

    let axis = longest_centroid_axis(order, leaves);
    order.sort_unstable_by(|left, right| compare_leaves(*left, *right, axis, leaves));
    control.check_cancelled()?;
    let middle = order.len() / 2;
    let (left_order, right_order) = order.split_at_mut(middle);
    let left = build_range(left_order, leaves, nodes, control)?;
    let right = build_range(right_order, leaves, nodes, control)?;
    let left_node = nodes[left];
    let right_node = nodes[right];
    let covered_point_count = left_node
        .covered_point_count
        .checked_add(right_node.covered_point_count)
        .ok_or(IndexError::CorruptWork {
            reason: "hierarchy Point count overflowed",
        })?;
    nodes.push(TemporaryNode {
        children: Some([left, right]),
        bounds: union(left_node.bounds, right_node.bounds)?,
        covered_point_count,
        leaf: None,
    });
    Ok(nodes.len() - 1)
}

fn longest_centroid_axis(order: &[usize], leaves: &[LeafRecord]) -> usize {
    let mut minimum = [f64::INFINITY; 3];
    let mut maximum = [f64::NEG_INFINITY; 3];
    for &index in order {
        let centroid = centroid(leaves[index].bounds);
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(centroid[axis]);
            maximum[axis] = maximum[axis].max(centroid[axis]);
        }
    }
    let extents = [
        maximum[0] - minimum[0],
        maximum[1] - minimum[1],
        maximum[2] - minimum[2],
    ];
    let mut longest = 0;
    for axis in 1..3 {
        if extents[axis].total_cmp(&extents[longest]) == Ordering::Greater {
            longest = axis;
        }
    }
    longest
}

fn compare_leaves(left: usize, right: usize, axis: usize, leaves: &[LeafRecord]) -> Ordering {
    centroid(leaves[left].bounds)[axis]
        .total_cmp(&centroid(leaves[right].bounds)[axis])
        .then_with(|| {
            leaves[left]
                .span
                .first_ordinal()
                .cmp(&leaves[right].span.first_ordinal())
        })
}

fn centroid(bounds: WorldBounds) -> [f64; 3] {
    let minimum = bounds.min();
    let maximum = bounds.max();
    [
        minimum[0] * 0.5 + maximum[0] * 0.5,
        minimum[1] * 0.5 + maximum[1] * 0.5,
        minimum[2] * 0.5 + maximum[2] * 0.5,
    ]
}

fn union(left: WorldBounds, right: WorldBounds) -> Result<WorldBounds, IndexError> {
    let left_minimum = left.min();
    let left_maximum = left.max();
    let right_minimum = right.min();
    let right_maximum = right.max();
    WorldBounds::new(
        [
            left_minimum[0].min(right_minimum[0]),
            left_minimum[1].min(right_minimum[1]),
            left_minimum[2].min(right_minimum[2]),
        ],
        [
            left_maximum[0].max(right_maximum[0]),
            left_maximum[1].max(right_maximum[1]),
            left_maximum[2].max(right_maximum[2]),
        ],
    )
    .map_err(|_| IndexError::CorruptWork {
        reason: "child bounds cannot form a finite union",
    })
}

fn finite_diagonal(bounds: WorldBounds) -> f64 {
    let minimum = bounds.min();
    let maximum = bounds.max();
    let dx = maximum[0] - minimum[0];
    let dy = maximum[1] - minimum[1];
    let dz = maximum[2] - minimum[2];
    let diagonal = dx.hypot(dy).hypot(dz);
    if diagonal.is_finite() {
        diagonal
    } else {
        f64::MAX
    }
}

fn preflight_memory(leaf_count: u64, node_count: u64, allowed: u64) -> Result<(), IndexError> {
    let bytes =
        |count: u64, item: usize| count.saturating_mul(u64::try_from(item).unwrap_or(u64::MAX));
    let required = bytes(leaf_count, mem::size_of::<LeafRecord>())
        .saturating_add(bytes(leaf_count, mem::size_of::<usize>()))
        .saturating_add(bytes(node_count, mem::size_of::<TemporaryNode>()))
        .saturating_add(bytes(node_count, mem::size_of::<usize>()).saturating_mul(3))
        .saturating_add(bytes(node_count, mem::size_of::<Option<usize>>()))
        .saturating_add(bytes(node_count, mem::size_of::<PlannedNode>()))
        .saturating_add(
            MAX_NODE_SAMPLES
                .saturating_mul(SAMPLE_BYTES)
                .saturating_mul(3),
        );
    require(required, allowed, IndexLimit::BuildWorkingBytes)
}

fn reserved_vec<T>(capacity: usize, allowed: u64) -> Result<Vec<T>, IndexError> {
    let required = u64::try_from(capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<T>()).unwrap_or(u64::MAX));
    require(required, allowed, IndexLimit::BuildWorkingBytes)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| IndexError::ResourceLimit {
            limit: IndexLimit::BuildWorkingBytes,
            required,
            allowed,
        })?;
    Ok(values)
}
