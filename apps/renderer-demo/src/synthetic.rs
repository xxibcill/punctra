use std::collections::{BTreeSet, VecDeque};

use point_view::{AvailableNode, AxisAlignedBox, NodeKey, NodeRequest, NodeStatus, PlanError};
use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch, PointId, ProtocolError,
    RenderPoint, SourceId, ViewGenerationKey,
};

pub(crate) const RESIDENT_POINT_BUDGET: u64 = 600_000;
pub(crate) const RESIDENT_BYTE_BUDGET: u64 = RESIDENT_POINT_BUDGET * ESTIMATED_GPU_BYTES_PER_POINT;
pub(crate) const RESIDENT_BATCH_BUDGET: u64 = 640;
pub(crate) const LOGICAL_POINT_COUNT: u64 = 16_777_216;
pub(crate) const TOTAL_NODE_COUNT: u64 = 5_461;
pub(crate) const SCENE_RADIUS: f64 = 2_500.0;
pub(crate) const SCENE_TARGET: [f64; 3] = [6_378_137.125, 13_756_432.625, 120.0];

const QUADTREE_DEPTH: u32 = 6;
const LEAF_TILES_PER_AXIS: u32 = 1 << QUADTREE_DEPTH;
const LEAF_POINTS_PER_AXIS: u32 = 64;
const INTERNAL_POINTS_PER_AXIS: u32 = 32;
const GLOBAL_POINTS_PER_AXIS: u32 = LEAF_TILES_PER_AXIS * LEAF_POINTS_PER_AXIS;
const SCENE_EXTENT: f64 = 2_048.0;
const HALF_SCENE_EXTENT: f64 = SCENE_EXTENT / 2.0;
const MIN_HEIGHT: f64 = -48.0;
const MAX_HEIGHT: f64 = 48.0;
const HEIGHT_SCALE: f32 = 28.0;
const SYNTHETIC_SOURCE: SourceId = SourceId::new([0x66; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeLayout {
    level: u32,
    column: u32,
    row: u32,
}

#[derive(Clone, Copy, Debug)]
struct SyntheticNode {
    node: AvailableNode,
    layout: NodeLayout,
    latest_version: BatchVersion,
}

impl SyntheticNode {
    fn next_version(self) -> BatchVersion {
        let version = self
            .latest_version
            .get()
            .checked_add(1)
            .expect("the interactive demo cannot exhaust batch versions");
        BatchVersion::new(version)
    }
}

#[derive(Debug)]
pub(crate) struct SyntheticScene {
    view_generation: ViewGenerationKey,
    nodes: Vec<SyntheticNode>,
    pending: VecDeque<NodeKey>,
}

impl SyntheticScene {
    pub(crate) fn new(view_generation: ViewGenerationKey) -> Result<Self, PlanError> {
        let capacity = usize::try_from(TOTAL_NODE_COUNT).expect("the node count fits in usize");
        let mut nodes = Vec::with_capacity(capacity);

        for level in 0..=QUADTREE_DEPTH {
            let cells_per_axis = 1_u32 << level;
            for row in 0..cells_per_axis {
                for column in 0..cells_per_axis {
                    let layout = NodeLayout { level, column, row };
                    nodes.push(SyntheticNode {
                        node: make_node(layout)?,
                        layout,
                        latest_version: BatchVersion::new(0),
                    });
                }
            }
        }

        debug_assert_eq!(nodes.len(), capacity);
        Ok(Self {
            view_generation,
            nodes,
            pending: VecDeque::new(),
        })
    }

    pub(crate) fn planning_nodes(&self) -> Vec<AvailableNode> {
        self.nodes.iter().map(|node| node.node).collect()
    }

    pub(crate) fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) {
        let previously_queued = self.pending.drain(..).collect::<BTreeSet<_>>();
        let newly_requested = requests
            .iter()
            .map(|request| request.node())
            .collect::<BTreeSet<_>>();
        let mut retained_requests = BTreeSet::new();
        for key in demanded_nodes.iter().copied() {
            let was_queued = previously_queued.contains(&key);
            let was_requested_now = newly_requested.contains(&key);
            if !was_queued && !was_requested_now {
                continue;
            }
            if was_queued {
                retained_requests.insert(key);
            }
            let node = &mut self.nodes[node_index(key)].node;
            if node.status() == NodeStatus::Missing && was_requested_now {
                *node = node.with_status(NodeStatus::Requested);
            }
            if node.status() == NodeStatus::Requested && (was_queued || was_requested_now) {
                self.pending.push_back(key);
            }
        }
        for key in previously_queued.difference(&retained_requests).copied() {
            let node = &mut self.nodes[node_index(key)].node;
            if node.status() == NodeStatus::Requested {
                *node = node.with_status(NodeStatus::Missing);
            }
        }
    }

    pub(crate) fn next_batch(&mut self) -> Result<Option<PointBatch>, ProtocolError> {
        while let Some(key) = self.pending.pop_front() {
            let index = node_index(key);
            let node = self.nodes[index];
            if node.node.status() != NodeStatus::Requested {
                continue;
            }

            let version = node.next_version();
            let batch = make_batch(self.view_generation, node.node, node.layout, version)?;
            self.nodes[index].latest_version = version;
            return Ok(Some(batch));
        }
        Ok(None)
    }

    pub(crate) fn mark_resident(&mut self, batch_key: BatchKey, version: BatchVersion) {
        let node = self.node_for_batch_mut(batch_key);
        debug_assert_eq!(node.node.status(), NodeStatus::Requested);
        debug_assert_eq!(version, node.latest_version);
        node.node = node.node.with_status(NodeStatus::Resident { version });
    }

    pub(crate) fn mark_retired(&mut self, batch_key: BatchKey, expected_version: BatchVersion) {
        let node = &mut self.node_for_batch_mut(batch_key).node;
        debug_assert_eq!(
            node.status(),
            NodeStatus::Resident {
                version: expected_version,
            }
        );
        *node = node.with_status(NodeStatus::Missing);
    }

    pub(crate) fn mark_rejected(&mut self, batch_key: BatchKey, version: BatchVersion) {
        let node = self.node_for_batch_mut(batch_key);
        debug_assert_eq!(node.node.status(), NodeStatus::Requested);
        debug_assert_eq!(node.latest_version, version);
        node.node = node.node.with_status(NodeStatus::Missing);
    }

    fn node_for_batch_mut(&mut self, batch_key: BatchKey) -> &mut SyntheticNode {
        let key =
            NodeKey::new(batch_key.get()).expect("synthetic batch keys are nonzero node keys");
        let index = node_index(key);
        &mut self.nodes[index]
    }

    pub(crate) fn resident_batches(&self) -> u64 {
        let count = self
            .nodes
            .iter()
            .filter(|node| matches!(node.node.status(), NodeStatus::Resident { .. }))
            .count();
        u64::try_from(count).expect("the resident node count fits in u64")
    }

    pub(crate) fn pending_batches(&self) -> u64 {
        u64::try_from(self.pending.len()).expect("the pending node count fits in u64")
    }

    pub(crate) fn highlight_ids() -> Vec<PointId> {
        let quarter = GLOBAL_POINTS_PER_AXIS / 4;
        let center = GLOBAL_POINTS_PER_AXIS / 2;
        [
            point_id(quarter, quarter),
            point_id(center, center),
            point_id(3 * quarter, 3 * quarter),
        ]
        .to_vec()
    }
}

fn make_node(layout: NodeLayout) -> Result<AvailableNode, PlanError> {
    let key = node_key(layout);
    let parent = parent_key(layout);
    let bounds = node_bounds(layout)?;
    let points_per_axis = points_per_axis(layout.level);
    let point_count = u64::from(points_per_axis) * u64::from(points_per_axis);
    let estimated_bytes = point_count * ESTIMATED_GPU_BYTES_PER_POINT;
    let geometric_error = if layout.level == QUADTREE_DEPTH {
        0.0
    } else {
        node_extent(layout.level) / 32.0
    };

    AvailableNode::new(
        key,
        parent,
        bounds,
        geometric_error,
        point_count,
        estimated_bytes,
        BatchKey::new(key.get()),
        NodeStatus::Missing,
    )
}

fn node_bounds(layout: NodeLayout) -> Result<AxisAlignedBox, PlanError> {
    let extent = node_extent(layout.level);
    let min_x = SCENE_TARGET[0] - HALF_SCENE_EXTENT + f64::from(layout.column) * extent;
    let min_y = SCENE_TARGET[1] - HALF_SCENE_EXTENT + f64::from(layout.row) * extent;
    AxisAlignedBox::new(
        [min_x, min_y, SCENE_TARGET[2] + MIN_HEIGHT],
        [min_x + extent, min_y + extent, SCENE_TARGET[2] + MAX_HEIGHT],
    )
}

fn make_batch(
    view_generation: ViewGenerationKey,
    node: AvailableNode,
    layout: NodeLayout,
    version: BatchVersion,
) -> Result<PointBatch, ProtocolError> {
    let points_per_axis = points_per_axis(layout.level);
    let point_capacity = usize::try_from(u64::from(points_per_axis).pow(2))
        .expect("the synthetic batch size fits in usize");
    let mut points = Vec::with_capacity(point_capacity);
    let region_points = GLOBAL_POINTS_PER_AXIS >> layout.level;
    let sample_stride = region_points / points_per_axis;
    let start_column = layout.column * region_points;
    let start_row = layout.row * region_points;
    let origin = node_origin(layout);

    for row in 0..points_per_axis {
        for column in 0..points_per_axis {
            let global_column = start_column + column * sample_stride;
            let global_row = start_row + row * sample_stride;
            points.push(make_point(global_column, global_row, origin)?);
        }
    }

    PointBatch::new(view_generation, node.batch_key(), version, origin, points)
}

#[allow(clippy::cast_possible_truncation)]
fn make_point(
    global_column: u32,
    global_row: u32,
    origin: [f64; 3],
) -> Result<RenderPoint, ProtocolError> {
    let spacing = SCENE_EXTENT / f64::from(GLOBAL_POINTS_PER_AXIS);
    let scene_x = -HALF_SCENE_EXTENT + (f64::from(global_column) + 0.5) * spacing;
    let scene_y = -HALF_SCENE_EXTENT + (f64::from(global_row) + 0.5) * spacing;
    let noise = point_noise(global_column, global_row);
    let height = terrain_height(scene_x as f32, scene_y as f32, noise);
    let world_x = SCENE_TARGET[0] + scene_x;
    let world_y = SCENE_TARGET[1] + scene_y;

    let relative_position = [
        (world_x - origin[0]) as f32,
        (world_y - origin[1]) as f32,
        height,
    ];

    RenderPoint::new(
        relative_position,
        terrain_color(height, noise),
        point_id(global_column, global_row),
    )
}

fn terrain_height(x: f32, y: f32, noise: f32) -> f32 {
    let broad_hills = (x * 0.0045).sin() * (y * 0.0038).cos() * HEIGHT_SCALE;
    let fine_ridges = ((x + y) * 0.014).sin() * 6.5;
    let drainage = -12.0 * (-((y - x * 0.28) * 0.009).powi(2)).exp();
    broad_hills + fine_ridges + drainage + noise
}

fn terrain_color(height: f32, noise: f32) -> [u8; 4] {
    let variation = if noise.is_sign_positive() { 10 } else { 0 };
    if height < -14.0 {
        [34, 104, 151, 255]
    } else if height < 1.0 {
        [42, 128_u8.saturating_add(variation), 92, 255]
    } else if height < 17.0 {
        [111, 151_u8.saturating_add(variation), 77, 255]
    } else if height < 29.0 {
        [177, 145, 91_u8.saturating_add(variation), 255]
    } else {
        [215, 218, 210, 255]
    }
}

fn point_noise(column: u32, row: u32) -> f32 {
    let mut hash = column.wrapping_mul(0xC2B2_AE35) ^ row.wrapping_mul(0x27D4_EB2F);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x7FEB_352D);
    hash ^= hash >> 15;
    let low_bits = u16::try_from(hash & u32::from(u16::MAX)).expect("the hash was masked to u16");
    let unit = f32::from(low_bits) / f32::from(u16::MAX);
    (unit - 0.5) * 0.8
}

