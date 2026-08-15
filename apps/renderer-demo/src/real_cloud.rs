use std::{collections::VecDeque, mem, time::Instant};

use point_index::{
    DisplayCoverage, DisplaySampleContract, IndexError, IndexNodeId, IndexReadSummary,
    NodeReadBudget, PreparedIndex,
};
use point_view::{AvailableNode, AxisAlignedBox, NodeKey, NodeRequest, NodeStatus};
use render_protocol::{
    BatchKey, BatchVersion, ESTIMATED_GPU_BYTES_PER_POINT, PointBatch, PointId, RenderPoint,
    ViewGenerationKey,
};

use crate::{
    diagnostic::{ViewFailure, ViewPhase},
    scene::{SceneMetrics, SceneResult},
};
use renderer_demo::display::{DisplayMode, PointColorizer};

pub(crate) const STAGING_POINT_BUDGET: u64 = 65_536;
pub(crate) const STAGING_BYTE_BUDGET: u64 = 16 * 1_024 * 1_024;
const QUEUE_BUDGET: QueueBudget = QueueBudget {
    max_nodes: 640,
    max_host_bytes: STAGING_BYTE_BUDGET,
};
pub(crate) const QUEUED_NODE_BUDGET: u64 = QUEUE_BUDGET.max_nodes;
pub(crate) const QUEUED_HOST_BYTE_BUDGET: u64 = QUEUE_BUDGET.max_host_bytes;
pub(crate) const HIERARCHY_BYTE_BUDGET: u64 = 512 * 1_024 * 1_024;
const HIERARCHY_FIXED_WORKING_BYTES: u64 = 64 * 1_024;
// The fixed allowance covers handles/containers. The per-node allowance covers
// PreparedIndex's retained IndexNode array, this bridge's side table and
// cached AvailableNode snapshot, plus point-view's simultaneous hierarchy
// clone, ordered indexes/sets, child lists, projections, and traversal arrays.
const HIERARCHY_WORKING_BYTES_PER_NODE: u64 = 2 * 1_024;
const HIGHLIGHT_ID_COUNT: usize = 3;

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
    colorizer: PointColorizer,
    nodes: Vec<RealNode>,
    planning_nodes: Vec<AvailableNode>,
    pending: VecDeque<NodeKey>,
    highlight_ids: Vec<PointId>,
    camera_target: [f64; 3],
    camera_radius: f64,
    bridge_ready_at: Instant,
    first_accepted_batch: bool,
    staged_points: u64,
    staged_bytes: u64,
    peak_queued_batches: u64,
    peak_queued_host_bytes: u64,
    peak_staged_points: u64,
    peak_staged_bytes: u64,
    cancelled_requests: u64,
    retired_batches: u64,
    rejected_batches: u64,
}

impl RealCloudScene {
    pub(crate) fn new(
        generation: ViewGenerationKey,
        index: PreparedIndex,
        display_mode: DisplayMode,
    ) -> SceneResult<Self> {
        Self::new_with_contract(generation, index, display_mode, false)
    }

    pub(crate) fn new_for_review(
        generation: ViewGenerationKey,
        index: PreparedIndex,
        display_mode: DisplayMode,
    ) -> SceneResult<Self> {
        Self::new_with_contract(generation, index, display_mode, true)
    }

