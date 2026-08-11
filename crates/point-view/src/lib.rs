//! Renderer-neutral adaptive view planning for hierarchical point clouds.
//!
//! The planner borrows host-owned node metadata, selects a visible level of
//! detail, and returns requests and generation-safe renderer retirements. It
//! performs no input/output and owns no renderer or GPU resources.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeSet;
use std::num::NonZeroU64;

use render_protocol::{BatchKey, BatchVersion, Camera, RenderUpdate, ViewGenerationKey, Viewport};
use thiserror::Error;

mod planning;

/// Stable, nonzero identity of one node in a host-owned hierarchy.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeKey(NonZeroU64);

impl NodeKey {
    /// Creates a node key.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::ZeroNodeKey`] when `value` is zero.
    pub const fn new(value: u64) -> Result<Self, PlanError> {
        match NonZeroU64::new(value) {
            Some(value) => Ok(Self(value)),
            None => Err(PlanError::ZeroNodeKey),
        }
    }

    /// Returns the host-selected nonzero value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Finite world-space axis-aligned bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisAlignedBox {
    min: [f64; 3],
    max: [f64; 3],
}

impl AxisAlignedBox {
    /// Creates bounds whose minimum does not exceed its maximum on any axis.
    ///
    /// Zero-volume bounds are valid.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::InvalidBounds`] for the first non-finite or
    /// reversed axis.
    pub fn new(min: [f64; 3], max: [f64; 3]) -> Result<Self, PlanError> {
        for axis in 0..3 {
            if !min[axis].is_finite() || !max[axis].is_finite() || min[axis] > max[axis] {
                return Err(PlanError::InvalidBounds { axis });
            }
        }
        Ok(Self { min, max })
    }

    /// Returns the inclusive minimum corner.
    #[must_use]
    pub const fn min(self) -> [f64; 3] {
        self.min
    }

    /// Returns the inclusive maximum corner.
    #[must_use]
    pub const fn max(self) -> [f64; 3] {
        self.max
    }
}

/// Host-observed loading or residency state of one node batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeStatus {
    /// No request is in flight and no batch is resident.
    Missing,
    /// A host request is already in flight.
    Requested,
    /// The batch is resident at the given renderer version.
    Resident {
        /// Exact resident version used by conditional retirement.
        version: BatchVersion,
    },
}

/// Immutable planning metadata for one hierarchy node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AvailableNode {
    key: NodeKey,
    parent: Option<NodeKey>,
    bounds: AxisAlignedBox,
    geometric_error: f64,
    point_count: u64,
    estimated_bytes: u64,
    batch_key: BatchKey,
    status: NodeStatus,
}

impl AvailableNode {
    /// Creates validated metadata for one non-empty batch.
    ///
    /// # Errors
    ///
    /// Returns an error for a self-parent, a non-finite or negative geometric
    /// error, or a zero point/byte cost.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: NodeKey,
        parent: Option<NodeKey>,
        bounds: AxisAlignedBox,
        geometric_error: f64,
        point_count: u64,
        estimated_bytes: u64,
        batch_key: BatchKey,
        status: NodeStatus,
    ) -> Result<Self, PlanError> {
        if parent == Some(key) {
            return Err(PlanError::SelfParent { key });
        }
        if !geometric_error.is_finite() || geometric_error < 0.0 {
            return Err(PlanError::InvalidGeometricError { key });
        }
        if point_count == 0 {
            return Err(PlanError::ZeroPointCount { key });
        }
        if estimated_bytes == 0 {
            return Err(PlanError::ZeroEstimatedBytes { key });
        }

        Ok(Self {
            key,
            parent,
            bounds,
            geometric_error,
            point_count,
            estimated_bytes,
            batch_key,
            status,
        })
    }

    /// Returns the stable node identity.
    #[must_use]
    pub const fn key(self) -> NodeKey {
        self.key
    }

    /// Returns the parent identity, or `None` for a root.
    #[must_use]
    pub const fn parent(self) -> Option<NodeKey> {
        self.parent
    }

    /// Returns world-space bounds.
    #[must_use]
    pub const fn bounds(self) -> AxisAlignedBox {
        self.bounds
    }

    /// Returns world-space geometric error.
    #[must_use]
    pub const fn geometric_error(self) -> f64 {
        self.geometric_error
    }

    /// Returns the logical point cost.
    #[must_use]
    pub const fn point_count(self) -> u64 {
        self.point_count
    }

    /// Returns the host's estimated resident-byte cost.
    #[must_use]
    pub const fn estimated_bytes(self) -> u64 {
        self.estimated_bytes
    }

    /// Returns the renderer batch identity.
    #[must_use]
    pub const fn batch_key(self) -> BatchKey {
        self.batch_key
    }

    /// Returns current host-observed state.
    #[must_use]
    pub const fn status(self) -> NodeStatus {
        self.status
    }

    /// Returns this metadata with updated host-observed state.
    #[must_use]
    pub const fn with_status(mut self, status: NodeStatus) -> Self {
        self.status = status;
        self
    }
}