fn node_origin(layout: NodeLayout) -> [f64; 3] {
    let extent = node_extent(layout.level);
    [
        SCENE_TARGET[0] - HALF_SCENE_EXTENT + (f64::from(layout.column) + 0.5) * extent,
        SCENE_TARGET[1] - HALF_SCENE_EXTENT + (f64::from(layout.row) + 0.5) * extent,
        SCENE_TARGET[2],
    ]
}

const fn points_per_axis(level: u32) -> u32 {
    if level == QUADTREE_DEPTH {
        LEAF_POINTS_PER_AXIS
    } else {
        INTERNAL_POINTS_PER_AXIS
    }
}

fn node_extent(level: u32) -> f64 {
    SCENE_EXTENT / f64::from(1_u32 << level)
}

fn node_key(layout: NodeLayout) -> NodeKey {
    let cells_per_axis = 1_u64 << layout.level;
    let key = level_offset(layout.level)
        + u64::from(layout.row) * cells_per_axis
        + u64::from(layout.column)
        + 1;
    NodeKey::new(key).expect("synthetic node keys are nonzero")
}

fn parent_key(layout: NodeLayout) -> Option<NodeKey> {
    (layout.level > 0).then(|| {
        node_key(NodeLayout {
            level: layout.level - 1,
            column: layout.column / 2,
            row: layout.row / 2,
        })
    })
}

