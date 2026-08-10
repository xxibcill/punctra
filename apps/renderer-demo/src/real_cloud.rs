use std::{collections::VecDeque, io, mem, time::Instant};

use point_index::{DisplayCoverage, IndexNodeId, IndexReadSummary, NodeReadBudget, PreparedIndex};
use point_view::{AvailableNode, AxisAlignedBox, NodeKey, NodeRequest, NodeStatus};
use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch, RenderPoint,
    ViewGenerationKey,
};

use crate::scene::{SceneMetrics, SceneResult};

const STAGING_POINT_BUDGET: u64 = 65_536;
const STAGING_BYTE_BUDGET: u64 = 16 * 1_024 * 1_024;
const QUEUE_BUDGET: QueueBudget = QueueBudget {
    max_nodes: 640,
    max_host_bytes: STAGING_BYTE_BUDGET,
};
const HIERARCHY_BYTE_BUDGET: u64 = 512 * 1_024 * 1_024;
const HIERARCHY_FIXED_WORKING_BYTES: u64 = 64 * 1_024;
// The fixed allowance covers handles/containers. The per-node allowance covers
// PreparedIndex's retained IndexNode array, this bridge's side table and
// cached AvailableNode snapshot, plus point-view's simultaneous hierarchy
// clone, ordered indexes/sets, child lists, projections, and traversal arrays.
const HIERARCHY_WORKING_BYTES_PER_NODE: u64 = 2 * 1_024;
// Index display samples contain positions only; this color makes no Source attribute claim.
const POSITION_ONLY_COLOR: [u8; 4] = [190, 205, 220, 255];

#[derive(Clone, Copy, Debug)]
struct QueueBudget {
    max_nodes: u64,
    max_host_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
struct RealNode {
    index_id: IndexNodeId,
    coverage: DisplayCoverage,
    covered_source_point_count: u64,
    latest_issued_version: u64,
}

#[derive(Debug)]
pub(crate) struct RealCloudScene {
    generation: ViewGenerationKey,
    index: PreparedIndex,
    nodes: Vec<RealNode>,
    planning_nodes: Vec<AvailableNode>,
    pending: VecDeque<NodeKey>,
    camera_target: [f64; 3],
    camera_radius: f64,
    bridge_ready_at: Instant,
    first_accepted_batch: bool,
    staged_points: u64,
    staged_bytes: u64,
    peak_queued_batches: u64,
    peak_staged_points: u64,
    peak_staged_bytes: u64,
    cancelled_requests: u64,
}

impl RealCloudScene {
    pub(crate) fn new(generation: ViewGenerationKey, index: PreparedIndex) -> SceneResult<Self> {
        let node_count = index.hierarchy().nodes().len();
        let preflight_hierarchy_bytes = hierarchy_charge(node_count)?;
        if preflight_hierarchy_bytes > HIERARCHY_BYTE_BUDGET {
            return Err(resource_limit(
                "application hierarchy bytes",
                HIERARCHY_BYTE_BUDGET,
            ));
        }
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(node_count)
            .map_err(|error| invalid_data(format!("could not reserve hierarchy: {error}")))?;
        let mut planning_nodes = Vec::new();
        planning_nodes
            .try_reserve_exact(node_count)
            .map_err(|error| {
                invalid_data(format!("could not reserve planning snapshot: {error}"))
            })?;
        if hierarchy_charge(nodes.capacity().max(planning_nodes.capacity()))?
            > HIERARCHY_BYTE_BUDGET
        {
            return Err(resource_limit(
                "application hierarchy bytes",
                HIERARCHY_BYTE_BUDGET,
            ));
        }
        for indexed in index.hierarchy().nodes() {
            let key = node_key(indexed.id())?;
            let parent = indexed.parent().map(node_key).transpose()?;
            let bounds = AxisAlignedBox::new(indexed.bounds().min(), indexed.bounds().max())?;
            let estimated_bytes = indexed
                .display_point_count()
                .checked_mul(ESTIMATED_GPU_BYTES_PER_POINT)
                .ok_or_else(|| invalid_data("index node renderer-byte cost overflowed"))?;
            let available = AvailableNode::new(
                key,
                parent,
                bounds,
                indexed.geometric_error(),
                indexed.display_point_count(),
                estimated_bytes,
                BatchKey::new(key.get()),
                NodeStatus::Missing,
            )?;
            nodes.push(RealNode {
                index_id: indexed.id(),
                coverage: indexed.coverage(),
                covered_source_point_count: indexed.covered_point_count(),
                latest_issued_version: 0,
            });
            planning_nodes.push(available);
        }

        let (camera_target, camera_radius) = camera_frame(index.descriptor().world_bounds())?;
        Ok(Self {
            generation,
            index,
            nodes,
            planning_nodes,
            pending: VecDeque::new(),
            camera_target,
            camera_radius,
            bridge_ready_at: Instant::now(),
            first_accepted_batch: false,
            staged_points: 0,
            staged_bytes: 0,
            peak_queued_batches: 0,
            peak_staged_points: 0,
            peak_staged_bytes: 0,
            cancelled_requests: 0,
        })
    }

