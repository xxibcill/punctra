use std::{mem, sync::Arc};

use foundation_runtime::CancellationToken;
use point_contracts::{PositionTransform, SourceId, WorldBounds};
use point_source::{Source, SourceSpan};

use crate::{
    CandidateLimits, IndexError, IndexLimit, NodeReadBudget,
    persistence::ArtifactReader,
    read::{self, IndexPointBatches},
};

/// Persisted index recipe version implemented by this crate.
pub const RECIPE_VERSION: u32 = 1;

/// Persisted complete/work-file schema version implemented by this crate.
pub const DISK_VERSION: u32 = 1;

const CANDIDATE_CANCELLATION_CADENCE: u64 = 1_024;

/// Stable nonzero identity of one hierarchy node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IndexNodeId(std::num::NonZeroU64);

impl IndexNodeId {
    /// Creates a node identity.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::ZeroNodeId`] for the reserved zero value.
    pub const fn new(value: u64) -> Result<Self, IndexError> {
        match std::num::NonZeroU64::new(value) {
            Some(value) => Ok(Self(value)),
            None => Err(IndexError::ZeroNodeId),
        }
    }

    /// Returns the nonzero persisted value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Whether a node's display values cover every Point in its Source span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayCoverage {
    /// A deterministic sparse display sample; not an exact Query result.
    Sampled,
    /// Every Point in a contiguous Source leaf is emitted.
    Complete,
}

impl DisplayCoverage {
    /// Reports whether display values cover every Source Point in the node.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// Immutable planning facts for one hierarchy node.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexNode {
    pub(crate) id: IndexNodeId,
    pub(crate) parent: Option<IndexNodeId>,
    pub(crate) bounds: WorldBounds,
    pub(crate) covered_point_count: u64,
    pub(crate) display_point_count: u64,
    pub(crate) geometric_error: f64,
    pub(crate) coverage: DisplayCoverage,
    pub(crate) children: Option<[IndexNodeId; 2]>,
    pub(crate) source_span: Option<SourceSpan>,
    pub(crate) sample_offset: u64,
    pub(crate) sample_checksum: [u8; 32],
}

impl IndexNode {
    /// Returns the stable node identity.
    #[must_use]
    pub const fn id(&self) -> IndexNodeId {
        self.id
    }

    /// Returns the parent identity, or `None` for the root.
    #[must_use]
    pub const fn parent(&self) -> Option<IndexNodeId> {
        self.parent
    }

    /// Returns exact inclusive world bounds.
    #[must_use]
    pub const fn bounds(&self) -> WorldBounds {
        self.bounds
    }

    /// Returns the number of Source Points covered by this node.
    #[must_use]
    pub const fn covered_point_count(&self) -> u64 {
        self.covered_point_count
    }

    /// Returns the number of display Points emitted by a complete node read.
    #[must_use]
    pub const fn display_point_count(&self) -> u64 {
        self.display_point_count
    }

    /// Returns the conservative world-space geometric error.
    #[must_use]
    pub const fn geometric_error(&self) -> f64 {
        self.geometric_error
    }

    /// Returns sampled or complete display Coverage.
    #[must_use]
    pub const fn coverage(&self) -> DisplayCoverage {
        self.coverage
    }

    /// Reports whether this node emits complete display Coverage.
    #[must_use]
    pub const fn coverage_complete(&self) -> bool {
        self.coverage.is_complete()
    }
}

/// Complete immutable hierarchy snapshot retained within the opening budget.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexHierarchy {
    nodes: Arc<Vec<IndexNode>>,
}

impl IndexHierarchy {
    pub(crate) fn new(nodes: Vec<IndexNode>) -> Self {
        Self {
            nodes: Arc::new(nodes),
        }
    }

    /// Returns the root, or `None` for an empty Source.
    #[must_use]
    pub fn root(&self) -> Option<&IndexNode> {
        self.nodes.first()
    }

    /// Returns nodes in stable root-first identity order.
    #[must_use]
    pub fn nodes(&self) -> &[IndexNode] {
        self.nodes.as_slice()
    }

    /// Returns one node when its identity belongs to this hierarchy.
    #[must_use]
    pub fn get(&self, id: IndexNodeId) -> Option<&IndexNode> {
        let index = usize::try_from(id.get().checked_sub(1)?).ok()?;
        self.nodes.get(index).filter(|node| node.id == id)
    }

    pub(crate) fn estimated_resident_bytes(&self) -> u64 {
        let count = u64::try_from(self.nodes.capacity()).unwrap_or(u64::MAX);
        count.saturating_mul(u64::try_from(mem::size_of::<IndexNode>()).unwrap_or(u64::MAX))
    }
}