/// Generation-stamped borrowed node metadata supplied by the host.
#[derive(Clone, Copy, Debug)]
pub struct AvailableNodes<'nodes> {
    view_generation: ViewGenerationKey,
    nodes: &'nodes [AvailableNode],
}

impl<'nodes> AvailableNodes<'nodes> {
    /// Borrows one complete hierarchy snapshot for a view generation.
    #[must_use]
    pub const fn new(view_generation: ViewGenerationKey, nodes: &'nodes [AvailableNode]) -> Self {
        Self {
            view_generation,
            nodes,
        }
    }

    /// Returns the generation shared by all node state in this snapshot.
    #[must_use]
    pub const fn view_generation(self) -> ViewGenerationKey {
        self.view_generation
    }

    /// Returns the host-owned node slice.
    #[must_use]
    pub const fn nodes(self) -> &'nodes [AvailableNode] {
        self.nodes
    }
}

/// Screen-error thresholds controlling level-of-detail transitions.
///
/// The default configuration centers transitions at `2.0` physical pixels
/// with `0.25` pixels of hysteresis. Previously coarse nodes therefore refine
/// above `2.25` pixels, while previously refined nodes coarsen below `1.75`
/// pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlannerConfig {
    max_error_pixels: f64,
    hysteresis_pixels: f64,
}

impl PlannerConfig {
    /// Creates a configuration with symmetric pixel hysteresis.
    ///
    /// Previously coarse nodes refine above `max_error_pixels +
    /// hysteresis_pixels`. Previously refined nodes coarsen below
    /// `max_error_pixels - hysteresis_pixels`.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError::InvalidPlannerConfig`] unless the maximum is
    /// finite and positive and hysteresis is finite in `[0, maximum)`.
    pub fn new(max_error_pixels: f64, hysteresis_pixels: f64) -> Result<Self, PlanError> {
        if !max_error_pixels.is_finite()
            || max_error_pixels <= 0.0
            || !hysteresis_pixels.is_finite()
            || hysteresis_pixels < 0.0
            || hysteresis_pixels >= max_error_pixels
        {
            return Err(PlanError::InvalidPlannerConfig);
        }
        Ok(Self {
            max_error_pixels,
            hysteresis_pixels,
        })
    }

    /// Returns the center level-of-detail error threshold.
    #[must_use]
    pub const fn max_error_pixels(self) -> f64 {
        self.max_error_pixels
    }

    /// Returns the symmetric hysteresis width in pixels.
    #[must_use]
    pub const fn hysteresis_pixels(self) -> f64 {
        self.hysteresis_pixels
    }
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            max_error_pixels: 2.0,
            hysteresis_pixels: 0.25,
        }
    }
}

/// Hard planning limits for points, estimated bytes, and batches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanningBudget {
    points: u64,
    estimated_bytes: u64,
    batches: u64,
}

impl PlanningBudget {
    /// Creates caller-selected limits. Zero permits no new resources.
    #[must_use]
    pub const fn new(max_points: u64, max_estimated_bytes: u64, max_batches: u64) -> Self {
        Self {
            points: max_points,
            estimated_bytes: max_estimated_bytes,
            batches: max_batches,
        }
    }

    /// Returns the maximum planned point count.
    #[must_use]
    pub const fn max_points(self) -> u64 {
        self.points
    }

    /// Returns the maximum planned estimated-byte count.
    #[must_use]
    pub const fn max_estimated_bytes(self) -> u64 {
        self.estimated_bytes
    }

    /// Returns the maximum planned batch count.
    #[must_use]
    pub const fn max_batches(self) -> u64 {
        self.batches
    }
}

/// Resource accounting for retained, requested, and newly requested batches.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceUsage {
    point_count: u64,
    estimated_bytes: u64,
    batch_count: u64,
}

impl ResourceUsage {
    /// Returns planned points.
    #[must_use]
    pub const fn point_count(self) -> u64 {
        self.point_count
    }

    /// Returns planned estimated bytes.
    #[must_use]
    pub const fn estimated_bytes(self) -> u64 {
        self.estimated_bytes
    }