    pub(crate) fn planning_nodes(&self) -> &[AvailableNode] {
        &self.planning_nodes
    }

    pub(crate) fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) -> SceneResult<()> {
        self.reconcile_requests_with_budget(demanded_nodes, requests, QUEUE_BUDGET)
    }

    fn reconcile_requests_with_budget(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
        budget: QueueBudget,
    ) -> SceneResult<()> {
        let mut next_pending = VecDeque::new();
        let mut reserved_host_bytes = 0;
        for key in self.pending.iter().copied() {
            if demanded_nodes.contains(&key) {
                let available = self.planning_nodes[self.node_index(key)];
                if available.status() == NodeStatus::Requested {
                    push_queued_node(
                        &mut next_pending,
                        key,
                        queued_node_reservation(available)?,
                        &mut reserved_host_bytes,
                        budget,
                    )?;
                }
            }
        }
        for key in requests.iter().map(|request| request.node()) {
            debug_assert!(demanded_nodes.contains(&key));
            if demanded_nodes.contains(&key) && !next_pending.contains(&key) {
                let status = self.planning_nodes[self.node_index(key)].status();
                if matches!(status, NodeStatus::Missing | NodeStatus::Requested) {
                    let available = self.planning_nodes[self.node_index(key)];
                    push_queued_node(
                        &mut next_pending,
                        key,
                        queued_node_reservation(available)?,
                        &mut reserved_host_bytes,
                        budget,
                    )?;
                }
            }
        }

        for pending_index in 0..self.pending.len() {
            let key = self.pending[pending_index];
            if next_pending.contains(&key) {
                continue;
            }
            let was_cancelled = {
                let index = self.node_index(key);
                let available = &mut self.planning_nodes[index];
                if available.status() == NodeStatus::Requested {
                    *available = available.with_status(NodeStatus::Missing);
                    true
                } else {
                    false
                }
            };
            if was_cancelled {
                self.cancelled_requests = self.cancelled_requests.saturating_add(1);
            }
        }
        for key in next_pending.iter().copied() {
            let index = self.node_index(key);
            let available = &mut self.planning_nodes[index];
            if available.status() == NodeStatus::Missing
                && requests.iter().any(|request| request.node() == key)
            {
                *available = available.with_status(NodeStatus::Requested);
            }
        }
        self.pending = next_pending;

        let queued = u64::try_from(self.pending.len()).unwrap_or(u64::MAX);
        self.peak_queued_batches = self.peak_queued_batches.max(queued);
        Ok(())
    }