    fn new_with_contract(
        generation: ViewGenerationKey,
        index: PreparedIndex,
        display_mode: DisplayMode,
        exact_review: bool,
    ) -> SceneResult<Self> {
        validate_display_contract(
            display_mode,
            index.descriptor().display_sample_contract(),
            exact_review,
        )?;
        let node_count = index.hierarchy().nodes().len();
        let (mut nodes, mut planning_nodes) = reserve_hierarchy_vectors(node_count)?;
        for indexed in index.hierarchy().nodes() {
            let key = node_key(indexed.id())?;
            let parent = indexed.parent().map(node_key).transpose()?;
            let bounds = AxisAlignedBox::new(indexed.bounds().min(), indexed.bounds().max())?;
            let estimated_bytes = indexed
                .display_point_count()
                .checked_mul(ESTIMATED_GPU_BYTES_PER_POINT)
                .ok_or_else(|| {
                    internal_failure(
                        ViewPhase::Hierarchy,
                        "index node renderer-byte cost overflowed",
                    )
                })?;
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
        let mut highlight_ids = Vec::new();
        highlight_ids
            .try_reserve_exact(HIGHLIGHT_ID_COUNT)
            .map_err(|error| {
                allocation_failure(
                    ViewPhase::Hierarchy,
                    format_args!("could not reserve highlight identities: {error}"),
                )
            })?;

        let world_bounds = index.descriptor().world_bounds();
        let (camera_target, camera_radius) = camera_frame(world_bounds)?;
        Ok(Self {
            generation,
            index,
            colorizer: PointColorizer::for_source(display_mode, world_bounds),
            nodes,
            planning_nodes,
            pending: VecDeque::new(),
            highlight_ids,
            camera_target,
            camera_radius,
            bridge_ready_at: Instant::now(),
            first_accepted_batch: false,
            staged_points: 0,
            staged_bytes: 0,
            peak_queued_batches: 0,
            peak_queued_host_bytes: 0,
            peak_staged_points: 0,
            peak_staged_bytes: 0,
            cancelled_requests: 0,
            retired_batches: 0,
            rejected_batches: 0,
        })
    }

    pub(crate) fn planning_nodes(&self) -> &[AvailableNode] {
        &self.planning_nodes
    }

    pub(crate) fn reconcile_requests(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
    ) -> SceneResult<u64> {
        self.reconcile_requests_with_budget(demanded_nodes, requests, QUEUE_BUDGET)
    }

    fn reconcile_requests_with_budget(
        &mut self,
        demanded_nodes: &[NodeKey],
        requests: &[NodeRequest],
        budget: QueueBudget,
    ) -> SceneResult<u64> {
        let mut next_pending = VecDeque::new();
        let mut reserved_host_bytes = 0;
        let retained_queue_bytes = queue_container_charge(self.pending.capacity())?;
        debug_assert!(
            requests
                .iter()
                .all(|request| demanded_nodes.contains(&request.node()))
        );
        for key in demanded_nodes.iter().copied() {
            let was_pending = self.pending.contains(&key);
            let newly_requested = requests.iter().any(|request| request.node() == key);
            if !was_pending && !newly_requested {
                continue;
            }
            let available = self.planning_nodes[self.node_index(key)];
            if matches!(
                available.status(),
                NodeStatus::Missing | NodeStatus::Requested
            ) && !admit_queued_node(
                &mut next_pending,
                key,
                queued_node_reservation(available)?,
                &mut reserved_host_bytes,
                retained_queue_bytes,
                budget,
            )? {
                break;
            }
        }

        let queued_host_bytes = queue_charge(
            next_pending.capacity(),
            reserved_host_bytes,
            retained_queue_bytes,
        )?;

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
        let mut issued = 0_u64;
        for key in next_pending.iter().copied() {
            let index = self.node_index(key);
            let available = &mut self.planning_nodes[index];
            if available.status() == NodeStatus::Missing
                && requests.iter().any(|request| request.node() == key)
            {
                *available = available.with_status(NodeStatus::Requested);
                issued = issued.saturating_add(1);
            }
        }
        self.pending = next_pending;

        let queued = u64::try_from(self.pending.len()).unwrap_or(u64::MAX);
        self.peak_queued_batches = self.peak_queued_batches.max(queued);
        self.peak_queued_host_bytes = self.peak_queued_host_bytes.max(queued_host_bytes);
        Ok(issued)
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
        self.capture_highlight_ids(batch.points());
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
        self.retired_batches = self.retired_batches.saturating_add(1);
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
        self.rejected_batches = self.rejected_batches.saturating_add(1);
        self.clear_staging();
    }

    pub(crate) const fn camera_target(&self) -> [f64; 3] {
        self.camera_target
    }

    pub(crate) const fn camera_radius(&self) -> f64 {
        self.camera_radius
    }

    pub(crate) fn highlight_ids(&self) -> Vec<PointId> {
        self.highlight_ids.clone()
    }

    pub(crate) fn metrics(&self) -> SceneMetrics {
        let mut metrics = SceneMetrics {
            logical_points: self.index.descriptor().source_point_count(),
            hierarchy_nodes: u64::try_from(self.planning_nodes.len()).unwrap_or(u64::MAX),
            queued_batches: u64::try_from(self.pending.len()).unwrap_or(u64::MAX),
            staged_points: self.staged_points,
            staged_bytes: self.staged_bytes,
            peak_queued_batches: self.peak_queued_batches,
            peak_queued_host_bytes: self.peak_queued_host_bytes,
            peak_staged_points: self.peak_staged_points,
            peak_staged_bytes: self.peak_staged_bytes,
            cancelled_requests: self.cancelled_requests,
            retired_batches: self.retired_batches,
            rejected_batches: self.rejected_batches,
            ..SceneMetrics::default()
        };
        for (available, real) in self.planning_nodes.iter().zip(&self.nodes) {
            match available.status() {
                NodeStatus::Missing => {
                    metrics.missing_nodes = metrics.missing_nodes.saturating_add(1);
                }
                NodeStatus::Requested => {
                    metrics.requested_nodes = metrics.requested_nodes.saturating_add(1);
                }
                NodeStatus::Resident { .. } => {
                    metrics.resident_batches = metrics.resident_batches.saturating_add(1);
                    metrics.resident_points = metrics
                        .resident_points
                        .saturating_add(available.point_count());
                    match real.coverage {
                        DisplayCoverage::Sampled => {
                            metrics.sampled_resident_batches =
                                metrics.sampled_resident_batches.saturating_add(1);
                            metrics.sampled_resident_points = metrics
                                .sampled_resident_points
                                .saturating_add(available.point_count());
                        }
                        DisplayCoverage::Complete => {
                            metrics.complete_resident_batches =
                                metrics.complete_resident_batches.saturating_add(1);
                            metrics.complete_resident_points = metrics
                                .complete_resident_points
                                .saturating_add(available.point_count());
                        }
                    }
                }
            }
        }
        metrics
    }

    fn materialize(
        &self,
        node: RealNode,
        available: AvailableNode,
    ) -> SceneResult<(PointBatch, u64)> {
        if available.point_count() > STAGING_POINT_BUDGET {
            return Err(resource_limit(
                ViewPhase::HostStaging,
                "node staging Points",
                STAGING_POINT_BUDGET,
            ));
        }
        let budget = NodeReadBudget::new(STAGING_POINT_BUDGET, STAGING_BYTE_BUDGET)
            .map_err(|error| index_read_failure(&error))?;
        let mut stream = self
            .index
            .read_node(node.index_id, budget)
            .map_err(|error| index_read_failure(&error))?;
        let origin = bounds_midpoint(available.bounds());
        let source = self.index.descriptor().source();
        let transform = self.index.descriptor().position_transform();
        let (mut points, mut peak_staged_bytes) = reserve_node_staging(available.point_count())?;
        let mut previous_ordinal = None;

        while let Some(batch) = stream.next().map_err(|error| index_read_failure(&error))? {
            if batch.node() != node.index_id
                || batch.source() != source
                || batch.transform() != transform
            {
                return Err(internal_failure(
                    ViewPhase::NodeRead,
                    "index node batch identity changed during one stream",
                ));
            }
            let attributes = batch.display_attributes();
            if self.colorizer.requires_attributes() && attributes.is_none() {
                return Err(internal_failure(
                    ViewPhase::NodeRead,
                    "index display Attribute rows did not match the selected display mode",
                ));
            }
            if attributes.is_some_and(|attributes| attributes.len() != batch.samples().len()) {
                return Err(internal_failure(
                    ViewPhase::NodeRead,
                    "index display Attributes were not row-aligned with samples",
                ));
            }
            for (row, sample) in batch.samples().iter().copied().enumerate() {
                if previous_ordinal.is_some_and(|previous| previous >= sample.ordinal()) {
                    return Err(internal_failure(
                        ViewPhase::NodeRead,
                        "index node samples were not globally sorted and unique",
                    ));
                }
                previous_ordinal = Some(sample.ordinal());
                let world = sample.world_position(transform);
                let relative = relative_position(world, origin)?;
                points.push(RenderPoint::new(
                    relative,
                    self.colorizer
                        .color(world[2], attributes.map(|values| values[row]))
                        .map_err(|error| internal_failure(ViewPhase::HostStaging, error))?,
                    sample.point_id(source),
                )?);
            }
            peak_staged_bytes = peak_staged_bytes.max(render_staging_charge(
                points.capacity(),
                batch.estimated_payload_bytes(),
            )?);
            if peak_staged_bytes > STAGING_BYTE_BUDGET {
                return Err(resource_limit(
                    ViewPhase::HostStaging,
                    "node staging bytes",
                    STAGING_BYTE_BUDGET,
                ));
            }
        }

        let summary = stream.summary().ok_or_else(|| {
            internal_failure(
                ViewPhase::NodeRead,
                "exhausted index node stream did not publish an exact summary",
            )
        })?;
        validate_summary(
            summary,
            node,
            available,
            source,
            u64::try_from(points.len()).unwrap_or(u64::MAX),
            self.index.descriptor().display_sample_contract(),
        )?;

        let version = node.latest_issued_version.checked_add(1).ok_or_else(|| {
            internal_failure(ViewPhase::HostStaging, "renderer batch version overflowed")
        })?;
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

    fn capture_highlight_ids(&mut self, points: &[RenderPoint]) {
        if !self.highlight_ids.is_empty() || points.is_empty() {
            return;
        }
        for numerator in 1..=HIGHLIGHT_ID_COUNT {
            let index = points
                .len()
                .saturating_mul(numerator)
                .checked_div(HIGHLIGHT_ID_COUNT + 1)
                .unwrap_or(0)
                .min(points.len() - 1);
            let point_id = points[index].point_id();
            if !self.highlight_ids.contains(&point_id) {
                self.highlight_ids.push(point_id);
            }
        }
    }

    fn clear_staging(&mut self) {
        self.staged_points = 0;
        self.staged_bytes = 0;
    }
}

fn reserve_hierarchy_vectors(
    node_count: usize,
) -> SceneResult<(Vec<RealNode>, Vec<AvailableNode>)> {
    if hierarchy_charge(node_count)? > HIERARCHY_BYTE_BUDGET {
        return Err(resource_limit(
            ViewPhase::Hierarchy,
            "application hierarchy bytes",
            HIERARCHY_BYTE_BUDGET,
        ));
    }
    let mut nodes = Vec::new();
    nodes.try_reserve_exact(node_count).map_err(|error| {
        allocation_failure(
            ViewPhase::Hierarchy,
            format_args!("could not reserve hierarchy: {error}"),
        )
    })?;
    let mut planning_nodes = Vec::new();
    planning_nodes
        .try_reserve_exact(node_count)
        .map_err(|error| {
            allocation_failure(
                ViewPhase::Hierarchy,
                format_args!("could not reserve planning snapshot: {error}"),
            )
        })?;
    if hierarchy_charge(nodes.capacity().max(planning_nodes.capacity()))? > HIERARCHY_BYTE_BUDGET {
        return Err(resource_limit(
            ViewPhase::Hierarchy,
            "application hierarchy bytes",
            HIERARCHY_BYTE_BUDGET,
        ));
    }
    Ok((nodes, planning_nodes))
}

fn reserve_node_staging(point_count: u64) -> SceneResult<(Vec<RenderPoint>, u64)> {
    let capacity = usize::try_from(point_count).map_err(|_| {
        internal_failure(
            ViewPhase::HostStaging,
            "node display count does not fit host address space",
        )
    })?;
    if render_staging_charge(capacity, 0)? > STAGING_BYTE_BUDGET {
        return Err(resource_limit(
            ViewPhase::HostStaging,
            "node staging bytes",
            STAGING_BYTE_BUDGET,
        ));
    }
    let mut points = Vec::new();
    points.try_reserve_exact(capacity).map_err(|error| {
        allocation_failure(
            ViewPhase::HostStaging,
            format_args!("could not reserve node staging: {error}"),
        )
    })?;
    let retained_bytes = render_staging_charge(points.capacity(), 0)?;
    if retained_bytes > STAGING_BYTE_BUDGET {
        return Err(resource_limit(
            ViewPhase::HostStaging,
            "node staging bytes",
            STAGING_BYTE_BUDGET,
        ));
    }
    Ok((points, retained_bytes))
}

/// Admits one node or defers it when only the aggregate queue budget is full.
///
/// A node that cannot fit an otherwise empty queue is a configuration error.
/// Once at least one higher-priority node is retained, reaching either aggregate
/// limit is normal backpressure and the caller leaves the remaining nodes for a
/// later plan.
fn admit_queued_node(
    pending: &mut VecDeque<NodeKey>,
    key: NodeKey,
    node_reservation: u64,
    reserved_host_bytes: &mut u64,
    retained_queue_bytes: u64,
    budget: QueueBudget,
) -> SceneResult<bool> {
    if budget.max_nodes == 0 {
        return Err(resource_limit(
            ViewPhase::Planning,
            "queued nodes",
            budget.max_nodes,
        ));
    }
    ensure_queue_bytes(
        1,
        node_reservation,
        retained_queue_bytes,
        budget.max_host_bytes,
    )?;

    let required_nodes = u64::try_from(pending.len())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    if required_nodes > budget.max_nodes {
        return Ok(false);
    }
    let next_reservations = reserved_host_bytes
        .checked_add(node_reservation)
        .ok_or_else(|| {
            internal_failure(
                ViewPhase::Planning,
                "queued node host reservations overflowed",
            )
        })?;
    if pending.len() == pending.capacity() {
        let required_capacity = pending.len().saturating_add(1);
        if queue_growth_charge(
            pending.capacity(),
            required_capacity,
            next_reservations,
            retained_queue_bytes,
        )? > budget.max_host_bytes
        {
            if pending.is_empty() {
                return Err(resource_limit(
                    ViewPhase::Planning,
                    "request queue host bytes",
                    budget.max_host_bytes,
                ));
            }
            return Ok(false);
        }
        let mut grown = VecDeque::new();
        grown
            .try_reserve_exact(required_capacity)
            .map_err(|error| {
                allocation_failure(
                    ViewPhase::Planning,
                    format_args!("could not reserve request queue: {error}"),
                )
            })?;
        if queue_growth_charge(
            pending.capacity(),
            grown.capacity(),
            next_reservations,
            retained_queue_bytes,
        )? > budget.max_host_bytes
        {
            if pending.is_empty() {
                return Err(resource_limit(
                    ViewPhase::Planning,
                    "request queue host bytes",
                    budget.max_host_bytes,
                ));
            }
            return Ok(false);
        }
        grown.extend(pending.iter().copied());
        *pending = grown;
    } else if queue_charge(pending.capacity(), next_reservations, retained_queue_bytes)?
        > budget.max_host_bytes
    {
        return Ok(false);
    }
    pending.push_back(key);
    *reserved_host_bytes = next_reservations;
    Ok(true)
}

fn queued_node_reservation(available: AvailableNode) -> SceneResult<u64> {
    let point_count = usize::try_from(available.point_count()).map_err(|_| {
        internal_failure(
            ViewPhase::Planning,
            "queued node Point count does not fit host address space",
        )
    })?;
    batch_staging_charge(point_count)
}

fn ensure_queue_bytes(
    capacity: usize,
    reservations: u64,
    retained_queue_bytes: u64,
    allowed: u64,
) -> SceneResult<()> {
    if queue_charge(capacity, reservations, retained_queue_bytes)? > allowed {
        return Err(resource_limit(
            ViewPhase::Planning,
            "request queue host bytes",
            allowed,
        ));
    }
    Ok(())
}

fn queue_charge(capacity: usize, reservations: u64, retained_queue_bytes: u64) -> SceneResult<u64> {
    queue_container_charge(capacity)?
        .checked_add(reservations)
        .and_then(|bytes| bytes.checked_add(retained_queue_bytes))
        .ok_or_else(|| {
            internal_failure(ViewPhase::Planning, "request queue byte charge overflowed")
        })
}

fn queue_growth_charge(
    old_capacity: usize,
    new_capacity: usize,
    reservations: u64,
    retained_queue_bytes: u64,
) -> SceneResult<u64> {
    queue_container_charge(old_capacity)?
        .checked_add(queue_container_charge(new_capacity)?)
        .and_then(|bytes| bytes.checked_add(reservations))
        .and_then(|bytes| bytes.checked_add(retained_queue_bytes))
        .ok_or_else(|| {
            internal_failure(
                ViewPhase::Planning,
                "overlapping request queue byte charge overflowed",
            )
        })
}

fn queue_container_charge(capacity: usize) -> SceneResult<u64> {
    let capacity = u64::try_from(capacity)
        .map_err(|_| internal_failure(ViewPhase::Planning, "request queue capacity overflowed"))?;
    let node_bytes = u64::try_from(mem::size_of::<NodeKey>()).map_err(|_| {
        internal_failure(
            ViewPhase::Planning,
            "request queue node size does not fit u64",
        )
    })?;
    let container_bytes = u64::try_from(mem::size_of::<VecDeque<NodeKey>>()).map_err(|_| {
        internal_failure(
            ViewPhase::Planning,
            "request queue container size does not fit u64",
        )
    })?;
    capacity
        .checked_mul(node_bytes)
        .and_then(|bytes| bytes.checked_add(container_bytes))
        .ok_or_else(|| {
            internal_failure(ViewPhase::Planning, "request queue byte charge overflowed")
        })
}

fn node_key(id: IndexNodeId) -> SceneResult<NodeKey> {
    Ok(NodeKey::new(id.get())?)
}

fn hierarchy_charge(node_capacity: usize) -> SceneResult<u64> {
    let node_count = u64::try_from(node_capacity).map_err(|_| {
        internal_failure(
            ViewPhase::Hierarchy,
            "application hierarchy capacity overflowed",
        )
    })?;
    node_count
        .checked_mul(HIERARCHY_WORKING_BYTES_PER_NODE)
        .and_then(|bytes| bytes.checked_add(HIERARCHY_FIXED_WORKING_BYTES))
        .ok_or_else(|| {
            internal_failure(
                ViewPhase::Hierarchy,
                "application hierarchy byte charge overflowed",
            )
        })
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
        next.is_finite().then_some(next).ok_or_else(|| {
            internal_failure(
                ViewPhase::Hierarchy,
                "Source bounds are too large for a finite camera frame",
            )
        })
    })?;
    let radius = (squared_diagonal.sqrt() * 1.25).max(1.0);
    if radius * 8.0 > f64::from(f32::MAX) {
        return Err(internal_failure(
            ViewPhase::Hierarchy,
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
        return Err(internal_failure(
            ViewPhase::HostStaging,
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
    let point_count = u64::try_from(render_points).map_err(|_| {
        internal_failure(
            ViewPhase::HostStaging,
            "staged renderer point count overflowed",
        )
    })?;
    let render_point_bytes = u64::try_from(mem::size_of::<RenderPoint>()).map_err(|_| {
        internal_failure(
            ViewPhase::HostStaging,
            "renderer point size does not fit u64",
        )
    })?;
    let container_bytes = u64::try_from(mem::size_of::<Container>()).map_err(|_| {
        internal_failure(
            ViewPhase::HostStaging,
            "staging container size does not fit u64",
        )
    })?;
    point_count
        .checked_mul(render_point_bytes)
        .and_then(|bytes| bytes.checked_add(container_bytes))
        .and_then(|bytes| bytes.checked_add(current_index_bytes))
        .ok_or_else(|| {
            internal_failure(
                ViewPhase::HostStaging,
                "node staging byte charge overflowed",
            )
        })
}

fn validate_summary(
    summary: &IndexReadSummary,
    node: RealNode,
    available: AvailableNode,
    source: render_protocol::SourceId,
    observed_points: u64,
    expected_display_contract: Option<DisplaySampleContract>,
) -> SceneResult<()> {
    if summary.node() != node.index_id
        || summary.source() != source
        || summary.emitted_point_count() != observed_points
        || summary.emitted_point_count() != available.point_count()
        || summary.coverage() != node.coverage
        || summary.covered_source_point_count() != node.covered_source_point_count
        || summary.display_sample_contract() != expected_display_contract
    {
        return Err(internal_failure(
            ViewPhase::NodeRead,
            "index terminal summary did not match the staged node batch",
        ));
    }
    Ok(())
}

fn validate_display_contract(
    mode: DisplayMode,
    contract: Option<DisplaySampleContract>,
    exact_review: bool,
) -> SceneResult<()> {
    match (mode, contract) {
        (DisplayMode::Neutral | DisplayMode::Elevation, None)
        | (DisplayMode::Intensity | DisplayMode::Classification, Some(_)) => Ok(()),
        (DisplayMode::Rgb, Some(contract)) if contract.rgb().is_some() => Ok(()),
        (DisplayMode::Rgb, Some(_)) => Err(Box::new(ViewFailure::invalid_request(
            "RGB display is unavailable because the verified Source lacks all three U16 RGB Attributes",
        ))),
        (DisplayMode::Rgb | DisplayMode::Intensity | DisplayMode::Classification, None) => {
            Err(Box::new(ViewFailure::invalid_request(
                "the selected attributed display requires a v2 inspection index",
            )))
        }
        (DisplayMode::Neutral | DisplayMode::Elevation, Some(_)) if exact_review => Ok(()),
        (DisplayMode::Neutral | DisplayMode::Elevation, Some(_)) => {
            Err(Box::new(ViewFailure::invalid_request(
                "neutral and elevation displays require the position-only v1 index recipe",
            )))
        }
    }
}

fn resource_limit(
    phase: ViewPhase,
    resource: &'static str,
    limit: u64,
) -> Box<dyn std::error::Error> {
    Box::new(ViewFailure::resource(
        phase,
        format_args!("{resource} exceeded the application limit of {limit}"),
    ))
}

fn allocation_failure(
    phase: ViewPhase,
    message: impl std::fmt::Display,
) -> Box<dyn std::error::Error> {
    Box::new(ViewFailure::resource(phase, message))
}

fn index_read_failure(error: &IndexError) -> Box<dyn std::error::Error> {
    Box::new(ViewFailure::index_read(error))
}

fn internal_failure(
    phase: ViewPhase,
    message: impl std::fmt::Display,
) -> Box<dyn std::error::Error> {
    Box::new(ViewFailure::internal(phase, message))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs, io,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
    };

    use point_contracts::{
        AttributeColumn, AttributeColumns, AttributeDataType, AttributeDefinition, AttributeId,
        AttributeValues, CoordinateReference, PositionTransform,
    };
    use point_index::PrepareLimits;
    use point_view::{AvailableNodes, PlanningBudget, ViewPlan, ViewPlanner};
    use render_protocol::{Camera, RenderLimits, RenderStateModel, RenderUpdate, Viewport};
    use source_memory::MemorySource;

    use crate::{
        diagnostic::{RecoveryAction, ViewFailureCode},
        orbit_camera::OrbitCamera,
        synthetic::{RESIDENT_BATCH_BUDGET, RESIDENT_BYTE_BUDGET, RESIDENT_POINT_BUDGET},
    };
    use renderer_demo::display::{DisplayMode, NEUTRAL_COLOR};

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
    fn first_coarse_batch_supplies_stable_highlight_identities() {
        let directory = TestDirectory::new().unwrap();
        let mut scene = fixture_scene(directory.path()).unwrap();

        let batch = materialize_first_batch(&mut scene);
        let displayed = batch
            .points()
            .iter()
            .map(RenderPoint::point_id)
            .collect::<BTreeSet<_>>();
        let highlights = scene.highlight_ids();

        assert!(!highlights.is_empty());
        assert!(highlights.len() <= HIGHLIGHT_ID_COUNT);
        assert!(
            highlights
                .iter()
                .all(|point_id| displayed.contains(point_id))
        );
        assert_eq!(highlights, scene.highlight_ids());
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
    fn queue_admission_defers_after_the_priority_prefix_fills_host_budget() {
        let first = NodeKey::new(1).unwrap();
        let second = NodeKey::new(2).unwrap();
        let node_reservation = 1_024;
        let mut pending = VecDeque::new();
        let mut reserved_host_bytes = 0;

        assert!(
            admit_queued_node(
                &mut pending,
                first,
                node_reservation,
                &mut reserved_host_bytes,
                0,
                QueueBudget {
                    max_nodes: 2,
                    max_host_bytes: u64::MAX,
                },
            )
            .unwrap()
        );
        let one_node_bytes = queue_charge(pending.capacity(), reserved_host_bytes, 0).unwrap();

        assert!(
            !admit_queued_node(
                &mut pending,
                second,
                node_reservation,
                &mut reserved_host_bytes,
                0,
                QueueBudget {
                    max_nodes: 2,
                    max_host_bytes: one_node_bytes,
                },
            )
            .unwrap()
        );
        assert_eq!(pending, VecDeque::from([first]));
        assert_eq!(reserved_host_bytes, node_reservation);
    }

    #[test]
    fn queue_growth_is_preflighted_before_reserving_a_larger_container() {
        let mut pending = VecDeque::with_capacity(1);
        for raw in 1..=pending.capacity() {
            pending.push_back(NodeKey::new(u64::try_from(raw).unwrap()).unwrap());
        }
        let original = pending.clone();
        let original_capacity = pending.capacity();
        let mut reserved_host_bytes = 0;
        let allowed = queue_charge(original_capacity, reserved_host_bytes, 0).unwrap();

        assert!(
            !admit_queued_node(
                &mut pending,
                NodeKey::new(u64::try_from(original_capacity + 1).unwrap()).unwrap(),
                0,
                &mut reserved_host_bytes,
                0,
                QueueBudget {
                    max_nodes: u64::try_from(original_capacity + 1).unwrap(),
                    max_host_bytes: allowed,
                },
            )
            .unwrap()
        );
        assert_eq!(pending, original);
        assert_eq!(pending.capacity(), original_capacity);
        assert_eq!(reserved_host_bytes, 0);
    }

    #[test]
    fn request_reconciliation_charges_the_old_and_rebuilt_queues_together() {
        let directory = TestDirectory::new().unwrap();
        let mut scene = fixture_scene(directory.path()).unwrap();
        let root = scene.planning_nodes[0];
        scene.planning_nodes[0] = root.with_status(NodeStatus::Requested);
        scene.pending = VecDeque::with_capacity(8);
        scene.pending.push_back(root.key());
        let original_pending = scene.pending.clone();
        let original_status = scene.planning_nodes[0].status();
        let old_queue_bytes = queue_container_charge(scene.pending.capacity()).unwrap();

        let error = scene
            .reconcile_requests_with_budget(
                &[root.key()],
                &[],
                QueueBudget {
                    max_nodes: 1,
                    max_host_bytes: old_queue_bytes,
                },
            )
            .expect_err("the retained old queue leaves no budget for the rebuilt queue");
        let failure = error.downcast_ref::<ViewFailure>().unwrap();
        assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
        assert_eq!(failure.phase(), ViewPhase::Planning);
        assert_eq!(scene.pending, original_pending);
        assert_eq!(scene.planning_nodes[0].status(), original_status);
    }

    #[test]
    fn allocation_failures_keep_resource_code_action_and_owning_phase() {
        for phase in [
            ViewPhase::Hierarchy,
            ViewPhase::Planning,
            ViewPhase::HostStaging,
        ] {
            let failure = allocation_failure(phase, "injected allocation failure")
                .downcast::<ViewFailure>()
                .unwrap();
            assert_eq!(failure.code(), ViewFailureCode::ResourceLimit);
            assert_eq!(failure.phase(), phase);
            assert_eq!(failure.action(), RecoveryAction::RaiseNamedLimit);
        }
    }

    #[test]
    fn retained_requests_follow_the_current_planner_priority() {
        let directory = TestDirectory::new().unwrap();
        let mut scene = fixture_scene(directory.path()).unwrap();
        let first = scene.planning_nodes[0];
        let second_key = NodeKey::new(2).unwrap();
        let second = AvailableNode::new(
            second_key,
            Some(first.key()),
            first.bounds(),
            first.geometric_error(),
            first.point_count(),
            first.estimated_bytes(),
            BatchKey::new(second_key.get()),
            NodeStatus::Requested,
        )
        .unwrap();
        scene.planning_nodes[0] = first.with_status(NodeStatus::Requested);
        scene.planning_nodes.push(second);
        scene.nodes.push(RealNode {
            index_id: IndexNodeId::new(second_key.get()).unwrap(),
            coverage: scene.nodes[0].coverage,
            covered_source_point_count: scene.nodes[0].covered_source_point_count,
            latest_issued_version: 0,
        });
        scene.pending = VecDeque::from([first.key(), second_key]);

        scene
            .reconcile_requests_with_budget(
                &[second_key, first.key()],
                &[],
                QueueBudget {
                    max_nodes: 1,
                    max_host_bytes: u64::MAX,
                },
            )
            .unwrap();

        assert_eq!(scene.pending, VecDeque::from([second_key]));
        assert_eq!(scene.planning_nodes[0].status(), NodeStatus::Missing);
        assert_eq!(scene.planning_nodes[1].status(), NodeStatus::Requested);
    }

    #[test]
    fn bridge_rejects_an_oversized_advertised_node_before_staging() {
        let directory = TestDirectory::new().unwrap();
        let mut scene = fixture_scene(directory.path()).unwrap();
        let original_metrics = scene.metrics();
        let oversized = requested_available(
            scene.planning_nodes[0],
            STAGING_POINT_BUDGET.saturating_add(1),
        );
        queue_available(&mut scene, 0, oversized);

        let error = scene.next_batch().unwrap_err();

        assert!(error.to_string().contains("node staging Points exceeded"));
        assert_eq!(scene.planning_nodes[0].status(), NodeStatus::Missing);
        assert_eq!(scene.nodes[0].latest_issued_version, 0);
        assert_eq!(scene.metrics(), original_metrics);
    }

    #[test]
    fn bridge_rejects_terminal_summary_count_and_coverage_mismatches() {
        let directory = TestDirectory::new().unwrap();

        let mut count_mismatch = fixture_scene(directory.path()).unwrap();
        let original_metrics = count_mismatch.metrics();
        let advertised = requested_available(
            count_mismatch.planning_nodes[0],
            count_mismatch.planning_nodes[0]
                .point_count()
                .saturating_add(1),
        );
        queue_available(&mut count_mismatch, 0, advertised);
        let error = count_mismatch.next_batch().unwrap_err();
        assert!(error.to_string().contains("terminal summary did not match"));
        assert_eq!(count_mismatch.metrics(), original_metrics);
        assert_eq!(
            count_mismatch.planning_nodes[0].status(),
            NodeStatus::Missing
        );

        let mut coverage_mismatch = fixture_scene(directory.path()).unwrap();
        assert_eq!(
            coverage_mismatch.nodes[0].coverage,
            DisplayCoverage::Complete
        );
        coverage_mismatch.nodes[0].coverage = DisplayCoverage::Sampled;
        let original_metrics = coverage_mismatch.metrics();
        let advertised = requested_available(
            coverage_mismatch.planning_nodes[0],
            coverage_mismatch.planning_nodes[0].point_count(),
        );
        queue_available(&mut coverage_mismatch, 0, advertised);
        let error = coverage_mismatch.next_batch().unwrap_err();
        assert!(error.to_string().contains("terminal summary did not match"));
        assert_eq!(coverage_mismatch.metrics(), original_metrics);
        assert_eq!(
            coverage_mismatch.planning_nodes[0].status(),
            NodeStatus::Missing
        );
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
        let peak_queued_host_bytes = scene.metrics().peak_queued_host_bytes;
        assert!(peak_queued_host_bytes > 0);
        assert!(peak_queued_host_bytes <= QUEUED_HOST_BYTE_BUDGET);

        let hidden = hidden_camera(&scene);
        let changed = plan(&mut planner, &scene, &hidden);
        assert!(changed.demanded_nodes().is_empty());
        scene
            .reconcile_requests(changed.demanded_nodes(), changed.requests())
            .unwrap();

        assert!(scene.pending.is_empty());
        assert_eq!(scene.planning_nodes[0].status(), NodeStatus::Missing);
        assert_eq!(scene.metrics().cancelled_requests, 1);
        assert_eq!(
            scene.metrics().peak_queued_host_bytes,
            peak_queued_host_bytes
        );
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
            assert_eq!(point.color(), NEUTRAL_COLOR);
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

    #[test]
    fn elevation_display_colors_index_samples_from_complete_source_z_bounds() {
        let neutral_directory = TestDirectory::new().unwrap();
        let elevation_directory = TestDirectory::new().unwrap();
        let mut neutral = fixture_scene(neutral_directory.path()).unwrap();
        let mut elevation =
            fixture_scene_with_mode(elevation_directory.path(), DisplayMode::Elevation).unwrap();

        let neutral_batch = materialize_first_batch(&mut neutral);
        let elevation_batch = materialize_first_batch(&mut elevation);
        assert_eq!(
            neutral_batch.world_origin().map(f64::to_bits),
            elevation_batch.world_origin().map(f64::to_bits)
        );
        assert_eq!(neutral_batch.point_count(), elevation_batch.point_count());
        for (neutral, elevation) in neutral_batch.points().iter().zip(elevation_batch.points()) {
            assert_eq!(neutral.point_id(), elevation.point_id());
            assert_eq!(
                neutral.relative_position().map(f32::to_bits),
                elevation.relative_position().map(f32::to_bits)
            );
            assert_eq!(neutral.color(), NEUTRAL_COLOR);
        }

        let colors = elevation_batch
            .points()
            .iter()
            .map(render_protocol::RenderPoint::color)
            .collect::<Vec<_>>();

        assert_eq!(
            colors,
            vec![
                [68, 1, 84, 255],
                [50, 103, 139, 255],
                [74, 182, 112, 255],
                [253, 231, 37, 255],
            ]
        );
    }

    #[test]
    fn attributed_modes_change_only_exact_display_color() {
        let mut batches = Vec::new();
        for mode in [
            DisplayMode::Rgb,
            DisplayMode::Intensity,
            DisplayMode::Classification,
        ] {
            let directory = TestDirectory::new().unwrap();
            let mut scene = attributed_fixture_scene(directory.path(), mode).unwrap();
            batches.push(materialize_first_batch(&mut scene));
        }

        for candidate in &batches[1..] {
            assert_eq!(
                batches[0].world_origin().map(f64::to_bits),
                candidate.world_origin().map(f64::to_bits)
            );
            for (expected, actual) in batches[0].points().iter().zip(candidate.points()) {
                assert_eq!(expected.point_id(), actual.point_id());
                assert_eq!(
                    expected.relative_position().map(f32::to_bits),
                    actual.relative_position().map(f32::to_bits)
                );
            }
        }
        assert_eq!(batches[0].points()[0].color(), [0, 128, 255, 255]);
        assert_eq!(batches[1].points()[0].color(), [0, 0, 0, 255]);
        assert_eq!(batches[1].points()[1].color(), [128, 128, 128, 255]);
        assert_eq!(batches[2].points()[0].color(), [139, 95, 57, 255]);
        assert_eq!(batches[2].points()[1].color(), [220, 70, 70, 255]);
    }

    fn materialize_first_batch(scene: &mut RealCloudScene) -> PointBatch {
        let visible = visible_camera(scene);
        let plan = plan(&mut ViewPlanner::default(), scene, &visible);
        scene
            .reconcile_requests(plan.demanded_nodes(), plan.requests())
            .unwrap();
        scene.next_batch().unwrap().unwrap()
    }

    fn fixture_scene(directory: &Path) -> SceneResult<RealCloudScene> {
        fixture_scene_with_mode(directory, DisplayMode::Neutral)
    }

    fn fixture_scene_with_mode(
        directory: &Path,
        display_mode: DisplayMode,
    ) -> SceneResult<RealCloudScene> {
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
        RealCloudScene::new(TEST_GENERATION, index, display_mode)
    }

    fn attributed_fixture_scene(
        directory: &Path,
        display_mode: DisplayMode,
    ) -> SceneResult<RealCloudScene> {
        let ticks = vec![[0, 0, 0], [1, 2, 3], [2, 4, 6], [3, 6, 9]];
        let point_count = ticks.len();
        let column = |id, name, data_type, values| {
            AttributeColumn::new(
                AttributeDefinition::new(AttributeId::new(id).unwrap(), name, data_type).unwrap(),
                values,
            )
            .unwrap()
        };
        let attributes = AttributeColumns::new(
            vec![
                column(
                    1,
                    "intensity",
                    AttributeDataType::U16,
                    AttributeValues::u16(vec![0, 32_768, u16::MAX, 1_000]),
                ),
                column(
                    6,
                    "classification",
                    AttributeDataType::U8,
                    AttributeValues::u8(vec![2, 6, 18, 19]),
                ),
                column(
                    16,
                    "red",
                    AttributeDataType::U16,
                    AttributeValues::u16(vec![0, u16::MAX, 32_768, 1_000]),
                ),
                column(
                    17,
                    "green",
                    AttributeDataType::U16,
                    AttributeValues::u16(vec![32_768, 0, u16::MAX, 2_000]),
                ),
                column(
                    18,
                    "blue",
                    AttributeDataType::U16,
                    AttributeValues::u16(vec![u16::MAX, 32_768, 0, 3_000]),
                ),
            ],
            point_count,
        )?;
        let input = MemorySource::from_columns(
            PositionTransform::new([4_000_000.0, 800_000.0, 120.0], [1.0; 3])?,
            CoordinateReference::Unknown,
            ticks,
            attributes,
        )?;
        let source = source_memory::open(input).blocking_wait()?;
        let index = point_index::prepare_with_recipe(
            source,
            directory.join("fixture.inspection.pidx"),
            display_mode.index_policy().recipe(),
            PrepareLimits::default(),
        )
        .blocking_wait()?;
        RealCloudScene::new(TEST_GENERATION, index, display_mode)
    }

    fn requested_available(original: AvailableNode, point_count: u64) -> AvailableNode {
        let estimated_bytes = point_count
            .checked_mul(ESTIMATED_GPU_BYTES_PER_POINT)
            .unwrap();
        AvailableNode::new(
            original.key(),
            original.parent(),
            original.bounds(),
            original.geometric_error(),
            point_count,
            estimated_bytes,
            original.batch_key(),
            NodeStatus::Requested,
        )
        .unwrap()
    }

    fn queue_available(scene: &mut RealCloudScene, index: usize, available: AvailableNode) {
        scene.planning_nodes[index] = available;
        scene.pending.push_back(available.key());
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