    /// Returns planned batches.
    #[must_use]
    pub const fn batch_count(self) -> u64 {
        self.batch_count
    }

    /// Reports whether this usage fits all limits.
    #[must_use]
    pub const fn fits_within(self, budget: PlanningBudget) -> bool {
        self.point_count <= budget.points
            && self.estimated_bytes <= budget.estimated_bytes
            && self.batch_count <= budget.batches
    }
}

/// One prioritized host loading request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeRequest {
    view_generation: ViewGenerationKey,
    node_key: NodeKey,
    batch_key: BatchKey,
    point_count: u64,
    estimated_bytes: u64,
    screen_space_error_pixels: f64,
}

impl NodeRequest {
    /// Returns the view generation for which this request is useful.
    #[must_use]
    pub const fn view_generation(self) -> ViewGenerationKey {
        self.view_generation
    }

    /// Returns the requested hierarchy node.
    #[must_use]
    pub const fn node(self) -> NodeKey {
        self.node_key
    }

    /// Returns the batch identity the host should publish.
    #[must_use]
    pub const fn batch_key(self) -> BatchKey {
        self.batch_key
    }

    /// Returns the request's logical point cost.
    #[must_use]
    pub const fn point_count(self) -> u64 {
        self.point_count
    }

    /// Returns the request's estimated resident-byte cost.
    #[must_use]
    pub const fn estimated_bytes(self) -> u64 {
        self.estimated_bytes
    }

    /// Returns projected error used for descending request priority.
    #[must_use]
    pub const fn screen_space_error_pixels(self) -> f64 {
        self.screen_space_error_pixels
    }
}

/// One resident node that must remain available for visible coverage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetainedNode {
    view_generation: ViewGenerationKey,
    node_key: NodeKey,
    batch_key: BatchKey,
    version: BatchVersion,
}

impl RetainedNode {
    /// Returns the view generation of the resident observation.
    #[must_use]
    pub const fn view_generation(self) -> ViewGenerationKey {
        self.view_generation
    }

    /// Returns the retained hierarchy node.
    #[must_use]
    pub const fn node_key(self) -> NodeKey {
        self.node_key
    }

    /// Returns the retained renderer batch.
    #[must_use]
    pub const fn batch_key(self) -> BatchKey {
        self.batch_key
    }

    /// Returns the exact resident version.
    #[must_use]
    pub const fn version(self) -> BatchVersion {
        self.version
    }
}

/// Generation-safe conditional removal of one resident batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Retirement {
    view_generation: ViewGenerationKey,
    batch_key: BatchKey,
    expected_version: BatchVersion,
}

impl Retirement {
    /// Returns the exact view generation observed during planning.
    #[must_use]
    pub const fn view_generation(self) -> ViewGenerationKey {
        self.view_generation
    }

    /// Returns the resident batch to remove.
    #[must_use]
    pub const fn batch_key(self) -> BatchKey {
        self.batch_key
    }

    /// Returns the exact version that must still be resident.
    #[must_use]
    pub const fn expected_version(self) -> BatchVersion {
        self.expected_version
    }

    /// Returns the conditional renderer update represented by this token.
    #[must_use]
    pub const fn render_update(self) -> RenderUpdate {
        RenderUpdate::Remove {
            view_generation: self.view_generation,
            key: self.batch_key,
            expected_version: self.expected_version,
        }
    }
}

/// Deterministic loading, retention, and retirement decisions for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewPlan {
    view_generation: ViewGenerationKey,
    demanded_nodes: Vec<NodeKey>,
    requests: Vec<NodeRequest>,
    retained: Vec<RetainedNode>,
    retirements: Vec<Retirement>,
    resource_usage: ResourceUsage,
}

impl ViewPlan {
    /// Returns the planned view generation.
    #[must_use]
    pub const fn view_generation(&self) -> ViewGenerationKey {
        self.view_generation
    }

    /// Returns every desired nonresident target by descending visual priority,
    /// then node key.
    ///
    /// This includes both missing nodes and nodes whose host work is already
    /// requested. Unlike [`Self::requests`], it describes current demand rather
    /// than only newly required loading, so a host can cancel requested work
    /// that is absent after a camera or viewport change.
    #[must_use]
    pub fn demanded_nodes(&self) -> &[NodeKey] {
        &self.demanded_nodes
    }

    /// Returns missing targets by descending projected error, then node key.
    #[must_use]
    pub fn requests(&self) -> &[NodeRequest] {
        &self.requests
    }