    /// Materializes no more than one queued hierarchy node.
    pub(crate) fn next_batch(&mut self) -> SceneResult<Option<PointBatch>> {
        let Some(key) = self.pending.pop_front() else {
            return Ok(None);
        };
        let node_index = self.node_index(key);
        let node = self.nodes[node_index];
        let available = self.planning_nodes[node_index];
        if available.status() != NodeStatus::Requested {
            return Ok(None);
        }

        let result = self.materialize(node, available);
        let (batch, staged_bytes) = match result {
            Ok(materialized) => materialized,
            Err(error) => {
                self.planning_nodes[node_index] = available.with_status(NodeStatus::Missing);
                self.clear_staging();
                return Err(error);
            }
        };
        self.staged_points = batch.point_count();
        self.staged_bytes = staged_bytes;
        self.peak_staged_points = self.peak_staged_points.max(self.staged_points);
        self.peak_staged_bytes = self.peak_staged_bytes.max(self.staged_bytes);
        self.nodes[node_index].latest_issued_version = batch.version().get();
        Ok(Some(batch))
    }

    pub(crate) fn mark_resident(&mut self, key: BatchKey, version: BatchVersion) {
        let node_key = NodeKey::new(key.get()).expect("index-backed renderer keys are nonzero");
        let node_index = self.node_index(node_key);
        let point_count = {
            let node = self.nodes[node_index];
            let available = &mut self.planning_nodes[node_index];
            debug_assert_eq!(available.status(), NodeStatus::Requested);
            debug_assert_eq!(version.get(), node.latest_issued_version);
            *available = available.with_status(NodeStatus::Resident { version });
            available.point_count()
        };
        self.clear_staging();

        if !self.first_accepted_batch {
            self.first_accepted_batch = true;
            println!(
                "First accepted visible real-cloud batch\n  node: {}\n  Points: {}\n  \
                 bridge-ready to accepted Upsert: {:.3} s",
                node_key.get(),
                point_count,
                self.bridge_ready_at.elapsed().as_secs_f64()
            );
        }
    }

    pub(crate) fn mark_retired(&mut self, key: BatchKey, expected_version: BatchVersion) {
        let node_key = NodeKey::new(key.get()).expect("index-backed renderer keys are nonzero");
        let node_index = self.node_index(node_key);
        let available = &mut self.planning_nodes[node_index];
        debug_assert_eq!(
            available.status(),
            NodeStatus::Resident {
                version: expected_version,
            }
        );
        *available = available.with_status(NodeStatus::Missing);
    }

    pub(crate) fn mark_rejected(&mut self, key: BatchKey, version: BatchVersion) {
        let node_key = NodeKey::new(key.get()).expect("index-backed renderer keys are nonzero");
        let node_index = self.node_index(node_key);
        debug_assert_eq!(
            self.planning_nodes[node_index].status(),
            NodeStatus::Requested
        );
        debug_assert_eq!(self.nodes[node_index].latest_issued_version, version.get());
        self.planning_nodes[node_index] =
            self.planning_nodes[node_index].with_status(NodeStatus::Missing);
        self.clear_staging();
    }

    pub(crate) const fn camera_target(&self) -> [f64; 3] {
        self.camera_target
    }

    pub(crate) const fn camera_radius(&self) -> f64 {
        self.camera_radius
    }

    pub(crate) fn metrics(&self) -> SceneMetrics {
        let resident_batches = self
            .planning_nodes
            .iter()
            .filter(|node| matches!(node.status(), NodeStatus::Resident { .. }))
            .count();
        SceneMetrics {
            logical_points: self.index.descriptor().source_point_count(),
            resident_batches: u64::try_from(resident_batches).unwrap_or(u64::MAX),
            queued_batches: u64::try_from(self.pending.len()).unwrap_or(u64::MAX),
            staged_points: self.staged_points,
            staged_bytes: self.staged_bytes,
            peak_queued_batches: self.peak_queued_batches,
            peak_staged_points: self.peak_staged_points,
            peak_staged_bytes: self.peak_staged_bytes,
            cancelled_requests: self.cancelled_requests,
        }
    }