/// Deterministic facts bound into a complete artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexDescriptor {
    pub(crate) source: SourceId,
    pub(crate) source_point_count: u64,
    pub(crate) position_transform: PositionTransform,
    pub(crate) world_bounds: Option<WorldBounds>,
    pub(crate) recipe_version: u32,
    pub(crate) disk_version: u32,
    pub(crate) node_count: u64,
    pub(crate) leaf_count: u64,
    pub(crate) artifact_checksum: [u8; 32],
}

impl IndexDescriptor {
    /// Returns the verified immutable Source identity.
    #[must_use]
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// Returns the complete Source Point count.
    #[must_use]
    pub const fn source_point_count(&self) -> u64 {
        self.source_point_count
    }

    /// Returns the exact Source position transform.
    #[must_use]
    pub const fn position_transform(&self) -> PositionTransform {
        self.position_transform
    }

    /// Returns complete Source bounds, or `None` for an empty Source.
    #[must_use]
    pub const fn world_bounds(&self) -> Option<WorldBounds> {
        self.world_bounds
    }

    /// Returns the deterministic construction-recipe version.
    #[must_use]
    pub const fn recipe_version(&self) -> u32 {
        self.recipe_version
    }

    /// Returns the persisted artifact schema version.
    #[must_use]
    pub const fn disk_version(&self) -> u32 {
        self.disk_version
    }

    /// Returns the complete hierarchy node count.
    #[must_use]
    pub const fn node_count(&self) -> u64 {
        self.node_count
    }

    /// Returns the fixed Source-block leaf count.
    #[must_use]
    pub const fn leaf_count(&self) -> u64 {
        self.leaf_count
    }

    /// Returns the BLAKE3 checksum covering all preceding artifact bytes.
    #[must_use]
    pub const fn artifact_checksum(&self) -> [u8; 32] {
        self.artifact_checksum
    }
}

/// How one `prepare` call obtained its complete artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrepareDisposition {
    /// Opened and verified an existing complete artifact.
    Opened,
    /// Built from Source ordinal zero.
    Built,
    /// Reused at least one durable Source-block frame and resumed.
    Resumed,
}

/// Noncanonical observational facts for one `prepare` call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrepareReport {
    pub(crate) disposition: PrepareDisposition,
    pub(crate) durable_points_reused: u64,
    pub(crate) source_points_read: u64,
    pub(crate) artifact_bytes: u64,
}

impl PrepareReport {
    /// Returns whether this call opened, built, or resumed.
    #[must_use]
    pub const fn disposition(self) -> PrepareDisposition {
        self.disposition
    }

    /// Returns Points represented by valid work frames before this call.
    #[must_use]
    pub const fn durable_points_reused(self) -> u64 {
        self.durable_points_reused
    }

    /// Returns Source Points decoded by this call.
    #[must_use]
    pub const fn source_points_read(self) -> u64 {
        self.source_points_read
    }

    /// Returns the final complete-artifact file length.
    #[must_use]
    pub const fn artifact_bytes(self) -> u64 {
        self.artifact_bytes
    }
}

/// Complete conservative candidate spans and exact plan facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidatePlan {
    spans: Box<[SourceSpan]>,
    candidate_point_count: u64,
    visited_node_count: u64,
}

impl CandidatePlan {
    /// Returns sorted, nonempty, disjoint Source spans.
    #[must_use]
    pub fn spans(&self) -> &[SourceSpan] {
        &self.spans
    }

    /// Returns the total Points in all candidate spans.
    #[must_use]
    pub const fn candidate_point_count(&self) -> u64 {
        self.candidate_point_count
    }

    /// Returns the exact hierarchy nodes visited.
    #[must_use]
    pub const fn visited_node_count(&self) -> u64 {
        self.visited_node_count
    }
}

/// Open complete index bound to its authoritative verified Source.
#[derive(Clone)]
pub struct PreparedIndex {
    pub(crate) source: Source,
    pub(crate) descriptor: IndexDescriptor,
    pub(crate) hierarchy: IndexHierarchy,
    pub(crate) prepare_report: PrepareReport,
    pub(crate) artifact: ArtifactReader,
}

impl PreparedIndex {
    /// Returns the authoritative verified Source retained by this complete index.
    ///
    /// Exact downstream operations use this handle so an index cannot be paired
    /// accidentally with a different Source instance.
    #[must_use]
    pub const fn source(&self) -> &Source {
        &self.source
    }

    /// Returns deterministic complete-artifact facts.
    #[must_use]
    pub const fn descriptor(&self) -> &IndexDescriptor {
        &self.descriptor
    }