const fn level_offset(level: u32) -> u64 {
    ((1_u64 << (2 * level)) - 1) / 3
}

fn node_index(key: NodeKey) -> usize {
    usize::try_from(key.get() - 1).expect("the synthetic node key fits in usize")
}

fn point_id(column: u32, row: u32) -> PointId {
    PointId::new(
        SYNTHETIC_SOURCE,
        u64::from(row) * u64::from(GLOBAL_POINTS_PER_AXIS) + u64::from(column),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use render_protocol::{RenderLimits, RenderStateModel, RenderUpdate, ViewId};

    use super::*;

    fn view_generation() -> ViewGenerationKey {
        ViewGenerationKey::new(ViewId::new(1), 1)
    }

    #[test]
    fn hierarchy_represents_more_than_ten_million_logical_points() {
        let scene = SyntheticScene::new(view_generation()).unwrap();

        assert_eq!(
            scene.nodes.len(),
            usize::try_from(TOTAL_NODE_COUNT).unwrap()
        );
        let represented_leaf_points = scene
            .nodes
            .iter()
            .filter(|node| node.layout.level == QUADTREE_DEPTH)
            .map(|node| node.node.point_count())
            .sum::<u64>();
        assert!(represented_leaf_points >= 10_000_000);
        assert_eq!(represented_leaf_points, LOGICAL_POINT_COUNT);
        assert_eq!(LOGICAL_POINT_COUNT, 16_777_216);
        assert_eq!(scene.nodes[0].node.parent(), None);
        assert!(
            scene
                .nodes
                .iter()
                .all(|node| node.node.status() == NodeStatus::Missing)
        );
    }

    #[test]
    fn requested_batches_are_deterministic_and_change_residency_explicitly() {
        let mut scene = SyntheticScene::new(view_generation()).unwrap();
        let root = scene.nodes[0].node;
        scene.nodes[0].node = root.with_status(NodeStatus::Requested);
        scene.pending.push_back(root.key());
        let first = scene.next_batch().unwrap().unwrap();
        scene.mark_resident(first.key(), first.version());
        scene.mark_retired(first.key(), first.version());
        scene.nodes[0].node = root.with_status(NodeStatus::Requested);
        scene.pending.push_back(root.key());
        let second = scene.next_batch().unwrap().unwrap();
        scene.mark_resident(second.key(), second.version());

        assert_eq!(first.points(), second.points());
        assert_eq!(
            first.world_origin().map(f64::to_bits),
            second.world_origin().map(f64::to_bits)
        );
        assert_eq!(first.version(), BatchVersion::new(1));
        assert_eq!(second.version(), BatchVersion::new(2));
        assert_eq!(scene.resident_batches(), 1);
        assert_eq!(scene.pending_batches(), 0);
    }

    #[test]
    fn every_batch_uses_unique_stable_point_identities() {
        let scene = SyntheticScene::new(view_generation()).unwrap();
        let leaf_index = node_index(node_key(NodeLayout {
            level: QUADTREE_DEPTH,
            column: 17,
            row: 29,
        }));
        let batch = make_batch(
            view_generation(),
            scene.nodes[leaf_index].node,
            scene.nodes[leaf_index].layout,
            BatchVersion::new(1),
        )
        .unwrap();
        let identities = batch
            .points()
            .iter()
            .map(RenderPoint::point_id)
            .collect::<BTreeSet<_>>();

        assert_eq!(identities.len(), batch.points().len());
        assert_eq!(batch.point_count(), u64::from(LEAF_POINTS_PER_AXIS).pow(2));
    }

    #[test]
    fn synthetic_point_ordinals_cover_the_zero_based_source_range() {
        assert_eq!(point_id(0, 0).ordinal(), 0);
        assert_eq!(
            point_id(GLOBAL_POINTS_PER_AXIS - 1, GLOBAL_POINTS_PER_AXIS - 1).ordinal(),
            LOGICAL_POINT_COUNT - 1
        );
    }

    #[test]
    fn highlighted_identities_are_present_in_coarse_coverage() {
        let scene = SyntheticScene::new(view_generation()).unwrap();
        let root = make_batch(
            view_generation(),
            scene.nodes[0].node,
            scene.nodes[0].layout,
            BatchVersion::new(1),
        )
        .unwrap();
        let root_ids = root
            .points()
            .iter()
            .map(RenderPoint::point_id)
            .collect::<BTreeSet<_>>();

        assert!(
            SyntheticScene::highlight_ids()
                .into_iter()
                .all(|identity| root_ids.contains(&identity))
        );
    }

    #[test]
    fn rematerialized_nodes_advance_renderer_batch_versions() {
        let generation = view_generation();
        let mut scene = SyntheticScene::new(generation).unwrap();
        let root = scene.nodes[0].node;
        let mut renderer_state = RenderStateModel::new(RenderLimits::new(
            root.estimated_bytes(),
            root.point_count(),
            1,
        ));
        renderer_state
            .apply(&RenderUpdate::Reset {
                view_generation: generation,
            })
            .unwrap();

        scene.nodes[0].node = root.with_status(NodeStatus::Requested);
        scene.pending.push_back(root.key());
        let first = scene.next_batch().unwrap().unwrap();
        let first_key = first.key();
        let first_version = first.version();
        renderer_state
            .apply(&RenderUpdate::Upsert { batch: first })
            .unwrap();
        scene.mark_resident(first_key, first_version);
        renderer_state
            .apply(&RenderUpdate::Remove {
                view_generation: generation,
                key: first_key,
                expected_version: first_version,
            })
            .unwrap();
        scene.mark_retired(first_key, first_version);

        scene.nodes[0].node = root.with_status(NodeStatus::Requested);
        scene.pending.push_back(root.key());
        let second = scene.next_batch().unwrap().unwrap();
        assert_eq!(second.version(), BatchVersion::new(2));
        let second_key = second.key();
        let second_version = second.version();
        renderer_state
            .apply(&RenderUpdate::Upsert { batch: second })
            .expect("a rematerialized node must use a strictly newer version");
        scene.mark_resident(second_key, second_version);
    }

    #[test]
    fn rejected_materialization_versions_are_not_reused() {
        let mut scene = SyntheticScene::new(view_generation()).unwrap();
        let root = scene.nodes[0].node;
        scene.nodes[0].node = root.with_status(NodeStatus::Requested);
        scene.pending.push_back(root.key());
        let rejected = scene.next_batch().unwrap().unwrap();
        scene.mark_rejected(rejected.key(), rejected.version());

        scene.nodes[0].node = root.with_status(NodeStatus::Requested);
        scene.pending.push_back(root.key());
        let retry = scene.next_batch().unwrap().unwrap();

        assert_eq!(rejected.version(), BatchVersion::new(1));
        assert_eq!(retry.version(), BatchVersion::new(2));
    }
}