    fn materialize(
        &self,
        node: RealNode,
        available: AvailableNode,
    ) -> SceneResult<(PointBatch, u64)> {
        if available.point_count() > STAGING_POINT_BUDGET {
            return Err(resource_limit("node staging Points", STAGING_POINT_BUDGET));
        }
        let budget = NodeReadBudget::new(STAGING_POINT_BUDGET, STAGING_BYTE_BUDGET)?;
        let mut stream = self.index.read_node(node.index_id, budget)?;
        let origin = bounds_midpoint(available.bounds());
        let source = self.index.descriptor().source();
        let transform = self.index.descriptor().position_transform();
        let capacity = usize::try_from(available.point_count())
            .map_err(|_| invalid_data("node display count does not fit host address space"))?;
        let reserved_staging_bytes = render_staging_charge(capacity, 0)?;
        if reserved_staging_bytes > STAGING_BYTE_BUDGET {
            return Err(resource_limit("node staging bytes", STAGING_BYTE_BUDGET));
        }
        let mut points = Vec::new();
        points
            .try_reserve_exact(capacity)
            .map_err(|error| invalid_data(format!("could not reserve node staging: {error}")))?;
        let actual_reserved_bytes = render_staging_charge(points.capacity(), 0)?;
        if actual_reserved_bytes > STAGING_BYTE_BUDGET {
            return Err(resource_limit("node staging bytes", STAGING_BYTE_BUDGET));
        }
        let mut peak_staged_bytes = actual_reserved_bytes;
        let mut previous_ordinal = None;

        while let Some(batch) = stream.next()? {
            if batch.node() != node.index_id
                || batch.source() != source
                || batch.transform() != transform
            {
                return Err(invalid_data(
                    "index node batch identity changed during one stream",
                ));
            }
            for sample in batch.samples().iter().copied() {
                if previous_ordinal.is_some_and(|previous| previous >= sample.ordinal()) {
                    return Err(invalid_data(
                        "index node samples were not globally sorted and unique",
                    ));
                }
                previous_ordinal = Some(sample.ordinal());
                let relative = relative_position(sample.world_position(transform), origin)?;
                points.push(RenderPoint::new(
                    relative,
                    POSITION_ONLY_COLOR,
                    sample.point_id(source),
                )?);
            }
            peak_staged_bytes = peak_staged_bytes.max(render_staging_charge(
                points.capacity(),
                batch.estimated_payload_bytes(),
            )?);
            if peak_staged_bytes > STAGING_BYTE_BUDGET {
                return Err(resource_limit("node staging bytes", STAGING_BYTE_BUDGET));
            }
        }

        let summary = stream.summary().ok_or_else(|| {
            invalid_data("exhausted index node stream did not publish an exact summary")
        })?;
        validate_summary(
            summary,
            node,
            available,
            source,
            u64::try_from(points.len()).unwrap_or(u64::MAX),
        )?;

        let version = node
            .latest_issued_version
            .checked_add(1)
            .ok_or_else(|| invalid_data("renderer batch version overflowed"))?;
        let batch = PointBatch::new(
            self.generation,
            available.batch_key(),
            BatchVersion::new(version),
            origin,
            points,
        )?;
        let retained_staging_bytes = batch_staging_charge(batch.points().len())?;
        Ok((batch, peak_staged_bytes.max(retained_staging_bytes)))
    }

    fn node_index(&self, key: NodeKey) -> usize {
        let index = usize::try_from(key.get() - 1)
            .expect("validated root-first index identities fit host address space");
        debug_assert_eq!(self.nodes[index].index_id.get(), key.get());
        index
    }

    fn clear_staging(&mut self) {
        self.staged_points = 0;
        self.staged_bytes = 0;
    }
}

fn push_queued_node(
    pending: &mut VecDeque<NodeKey>,
    key: NodeKey,
    node_reservation: u64,
    reserved_host_bytes: &mut u64,
    budget: QueueBudget,
) -> SceneResult<()> {
    let required_nodes = u64::try_from(pending.len())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    if required_nodes > budget.max_nodes {
        return Err(resource_limit("queued nodes", budget.max_nodes));
    }
    let next_reservations = reserved_host_bytes
        .checked_add(node_reservation)
        .ok_or_else(|| invalid_data("queued node host reservations overflowed"))?;
    if pending.len() == pending.capacity() {
        ensure_queue_bytes(
            pending.len().saturating_add(1),
            next_reservations,
            budget.max_host_bytes,
        )?;
        pending
            .try_reserve_exact(1)
            .map_err(|error| invalid_data(format!("could not reserve request queue: {error}")))?;
    }
    ensure_queue_bytes(pending.capacity(), next_reservations, budget.max_host_bytes)?;
    pending.push_back(key);
    *reserved_host_bytes = next_reservations;
    Ok(())
}