    /// Returns required resident coverage in ascending node-key order.
    #[must_use]
    pub fn retained_nodes(&self) -> &[RetainedNode] {
        &self.retained
    }

    /// Returns safe conditional removals in ascending batch-key order.
    #[must_use]
    pub fn retirements(&self) -> &[Retirement] {
        &self.retirements
    }

    /// Returns the conservative resource footprint used for budget decisions.
    #[must_use]
    pub const fn resource_usage(&self) -> ResourceUsage {
        self.resource_usage
    }
}

/// Stateful hysteresis around an otherwise pure planning operation.
///
/// The default planner uses [`PlannerConfig::default`].
#[derive(Clone, Debug)]
pub struct ViewPlanner {
    config: PlannerConfig,
    active_generation: Option<ViewGenerationKey>,
    refined_nodes: BTreeSet<NodeKey>,
}

impl ViewPlanner {
    /// Creates an empty planner with caller-selected thresholds.
    #[must_use]
    pub const fn new(config: PlannerConfig) -> Self {
        Self {
            config,
            active_generation: None,
            refined_nodes: BTreeSet::new(),
        }
    }

    /// Returns the current immutable configuration.
    #[must_use]
    pub const fn config(&self) -> PlannerConfig {
        self.config
    }

    /// Plans visible level of detail without performing I/O or renderer work.
    ///
    /// # Errors
    ///
    /// Returns [`PlanError`] when the supplied hierarchy snapshot is invalid or
    /// conservative resource accounting overflows.
    pub fn plan(
        &mut self,
        camera: &Camera,
        viewport: Viewport,
        available_nodes: AvailableNodes<'_>,
        budget: PlanningBudget,
    ) -> Result<ViewPlan, PlanError> {
        planning::plan(self, camera, viewport, available_nodes, budget)
    }
}

impl Default for ViewPlanner {
    fn default() -> Self {
        Self::new(PlannerConfig::default())
    }
}

/// Invalid inputs or arithmetic failures encountered during planning.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PlanError {
    /// Node key zero is reserved as an invalid sentinel.
    #[error("node keys must be nonzero")]
    ZeroNodeKey,
    /// One bounds axis was non-finite or reversed.
    #[error("axis-aligned bounds axis {axis} is non-finite or reversed")]
    InvalidBounds {
        /// Zero-based coordinate axis.
        axis: usize,
    },
    /// A node named itself as its parent.
    #[error("node {key:?} cannot be its own parent")]
    SelfParent {
        /// Invalid node.
        key: NodeKey,
    },
    /// Geometric error was negative, NaN, or infinite.
    #[error("node {key:?} geometric error must be finite and nonnegative")]
    InvalidGeometricError {
        /// Invalid node.
        key: NodeKey,
    },
    /// A node's batch contained no logical points.
    #[error("node {key:?} point count must be nonzero")]
    ZeroPointCount {
        /// Invalid node.
        key: NodeKey,
    },
    /// A node's estimated resident-byte cost was zero.
    #[error("node {key:?} estimated byte cost must be nonzero")]
    ZeroEstimatedBytes {
        /// Invalid node.
        key: NodeKey,
    },
    /// The configured maximum or hysteresis was invalid.
    #[error(
        "maximum error must be finite and positive and hysteresis must be finite in [0, maximum)"
    )]
    InvalidPlannerConfig,
    /// Two entries used the same hierarchy node key.
    #[error("node key {key:?} appears more than once")]
    DuplicateNodeKey {
        /// Duplicate identity.
        key: NodeKey,
    },
    /// Two entries used the same renderer batch key.
    #[error("batch key {key:?} appears more than once")]
    DuplicateBatchKey {
        /// Duplicate identity.
        key: BatchKey,
    },
    /// A parent link referred outside the supplied snapshot.
    #[error("node {key:?} refers to missing parent {parent:?}")]
    MissingParent {
        /// Child node.
        key: NodeKey,
        /// Missing parent.
        parent: NodeKey,
    },
    /// Parent links contained a cycle.
    #[error("parent links contain a cycle through node {key:?}")]
    ParentCycle {
        /// Stable representative of the cycle.
        key: NodeKey,
    },
    /// A child's bounds escaped its parent's inclusive bounds.
    #[error("node {key:?} bounds are outside parent {parent:?}")]
    ChildOutsideParent {
        /// Invalid child.
        key: NodeKey,
        /// Its parent.
        parent: NodeKey,
    },
    /// Point, byte, or batch accounting exceeded `u64`.
    #[error("planning resource usage exceeds the supported integer range")]
    ResourceUsageOverflow,
}