    /// Returns the complete resident hierarchy snapshot.
    #[must_use]
    pub const fn hierarchy(&self) -> &IndexHierarchy {
        &self.hierarchy
    }

    /// Returns observational facts for the `prepare` call that produced this handle.
    #[must_use]
    pub const fn prepare_report(&self) -> &PrepareReport {
        &self.prepare_report
    }

    /// Returns conservative Source spans for one inclusive world box.
    ///
    /// # Errors
    ///
    /// Returns a resource error instead of a partial plan.
    pub fn candidates(
        &self,
        bounds: WorldBounds,
        limits: CandidateLimits,
    ) -> Result<CandidatePlan, IndexError> {
        candidates(&self.hierarchy, bounds, limits, None)
    }

    /// Returns conservative Source spans while observing cooperative cancellation.
    ///
    /// Cancellation is checked before traversal, after every bounded group of
    /// visited hierarchy nodes, and after traversal completes.
    ///
    /// # Errors
    ///
    /// Returns a cancellation or resource error instead of a partial plan.
    pub fn candidates_with_cancellation(
        &self,
        bounds: WorldBounds,
        limits: CandidateLimits,
        cancellation: &CancellationToken,
    ) -> Result<CandidatePlan, IndexError> {
        candidates(&self.hierarchy, bounds, limits, Some(cancellation))
    }

    /// Starts a bounded stream of display-only exact position samples.
    ///
    /// # Errors
    ///
    /// Returns an unknown-node, Source, artifact, or resource error.
    pub fn read_node(
        &self,
        node: IndexNodeId,
        budget: NodeReadBudget,
    ) -> Result<IndexPointBatches, IndexError> {
        let node = self
            .hierarchy
            .get(node)
            .ok_or(IndexError::UnknownNode { node: node.get() })?;
        read::start(self, node, budget)
    }
}

impl std::fmt::Debug for PreparedIndex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedIndex")
            .field("descriptor", &self.descriptor)
            .field("prepare_report", &self.prepare_report)
            .finish_non_exhaustive()
    }
}

fn candidates(
    hierarchy: &IndexHierarchy,
    request: WorldBounds,
    limits: CandidateLimits,
    cancellation: Option<&CancellationToken>,
) -> Result<CandidatePlan, IndexError> {
    check_candidate_cancellation(cancellation)?;
    let Some(root) = hierarchy.root() else {
        return Ok(CandidatePlan {
            spans: Box::new([]),
            candidate_point_count: 0,
            visited_node_count: 0,
        });
    };
    let mut stack = Vec::new();
    let mut spans = Vec::new();
    push_charged(
        &mut stack,
        root.id,
        spans.capacity(),
        limits.max_working_bytes(),
    )?;
    let mut visited = 0_u64;
    let mut candidate_points = 0_u64;
    while let Some(id) = stack.pop() {
        visited = checked_limit(
            visited,
            1,
            IndexLimit::VisitedHierarchyNodes,
            limits.max_visited_nodes(),
        )?;
        if visited.is_multiple_of(CANDIDATE_CANCELLATION_CADENCE) {
            check_candidate_cancellation(cancellation)?;
        }
        check_working_bytes(
            stack.capacity(),
            spans.capacity(),
            limits.max_working_bytes(),
        )?;
        let node = hierarchy
            .get(id)
            .expect("validated child identities resolve in the same hierarchy");
        if !intersects(node.bounds, request) {
            continue;
        }
        if let Some(span) = node.source_span {
            candidate_points = checked_limit(
                candidate_points,
                span.point_count(),
                IndexLimit::CandidatePoints,
                limits.max_candidate_points(),
            )?;
            push_span_charged(
                stack.capacity(),
                &mut spans,
                span,
                limits.max_working_bytes(),
            )?;
        } else if let Some([left, right]) = node.children {
            push_charged(
                &mut stack,
                right,
                spans.capacity(),
                limits.max_working_bytes(),
            )?;
            push_charged(
                &mut stack,
                left,
                spans.capacity(),
                limits.max_working_bytes(),
            )?;
        }
    }
    check_candidate_cancellation(cancellation)?;
    spans.sort_unstable_by_key(|span| span.first_ordinal());
    merge_adjacent(&mut spans)?;
    let output_count = u64::try_from(spans.len()).unwrap_or(u64::MAX);
    if output_count > limits.max_output_spans() {
        return Err(IndexError::ResourceLimit {
            limit: IndexLimit::CandidateSourceSpans,
            required: output_count,
            allowed: limits.max_output_spans(),
        });
    }
    check_candidate_cancellation(cancellation)?;
    Ok(CandidatePlan {
        spans: spans.into_boxed_slice(),
        candidate_point_count: candidate_points,
        visited_node_count: visited,
    })
}