fn queued_node_reservation(available: AvailableNode) -> SceneResult<u64> {
    let point_count = usize::try_from(available.point_count())
        .map_err(|_| invalid_data("queued node Point count does not fit host address space"))?;
    batch_staging_charge(point_count)
}

fn ensure_queue_bytes(capacity: usize, reservations: u64, allowed: u64) -> SceneResult<()> {
    let capacity =
        u64::try_from(capacity).map_err(|_| invalid_data("request queue capacity overflowed"))?;
    let node_bytes = u64::try_from(mem::size_of::<NodeKey>())
        .map_err(|_| invalid_data("request queue node size does not fit u64"))?;
    let container_bytes = u64::try_from(mem::size_of::<VecDeque<NodeKey>>())
        .map_err(|_| invalid_data("request queue container size does not fit u64"))?;
    let required = capacity
        .checked_mul(node_bytes)
        .and_then(|bytes| bytes.checked_add(container_bytes))
        .and_then(|bytes| bytes.checked_add(reservations))
        .ok_or_else(|| invalid_data("request queue byte charge overflowed"))?;
    if required > allowed {
        return Err(resource_limit("request queue host bytes", allowed));
    }
    Ok(())
}

fn node_key(id: IndexNodeId) -> SceneResult<NodeKey> {
    Ok(NodeKey::new(id.get())?)
}

fn hierarchy_charge(node_capacity: usize) -> SceneResult<u64> {
    let node_count = u64::try_from(node_capacity)
        .map_err(|_| invalid_data("application hierarchy capacity overflowed"))?;
    node_count
        .checked_mul(HIERARCHY_WORKING_BYTES_PER_NODE)
        .and_then(|bytes| bytes.checked_add(HIERARCHY_FIXED_WORKING_BYTES))
        .ok_or_else(|| invalid_data("application hierarchy byte charge overflowed"))
}

fn bounds_midpoint(bounds: AxisAlignedBox) -> [f64; 3] {
    let min = bounds.min();
    let max = bounds.max();
    std::array::from_fn(|axis| min[axis] * 0.5 + max[axis] * 0.5)
}

fn camera_frame(bounds: Option<point_contracts::WorldBounds>) -> SceneResult<([f64; 3], f64)> {
    let Some(bounds) = bounds else {
        return Ok(([0.0; 3], 100.0));
    };
    let target = std::array::from_fn(|axis| bounds.min()[axis] * 0.5 + bounds.max()[axis] * 0.5);
    let squared_diagonal = (0..3).try_fold(0.0, |sum, axis| {
        let extent = bounds.max()[axis] - bounds.min()[axis];
        let next = sum + extent * extent;
        next.is_finite()
            .then_some(next)
            .ok_or_else(|| invalid_data("Source bounds are too large for a finite camera frame"))
    })?;
    let radius = (squared_diagonal.sqrt() * 1.25).max(1.0);
    if radius * 8.0 > f64::from(f32::MAX) {
        return Err(invalid_data(
            "Source bounds are too large for finite renderer camera clipping",
        ));
    }
    Ok((target, radius))
}

#[allow(clippy::cast_possible_truncation)]
fn relative_position(world: [f64; 3], origin: [f64; 3]) -> SceneResult<[f32; 3]> {
    let relative = std::array::from_fn(|axis| world[axis] - origin[axis]);
    if relative.iter().any(|value| {
        !value.is_finite() || *value < f64::from(f32::MIN) || *value > f64::from(f32::MAX)
    }) {
        return Err(invalid_data(
            "origin-relative Source position does not fit finite renderer coordinates",
        ));
    }
    Ok(relative.map(|value| value as f32))
}

fn render_staging_charge(render_capacity: usize, current_index_bytes: u64) -> SceneResult<u64> {
    staging_charge::<Vec<RenderPoint>>(render_capacity, current_index_bytes)
}

fn batch_staging_charge(render_points: usize) -> SceneResult<u64> {
    staging_charge::<PointBatch>(render_points, 0)
}

fn staging_charge<Container>(render_points: usize, current_index_bytes: u64) -> SceneResult<u64> {
    let point_count = u64::try_from(render_points)
        .map_err(|_| invalid_data("staged renderer point count overflowed"))?;
    let render_point_bytes = u64::try_from(mem::size_of::<RenderPoint>())
        .map_err(|_| invalid_data("renderer point size does not fit u64"))?;
    let container_bytes = u64::try_from(mem::size_of::<Container>())
        .map_err(|_| invalid_data("staging container size does not fit u64"))?;
    point_count
        .checked_mul(render_point_bytes)
        .and_then(|bytes| bytes.checked_add(container_bytes))
        .and_then(|bytes| bytes.checked_add(current_index_bytes))
        .ok_or_else(|| invalid_data("node staging byte charge overflowed"))
}

fn validate_summary(
    summary: &IndexReadSummary,
    node: RealNode,
    available: AvailableNode,
    source: render_protocol::SourceId,
    observed_points: u64,
) -> SceneResult<()> {
    if summary.node() != node.index_id
        || summary.source() != source
        || summary.emitted_point_count() != observed_points
        || summary.emitted_point_count() != available.point_count()
        || summary.coverage() != node.coverage
        || summary.covered_source_point_count() != node.covered_source_point_count
    {
        return Err(invalid_data(
            "index terminal summary did not match the staged node batch",
        ));
    }
    Ok(())
}

fn resource_limit(resource: &'static str, limit: u64) -> Box<dyn std::error::Error> {
    invalid_data(format!(
        "{resource} exceeded the application limit of {limit}"
    ))
}