fn check_candidate_cancellation(
    cancellation: Option<&CancellationToken>,
) -> Result<(), IndexError> {
    if let Some(cancellation) = cancellation {
        cancellation.check()?;
    }
    Ok(())
}

fn checked_limit(
    current: u64,
    added: u64,
    limit: IndexLimit,
    allowed: u64,
) -> Result<u64, IndexError> {
    let required = current.saturating_add(added);
    if required > allowed {
        return Err(IndexError::ResourceLimit {
            limit,
            required,
            allowed,
        });
    }
    Ok(required)
}

fn check_working_bytes(
    stack_capacity: usize,
    span_capacity: usize,
    allowed: u64,
) -> Result<(), IndexError> {
    let stack_bytes = u64::try_from(stack_capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<IndexNodeId>()).unwrap_or(u64::MAX));
    let span_bytes = u64::try_from(span_capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<SourceSpan>()).unwrap_or(u64::MAX));
    let required = stack_bytes.saturating_add(span_bytes);
    if required > allowed {
        return Err(IndexError::ResourceLimit {
            limit: IndexLimit::CandidateWorkingBytes,
            required,
            allowed,
        });
    }
    Ok(())
}

fn push_charged(
    stack: &mut Vec<IndexNodeId>,
    value: IndexNodeId,
    span_capacity: usize,
    allowed: u64,
) -> Result<(), IndexError> {
    if stack.len() == stack.capacity() {
        preflight_capacity::<IndexNodeId, SourceSpan>(stack.len() + 1, span_capacity, allowed)?;
        stack
            .try_reserve_exact(1)
            .map_err(|_| IndexError::ResourceLimit {
                limit: IndexLimit::CandidateWorkingBytes,
                required: allowed.saturating_add(1),
                allowed,
            })?;
    }
    check_working_bytes(stack.capacity(), span_capacity, allowed)?;
    stack.push(value);
    check_working_bytes(stack.capacity(), span_capacity, allowed)
}

fn push_span_charged(
    stack_capacity: usize,
    spans: &mut Vec<SourceSpan>,
    value: SourceSpan,
    allowed: u64,
) -> Result<(), IndexError> {
    if spans.len() == spans.capacity() {
        preflight_capacity::<IndexNodeId, SourceSpan>(stack_capacity, spans.len() + 1, allowed)?;
        spans
            .try_reserve_exact(1)
            .map_err(|_| IndexError::ResourceLimit {
                limit: IndexLimit::CandidateWorkingBytes,
                required: allowed.saturating_add(1),
                allowed,
            })?;
    }
    check_working_bytes(stack_capacity, spans.capacity(), allowed)?;
    spans.push(value);
    check_working_bytes(stack_capacity, spans.capacity(), allowed)
}

fn preflight_capacity<Stack, Span>(
    stack_capacity: usize,
    span_capacity: usize,
    allowed: u64,
) -> Result<(), IndexError> {
    let required = u64::try_from(stack_capacity)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(mem::size_of::<Stack>()).unwrap_or(u64::MAX))
        .saturating_add(
            u64::try_from(span_capacity)
                .unwrap_or(u64::MAX)
                .saturating_mul(u64::try_from(mem::size_of::<Span>()).unwrap_or(u64::MAX)),
        );
    if required > allowed {
        return Err(IndexError::ResourceLimit {
            limit: IndexLimit::CandidateWorkingBytes,
            required,
            allowed,
        });
    }
    Ok(())
}

fn intersects(left: WorldBounds, right: WorldBounds) -> bool {
    (0..3)
        .all(|axis| left.min()[axis] <= right.max()[axis] && right.min()[axis] <= left.max()[axis])
}

fn merge_adjacent(spans: &mut Vec<SourceSpan>) -> Result<(), IndexError> {
    if spans.len() < 2 {
        return Ok(());
    }
    let mut retained = 0;
    for read in 1..spans.len() {
        let current = spans[read];
        if spans[retained].end_ordinal() == current.first_ordinal() {
            spans[retained] = SourceSpan::new(
                spans[retained].first_ordinal(),
                spans[retained]
                    .point_count()
                    .checked_add(current.point_count())
                    .ok_or(IndexError::CorruptArtifact {
                        reason: "candidate Source span count overflowed",
                    })?,
            )?;
        } else {
            retained += 1;
            spans[retained] = current;
        }
    }
    spans.truncate(retained + 1);
    Ok(())
}