fn invalid_data(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidData, message.into()))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };

    use point_contracts::{AttributeColumns, CoordinateReference, PositionTransform};
    use point_index::PrepareLimits;
    use point_view::{AvailableNodes, PlanningBudget, ViewPlan, ViewPlanner};
    use render_protocol::{Camera, RenderLimits, RenderStateModel, RenderUpdate, Viewport};
    use source_memory::MemorySource;

    use crate::{
        orbit_camera::OrbitCamera,
        synthetic::{RESIDENT_BATCH_BUDGET, RESIDENT_BYTE_BUDGET, RESIDENT_POINT_BUDGET},
    };

    use super::*;

    const TEST_GENERATION: ViewGenerationKey =
        ViewGenerationKey::new(render_protocol::ViewId::new(91), 1);

    #[test]
    fn hierarchy_preflight_has_an_exact_working_set_boundary() {
        let maximum_nodes = usize::try_from(
            (HIERARCHY_BYTE_BUDGET - HIERARCHY_FIXED_WORKING_BYTES)
                / HIERARCHY_WORKING_BYTES_PER_NODE,
        )
        .unwrap();

        assert!(hierarchy_charge(maximum_nodes).unwrap() <= HIERARCHY_BYTE_BUDGET);
        assert!(hierarchy_charge(maximum_nodes + 1).unwrap() > HIERARCHY_BYTE_BUDGET);

        let always_live_metadata = mem::size_of::<point_index::IndexNode>()
            + mem::size_of::<RealNode>()
            + mem::size_of::<AvailableNode>();
        assert!(
            usize::try_from(HIERARCHY_WORKING_BYTES_PER_NODE).unwrap() > always_live_metadata,
            "the transient planner allowance must exceed the always-live metadata"
        );
    }

    #[test]
    fn queue_admission_limits_fail_without_mutating_scene_state() {
        let directory = TestDirectory::new().unwrap();
        let mut scene = fixture_scene(directory.path()).unwrap();
        let visible = visible_camera(&scene);
        let mut planner = ViewPlanner::default();
        let plan = plan(&mut planner, &scene, &visible);
        assert_eq!(plan.requests().len(), 1);
        let original_status = scene.planning_nodes[0].status();
        let original_metrics = scene.metrics();

        assert!(
            scene
                .reconcile_requests_with_budget(
                    plan.demanded_nodes(),
                    plan.requests(),
                    QueueBudget {
                        max_nodes: 0,
                        max_host_bytes: QUEUE_BUDGET.max_host_bytes,
                    },
                )
                .is_err()
        );
        assert!(scene.pending.is_empty());
        assert_eq!(scene.planning_nodes[0].status(), original_status);
        assert_eq!(scene.metrics(), original_metrics);

        assert!(
            scene
                .reconcile_requests_with_budget(
                    plan.demanded_nodes(),
                    plan.requests(),
                    QueueBudget {
                        max_nodes: 1,
                        max_host_bytes: 0,
                    },
                )
                .is_err()
        );
        assert!(scene.pending.is_empty());
        assert_eq!(scene.planning_nodes[0].status(), original_status);
        assert_eq!(scene.metrics(), original_metrics);
    }

    #[test]
    fn camera_change_prunes_an_already_requested_real_node() {
        let directory = TestDirectory::new().unwrap();
        let mut scene = fixture_scene(directory.path()).unwrap();
        let cached_snapshot = scene.planning_nodes().as_ptr();
        assert_eq!(scene.planning_nodes().as_ptr(), cached_snapshot);
        let mut planner = ViewPlanner::default();
        let visible = visible_camera(&scene);
        let first = plan(&mut planner, &scene, &visible);

        assert_eq!(first.demanded_nodes().len(), 1);
        scene
            .reconcile_requests(first.demanded_nodes(), first.requests())
            .unwrap();
        assert_eq!(scene.pending.len(), 1);
        assert_eq!(scene.planning_nodes[0].status(), NodeStatus::Requested);

        let hidden = hidden_camera(&scene);
        let changed = plan(&mut planner, &scene, &hidden);
        assert!(changed.demanded_nodes().is_empty());
        scene
            .reconcile_requests(changed.demanded_nodes(), changed.requests())
            .unwrap();

        assert!(scene.pending.is_empty());
        assert_eq!(scene.planning_nodes[0].status(), NodeStatus::Missing);
        assert_eq!(scene.metrics().cancelled_requests, 1);
    }

    #[test]
    fn source_samples_cross_one_exact_atomic_upsert_with_monotonic_versions() {
        let directory = TestDirectory::new().unwrap();
        let mut scene = fixture_scene(directory.path()).unwrap();
        let source = scene.index.descriptor().source();
        let visible = visible_camera(&scene);
        let mut planner = ViewPlanner::default();
        let first_plan = plan(&mut planner, &scene, &visible);
        scene
            .reconcile_requests(first_plan.demanded_nodes(), first_plan.requests())
            .unwrap();

        let first = scene.next_batch().unwrap().unwrap();
        assert_eq!(first.version(), BatchVersion::new(1));
        assert_eq!(first.point_count(), 4);
        assert!(scene.next_batch().unwrap().is_none());
        for (expected_ordinal, point) in first.points().iter().enumerate() {
            assert_eq!(point.point_id().source(), source);
            assert_eq!(
                point.point_id().ordinal(),
                u64::try_from(expected_ordinal).unwrap()
            );
            let world_x = first.world_origin()[0] + f64::from(point.relative_position()[0]);
            let expected_world_x =
                4_000_000.0 + f64::from(u32::try_from(expected_ordinal).unwrap());
            assert_eq!(world_x.to_bits(), expected_world_x.to_bits());
        }

        let rejected_key = first.key();
        let rejected_version = first.version();
        let mut rejecting_renderer = RenderStateModel::new(RenderLimits::new(0, 0, 0));
        rejecting_renderer
            .apply(&RenderUpdate::Reset {
                view_generation: TEST_GENERATION,
            })
            .unwrap();
        assert!(
            rejecting_renderer
                .apply(&RenderUpdate::Upsert { batch: first })
                .is_err()
        );
        scene.mark_rejected(rejected_key, rejected_version);
        assert_eq!(scene.planning_nodes[0].status(), NodeStatus::Missing);
        assert_eq!(scene.metrics().staged_points, 0);

        let retry_plan = plan(&mut planner, &scene, &visible);
        scene
            .reconcile_requests(retry_plan.demanded_nodes(), retry_plan.requests())
            .unwrap();
        let first = scene.next_batch().unwrap().unwrap();
        assert_eq!(first.version(), BatchVersion::new(2));

        let first_key = first.key();
        let first_version = first.version();
        let mut renderer = renderer_state();
        renderer
            .apply(&RenderUpdate::Upsert { batch: first })
            .unwrap();
        scene.mark_resident(first_key, first_version);
        assert_eq!(scene.metrics().staged_points, 0);
        assert_eq!(scene.metrics().resident_batches, 1);

        renderer
            .apply(&RenderUpdate::Remove {
                view_generation: TEST_GENERATION,
                key: first_key,
                expected_version: first_version,
            })
            .unwrap();
        scene.mark_retired(first_key, first_version);

        let second_plan = plan(&mut planner, &scene, &visible);
        scene
            .reconcile_requests(second_plan.demanded_nodes(), second_plan.requests())
            .unwrap();
        let second = scene.next_batch().unwrap().unwrap();
        assert_eq!(second.version(), BatchVersion::new(3));
        let second_key = second.key();
        let second_version = second.version();
        let update = RenderUpdate::Upsert { batch: second };
        let accepted = renderer.apply(&update).unwrap();
        scene.mark_resident(second_key, second_version);

        assert_eq!(accepted.report().resident().batch_count(), 1);
        assert_eq!(accepted.report().resident().point_count(), 4);
        assert_eq!(scene.metrics().resident_batches, 1);
    }

    fn fixture_scene(directory: &Path) -> SceneResult<RealCloudScene> {
        let ticks = vec![[0, 0, 0], [1, 2, 3], [2, 4, 6], [3, 6, 9]];
        let point_count = ticks.len();
        let input = MemorySource::from_columns(
            PositionTransform::new([4_000_000.0, 800_000.0, 120.0], [1.0; 3])?,
            CoordinateReference::Unknown,
            ticks,
            AttributeColumns::empty(point_count),
        )?;
        let source = source_memory::open(input).blocking_wait()?;
        let index = point_index::prepare(
            source,
            directory.join("fixture.pidx"),
            PrepareLimits::default(),
        )
        .blocking_wait()?;
        RealCloudScene::new(TEST_GENERATION, index)
    }

    fn visible_camera(scene: &RealCloudScene) -> Camera {
        OrbitCamera::new(scene.camera_target(), scene.camera_radius())
            .as_render_camera()
            .unwrap()
    }

    fn hidden_camera(scene: &RealCloudScene) -> Camera {
        let target = scene.camera_target();
        let eye = [target[0] + 10.0, target[1], target[2]];
        Camera::perspective(
            eye,
            [eye[0] + 10.0, eye[1], eye[2]],
            [0.0, 0.0, 1.0],
            std::f32::consts::FRAC_PI_4,
            0.5,
            20_000.0,
        )
        .unwrap()
    }

    fn plan(planner: &mut ViewPlanner, scene: &RealCloudScene, camera: &Camera) -> ViewPlan {
        let nodes = scene.planning_nodes();
        planner
            .plan(
                camera,
                Viewport::new(1_280, 800).unwrap(),
                AvailableNodes::new(TEST_GENERATION, nodes),
                PlanningBudget::new(
                    RESIDENT_POINT_BUDGET,
                    RESIDENT_BYTE_BUDGET,
                    RESIDENT_BATCH_BUDGET,
                ),
            )
            .unwrap()
    }

    fn renderer_state() -> RenderStateModel {
        let mut renderer = RenderStateModel::new(RenderLimits::new(
            RESIDENT_BYTE_BUDGET,
            RESIDENT_POINT_BUDGET,
            RESIDENT_BATCH_BUDGET,
        ));
        renderer
            .apply(&RenderUpdate::Reset {
                view_generation: TEST_GENERATION,
            })
            .unwrap();
        renderer
    }

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> io::Result<Self> {
            static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);
            loop {
                let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "punctra-renderer-demo-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Ok(Self(path)),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
